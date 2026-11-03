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
            text: &norelly_render::search_text::MimeTextExtractor,
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
            .sync_all(&mut session, ext, |id| changed.push(id))
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
