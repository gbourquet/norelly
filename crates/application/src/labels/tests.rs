use norelly_domain::conversation::ConversationScope;
use norelly_domain::fetch_policy::FetchPolicy;
use norelly_domain::labels::{LabelDisplay, MAX_REWRITE_ATTEMPTS, REWRITE_BATCH};
use norelly_domain::mailbox::{Flags, MailboxId, Uid};
use norelly_domain::ports::{
    ConversationQuery, MessageId, MessageListQuery, RemoteMailboxSession, StoredMailbox,
};
use norelly_domain::sync::SyncExtensions;
use norelly_test_support::fake_imap::{FakeImap, FakeImapSession};
use norelly_test_support::mail_contracts::{InMemoryBlobStore, InMemoryMailStore};
use norelly_test_support::session_contract::contract_message;

use super::*;
use crate::sync::SyncAccount;

fn account() -> AccountId {
    AccountId::new(1).unwrap_or_else(|_| panic!("compte"))
}

fn kw(raw: &str) -> Keyword {
    Keyword::parse(raw).unwrap_or_else(|| panic!("mot-clé {raw}"))
}

fn name(raw: &str) -> LabelName {
    LabelName::parse(raw).unwrap_or_else(|e| panic!("{e}"))
}

fn color(raw: &str) -> LabelColor {
    LabelColor::parse(raw).unwrap_or_else(|| panic!("couleur"))
}

struct World {
    imap: FakeImap,
    store: InMemoryMailStore,
    blobs: InMemoryBlobStore,
    session: FakeImapSession,
    ext: SyncExtensions,
    boxes: Vec<StoredMailbox>,
}

/// INBOX : `inbox` messages portant `Vieux` (le premier aussi `Autre`) ;
/// Archives : un message portant `Vieux`.
async fn world(inbox: u32) -> World {
    let imap = FakeImap::new();
    imap.create_mailbox("Archives", &["\\Archive"]);
    for n in 1..=inbox {
        let tokens: &[&str] = if n == 1 {
            &["Vieux", "Autre"]
        } else {
            &["Vieux"]
        };
        imap.append(
            "INBOX",
            &contract_message(n),
            Flags::from_tokens(tokens.iter().copied()),
            100 + i64::from(n),
        );
    }
    imap.append(
        "Archives",
        &contract_message(900),
        Flags::from_tokens(["Vieux"]),
        50,
    );
    let mut session = imap.session().unwrap_or_else(|e| panic!("{e}"));
    let ext = session
        .enable_sync_extensions()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut w = World {
        imap,
        store: InMemoryMailStore::default(),
        blobs: InMemoryBlobStore::default(),
        session,
        ext,
        boxes: Vec::new(),
    };
    w.sync().await;
    w
}

impl World {
    async fn sync(&mut self) {
        let use_case = SyncAccount {
            account: account(),
            store: &self.store,
            blobs: &self.blobs,
            policy: FetchPolicy::default(),
            text: std::sync::Arc::new(norelly_render::search_text::MimeTextExtractor),
        };
        self.boxes = use_case
            .sync_all(&mut self.session, self.ext, |_, _| {})
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    }

    fn box_id(&self, name: &str) -> MailboxId {
        self.boxes
            .iter()
            .find(|m| m.mailbox.name.as_str() == name)
            .map(|m| m.id)
            .unwrap_or_else(|| panic!("boîte {name}"))
    }

    async fn labels(&self) -> Vec<Label> {
        ListLabels { store: &self.store }
            .execute(account())
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    }

    async fn keywords(&self) -> Vec<String> {
        self.labels()
            .await
            .into_iter()
            .map(|l| l.keyword.as_str().to_owned())
            .collect()
    }

    /// Mots-clés locaux des messages d'une boîte, par UID.
    async fn local(&self, mailbox: &str) -> Vec<(u32, Vec<String>)> {
        let mut items = Vec::new();
        let mut cursor = None;
        loop {
            let page = self
                .store
                .page(self.box_id(mailbox), cursor, 200)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            items.extend(page.items.into_iter().map(|i| {
                (
                    i.uid.get(),
                    i.flags
                        .keywords()
                        .iter()
                        .map(|k| k.as_str().to_owned())
                        .collect(),
                )
            }));
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        items.sort();
        items
    }

    /// UID du serveur qui portent `keyword` dans `mailbox`.
    fn server(&self, mailbox: &str, keyword: &str) -> Vec<u32> {
        self.imap
            .flags_of(mailbox)
            .iter()
            .filter(|(_, f)| f.keywords().contains(&kw(keyword)))
            .map(|(u, _)| u.get())
            .collect()
    }

    async fn rewrite(&mut self) -> Result<RewriteReport, ReplayFailure> {
        ReplayRewrites {
            account: account(),
            store: &self.store,
        }
        .execute(&mut self.session)
        .await
    }

    async fn pending(&self) -> usize {
        self.store
            .label_rewrites(account())
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .len()
    }
}

#[tokio::test]
async fn the_list_merges_saved_labels_and_keywords_seen_on_the_server() {
    let w = world(2).await;
    assert_eq!(w.keywords().await, ["Autre", "Vieux"]);
    let created = CreateLabel { store: &w.store }
        .execute(account(), &name("À faire"), Some(color("#1565C0")))
        .await;
    assert_eq!(created, Ok(kw("&AMAAIA-faire")));
    let labels = w.labels().await;
    let a_faire = labels.iter().find(|l| l.keyword == kw("&AMAAIA-faire"));
    assert_eq!(
        a_faire.map(|l| (&l.display, l.color)),
        Some((
            &LabelDisplay::Named("À faire".into()),
            Some(color("#1565c0"))
        ))
    );
    assert_eq!(
        CreateLabel { store: &w.store }
            .execute(account(), &name("vieux"), None)
            .await,
        Err(LabelFailure::Invalid(LabelError::AlreadyExists))
    );
}

#[tokio::test]
async fn colors_are_set_on_listed_labels_only() {
    let w = world(1).await;
    let set = SetLabelColor { store: &w.store };
    assert_eq!(
        set.execute(account(), &kw("Vieux"), Some(color("#ffeb3b")))
            .await,
        Ok(())
    );
    assert_eq!(
        w.labels()
            .await
            .iter()
            .find(|l| l.keyword == kw("Vieux"))
            .and_then(|l| l.color),
        Some(color("#ffeb3b"))
    );
    assert_eq!(
        set.execute(account(), &kw("Absent"), None).await,
        Err(LabelFailure::Invalid(LabelError::Unknown))
    );
}

#[tokio::test]
async fn deleting_without_removal_hides_the_label_until_it_is_created_again() {
    let w = world(1).await;
    DeleteLabel { store: &w.store }
        .execute(account(), &kw("Vieux"), false)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(w.keywords().await, ["Autre"]);
    assert_eq!(
        ListLabels { store: &w.store }
            .hidden(account())
            .await
            .unwrap_or_else(|e| panic!("{e}")),
        BTreeSet::from([kw("Vieux")])
    );
    // Rien ne change sur les messages.
    assert_eq!(w.pending().await, 0);
    assert_eq!(w.local("INBOX").await[0].1, ["Autre", "Vieux"]);
    // Créée de nouveau sous le même nom : de retour.
    CreateLabel { store: &w.store }
        .execute(account(), &name("Vieux"), None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(w.keywords().await, ["Autre", "Vieux"]);
    assert_eq!(
        DeleteLabel { store: &w.store }
            .execute(account(), &kw("Absent"), false)
            .await,
        Err(LabelFailure::Invalid(LabelError::Unknown))
    );
}

#[tokio::test]
async fn renaming_rewrites_every_message_of_the_server_in_batches() {
    let mut w = world(u32::try_from(REWRITE_BATCH).unwrap_or(500) + 1).await;
    SetLabelColor { store: &w.store }
        .execute(account(), &kw("Vieux"), Some(color("#1565c0")))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let renamed = RenameLabel { store: &w.store }
        .execute(account(), &kw("Vieux"), &name("Neuf"))
        .await;
    assert_eq!(renamed, Ok(kw("Neuf")));
    // Tout de suite en local, couleur comprise.
    let first = w
        .local("INBOX")
        .await
        .into_iter()
        .find(|(uid, _)| *uid == 1);
    assert_eq!(
        first.map(|(_, k)| k),
        Some(vec!["Autre".to_owned(), "Neuf".to_owned()])
    );
    assert_eq!(w.local("Archives").await[0].1, ["Neuf"]);
    let labels = w.labels().await;
    assert_eq!(
        labels
            .iter()
            .map(|l| (l.keyword.as_str(), l.color))
            .collect::<Vec<_>>(),
        [("Autre", None), ("Neuf", Some(color("#1565c0")))]
    );
    // Puis sur le serveur, par lots bornés, dans tous les dossiers.
    let report = w.rewrite().await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.completed, 1);
    assert!(w.server("INBOX", "Vieux").is_empty());
    assert!(w.server("Archives", "Vieux").is_empty());
    assert_eq!(w.server("INBOX", "Neuf").len(), REWRITE_BATCH + 1);
    assert_eq!(w.server("Archives", "Neuf"), [1]);
    assert!(
        w.imap.keyword_stores().iter().all(|n| *n <= REWRITE_BATCH),
        "{:?}",
        w.imap.keyword_stores()
    );
    assert_eq!(w.pending().await, 0);
    // La synchronisation suivante confirme.
    w.sync().await;
    assert_eq!(w.keywords().await, ["Autre", "Neuf"]);
}

#[tokio::test]
async fn an_interrupted_rename_resumes_and_ends_consistent() {
    let mut w = world(3).await;
    RenameLabel { store: &w.store }
        .execute(account(), &kw("Vieux"), &name("Neuf"))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    // Coupure au milieu : après la première boîte.
    w.imap.cut_after(Some(5));
    assert!(w.rewrite().await.is_err());
    assert_eq!(w.pending().await, 1, "réécriture gardée");
    // Pendant la réécriture, l'ancien nom n'est pas listé.
    w.imap.cut_after(None);
    w.session = w.imap.session().unwrap_or_else(|e| panic!("{e}"));
    w.sync().await;
    assert_eq!(w.keywords().await, ["Autre", "Neuf"]);
    let report = w.rewrite().await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.completed, 1);
    for mailbox in ["INBOX", "Archives"] {
        assert!(w.server(mailbox, "Vieux").is_empty(), "{mailbox}");
    }
    assert_eq!(w.server("INBOX", "Neuf"), [1, 2, 3]);
    assert_eq!(w.server("Archives", "Neuf"), [1]);
}

#[tokio::test]
async fn deleting_with_removal_clears_the_keyword_everywhere() {
    let mut w = world(2).await;
    DeleteLabel { store: &w.store }
        .execute(account(), &kw("Vieux"), true)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(w.keywords().await, ["Autre"]);
    assert_eq!(w.local("INBOX").await[1].1, Vec::<String>::new());
    w.rewrite().await.unwrap_or_else(|e| panic!("{e}"));
    assert!(w.server("INBOX", "Vieux").is_empty());
    assert!(w.server("Archives", "Vieux").is_empty());
    assert_eq!(w.server("INBOX", "Autre"), [1], "autres mots-clés intacts");
}

#[tokio::test]
async fn a_folder_refusing_the_new_name_is_left_as_is() {
    let mut w = world(1).await;
    w.imap
        .set_permanent_flags(Some(&["\\Seen", "Vieux", "Autre"]));
    RenameLabel { store: &w.store }
        .execute(account(), &kw("Vieux"), &name("Neuf"))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let report = w.rewrite().await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.completed, 1);
    assert_eq!(w.server("INBOX", "Vieux"), [1], "laissé tel quel");
    assert!(w.server("INBOX", "Neuf").is_empty());
}

#[tokio::test]
async fn a_rewrite_failing_forever_is_abandoned() {
    let mut w = world(1).await;
    RenameLabel { store: &w.store }
        .execute(account(), &kw("Vieux"), &name("Neuf"))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let ids = w
        .store
        .label_rewrites(account())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    for _ in 0..MAX_REWRITE_ATTEMPTS {
        w.store
            .claim_rewrite(account(), ids[0].0)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let report = w.rewrite().await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((report.completed, report.abandoned), (0, 1));
    assert_eq!(w.pending().await, 0);
    assert_eq!(w.server("INBOX", "Vieux"), [1], "rien envoyé");
}

#[tokio::test]
async fn renaming_is_validated_by_the_domain() {
    let w = world(1).await;
    let rename = RenameLabel { store: &w.store };
    assert_eq!(
        rename
            .execute(account(), &kw("Vieux"), &name("autre"))
            .await,
        Err(LabelFailure::Invalid(LabelError::AlreadyExists))
    );
    assert_eq!(
        rename.execute(account(), &kw("Absent"), &name("X")).await,
        Err(LabelFailure::Invalid(LabelError::Unknown))
    );
    assert_eq!(w.pending().await, 0);
}

#[tokio::test]
async fn the_label_menu_of_a_selection() {
    let w = world(2).await;
    let inbox = w.box_id("INBOX");
    let conversations: Vec<_> = w
        .store
        .conversations(ConversationScope::Mailbox(inbox), None, 10)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .items
        .into_iter()
        .map(|c| c.id)
        .collect();
    let menu = LabelMenuFor { store: &w.store }
        .execute(&conversations, &ConversationScope::Mailbox(inbox))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(menu.menu.can_create);
    assert_eq!(
        menu.menu
            .choices
            .iter()
            .map(|c| (c.keyword.as_str(), c.checked))
            .collect::<Vec<_>>(),
        [("Autre", false), ("Vieux", true)]
    );
    assert_eq!(menu.accounts, [account()]);
    assert_eq!(menu.labels.len(), 2);
    let _ = MessageId::new(1);
    let _ = Uid::new(1);
}
