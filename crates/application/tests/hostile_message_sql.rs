//! Message piégé contre la base : des charges d'injection SQL (et de
//! rupture du JSON stocké) dans tous les champs qu'un expéditeur ou un
//! serveur contrôle (sujet, expéditeur, destinataires, Message-ID,
//! références, mots-clés, nom de boîte, corps, nom de pièce jointe)
//! traversent toute la chaîne
//! réelle : faux serveur IMAP, `mail-parser`, domaine, SQLite sur disque,
//! fichiers ; puis dans tous les champs d'une fiche du carnet, comme filtre
//! et comme saisie d'un destinataire, et dans un fichier de contacts
//! importé (lecture, plan, import en une transaction, complément), dont les
//! étiquettes d'adresse et de téléphone d'un export Google et des adresses
//! dont la partie locale contient `'`. Elles doivent être stockées comme de
//! simples données : schéma inchangé, base intègre, aucune autre ligne touchée, valeurs relues à
//! l'identique (une fois assainies pour l'affichage).
//!
//! Sans Docker : exécuté avec les tests unitaires (`cargo nextest run`).

mod common;

use common::local;
use norelly_application::actions::{PerformAction, ReplayActions};
use norelly_application::compose::{PrepareReply, QueueMessage};
use norelly_application::sync::SyncAccount;
use norelly_domain::actions::{MessageAction, OpKind};
use norelly_domain::address::EmailAddress;
use norelly_domain::compose::ReplyKind;
use norelly_domain::compose::draft::{Draft, Recipient, RemoteCopy};
use norelly_domain::conversation::ConversationScope;
use norelly_domain::fetch_policy::FetchPolicy;
use norelly_domain::mailbox::{Flags, Keyword, header_text};
use norelly_domain::mailbox::{MailboxName, MailboxRole, RemoteMailbox, Uid, UidValidity};
use norelly_domain::ports::{
    AccountRepository, BlobStore, ConversationQuery, DraftStore, FolderTreeQuery, MailStore,
    MessageListQuery, OutboxStore, OutgoingMessage, PageSize, PendingOpStore, ReadingPreferences,
    RemoteMailboxSession, StoredBody,
};
use norelly_domain::ports::{SearchIndex, SearchRequest, SearchScope};
use norelly_domain::search::parse_query;
use norelly_render::compose::MimeComposer;
use norelly_sqlite::SqliteAccountRepository;
use norelly_sqlite::compose::SqliteComposeStore;
use norelly_sqlite::preferences::SqliteReadingPreferences;
use norelly_test_support::fake_imap::FakeImap;

/// Corps tous complets localement : aucun téléchargement attendu.
struct NoFetch;

impl norelly_domain::ports::RemoteBodyFetcher for NoFetch {
    async fn fetch(
        &self,
        _: norelly_domain::account::AccountId,
        _: &MailboxName,
        _: Uid,
        _: &norelly_domain::fetch_policy::BodyFetch,
    ) -> Result<
        Option<norelly_domain::ports::FetchedBody>,
        norelly_domain::connection::ConnectionError,
    > {
        Ok(None)
    }
}

/// Instant des actions (secondes Unix).
const NOW: i64 = 1_800_000_000;

/// Charges classiques : sortie de chaîne, commentaires, requêtes
/// empilées, sous-requêtes, `ATTACH`, rupture du JSON, octet nul.
const PAYLOADS: &[&str] = &[
    "'); DROP TABLE messages; --",
    "\" OR 1=1 --",
    "x' || (SELECT group_concat(sql) FROM sqlite_schema) || '",
    "'; ATTACH DATABASE '/tmp/pwn.db' AS pwn; CREATE TABLE pwn.t(x); --",
    "Robert'); DELETE FROM accounts; UPDATE mailboxes SET role='inbox'; --",
    "\"},{\"name\":\"injecté\",\"address\":\"pirate@example.com\"}]",
    "'; PRAGMA writable_schema = 1; --",
    "\\'; SELECT load_extension('evil'); --",
];

/// Message brut dont chaque champ porte une charge.
fn trapped_message(n: usize) -> Vec<u8> {
    let p = |i: usize| PAYLOADS[(n + i) % PAYLOADS.len()];
    format!(
        "From: \"{}\" <a{n}@example.com>\r\n\
         To: \"{}\" <b@example.com>, \"{}\" <c@example.com>\r\n\
         Cc: {} <d@example.com>\r\n\
         Subject: {}\r\n\
         Message-ID: <{}@example.com>\r\n\
         In-Reply-To: <{}@example.com>\r\n\
         References: <{}@example.com> <ok@example.com>\r\n\
         Date: Mon, 2 Nov 2026 10:00:00 +0000\r\n\
         \r\n\
         {}\r\n\0\r\n",
        p(0).replace('"', "'"),
        p(1).replace('"', "'"),
        p(2).replace('"', "'"),
        p(3),
        p(4),
        p(5).replace(['<', '>', ' '], ""),
        p(6).replace(['<', '>', ' '], ""),
        p(7).replace(['<', '>', ' '], ""),
        PAYLOADS.join("\r\n")
    )
    .into_bytes()
}

#[tokio::test]
async fn trapped_messages_cannot_alter_the_database() -> Result<(), Box<dyn std::error::Error>> {
    // Fichier que créerait un ATTACH injecté (reste éventuel d'une
    // exécution précédente retiré d'abord).
    let pwned = std::path::Path::new("/tmp/pwn.db");
    let _ = std::fs::remove_file(pwned);
    let l = local("sql-injection").await;
    let schema_before = l.db.schema().await?;
    let accounts = SqliteAccountRepository::new(l.db.clone());
    let accounts_before = accounts.list().await?;

    // Serveur : boîte au nom piégé, messages piégés, mots-clés piégés.
    let imap = FakeImap::new();
    let box_name = "Boîte'); DROP TABLE mailboxes; --";
    imap.create_mailbox(box_name, &[]);
    let mut keywords = Flags::from_tokens(["\\Seen"]);
    for raw in ["x';DROP-TABLE--", "1=1--", "$Junk"] {
        let k = Keyword::parse(raw).ok_or("mot-clé refusé par le domaine")?;
        keywords.insert_keyword(k);
    }
    for n in 0..PAYLOADS.len() {
        imap.append(
            "INBOX",
            &trapped_message(n),
            keywords.clone(),
            1_700_000_000,
        );
        imap.append(
            box_name,
            &trapped_message(n + 3),
            Flags::default(),
            1_700_000_000,
        );
    }

    // Synchronisation réelle, deux fois (la seconde ne doit rien changer).
    let sync = SyncAccount {
        account: l.account,
        store: &*l.store,
        blobs: &*l.blobs,
        policy: FetchPolicy::default(),
        text: std::sync::Arc::new(norelly_render::search_text::MimeTextExtractor),
    };
    for _ in 0..2 {
        let mut session = imap.session()?;
        let ext = session.enable_sync_extensions().await?;
        sync.sync_all(&mut session, ext, |_, _| {}).await?;
    }

    // Base intègre, schéma identique, comptes intacts.
    assert_eq!(l.db.integrity_problems().await?, Vec::<String>::new());
    assert_eq!(l.db.schema().await?, schema_before, "schéma modifié");
    assert_eq!(accounts.list().await?, accounts_before, "comptes modifiés");
    assert!(!pwned.exists(), "ATTACH exécuté");

    // Boîte au nom piégé : stockée telle quelle, rôle ordinaire.
    let tree = l.store.tree(l.account).await?;
    let trapped_box = tree
        .iter()
        .find(|n| n.name.as_str() == box_name)
        .ok_or("boîte au nom piégé absente")?;
    assert_eq!(trapped_box.total as usize, PAYLOADS.len());
    let inbox = tree
        .iter()
        .find(|n| n.name.is_inbox())
        .ok_or("INBOX absente")?;
    assert_eq!(inbox.total as usize, PAYLOADS.len());
    assert_eq!(
        tree.len(),
        2,
        "aucune boîte créée ni supprimée par injection"
    );

    // Valeurs relues à l'identique (assainies pour l'affichage).
    let page = l.store.page(inbox.id, None, 50).await?;
    let mut subjects: Vec<String> = page.items.iter().map(|i| i.subject.clone()).collect();
    subjects.sort();
    let mut expected: Vec<String> = (0..PAYLOADS.len())
        .map(|n| header_text(PAYLOADS[(n + 4) % PAYLOADS.len()], 998))
        .collect();
    expected.sort();
    assert_eq!(subjects, expected, "sujets piégés relus comme données");
    for item in &page.items {
        let from = item.from.as_ref().ok_or("expéditeur absent")?;
        assert!(from.address.ends_with("@example.com"), "{from:?}");
        assert!(
            !from.name.contains("injecté") || PAYLOADS.iter().any(|p| p.contains("injecté")),
            "JSON rompu"
        );
        assert_eq!(
            item.flags.keywords(),
            keywords.keywords(),
            "mots-clés relus"
        );
    }

    // État de synchronisation cohérent et corps intacts dans les fichiers.
    let local_inbox = l.store.local_mailbox(inbox.id).await?;
    assert_eq!(local_inbox.messages.len(), PAYLOADS.len());
    for uid in local_inbox.messages.keys() {
        let Some(StoredBody::Full(blob)) = l.store.body_of(inbox.id, *uid).await? else {
            return Err(format!("corps absent pour {uid}").into());
        };
        let raw = l
            .blobs
            .get(l.account, &blob)
            .await?
            .ok_or("contenu absent")?;
        assert!(
            (0..PAYLOADS.len()).any(|n| trapped_message(n) == raw),
            "corps altéré"
        );
    }

    // Conversations : identifiants et clés de sujet piégés
    // stockés comme données, relus par les requêtes d'écran.
    let conversations = l
        .store
        .conversations(ConversationScope::Mailbox(inbox.id), None, 100)
        .await?;
    let listed: u32 = conversations.items.iter().map(|c| c.messages).sum();
    assert_eq!(
        listed as usize,
        PAYLOADS.len(),
        "messages perdus par le regroupement"
    );
    for c in &conversations.items {
        assert!(expected.contains(&c.subject), "sujet {:?}", c.subject);
        let messages = l.store.conversation_messages(c.id).await?;
        assert!(!messages.is_empty());
    }
    let unified = l
        .store
        .conversations(ConversationScope::unified_inbox(), None, 100)
        .await?;
    assert_eq!(unified.items.len(), conversations.items.len());

    // Préférences de lecture : adresse et réglage stockés comme données.
    let prefs = SqliteReadingPreferences::new(l.db.clone());
    for raw in ["'--@example.com", "x'||'@example.com", "a\"b@example.com"] {
        let Ok(sender) = EmailAddress::parse(raw) else {
            continue;
        };
        prefs.allow_remote_images(&sender).await?;
        assert!(prefs.remote_images_allowed(&sender).await?);
    }
    let other = EmailAddress::parse("autre@example.com")?;
    assert!(
        !prefs.remote_images_allowed(&other).await?,
        "autorisation étendue par injection"
    );
    prefs.set_prefer_plain_text(true).await?;
    assert!(prefs.prefer_plain_text().await?);

    // Actions : le nom de boîte piégé est copié dans la file
    // (`pending_ops.mailbox`/`target`), les mots-clés piégés sont relus par
    // `action_target`, puis tout est rejoué sur le serveur.
    let ids: Vec<_> = page.items.iter().map(|i| i.id).collect();
    let perform = PerformAction { store: &*l.store };
    perform
        .execute(&ids, MessageAction::MarkFlagged(true), NOW)
        .await?;
    let moved = perform
        .execute(&ids, MessageAction::Move(trapped_box.id), NOW)
        .await?;
    assert_eq!(moved.recorded.len(), PAYLOADS.len());
    let pending = l.store.pending(l.account).await?;
    assert_eq!(pending.len(), 2 * PAYLOADS.len());
    for (_, op) in &pending {
        assert!(op.mailbox.is_inbox(), "boîte source relue : {op:?}");
        if let OpKind::Move { to } = &op.kind {
            assert_eq!(to.as_str(), box_name, "destination relue à l'identique");
        }
    }
    assert_eq!(
        l.store.tree(l.account).await?.len(),
        2,
        "aucune boîte créée par injection"
    );
    l.store
        .set_archive_fallback(l.account, Some(trapped_box.id))
        .await?;
    assert_eq!(
        l.store.archive_fallback(l.account).await?,
        Some(trapped_box.id)
    );
    // Boîte piégée renommée sur le serveur en un autre nom piégé, avec une
    // boîte locale périmée à ce nom : boîtes et file renommées par des
    // requêtes paramétrées, noms relus à l'identique.
    let renamed_name = "Renommée\"); DELETE FROM pending_ops; --";
    let remote =
        |name: &str, role: MailboxRole| -> Result<RemoteMailbox, Box<dyn std::error::Error>> {
            Ok(RemoteMailbox {
                name: MailboxName::parse(name)?,
                delimiter: Some('/'),
                role,
                selectable: true,
            })
        };
    l.store
        .replace_mailboxes(
            l.account,
            &[
                remote("INBOX", MailboxRole::Inbox)?,
                remote(box_name, MailboxRole::Normal)?,
                remote(renamed_name, MailboxRole::Normal)?,
            ],
        )
        .await?;
    let (old, new) = (
        MailboxName::parse(box_name)?,
        MailboxName::parse(renamed_name)?,
    );
    imap.session()?.rename_mailbox(&old, &new).await?;
    assert_eq!(l.store.rename_mailboxes(l.account, &old, &new).await?, 1);
    assert_eq!(l.db.integrity_problems().await?, Vec::<String>::new());
    assert_eq!(l.db.schema().await?, schema_before, "schéma modifié");
    let tree = l.store.tree(l.account).await?;
    assert_eq!(tree.len(), 2, "boîte périmée remplacée, rien d'autre");
    assert!(
        tree.iter()
            .any(|n| n.id == trapped_box.id && n.name.as_str() == renamed_name),
        "même boîte, nom relu à l'identique : {tree:?}"
    );
    let pending = l.store.pending(l.account).await?;
    assert_eq!(pending.len(), 2 * PAYLOADS.len(), "file intacte");
    for (_, op) in &pending {
        if let OpKind::Move { to } = &op.kind {
            assert_eq!(to.as_str(), renamed_name, "destination renommée");
        }
    }

    let mut session = imap.session()?;
    let replayed = ReplayActions {
        account: l.account,
        store: &*l.store,
    }
    .execute(&mut session, NOW + 60)
    .await?;
    assert_eq!(replayed.applied, 2 * PAYLOADS.len());
    assert!(replayed.dropped.is_empty(), "{:?}", replayed.dropped);
    assert!(l.store.pending(l.account).await?.is_empty());
    // Sur le serveur : tout est dans la boîte piégée, suivi, mots-clés
    // intacts.
    assert!(imap.flags_of("INBOX").is_empty());
    let on_server = imap.flags_of(renamed_name);
    assert_eq!(on_server.len(), 2 * PAYLOADS.len());
    assert_eq!(
        on_server
            .values()
            .filter(|f| f.flagged && f.keywords() == keywords.keywords())
            .count(),
        PAYLOADS.len()
    );
    let mut session = imap.session()?;
    let ext = session.enable_sync_extensions().await?;
    sync.sync_all(&mut session, ext, |_, _| {}).await?;
    l.store.purge_placeholders(l.account, None).await?;
    let tree = l.store.tree(l.account).await?;
    let total = |pred: &dyn Fn(&str) -> bool| {
        tree.iter()
            .filter(|n| pred(n.name.as_str()))
            .map(|n| n.total)
            .sum::<u32>()
    };
    assert_eq!(total(&|n| n == "INBOX"), 0);
    assert_eq!(total(&|n| n == renamed_name) as usize, 2 * PAYLOADS.len());

    // Rédaction : brouillon et file d'envoi aux champs piégés,
    // relus à l'identique, sans effet sur la base.
    let compose = SqliteComposeStore::new(l.db.clone());
    let mut draft = Draft::empty(l.account);
    draft.subject = PAYLOADS[0].to_owned();
    draft.html = Some(PAYLOADS.join("<br>"));
    draft.to = vec![Recipient::new(
        PAYLOADS[4],
        EmailAddress::parse("x'--@example.com")?,
    )];
    let saved = compose.save_draft(None, &draft, NOW).await?;
    assert_eq!(compose.draft(saved.id).await?.map(|d| d.draft), Some(draft));
    let outgoing = OutgoingMessage {
        from: PAYLOADS[1].to_owned(),
        recipients: PAYLOADS.iter().map(|p| (*p).to_owned()).collect(),
        raw: PAYLOADS.join("\r\n").into_bytes(),
    };
    compose
        .enqueue(
            l.account,
            &outgoing,
            b"\0'); DROP TABLE outbox; --",
            PAYLOADS[2],
            Some(saved.id),
            NOW,
        )
        .await?;
    let queued = compose.outbox(l.account).await?;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].message, outgoing);
    assert_eq!(queued[0].subject, PAYLOADS[2]);
    assert!(compose.draft(saved.id).await?.is_none());

    // Recherche : chaque charge comme requête, sans effet ; les
    // messages piégés sont retrouvés par leur texte, relus comme données.
    let search = |text: &str| SearchRequest {
        query: parse_query(text).unwrap_or_default(),
        scope: SearchScope::All,
        utc_offset_seconds: 0,
        offset: 0,
        limit: 100,
    };
    for payload in PAYLOADS {
        l.store.search(search(payload)).await?;
    }
    let found = l.store.search(search("\"DROP TABLE messages\"")).await?;
    assert_eq!(
        found.items.len(),
        2 * PAYLOADS.len(),
        "texte des corps indexé"
    );
    let all_subjects: Vec<String> = PAYLOADS.iter().map(|p| header_text(p, 998)).collect();
    for hit in &found.items {
        assert!(
            all_subjects.contains(&hit.subject),
            "sujet {:?}",
            hit.subject
        );
    }

    // Copie distante d'un brouillon dans la boîte au nom piégé :
    // enregistrée, mise à la corbeille puis oubliée, comme une donnée.
    let copy = RemoteCopy {
        mailbox: MailboxName::parse(box_name)?,
        validity: UidValidity::new(7).ok_or("validité")?,
        uid: Uid::new(3).ok_or("uid")?,
    };
    let mut draft = Draft::empty(l.account);
    draft.subject = "copie".into();
    let saved = compose.save_draft(None, &draft, NOW).await?;
    let (synced, _) = saved.sync.remote_saved(1, Some(copy.clone()), NOW);
    assert!(
        compose
            .set_remote(l.account, saved.id, synced, None)
            .await?
    );
    compose.delete_draft(saved.id).await?;
    assert_eq!(
        compose.remote_trash(l.account).await?,
        std::slice::from_ref(&copy)
    );
    compose.forget_remote(l.account, copy).await?;
    assert!(compose.remote_trash(l.account).await?.is_empty());

    // Transfert et reprise : pièce jointe au nom piégé, rangée
    // dans les blobs et relue avec le brouillon ; envoi programmé
    // (`outbox.draft_content`) puis repris, à l'identique.
    let attachment_body = PAYLOADS.join("\n");
    let with_attachment = format!(
        "From: \"{}\" <a@example.com>\r\nTo: <b@example.com>\r\nSubject: pj {}\r\n\
         Content-Type: multipart/mixed; boundary=b\r\n\r\n\
         --b\r\nContent-Type: text/plain\r\n\r\n{}\r\n\
         --b\r\nContent-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"{}\"\r\n\r\n{attachment_body}\r\n--b--\r\n",
        PAYLOADS[0].replace('"', "'"),
        PAYLOADS[4],
        PAYLOADS[5],
        PAYLOADS[3].replace('"', "'"),
    );
    imap.append(
        renamed_name,
        with_attachment.as_bytes(),
        Flags::default(),
        1_700_000_000,
    );
    let mut session = imap.session()?;
    let ext = session.enable_sync_extensions().await?;
    sync.sync_all(&mut session, ext, |_, _| {}).await?;
    let source = l
        .store
        .page(trapped_box.id, None, 100)
        .await?
        .items
        .into_iter()
        .find(|i| i.subject.starts_with("pj "))
        .ok_or("message à pièce jointe absent")?;
    let prepare = PrepareReply {
        locator: &*l.store,
        blobs: &*l.blobs,
        composer: &MimeComposer,
        fetcher: &NoFetch,
    };
    let me = Recipient::new(PAYLOADS[6], EmailAddress::parse("moi@example.com")?);
    let mut forwarded = prepare
        .execute(
            source.id,
            ReplyKind::Forward,
            std::slice::from_ref(me.address()),
            PAYLOADS[7],
        )
        .await?;
    let resumed = prepare.resume(source.id).await?;
    for draft in [&forwarded, &resumed] {
        assert_eq!(draft.attachments.len(), 1, "{:?}", draft.attachments);
        let a = &draft.attachments[0];
        let bytes = l
            .blobs
            .get(l.account, &a.blob)
            .await?
            .ok_or("pièce absente")?;
        assert!(String::from_utf8_lossy(&bytes).starts_with(&attachment_body));
        let saved = compose.save_draft(None, draft, NOW).await?;
        assert_eq!(
            compose.draft(saved.id).await?.map(|d| d.draft).as_ref(),
            Some(draft),
            "brouillon transféré ou repris relu à l'identique"
        );
        compose.delete_draft(saved.id).await?;
    }
    forwarded.to = vec![Recipient::new(
        PAYLOADS[1],
        EmailAddress::parse("b@example.com")?,
    )];
    let queue = QueueMessage {
        store: &compose,
        blobs: &*l.blobs,
        composer: &MimeComposer,
    };
    let scheduled = queue
        .schedule(None, &forwarded, &me, NOW, NOW + 3_600)
        .await?;
    let queued = compose.outbox(l.account).await?;
    assert!(queued.iter().any(|q| q.id == scheduled));
    let back = compose
        .unschedule(scheduled, NOW + 60)
        .await?
        .ok_or("envoi programmé non repris")?;
    assert_eq!(
        back.draft, forwarded,
        "brouillon programmé relu à l'identique"
    );

    assert_eq!(l.db.integrity_problems().await?, Vec::<String>::new());
    assert_eq!(l.db.schema().await?, schema_before, "schéma modifié");
    Ok(())
}

/// Charge en nom de fiche : un nom ne peut pas imiter une adresse (refusé
/// à la saisie), `@`, `<` et `>` y sont donc remplacés (`@` par un mot
/// qui ne l'écrit pas en toutes lettres) ; guillemets,
/// apostrophes, points-virgules et accolades restent tels quels.
fn as_name(payload: &str) -> String {
    payload
        .replace('@', " chez ")
        .replace('<', "(")
        .replace('>', ")")
}

/// Fiche dont chaque champ texte porte une charge (décalée de `n`).
fn trapped_contact(n: usize) -> norelly_domain::contacts::ContactInput {
    use norelly_domain::contacts::{
        ContactInput, EmailInput, PhoneInput, PostalAddressInput, WebsiteInput,
    };
    let p = |i: usize| PAYLOADS[(n + i) % PAYLOADS.len()].to_owned();
    // Éléments d'adresse postale : au plus 200 caractères.
    let part = |i: usize| p(i).chars().take(200).collect::<String>();
    let name = |i: usize| as_name(&p(i));
    // Étiquettes et numéros : au plus 64 caractères.
    let short = |i: usize| p(i).chars().take(64).collect::<String>();
    ContactInput {
        display_name: name(0),
        given_name: name(1),
        family_name: name(2),
        nickname: name(3),
        organization: p(4),
        notes: PAYLOADS.join("\n"),
        emails: vec![
            EmailInput {
                address: format!("c{n}@example.com"),
                label: short(5),
                preferred: true,
            },
            EmailInput {
                address: format!("c{n}.bis@example.org"),
                label: short(6),
                preferred: false,
            },
        ],
        phones: vec![PhoneInput {
            number: short(7),
            label: short(0),
        }],
        title: p(5),
        birthday: if n.is_multiple_of(2) {
            "1985-04-12".into()
        } else {
            "--02-29".into()
        },
        postal_addresses: vec![PostalAddressInput {
            street: format!("{}\n{}", part(6), part(7)),
            locality: part(0),
            region: part(1),
            postal_code: part(2),
            country: part(3),
            label: short(4),
        }],
        websites: vec![
            // Charge entière, encodée (elle contient des espaces, qu'un
            // site n'admet pas)…
            WebsiteInput {
                url: format!("https://example.com/{n}?q={}", encoded(&p(5))),
                label: short(6),
            },
            // … et charges brutes sans espace, admises telles quelles.
            WebsiteInput {
                url: format!("https://example.com/{n}');DROP/**/TABLE/**/contacts;--"),
                label: String::new(),
            },
            WebsiteInput {
                url: format!("https://example.com/{n}'||sqlite_version()||'\"%00"),
                label: String::new(),
            },
        ],
        favorite: n.is_multiple_of(2),
    }
}

/// Texte en pourcentages (octets hors lettres et chiffres ASCII).
fn encoded(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[tokio::test]
async fn trapped_contacts_cannot_alter_the_database() -> Result<(), Box<dyn std::error::Error>> {
    use norelly_application::contacts::{CreateContact, SuggestRecipients, UpdateContact};
    use norelly_domain::contacts::{ContactFilter, ContactInput};
    use norelly_domain::ports::{ContactBook, ContactDirectory};
    use norelly_sqlite::contacts::SqliteContactBook;

    let l = local("sql-injection-contacts").await;
    let schema_before = l.db.schema().await?;
    let accounts = SqliteAccountRepository::new(l.db.clone());
    let accounts_before = accounts.list().await?;
    let book = SqliteContactBook::new(l.db.clone());

    // Fiches piégées : enregistrées, relues à l'identique.
    let mut created = Vec::new();
    for n in 0..PAYLOADS.len() {
        let contact = CreateContact { book: &book }
            .execute(trapped_contact(n))
            .await?;
        assert_eq!(book.get(contact.id()).await?.as_ref(), Some(&contact));
        created.push(contact);
    }

    // Chaque charge (sous sa forme de nom), comme filtre de la liste et
    // comme saisie d'un destinataire : de simples données, qui retrouvent
    // leur fiche.
    let suggest = SuggestRecipients {
        collected: &*l.store,
        directory: &book,
    };
    for (n, payload) in PAYLOADS.iter().map(|p| as_name(p)).enumerate() {
        let payload = payload.as_str();
        let page = book
            .page(&ContactFilter::new(payload, false), None, PageSize::new(50))
            .await?;
        assert!(
            page.items.iter().any(|i| i.id == created[n].id()),
            "{payload:?} : fiche non retrouvée"
        );
        let found = suggest.execute(l.account, payload, 20).await?;
        assert!(
            found
                .iter()
                .any(|s| s.address == format!("c{n}@example.com")),
            "{payload:?} : suggestion absente ({found:?})"
        );
    }
    // Charges brutes (« @ » compris, préfixe de paramètre nommé en SQL),
    // comme filtre et comme saisie : la fiche dont l'organisation porte la
    // charge est retrouvée, sans erreur.
    let len = PAYLOADS.len();
    for (k, payload) in PAYLOADS.iter().enumerate() {
        let page = book
            .page(&ContactFilter::new(payload, false), None, PageSize::new(50))
            .await?;
        let owner = created[(k + len - 4) % len].id();
        assert!(
            page.items.iter().any(|i| i.id == owner),
            "{payload:?} : fiche à l'organisation piégée non retrouvée"
        );
        suggest.execute(l.account, payload, 20).await?;
    }

    // Modification et suppression piégées.
    let renamed = UpdateContact { book: &book }
        .execute(
            created[0].id(),
            ContactInput {
                display_name: as_name(PAYLOADS[4]),
                given_name: as_name(PAYLOADS[7]),
                ..trapped_contact(0)
            },
        )
        .await?;
    assert_eq!(book.get(renamed.id()).await?, Some(renamed.clone()));
    let ids: Vec<_> = created.iter().map(|c| c.id()).collect();
    assert_eq!(book.delete_many(&ids).await?, PAYLOADS.len());
    assert!(
        book.page(&ContactFilter::default(), None, PageSize::new(50))
            .await?
            .items
            .is_empty()
    );

    assert_eq!(accounts.list().await?, accounts_before, "comptes modifiés");
    assert_eq!(l.db.integrity_problems().await?, Vec::<String>::new());
    assert_eq!(l.db.schema().await?, schema_before, "schéma modifié");
    Ok(())
}

#[tokio::test]
async fn trapped_contact_lists_cannot_alter_the_database() -> Result<(), Box<dyn std::error::Error>>
{
    use norelly_application::contact_lists::{
        CreateContactList, ExpandContactList, ListContactLists, SuggestContactLists,
        UpdateContactList,
    };
    use norelly_application::contacts::CreateContact;
    use norelly_domain::contact_lists::ContactListInput;
    use norelly_domain::ports::{ContactListBook, ContactListDirectory};
    use norelly_sqlite::contacts::SqliteContactBook;

    let l = local("sql-injection-contact-lists").await;
    let schema_before = l.db.schema().await?;
    let accounts = SqliteAccountRepository::new(l.db.clone());
    let accounts_before = accounts.list().await?;
    let book = SqliteContactBook::new(l.db.clone());
    let member = CreateContact { book: &book }
        .execute(trapped_contact(0))
        .await?;

    // Listes piégées (nom et description) : enregistrées, relues à
    // l'identique, retrouvées par leur nom, dépliées.
    let mut created = Vec::new();
    for (n, payload) in PAYLOADS.iter().enumerate() {
        let list = CreateContactList { book: &book }
            .execute(ContactListInput {
                name: format!("{n} {payload}"),
                description: PAYLOADS[(n + 1) % PAYLOADS.len()].to_owned(),
                members: vec![member.id().get()],
                bcc: n.is_multiple_of(2),
            })
            .await?;
        assert_eq!(book.get_list(list.id()).await?.as_ref(), Some(&list));
        created.push(list);
    }
    for (n, payload) in PAYLOADS.iter().enumerate() {
        let name = format!("{n} {payload}");
        let page = ListContactLists { directory: &book }
            .execute(&name, None, PageSize::new(50))
            .await?;
        assert!(
            page.items.iter().any(|i| i.id == created[n].id()),
            "{payload:?} : liste non retrouvée"
        );
        let found = SuggestContactLists { directory: &book }
            .execute(&name, 5)
            .await?;
        assert!(found.iter().any(|i| i.id == created[n].id()), "{payload:?}");
        let expanded = ExpandContactList {
            lists: &book,
            contacts: &book,
        }
        .execute(created[n].id())
        .await?;
        assert_eq!(expanded.recipients.len(), 1);
    }

    // Modification et suppression piégées.
    let renamed = UpdateContactList { book: &book }
        .execute(
            created[0].id(),
            ContactListInput {
                name: PAYLOADS[3].to_owned(),
                ..created[0].draft().to_input()
            },
        )
        .await?;
    assert_eq!(book.get_list(renamed.id()).await?, Some(renamed));
    let ids: Vec<_> = created.iter().map(|c| c.id()).collect();
    assert_eq!(book.delete_lists(&ids).await?, PAYLOADS.len());

    assert_eq!(accounts.list().await?, accounts_before, "comptes modifiés");
    assert_eq!(l.db.integrity_problems().await?, Vec::<String>::new());
    assert_eq!(l.db.schema().await?, schema_before, "schéma modifié");
    Ok(())
}

/// Cellule CSV entre guillemets.
fn csv_cell(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

/// Export CSV de Thunderbird dont chaque champ texte porte une charge ;
/// `extra` : seconde adresse de chaque fiche (complément).
fn trapped_csv(extra: bool) -> Vec<u8> {
    let mut out = String::from(
        "First Name,Last Name,Display Name,Nickname,Primary Email,Secondary Email,Work Phone,Organization,Notes\r\n",
    );
    for n in 0..PAYLOADS.len() {
        let p = |i: usize| PAYLOADS[(n + i) % PAYLOADS.len()];
        let second = if extra {
            format!("i{n}.bis@example.org")
        } else {
            String::new()
        };
        let row = [
            csv_cell(p(0)),
            csv_cell(p(1)),
            csv_cell(p(2)),
            csv_cell(p(3)),
            format!("i{n}@example.com"),
            second,
            csv_cell(&p(4).chars().take(64).collect::<String>()),
            csv_cell(p(5)),
            csv_cell(&PAYLOADS.join("\n")),
        ];
        out.push_str(&row.join(","));
        out.push_str("\r\n");
    }
    out.into_bytes()
}

#[tokio::test]
async fn trapped_import_cannot_alter_the_database() -> Result<(), Box<dyn std::error::Error>> {
    use norelly_application::contact_import::{ApplyContactImport, PreviewContactImport};
    use norelly_contacts_io::{ContactsIo, IcuNfc};
    use norelly_domain::contact_import::DuplicatePolicy;
    use norelly_domain::contacts::ContactFilter;
    use norelly_domain::ports::ContactDirectory;
    use norelly_sqlite::contacts::SqliteContactBook;

    let l = local("sql-injection-import").await;
    let schema_before = l.db.schema().await?;
    let accounts = SqliteAccountRepository::new(l.db.clone());
    let accounts_before = accounts.list().await?;
    let book = SqliteContactBook::new(l.db.clone());

    // Fichier piégé lu, chiffré puis importé : chaque fiche, de simples
    // données.
    let preview = PreviewContactImport {
        codec: &ContactsIo,
        directory: &book,
        normalizer: &IcuNfc,
    }
    .execute(&trapped_csv(false))
    .await?;
    assert!(
        preview.skip.rejected.is_empty(),
        "{:?}",
        preview.skip.rejected
    );
    assert_eq!(preview.skip.create.len(), PAYLOADS.len());
    let apply = ApplyContactImport {
        book: &book,
        normalizer: &IcuNfc,
    };
    let outcome = apply.execute(&preview.cards, DuplicatePolicy::Skip).await?;
    assert_eq!(outcome.created, PAYLOADS.len());
    let page = book
        .page(&ContactFilter::default(), None, PageSize::new(50))
        .await?;
    assert_eq!(page.items.len(), PAYLOADS.len());
    for item in &page.items {
        let contact = book
            .get(item.id)
            .await?
            .unwrap_or_else(|| panic!("fiche {} relue", item.id));
        assert_eq!(contact.draft().notes(), Some(PAYLOADS.join("\n").as_str()));
    }
    // Chaque charge retrouve sa fiche comme filtre.
    for payload in PAYLOADS {
        let found = book
            .page(&ContactFilter::new(payload, false), None, PageSize::new(50))
            .await?;
        assert!(!found.items.is_empty(), "{payload:?} : aucune fiche");
    }

    // Second import piégé : les mêmes fiches complétées d'une adresse.
    let cards = PreviewContactImport {
        codec: &ContactsIo,
        directory: &book,
        normalizer: &IcuNfc,
    }
    .execute(&trapped_csv(true))
    .await?
    .cards;
    let outcome = apply.execute(&cards, DuplicatePolicy::Complete).await?;
    assert_eq!((outcome.created, outcome.completed), (0, PAYLOADS.len()));
    let page = book
        .page(&ContactFilter::default(), None, PageSize::new(50))
        .await?;
    for item in &page.items {
        let contact = book
            .get(item.id)
            .await?
            .unwrap_or_else(|| panic!("fiche {} relue", item.id));
        assert_eq!(contact.draft().emails().len(), 2);
    }

    assert_eq!(accounts.list().await?, accounts_before, "comptes modifiés");
    assert_eq!(l.db.integrity_problems().await?, Vec::<String>::new());
    assert_eq!(l.db.schema().await?, schema_before, "schéma modifié");
    Ok(())
}

/// Charge tronquée à la longueur d'une étiquette ou d'un numéro, sans
/// espace de bord (que le domaine retire).
fn short_payload(i: usize) -> String {
    let cut: String = PAYLOADS[i % PAYLOADS.len()].chars().take(64).collect();
    cut.trim().to_owned()
}

/// Adresses piégées de la fiche `n` : parties locales contenant `'`
/// (valides en RFC 5322), dont une qui commence par `'`. Les deux du
/// milieu partagent une cellule Google (« ::: ») et son étiquette ; la
/// dernière n'est que dans le complément.
fn apostrophe_addresses(n: usize) -> [String; 4] {
    [
        format!("o'brien'--{n}@example.org"),
        format!("'{n}'or'1'='1@example.com"),
        format!("d'artagnan'or''='{n}@example.fr"),
        format!("x'||'{n}'='{n}'--@example.net"),
    ]
}

/// (adresse, étiquette, préférée) attendues de la fiche `n`, complétée
/// ou non.
fn expected_google_emails(n: usize, completed: bool) -> Vec<(String, Option<String>, bool)> {
    let [first, second, second_bis, third] = apostrophe_addresses(n);
    let mut emails = vec![
        (first, Some(short_payload(n + 1)), true),
        (second, Some(short_payload(n + 2)), false),
        (second_bis, Some(short_payload(n + 2)), false),
    ];
    if completed {
        emails.push((third, Some(short_payload(n + 3)), false));
    }
    emails
}

/// Export CSV de Google Contacts dont les étiquettes d'adresse et de
/// téléphone portent une charge, avec des adresses à apostrophes ;
/// `extra` : dernière adresse de chaque fiche (complément).
fn trapped_google_csv(extra: bool) -> Vec<u8> {
    let mut out = String::from(
        "Name,E-mail 1 - Label,E-mail 1 - Value,E-mail 2 - Label,E-mail 2 - Value,\
         E-mail 3 - Label,E-mail 3 - Value,Phone 1 - Label,Phone 1 - Value\r\n",
    );
    for (n, payload) in PAYLOADS.iter().enumerate() {
        let [first, second, second_bis, third] = apostrophe_addresses(n);
        let (third_label, third) = if extra {
            (csv_cell(&short_payload(n + 3)), csv_cell(&third))
        } else {
            (String::new(), String::new())
        };
        let row = [
            csv_cell(&format!("{n} {payload}")),
            // « * » : adresse principale chez Google.
            csv_cell(&format!("* {}", short_payload(n + 1))),
            csv_cell(&first),
            csv_cell(&short_payload(n + 2)),
            csv_cell(&format!("{second} ::: {second_bis}")),
            third_label,
            third,
            csv_cell(&short_payload(n + 4)),
            csv_cell(&short_payload(n + 5)),
        ];
        out.push_str(&row.join(","));
        out.push_str("\r\n");
    }
    out.into_bytes()
}

#[tokio::test]
async fn trapped_google_labels_and_apostrophes_cannot_alter_the_database()
-> Result<(), Box<dyn std::error::Error>> {
    use norelly_application::contact_import::{ApplyContactImport, PreviewContactImport};
    use norelly_application::contacts::SuggestRecipients;
    use norelly_contacts_io::{ContactsIo, IcuNfc};
    use norelly_domain::contact_import::DuplicatePolicy;
    use norelly_domain::contacts::{Contact, ContactFilter};
    use norelly_domain::ports::ContactDirectory;
    use norelly_sqlite::contacts::SqliteContactBook;

    let l = local("sql-injection-import-google").await;
    let schema_before = l.db.schema().await?;
    let accounts = SqliteAccountRepository::new(l.db.clone());
    let accounts_before = accounts.list().await?;
    let book = SqliteContactBook::new(l.db.clone());

    // Fiche n° `n` retrouvée par sa première adresse (le nom piégé qui
    // imite une adresse est retiré à l'import), relue entière.
    let card = |n: usize| {
        let book = &book;
        async move {
            let [first, ..] = apostrophe_addresses(n);
            let page = book
                .page(&ContactFilter::new(&first, false), None, PageSize::new(50))
                .await?;
            let [item] = &page.items[..] else {
                panic!("{first:?} : {} fiches", page.items.len());
            };
            let contact: Contact = book
                .get(item.id)
                .await?
                .unwrap_or_else(|| panic!("fiche {} relue", item.id));
            Ok::<_, Box<dyn std::error::Error>>(contact)
        }
    };
    // (adresse, étiquette, préférée) de chaque adresse d'une fiche.
    let emails = |c: &Contact| {
        c.draft()
            .emails()
            .iter()
            .map(|e| {
                (
                    e.address().to_string(),
                    e.label().map(str::to_owned),
                    e.preferred(),
                )
            })
            .collect::<Vec<_>>()
    };
    // Seul téléphone de la fiche n° `n`, étiquette piégée comprise.
    let assert_phone = |c: &Contact, n: usize| {
        let [phone] = c.draft().phones() else {
            panic!("{:?}", c.draft().phones());
        };
        assert_eq!(
            (phone.number(), phone.label()),
            (
                short_payload(n + 5).as_str(),
                Some(short_payload(n + 4).as_str())
            )
        );
    };

    // Fichier piégé lu, chiffré puis importé : étiquettes et adresses, de
    // simples données relues à l'identique.
    let preview = PreviewContactImport {
        codec: &ContactsIo,
        directory: &book,
        normalizer: &IcuNfc,
    }
    .execute(&trapped_google_csv(false))
    .await?;
    assert!(
        preview.skip.rejected.is_empty(),
        "{:?}",
        preview.skip.rejected
    );
    assert_eq!(preview.skip.create.len(), PAYLOADS.len());
    let apply = ApplyContactImport {
        book: &book,
        normalizer: &IcuNfc,
    };
    let outcome = apply.execute(&preview.cards, DuplicatePolicy::Skip).await?;
    assert_eq!(outcome.created, PAYLOADS.len());
    let suggest = SuggestRecipients {
        collected: &*l.store,
        directory: &book,
    };
    for n in 0..PAYLOADS.len() {
        let contact = card(n).await?;
        let [first, second, second_bis, _] = apostrophe_addresses(n);
        assert_eq!(emails(&contact), expected_google_emails(n, false));
        assert_phone(&contact, n);
        // Chaque adresse à apostrophes, comme filtre et comme saisie d'un
        // destinataire, retrouve sa fiche et seulement elle.
        for address in [&first, &second, &second_bis] {
            let found = book
                .page(&ContactFilter::new(address, false), None, PageSize::new(50))
                .await?;
            assert_eq!(
                found.items.iter().map(|i| i.id).collect::<Vec<_>>(),
                [contact.id()],
                "{address:?}"
            );
            let suggested = suggest.execute(l.account, address, 20).await?;
            assert!(
                suggested.iter().any(|s| &s.address == address),
                "{address:?} : suggestion absente ({suggested:?})"
            );
        }
    }

    // Second import piégé : les mêmes fiches complétées d'une adresse à
    // apostrophes et de son étiquette piégée ; le reste de la fiche, relu
    // puis réécrit, ne change pas.
    let preview = PreviewContactImport {
        codec: &ContactsIo,
        directory: &book,
        normalizer: &IcuNfc,
    }
    .execute(&trapped_google_csv(true))
    .await?;
    assert!(
        preview.complete.rejected.is_empty(),
        "{:?}",
        preview.complete.rejected
    );
    let outcome = apply
        .execute(&preview.cards, DuplicatePolicy::Complete)
        .await?;
    assert_eq!((outcome.created, outcome.completed), (0, PAYLOADS.len()));
    for n in 0..PAYLOADS.len() {
        let contact = card(n).await?;
        assert_eq!(emails(&contact), expected_google_emails(n, true));
        assert_phone(&contact, n);
    }
    assert_eq!(
        book.page(&ContactFilter::default(), None, PageSize::new(50))
            .await?
            .items
            .len(),
        PAYLOADS.len()
    );

    assert_eq!(accounts.list().await?, accounts_before, "comptes modifiés");
    assert_eq!(l.db.integrity_problems().await?, Vec::<String>::new());
    assert_eq!(l.db.schema().await?, schema_before, "schéma modifié");
    Ok(())
}

/// Étiquettes piégées : mots-clés et `PERMANENTFLAGS` annoncés par le
/// serveur, puis listés, colorés, posés et retirés (file d'opérations),
/// cherchés, renommés et retirés de tous les messages (réécritures) ; tout
/// reste de la donnée : schéma inchangé, base intègre, comptes intacts.
#[tokio::test]
async fn trapped_keywords_cannot_alter_the_database() -> Result<(), Box<dyn std::error::Error>> {
    use norelly_application::labels::{
        DeleteLabel, ListLabels, RenameLabel, ReplayRewrites, SetLabelColor,
    };
    use norelly_domain::labels::{LabelColor, LabelName};
    use norelly_domain::ports::LabelStore;

    let l = local("labels-injection").await;
    let schema_before = l.db.schema().await?;
    let accounts = SqliteAccountRepository::new(l.db.clone());
    let accounts_before = accounts.list().await?;
    let trapped = "x';DROP-TABLE--";
    let other = "1=1--";

    let imap = FakeImap::new();
    imap.set_permanent_flags(Some(&["\\Seen", trapped, other, "\\*"]));
    let flags = Flags::from_tokens([trapped, other]);
    imap.append("INBOX", &trapped_message(0), flags, 1_700_000_000);
    let sync = SyncAccount {
        account: l.account,
        store: &*l.store,
        blobs: &*l.blobs,
        policy: FetchPolicy::default(),
        text: std::sync::Arc::new(norelly_render::search_text::MimeTextExtractor),
    };
    let mut session = imap.session()?;
    let ext = session.enable_sync_extensions().await?;
    sync.sync_all(&mut session, ext, |_, _| {}).await?;

    // Listé, coloré.
    let keyword = Keyword::parse(trapped).ok_or("mot-clé")?;
    let labels = ListLabels { store: &*l.store }.execute(l.account).await?;
    assert!(labels.iter().any(|x| x.keyword == keyword));
    SetLabelColor { store: &*l.store }
        .execute(
            l.account,
            &keyword,
            Some(LabelColor::parse("#123456").ok_or("couleur")?),
        )
        .await?;

    // Retiré puis rejoué : PERMANENTFLAGS piégé noté sur la boîte.
    let inbox = l
        .store
        .tree(l.account)
        .await?
        .into_iter()
        .find(|n| n.name.is_inbox())
        .ok_or("INBOX")?;
    let id = l.store.page(inbox.id, None, 10).await?.items[0].id;
    PerformAction { store: &*l.store }
        .execute(
            &[id],
            MessageAction::SetLabel {
                keyword: keyword.clone(),
                value: false,
            },
            NOW,
        )
        .await?;
    ReplayActions {
        account: l.account,
        store: &*l.store,
    }
    .execute(&mut session, NOW + 60)
    .await?;
    let permissions = l.store.keyword_permissions(l.account).await?;
    assert_eq!(permissions.len(), 1, "{permissions:?}");

    // Cherché.
    let mut query = parse_query(&format!("étiquette:\"{trapped}\""))?;
    query.resolve_labels(std::slice::from_ref(&keyword));
    l.store
        .search(SearchRequest {
            query,
            scope: SearchScope::All,
            utc_offset_seconds: 0,
            offset: 0,
            limit: 10,
        })
        .await?;

    // Renommé et retiré de tous les messages (réécritures rejouées).
    RenameLabel { store: &*l.store }
        .execute(l.account, &keyword, &LabelName::parse("Propre")?)
        .await?;
    DeleteLabel { store: &*l.store }
        .execute(l.account, &Keyword::parse(other).ok_or("mot-clé")?, true)
        .await?;
    ReplayRewrites {
        account: l.account,
        store: &*l.store,
    }
    .execute(&mut session)
    .await?;
    assert!(l.store.label_rewrites(l.account).await?.is_empty());

    assert_eq!(l.db.integrity_problems().await?, Vec::<String>::new());
    assert_eq!(l.db.schema().await?, schema_before, "schéma modifié");
    assert_eq!(accounts.list().await?, accounts_before, "comptes modifiés");
    assert_eq!(l.store.tree(l.account).await?.len(), 1, "boîtes intactes");
    Ok(())
}
