use norelly_domain::fetch_policy::FetchPolicy;
use norelly_domain::mailbox::{Flags, MailboxId, Uid};
use norelly_domain::ports::{BlobStore, MailStore, MessageListQuery, StoredBody};
use norelly_test_support::fake_imap::{FakeImap, FakeImapSession};
use norelly_test_support::mail_contracts::{InMemoryBlobStore, InMemoryMailStore};

use super::*;

fn account() -> AccountId {
    AccountId::new(1).unwrap_or_else(|_| panic!("id"))
}

fn raw(n: u32) -> Vec<u8> {
    format!(
        "From: Expéditeur {n} <e{n}@example.com>\r\nSubject: Message {n}\r\n\
         Message-ID: <{n}@example.com>\r\n\r\nCorps du message {n}.\r\n"
    )
    .into_bytes()
}

fn seen() -> Flags {
    Flags::from_tokens(["\\Seen"])
}

struct World {
    imap: FakeImap,
    store: InMemoryMailStore,
    blobs: InMemoryBlobStore,
    policy: FetchPolicy,
}

impl World {
    fn new() -> Self {
        let imap = FakeImap::new();
        imap.create_mailbox("Sent", &["\\Sent"]);
        imap.create_mailbox("Archives", &[]);
        imap.create_mailbox("[Virtuel]", &["\\Noselect"]);
        Self {
            imap,
            store: InMemoryMailStore::default(),
            blobs: InMemoryBlobStore::default(),
            policy: FetchPolicy::default(),
        }
    }

    fn append(&self, mailbox: &str, n: u32, flags: Flags) -> Uid {
        self.imap
            .append(mailbox, &raw(n), flags, 1_700_000_000 + i64::from(n))
            .unwrap_or_else(|| panic!("boîte {mailbox}"))
    }

    fn use_case(&self) -> SyncAccount<'_, InMemoryMailStore, InMemoryBlobStore> {
        SyncAccount {
            account: account(),
            store: &self.store,
            blobs: &self.blobs,
            policy: self.policy,
            text: std::sync::Arc::new(norelly_render::search_text::MimeTextExtractor),
        }
    }

    async fn session(&self) -> Result<(FakeImapSession, SyncExtensions), SyncFailure> {
        let mut session = self.imap.session()?;
        let ext = session.enable_sync_extensions().await?;
        Ok((session, ext))
    }

    /// Synchronisation complète ; renvoie les boîtes modifiées.
    async fn sync(&self) -> Result<Vec<MailboxId>, SyncFailure> {
        let (mut session, ext) = self.session().await?;
        let mut changed = Vec::new();
        self.use_case()
            .sync_all(&mut session, ext, |m, _| changed.push(m.id))
            .await?;
        Ok(changed)
    }

    async fn id(&self, name: &str) -> MailboxId {
        use norelly_domain::ports::FolderTreeQuery;
        self.store
            .tree(account())
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .find(|n| n.name.as_str() == name)
            .map(|n| n.id)
            .unwrap_or_else(|| panic!("boîte {name} absente localement"))
    }

    /// Vérifie que chaque boîte locale reflète exactement le serveur, avec
    /// tous les corps.
    async fn assert_converged(&self, names: &[&str]) {
        for name in names {
            let id = self.id(name).await;
            let local = self
                .store
                .local_mailbox(id)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(local.messages, self.imap.flags_of(name), "boîte {name}");
            assert_eq!(
                local.state.uid_validity,
                self.imap.uid_validity(name),
                "{name}"
            );
            for uid in local.messages.keys() {
                let body = self.store.body_of(id, *uid);
                assert!(
                    matches!(body, Some(StoredBody::Full(_) | StoredBody::Partial { .. })),
                    "{name} UID {uid} : corps {body:?}"
                );
            }
        }
    }
}

#[tokio::test]
async fn initial_sync_fetches_folders_messages_and_bodies() -> Result<(), SyncFailure> {
    let w = World::new();
    for n in 1..=3 {
        w.append("INBOX", n, if n == 1 { seen() } else { Flags::default() });
    }
    w.append("Sent", 10, seen());
    let changed = w.sync().await?;
    assert_eq!(changed.len(), 2, "INBOX et Sent");
    w.assert_converged(&["INBOX", "Sent", "Archives"]).await;

    use norelly_domain::ports::FolderTreeQuery;
    let tree = w.store.tree(account()).await?;
    let summary: Vec<(&str, u32, u32, bool)> = tree
        .iter()
        .map(|n| (n.name.as_str(), n.total, n.unread, n.selectable))
        .collect();
    assert_eq!(
        summary,
        [
            ("INBOX", 3, 2, true),
            ("Sent", 1, 0, true),
            ("Archives", 0, 0, true),
            ("[Virtuel]", 0, 0, false)
        ]
    );
    let page = w.store.page(w.id("INBOX").await, None, 10).await?;
    assert_eq!(
        page.items
            .iter()
            .map(|i| i.subject.as_str())
            .collect::<Vec<_>>(),
        ["Message 3", "Message 2", "Message 1"]
    );
    // Corps : le message brut complet est retrouvé dans le stockage.
    let inbox = w.id("INBOX").await;
    let Some(StoredBody::Full(blob)) = w
        .store
        .body_of(inbox, Uid::new(2).unwrap_or_else(|| panic!()))
    else {
        panic!("corps complet attendu");
    };
    assert_eq!(w.blobs.get(account(), &blob).await?, Some(raw(2)));
    // Rien à refaire.
    assert!(w.sync().await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn incremental_sync_with_every_extension_set() -> Result<(), SyncFailure> {
    for (condstore, qresync) in [(true, true), (true, false), (false, false)] {
        let w = World::new();
        w.imap.set_extensions(condstore, qresync, true);
        let uids: Vec<Uid> = (1..=5)
            .map(|n| w.append("INBOX", n, Flags::default()))
            .collect();
        w.sync().await?;
        // Changements côté serveur.
        w.imap.set_flags("INBOX", uids[0], seen());
        w.imap.expunge("INBOX", uids[1]);
        w.imap.expunge("INBOX", uids[4]);
        w.append("INBOX", 6, Flags::default());
        w.append("Archives", 7, seen());
        let changed = w.sync().await?;
        assert_eq!(changed.len(), 2, "condstore={condstore} qresync={qresync}");
        w.assert_converged(&["INBOX", "Archives"]).await;
        assert!(w.sync().await?.is_empty(), "idempotent");
    }
    Ok(())
}

#[tokio::test]
async fn uidvalidity_change_triggers_full_resync() -> Result<(), SyncFailure> {
    let w = World::new();
    for n in 1..=4 {
        w.append("Archives", n, Flags::default());
    }
    w.sync().await?;
    let before = w.store.local_mailbox(w.id("Archives").await).await?;
    w.imap
        .expunge("Archives", Uid::new(1).unwrap_or_else(|| panic!()));
    w.imap.recreate_mailbox("Archives");
    w.sync().await?;
    let after = w.store.local_mailbox(w.id("Archives").await).await?;
    assert_ne!(before.state.uid_validity, after.state.uid_validity);
    assert_eq!(
        after.messages.keys().map(|u| u.get()).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    w.assert_converged(&["Archives"]).await;
    Ok(())
}

#[tokio::test]
async fn deleted_server_mailbox_is_removed_locally() -> Result<(), SyncFailure> {
    let w = World::new();
    w.append("Archives", 1, Flags::default());
    w.sync().await?;
    w.imap.delete_mailbox("Archives");
    w.sync().await?;
    use norelly_domain::ports::FolderTreeQuery;
    let names: Vec<String> = w
        .store
        .tree(account())
        .await?
        .iter()
        .map(|n| n.name.to_string())
        .collect();
    assert!(!names.iter().any(|n| n == "Archives"), "{names:?}");
    Ok(())
}

#[tokio::test]
async fn large_messages_are_fetched_partially() -> Result<(), SyncFailure> {
    let mut w = World::new();
    w.policy = FetchPolicy {
        attachment_threshold: 10,
    };
    w.append("INBOX", 1, Flags::default());
    w.sync().await?;
    let inbox = w.id("INBOX").await;
    let body = w
        .store
        .body_of(inbox, Uid::new(1).unwrap_or_else(|| panic!()));
    let Some(StoredBody::Partial { header, .. }) = body else {
        panic!("corps partiel attendu : {body:?}");
    };
    let header = w.blobs.get(account(), &header).await?.unwrap_or_default();
    assert!(header.starts_with(b"From: "));
    assert!(
        !header.windows(5).any(|x| x == b"Corps"),
        "le corps n'est pas téléchargé"
    );
    Ok(())
}

/// Une coupure à n'importe quelle étape, puis une reprise : l'état local
/// converge vers celui du serveur.
#[tokio::test]
async fn interrupted_sync_converges_after_resume() -> Result<(), SyncFailure> {
    // Nombre d'opérations d'une synchronisation complète de référence.
    let total = {
        let w = World::new();
        for n in 1..=30 {
            w.append("INBOX", n, Flags::default());
        }
        let start = w.imap.operations();
        w.sync().await?;
        w.imap.operations() - start
    };
    assert!(total > 10);
    for cut in 0..total {
        let w = World::new();
        for n in 1..=30 {
            w.append(
                "INBOX",
                n,
                if n % 2 == 0 { seen() } else { Flags::default() },
            );
        }
        for n in 31..=35 {
            w.append("Archives", n, Flags::default());
        }
        w.imap.cut_after(Some(cut));
        let first = w.sync().await;
        assert!(first.is_err(), "coupure après {cut} opérations");
        // Changements pendant la coupure.
        w.imap.cut_after(None);
        w.imap
            .expunge("INBOX", Uid::new(3).unwrap_or_else(|| panic!()));
        w.imap.set_flags(
            "INBOX",
            Uid::new(4).unwrap_or_else(|| panic!()),
            Flags::default(),
        );
        w.append("INBOX", 36, Flags::default());
        w.sync().await?;
        w.assert_converged(&["INBOX", "Archives", "Sent"]).await;
    }
    Ok(())
}

#[tokio::test]
async fn downloaded_bodies_are_searchable() -> Result<(), SyncFailure> {
    use norelly_domain::ports::{SearchIndex, SearchRequest, SearchScope};
    let w = World::new();
    w.append("INBOX", 1, seen());
    w.sync().await?;
    let page = w
        .store
        .search(SearchRequest {
            query: norelly_domain::search::parse_query("corps").unwrap_or_default(),
            scope: SearchScope::All,
            utc_offset_seconds: 0,
            offset: 0,
            limit: 10,
        })
        .await?;
    let found: Vec<(String, String)> = page
        .items
        .into_iter()
        .map(|h| (h.subject, h.snippet))
        .collect();
    assert_eq!(
        found,
        [("Message 1".to_owned(), "Corps du message 1.".to_owned())]
    );
    Ok(())
}

#[derive(Debug)]
struct Stuck;

impl norelly_domain::ports::SearchTextExtractor for Stuck {
    fn searchable_text(
        &self,
        _: &FetchedBody,
        _: Option<&norelly_domain::fetch_policy::MimePart>,
    ) -> String {
        std::thread::sleep(std::time::Duration::from_secs(2));
        "trop tard".into()
    }
}

/// L'extraction du texte d'un message
/// hostile ne bloque pas le fil asynchrone et s'arrête au délai maximal.
#[tokio::test]
async fn text_extraction_is_bounded_in_time() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    let body = FetchedBody::Full(raw(1));
    let real: Arc<dyn norelly_domain::ports::SearchTextExtractor> =
        Arc::new(norelly_render::search_text::MimeTextExtractor);
    let text = extract_text(&real, &body, None, Duration::from_secs(5)).await;
    assert!(text.contains("Corps du message 1."), "{text:?}");
    let stuck: Arc<dyn norelly_domain::ports::SearchTextExtractor> = Arc::new(Stuck);
    let started = Instant::now();
    let text = extract_text(&stuck, &body, None, Duration::from_millis(100)).await;
    assert_eq!(text, "");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn unread_arrivals_are_reported_except_on_first_sync() -> Result<(), SyncFailure> {
    let w = World::new();
    w.append("INBOX", 1, Flags::default());
    let (mut session, ext) = w.session().await?;
    let mut first = Vec::new();
    w.use_case()
        .sync_all(&mut session, ext, |m, r| {
            first.push((m.mailbox.name.to_string(), r.arrivals.clone()))
        })
        .await?;
    assert!(
        first.iter().all(|(_, a)| a.is_none()),
        "première synchronisation : {first:?}"
    );
    w.append("INBOX", 2, Flags::default());
    w.append("INBOX", 3, seen());
    let mut second = Vec::new();
    w.use_case()
        .sync_all(&mut session, ext, |m, r| {
            second.push((m.mailbox.name.to_string(), r.arrivals.clone()))
        })
        .await?;
    let inbox = second
        .iter()
        .find(|(n, _)| n == "INBOX")
        .and_then(|(_, a)| a.clone())
        .unwrap_or_else(|| panic!("arrivées non signalées : {second:?}"));
    assert_eq!(inbox.count, 1, "seul le message non lu");
    assert_eq!(
        inbox.latest.map(|l| l.subject),
        Some("Message 2".to_owned())
    );
    Ok(())
}

/// Étape observée, sous forme comparable.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    Folders(usize),
    Progress(usize, usize),
    Headers(String),
    HeadersDone(String),
    Bodies(String),
    Done(String),
}

#[tokio::test]
async fn folders_headers_and_progress_are_reported_as_they_arrive() -> Result<(), SyncFailure> {
    let w = World::new();
    w.append("INBOX", 1, Flags::default());
    w.append("Archives", 2, Flags::default());
    let (mut session, ext) = w.session().await?;
    let mut seen = Vec::new();
    w.use_case()
        .sync_all_observed(&mut session, ext, |step| {
            seen.push(match step {
                SyncStep::Folders(all) => Seen::Folders(all.len()),
                SyncStep::Progress { done, total } => Seen::Progress(done, total),
                SyncStep::Headers(m) => Seen::Headers(m.mailbox.name.to_string()),
                SyncStep::HeadersDone(m) => Seen::HeadersDone(m.mailbox.name.to_string()),
                SyncStep::Bodies(m) => Seen::Bodies(m.mailbox.name.to_string()),
                SyncStep::MailboxDone(m, _) => Seen::Done(m.mailbox.name.to_string()),
            });
        })
        .await?;
    // Trois boîtes sélectionnables (INBOX, Sent, Archives) sur quatre.
    assert_eq!(seen.first(), Some(&Seen::Folders(4)), "{seen:?}");
    assert_eq!(seen.get(1), Some(&Seen::Progress(0, 3)), "{seen:?}");
    assert_eq!(seen.last(), Some(&Seen::Progress(3, 3)), "{seen:?}");
    for name in ["INBOX", "Archives"] {
        let headers = seen
            .iter()
            .position(|s| *s == Seen::Headers(name.to_owned()));
        let all_headers = seen
            .iter()
            .position(|s| *s == Seen::HeadersDone(name.to_owned()));
        let bodies = seen
            .iter()
            .position(|s| *s == Seen::Bodies(name.to_owned()));
        let done = seen.iter().position(|s| *s == Seen::Done(name.to_owned()));
        assert!(
            matches!(
                (headers, all_headers, bodies, done),
                (Some(h), Some(a), Some(b), Some(d)) if h < a && a < b && b < d
            ),
            "en-têtes signalés, terminés, puis corps, avant la fin de {name} : {seen:?}"
        );
    }
    // Dossier sans nouveauté (Sent) : ni lot ni fin des en-têtes.
    assert!(
        !seen.contains(&Seen::HeadersDone("Sent".to_owned())),
        "{seen:?}"
    );
    // Progression croissante, un pas par boîte.
    let progress: Vec<usize> = seen
        .iter()
        .filter_map(|s| match s {
            Seen::Progress(done, _) => Some(*done),
            _ => None,
        })
        .collect();
    assert_eq!(progress, vec![0, 1, 2, 3]);
    Ok(())
}

#[test]
fn a_reset_mailbox_counts_as_changed() {
    let report = MailboxReport {
        reset: true,
        ..MailboxReport::default()
    };
    assert!(report.changed());
}

#[tokio::test]
async fn inbox_first_and_virtual_folders_last() -> Result<(), SyncFailure> {
    let imap = FakeImap::new();
    // Ordre du serveur : « Tous » (\All) avant « Archives ».
    imap.create_mailbox("Tous", &["\\All"]);
    imap.create_mailbox("Archives", &[]);
    let w = World {
        imap,
        store: InMemoryMailStore::default(),
        blobs: InMemoryBlobStore::default(),
        policy: FetchPolicy::default(),
    };
    w.append("Tous", 1, Flags::default());
    w.append("Archives", 2, Flags::default());
    w.append("INBOX", 3, Flags::default());
    let (mut session, ext) = w.session().await?;
    let mut order = Vec::new();
    w.use_case()
        .sync_all_observed(&mut session, ext, |step| {
            if let SyncStep::Headers(m) = step {
                order.push(m.mailbox.name.to_string());
            }
        })
        .await?;
    assert_eq!(order, vec!["INBOX", "Archives", "Tous"]);
    Ok(())
}
