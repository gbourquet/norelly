//! Message piégé contre la base : des charges d'injection SQL (et de
//! rupture du JSON stocké) dans tous les champs qu'un expéditeur ou un
//! serveur contrôle (sujet, expéditeur, destinataires, Message-ID,
//! références, mots-clés, nom de boîte, corps) traversent toute la chaîne
//! réelle : faux serveur IMAP, `mail-parser`, domaine, SQLite sur disque,
//! fichiers. Elles doivent être stockées comme de simples données : schéma
//! inchangé, base intègre, aucune autre ligne touchée, valeurs relues à
//! l'identique (une fois assainies pour l'affichage).
//!
//! Sans Docker : exécuté avec les tests unitaires (`cargo nextest run`).

mod common;

use common::local;
use norelly_application::sync::SyncAccount;
use norelly_domain::address::EmailAddress;
use norelly_domain::conversation::ConversationScope;
use norelly_domain::fetch_policy::FetchPolicy;
use norelly_domain::mailbox::{Flags, Keyword, header_text};
use norelly_domain::ports::{
    AccountRepository, BlobStore, ConversationQuery, FolderTreeQuery, MailStore, MessageListQuery,
    ReadingPreferences, RemoteMailboxSession, StoredBody,
};
use norelly_sqlite::SqliteAccountRepository;
use norelly_sqlite::preferences::SqliteReadingPreferences;
use norelly_test_support::fake_imap::FakeImap;

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
    };
    for _ in 0..2 {
        let mut session = imap.session()?;
        let ext = session.enable_sync_extensions().await?;
        sync.sync_all(&mut session, ext, |_| {}).await?;
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

    // Conversations (phase 5) : identifiants et clés de sujet piégés
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

    assert_eq!(l.db.integrity_problems().await?, Vec::<String>::new());
    assert_eq!(l.db.schema().await?, schema_before, "schéma modifié");
    Ok(())
}
