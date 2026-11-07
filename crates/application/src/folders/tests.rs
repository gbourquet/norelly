use norelly_domain::ports::RemoteMailboxSession;
use norelly_test_support::fake_imap::FakeImap;

use super::*;

fn name(raw: &str) -> MailboxName {
    MailboxName::parse(raw).unwrap_or_else(|e| panic!("{raw} : {e}"))
}

async fn names(imap: &FakeImap) -> Vec<String> {
    let mut session = imap.session().unwrap_or_else(|e| panic!("{e}"));
    session
        .list_mailboxes()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .map(|m| m.name.as_str().to_owned())
        .collect()
}

#[tokio::test]
async fn creates_the_folder_on_the_server() {
    let imap = FakeImap::new();
    let mut session = imap.session().unwrap_or_else(|e| panic!("{e}"));
    let created = CreateFolder {
        label: " Factures ",
        parent: None,
    }
    .execute(&mut session)
    .await;
    assert_eq!(created, Ok(name("Factures")));
    assert!(names(&imap).await.contains(&"Factures".to_owned()));
}

#[tokio::test]
async fn creates_a_subfolder_under_its_parent() {
    let imap = FakeImap::new();
    imap.create_mailbox("Projets", &[]);
    let mut session = imap.session().unwrap_or_else(|e| panic!("{e}"));
    let parent = name("Projets");
    let created = CreateFolder {
        label: "2027",
        parent: Some(&parent),
    }
    .execute(&mut session)
    .await;
    assert_eq!(created, Ok(name("Projets/2027")));
    assert!(names(&imap).await.contains(&"Projets/2027".to_owned()));
}

#[tokio::test]
async fn name_decided_from_the_current_server_list() {
    // Créé par un autre client depuis la dernière synchronisation.
    let imap = FakeImap::new();
    imap.create_mailbox("Factures", &[]);
    let mut session = imap.session().unwrap_or_else(|e| panic!("{e}"));
    let created = CreateFolder {
        label: "factures",
        parent: None,
    }
    .execute(&mut session)
    .await;
    assert_eq!(
        created,
        Err(FolderFailure::Invalid(NewFolderError::AlreadyExists))
    );
}

#[tokio::test]
async fn invalid_name_sends_nothing() {
    let imap = FakeImap::new();
    let mut session = imap.session().unwrap_or_else(|e| panic!("{e}"));
    let before = imap.operations();
    let created = CreateFolder {
        label: "a/b",
        parent: None,
    }
    .execute(&mut session)
    .await;
    assert_eq!(
        created,
        Err(FolderFailure::Invalid(NewFolderError::ContainsDelimiter(
            '/'
        )))
    );
    // Seule la liste a été lue.
    assert_eq!(imap.operations(), before + 1);
    assert_eq!(names(&imap).await, ["INBOX"]);
}

#[tokio::test]
async fn connection_loss_is_reported() {
    let imap = FakeImap::new();
    let mut session = imap.session().unwrap_or_else(|e| panic!("{e}"));
    imap.cut_after(Some(1));
    let created = CreateFolder {
        label: "Factures",
        parent: None,
    }
    .execute(&mut session)
    .await;
    assert_eq!(
        created,
        Err(FolderFailure::Connection(ConnectionError::Closed))
    );
    imap.cut_after(None);
    assert_eq!(names(&imap).await, ["INBOX"]);
}
