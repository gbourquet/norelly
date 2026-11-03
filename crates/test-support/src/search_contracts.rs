//! Suite de contrat de [`SearchIndex`], jouée contre le faux en mémoire
//! ([`crate::mail_contracts::InMemoryMailStore`]) et contre SQLite (FTS5).
//!
//! Le classement exact dépend du moteur : le contrat ne vérifie l'ordre que
//! dans des cas nets (terme dans le sujet contre terme dans les
//! destinataires ; dates pour une requête sans texte).

use norelly_domain::account::AccountId;
use norelly_domain::fetch_policy::{Disposition, MimePart};
use norelly_domain::mailbox::{
    Correspondent, Flags, MailboxId, MailboxRole, MessageHeaders, MessageMeta,
};
use norelly_domain::ports::{
    BlobId, MAX_INDEXED_TEXT_CHARS, MAX_PAGE_SIZE, MAX_SEARCH_RESULTS, MAX_SNIPPET_CHARS,
    MailStore, SearchIndex, SearchPage, SearchRequest, SearchScope, StoreError, StoredBody,
    StoredMailbox, StoredMessage, SyncBatch,
};
use norelly_domain::search::{Date, parse_query};

use crate::mail_contracts::{mailbox, sha256_hex, state, uid};

/// Message de test décrit champ par champ.
struct Mail<'a> {
    n: u32,
    date: i64,
    subject: &'a str,
    from: (&'a str, &'a str),
    to: &'a [(&'a str, &'a str)],
    cc: &'a [(&'a str, &'a str)],
    message_id: &'a str,
    flags: &'a [&'a str],
    attachment: bool,
}

impl Mail<'_> {
    fn stored(&self) -> StoredMessage {
        let people = |list: &[(&str, &str)]| -> Vec<Correspondent> {
            list.iter().map(|(n, a)| Correspondent::new(n, a)).collect()
        };
        let structure = self.attachment.then(|| MimePart::Multipart {
            subtype: "mixed".into(),
            parts: vec![
                MimePart::Leaf {
                    media_type: "text".into(),
                    subtype: "plain".into(),
                    size: 100,
                    disposition: Disposition::Unspecified,
                    filename: None,
                },
                MimePart::Leaf {
                    media_type: "application".into(),
                    subtype: "pdf".into(),
                    size: 1000,
                    disposition: Disposition::Attachment,
                    filename: Some("devis.pdf".into()),
                },
            ],
        });
        StoredMessage {
            meta: MessageMeta {
                uid: uid(self.n),
                flags: Flags::from_tokens(self.flags.to_vec()),
                modseq: None,
                internal_date: self.date,
                size: 100,
                headers: MessageHeaders {
                    subject: self.subject.to_owned(),
                    from: people(&[self.from]),
                    to: people(self.to),
                    cc: people(self.cc),
                    date: Some(self.date),
                    message_id: Some(self.message_id.to_owned()),
                    ..MessageHeaders::default()
                },
                structure,
            },
            body: StoredBody::Missing,
        }
    }
}

const ALICE: (&str, &str) = ("Alice Martin", "alice@example.com");
const BOB: (&str, &str) = ("Bob Durand", "bob@example.com");
const CAROLE: (&str, &str) = ("Carole Petit", "carole@example.org");

/// Jour à minuit UTC, en secondes Unix.
fn day(year: i32, month: u8, d: u8) -> i64 {
    Date::new(year, month, d).map_or(0, |d| d.start_timestamp(0))
}

fn id_of(list: &[StoredMailbox], name: &str) -> MailboxId {
    list.iter()
        .find(|m| m.mailbox.name.as_str() == name)
        .map(|m| m.id)
        .unwrap_or_else(|| panic!("boîte {name} absente"))
}

async fn put<S: MailStore>(
    store: &S,
    mailbox: MailboxId,
    mails: &[Mail<'_>],
) -> Result<(), StoreError> {
    store
        .apply_sync(
            mailbox,
            &SyncBatch {
                reset: false,
                removed: vec![],
                flag_changes: vec![],
                new: mails.iter().map(Mail::stored).collect(),
                state: state(1, 1, 1000),
            },
        )
        .await
}

fn request(query: &str, scope: SearchScope) -> SearchRequest {
    SearchRequest {
        query: parse_query(query).unwrap_or_else(|e| panic!("requête {query:?} : {e}")),
        scope,
        utc_offset_seconds: 0,
        offset: 0,
        limit: 50,
    }
}

/// Sujets trouvés, dans l'ordre renvoyé.
async fn subjects<S: SearchIndex>(
    store: &S,
    query: &str,
    scope: SearchScope,
) -> Result<Vec<String>, StoreError> {
    let page = store.search(request(query, scope)).await?;
    Ok(page.items.into_iter().map(|h| h.subject).collect())
}

/// Sujets trouvés, triés (quand l'ordre n'est pas spécifié).
async fn found<S: SearchIndex>(
    store: &S,
    query: &str,
    scope: SearchScope,
) -> Result<Vec<String>, StoreError> {
    let mut list = subjects(store, query, scope).await?;
    list.sort();
    Ok(list)
}

/// Contrat de [`SearchIndex`] pour le compte `account` (qui doit exister
/// et n'avoir aucune boîte) ; `other` : un second compte.
pub async fn search_index_contract<S>(
    store: &S,
    account: AccountId,
    other: AccountId,
) -> Result<(), StoreError>
where
    S: MailStore + SearchIndex,
{
    let boxes = store
        .replace_mailboxes(
            account,
            &[
                mailbox("INBOX", MailboxRole::Inbox, true),
                mailbox("Archives", MailboxRole::Archive, true),
                mailbox("[Gmail]/Tous les messages", MailboxRole::All, true),
            ],
        )
        .await?;
    let (inbox, archives, all) = (
        id_of(&boxes, "INBOX"),
        id_of(&boxes, "Archives"),
        id_of(&boxes, "[Gmail]/Tous les messages"),
    );
    let other_inbox = id_of(
        &store
            .replace_mailboxes(other, &[mailbox("INBOX", MailboxRole::Inbox, true)])
            .await?,
        "INBOX",
    );
    let jan = day(2026, 1, 15);
    let feb = day(2026, 2, 10);
    let mar = day(2026, 3, 5);
    let facture = Mail {
        n: 1,
        date: jan,
        subject: "Facture d'avril",
        from: ALICE,
        to: &[BOB],
        cc: &[],
        message_id: "facture@example.com",
        flags: &[],
        attachment: true,
    };
    let reunion = Mail {
        n: 2,
        date: feb,
        subject: "Réunion de lundi",
        from: BOB,
        to: &[CAROLE],
        cc: &[ALICE],
        message_id: "reunion@example.com",
        flags: &["\\Seen", "\\Flagged"],
        attachment: false,
    };
    let carole_arrive = Mail {
        n: 3,
        date: mar,
        subject: "Carole arrive",
        from: BOB,
        to: &[BOB],
        cc: &[],
        message_id: "arrive@example.com",
        flags: &["\\Seen"],
        attachment: false,
    };
    let supprime = Mail {
        n: 4,
        date: mar,
        subject: "Facture supprimée",
        from: ALICE,
        to: &[BOB],
        cc: &[],
        message_id: "supprime@example.com",
        flags: &["\\Deleted"],
        attachment: false,
    };
    put(store, inbox, &[facture, reunion, carole_arrive, supprime]).await?;
    put(
        store,
        archives,
        &[Mail {
            n: 1,
            date: day(2026, 1, 20),
            subject: "Compte-rendu de la réunion",
            from: CAROLE,
            to: &[ALICE],
            cc: &[],
            message_id: "cr@example.com",
            flags: &["\\Seen"],
            attachment: false,
        }],
    )
    .await?;
    put(
        store,
        all,
        &[
            // Copie de la facture : listée une seule fois.
            Mail {
                n: 1,
                date: jan,
                subject: "Facture d'avril",
                from: ALICE,
                to: &[BOB],
                cc: &[],
                message_id: "facture@example.com",
                flags: &[],
                attachment: true,
            },
            // Archivé chez Gmail : présent seulement dans `\All`.
            Mail {
                n: 2,
                date: day(2025, 12, 1),
                subject: "Billet de train",
                from: CAROLE,
                to: &[BOB],
                cc: &[],
                message_id: "train@example.com",
                flags: &["\\Seen"],
                attachment: false,
            },
        ],
    )
    .await?;
    put(
        store,
        other_inbox,
        &[Mail {
            n: 1,
            date: feb,
            subject: "Facture de l'autre compte",
            from: ALICE,
            to: &[BOB],
            cc: &[],
            message_id: "autre@example.com",
            flags: &[],
            attachment: false,
        }],
    )
    .await?;

    let mine = SearchScope::Account(account);
    let everywhere = SearchScope::All;

    // --- Portée, visibilité, doublons de `\All` ---
    assert_eq!(
        found(store, "facture", everywhere).await?,
        ["Facture d'avril", "Facture de l'autre compte"],
        "tous les comptes ; copie de \\All et message \\Deleted écartés"
    );
    assert_eq!(found(store, "facture", mine).await?, ["Facture d'avril"]);
    assert_eq!(
        found(store, "facture", SearchScope::Mailbox(all)).await?,
        ["Facture d'avril"],
        "dans le dossier \\All lui-même, la copie est listée"
    );
    assert_eq!(found(store, "train", mine).await?, ["Billet de train"]);
    assert_eq!(
        found(store, "facture", SearchScope::Mailbox(archives)).await?,
        Vec::<String>::new()
    );
    let hit = store
        .search(request("facture", SearchScope::Mailbox(inbox)))
        .await?;
    let first = hit
        .items
        .first()
        .ok_or_else(|| StoreError("facture introuvable".into()))?;
    assert_eq!(
        (first.account, first.mailbox, first.uid),
        (account, inbox, uid(1))
    );
    assert_eq!(first.date, jan);
    assert_eq!(first.from, Some(Correspondent::new(ALICE.0, ALICE.1)));
    assert!(first.has_attachments && !first.flags.seen);
    assert!(first.conversation.is_some());
    assert_eq!(first.snippet, "", "texte du message pas encore connu");

    // --- Préfixe, casse, diacritiques ---
    assert_eq!(found(store, "FACT", mine).await?, ["Facture d'avril"]);
    assert_eq!(
        found(store, "reunion", mine).await?,
        ["Compte-rendu de la réunion", "Réunion de lundi"]
    );
    assert_eq!(found(store, "RÉU lun", mine).await?, ["Réunion de lundi"]);
    assert_eq!(
        found(store, "unio", mine).await?,
        Vec::<String>::new(),
        "préfixe seulement"
    );

    // --- Champs ---
    assert_eq!(
        found(store, "alice", mine).await?,
        [
            "Compte-rendu de la réunion",
            "Facture d'avril",
            "Réunion de lundi"
        ]
    );
    assert_eq!(found(store, "from:alice", mine).await?, ["Facture d'avril"]);
    assert_eq!(
        found(store, "to:alice", mine).await?,
        ["Compte-rendu de la réunion", "Réunion de lundi"],
        "À et Cc"
    );
    assert_eq!(
        found(store, "from:example.org", mine).await?,
        ["Billet de train", "Compte-rendu de la réunion"]
    );
    assert_eq!(
        found(store, "subject:réunion", mine).await?,
        ["Compte-rendu de la réunion", "Réunion de lundi"]
    );
    assert_eq!(
        found(store, "subject:carole", mine).await?,
        ["Carole arrive"]
    );

    // --- Pertinence : le sujet passe avant les destinataires ---
    assert_eq!(
        subjects(store, "carole", SearchScope::Mailbox(inbox)).await?,
        ["Carole arrive", "Réunion de lundi"]
    );

    // --- Phrases ---
    assert_eq!(
        found(store, "\"compte rendu\"", mine).await?,
        ["Compte-rendu de la réunion"]
    );
    assert_eq!(
        found(store, "\"rendu compte\"", mine).await?,
        Vec::<String>::new()
    );
    assert_eq!(
        found(store, "\"factur\"", mine).await?,
        Vec::<String>::new(),
        "une phrase n'est pas un préfixe"
    );
    assert_eq!(
        found(store, "compte-rendu", mine).await?,
        ["Compte-rendu de la réunion"]
    );

    // --- Négation ---
    assert_eq!(
        found(store, "réunion -compte", mine).await?,
        ["Réunion de lundi"]
    );
    assert_eq!(
        found(store, "-réunion -facture -train", mine).await?,
        ["Carole arrive"],
        "négations seules"
    );
    assert_eq!(
        found(store, "alice -from:alice -to:carole", mine).await?,
        ["Compte-rendu de la réunion"]
    );

    // --- Filtres ---
    assert_eq!(found(store, "is:unread", mine).await?, ["Facture d'avril"]);
    assert_eq!(
        found(store, "is:flagged", mine).await?,
        ["Réunion de lundi"]
    );
    assert_eq!(
        found(store, "has:attachment", mine).await?,
        ["Facture d'avril"]
    );
    assert_eq!(
        found(store, "is:read -is:flagged -has:attachment from:bob", mine).await?,
        ["Carole arrive"]
    );
    assert_eq!(
        subjects(store, "after:2026-01-20", mine).await?,
        [
            "Carole arrive",
            "Réunion de lundi",
            "Compte-rendu de la réunion"
        ],
        "sans texte : du plus récent au plus ancien ; after: inclut le jour"
    );
    assert_eq!(
        subjects(store, "before:2026-01-20", mine).await?,
        ["Facture d'avril", "Billet de train"],
        "before: exclut le jour"
    );
    // Fuseau : à UTC-14, le 20 janvier à 0 h UTC est encore le 19 (heure
    // locale), donc avant le 20.
    let mut shifted = request("before:2026-01-20", mine);
    shifted.utc_offset_seconds = -14 * 3600;
    assert_eq!(store.search(shifted).await?.items.len(), 3);

    // --- Requêtes vides ou sans mot ---
    assert_eq!(
        store.search(request("", mine)).await?,
        SearchPage::default()
    );
    assert_eq!(found(store, "-- ! ?", mine).await?, Vec::<String>::new());

    // --- La syntaxe du moteur n'est jamais interprétée ---
    for hostile in [
        "sender:bob",
        "subject : facture",
        "NEAR(facture avril)",
        "facture OR zzz",
        "facture AND zzz",
        "NOT facture",
        "facture*",
        "^facture",
        "\"facture",
        "facture\"",
        "(facture",
        "{subject}: facture",
        "subject:\"fact\" * ",
        "'",
        "%",
        "_",
        "\\",
        "facture'; DROP TABLE messages; --",
    ] {
        let page = store.search(request(hostile, mine)).await;
        assert!(page.is_ok(), "{hostile:?} : {page:?}");
    }
    assert_eq!(
        found(store, "sender:bob", mine).await?,
        Vec::<String>::new(),
        "pas de filtre de colonne du moteur"
    );
    assert_eq!(
        found(store, "facture OR zzz", mine).await?,
        Vec::<String>::new()
    );
    assert_eq!(
        found(store, "NOT facture", mine).await?,
        Vec::<String>::new()
    );
    assert_eq!(found(store, "facture*", mine).await?, ["Facture d'avril"]);
    assert_eq!(found(store, "%", mine).await?, Vec::<String>::new());

    // --- Texte des messages ---
    let blob = BlobId::parse(&sha256_hex(b"corps")).ok_or_else(|| StoreError("blob".into()))?;
    let body = StoredBody::Full(blob);
    let text = format!(
        "Bonjour Bob,\n\n{}les cerises sont mûres.\r\n{}",
        "bla ".repeat(100),
        "fin ".repeat(100)
    );
    store.set_body(inbox, uid(2), &body, Some(&text)).await?;
    assert_eq!(found(store, "cerises", mine).await?, ["Réunion de lundi"]);
    assert_eq!(found(store, "MURES", mine).await?, ["Réunion de lundi"]);
    assert_eq!(
        found(store, "\"cerises sont\"", mine).await?,
        ["Réunion de lundi"]
    );
    assert_eq!(
        found(store, "subject:cerises", mine).await?,
        Vec::<String>::new(),
        "le texte n'est cherché que sans champ"
    );
    let hit = store.search(request("cerises", mine)).await?;
    let snippet = hit
        .items
        .first()
        .map(|h| h.snippet.clone())
        .unwrap_or_default();
    assert!(
        snippet.contains("cerises"),
        "extrait autour du terme : {snippet:?}"
    );
    assert!(snippet.chars().count() <= MAX_SNIPPET_CHARS, "{snippet:?}");
    assert!(!snippet.contains(['\n', '\r']), "{snippet:?}");
    let hit = store.search(request("is:flagged", mine)).await?;
    let snippet = hit
        .items
        .first()
        .map(|h| h.snippet.clone())
        .unwrap_or_default();
    assert!(
        snippet.starts_with("Bonjour Bob, bla"),
        "sans texte cherché : le début ({snippet:?})"
    );
    // Corps réenregistré sans texte : le texte reste.
    store.set_body(inbox, uid(2), &body, None).await?;
    assert_eq!(found(store, "cerises", mine).await?, ["Réunion de lundi"]);
    // Le texte compte moins que le sujet.
    store
        .set_body(inbox, uid(3), &body, Some("On se voit lundi ?"))
        .await?;
    assert_eq!(
        subjects(store, "lundi", SearchScope::Mailbox(inbox)).await?,
        ["Réunion de lundi", "Carole arrive"]
    );
    // Texte vidé.
    store.set_body(inbox, uid(2), &body, Some("")).await?;
    assert_eq!(found(store, "cerises", mine).await?, Vec::<String>::new());
    // Texte borné : ce qui dépasse n'est pas indexé.
    let huge = format!(
        "{} débutmot {} finmot",
        "a".repeat(10),
        "b ".repeat(MAX_INDEXED_TEXT_CHARS / 2)
    );
    store.set_body(inbox, uid(2), &body, Some(&huge)).await?;
    assert_eq!(found(store, "débutmot", mine).await?, ["Réunion de lundi"]);
    assert_eq!(found(store, "finmot", mine).await?, Vec::<String>::new());
    store.set_body(inbox, uid(2), &body, Some("")).await?;

    // --- Pagination ---
    let mut paged = request("-zzz", mine);
    paged.limit = 2;
    let page1 = store.search(paged.clone()).await?;
    assert_eq!(page1.items.len(), 2);
    assert_eq!(page1.next, Some(2));
    paged.offset = 2;
    let page2 = store.search(paged.clone()).await?;
    assert_eq!(page2.items.len(), 2);
    assert_eq!(page2.next, Some(4));
    paged.offset = 4;
    let page3 = store.search(paged.clone()).await?;
    assert_eq!(page3.items.len(), 1);
    assert_eq!(page3.next, None);
    let mut seen: Vec<_> = [page1, page2, page3]
        .iter()
        .flat_map(|p| p.items.iter().map(|h| h.id))
        .collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 5, "pages disjointes");
    paged.offset = MAX_SEARCH_RESULTS;
    assert_eq!(store.search(paged.clone()).await?, SearchPage::default());
    paged.offset = MAX_SEARCH_RESULTS - 1;
    paged.limit = 10;
    assert_eq!(store.search(paged.clone()).await?.next, None, "plafond");
    paged.offset = 0;
    paged.limit = MAX_PAGE_SIZE + 1_000;
    assert_eq!(store.search(paged.clone()).await?.items.len(), 5);
    paged.limit = 0;
    assert!(store.search(paged).await?.items.is_empty());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_contracts::InMemoryMailStore;

    #[tokio::test]
    async fn in_memory_search_passes_contract() -> Result<(), StoreError> {
        let (a, b) = (
            AccountId::new(1).map_err(|e| StoreError(e.to_string()))?,
            AccountId::new(2).map_err(|e| StoreError(e.to_string()))?,
        );
        search_index_contract(&InMemoryMailStore::default(), a, b).await
    }
}
