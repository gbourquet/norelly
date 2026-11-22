use super::*;
use crate::fetch_policy::PartPath;
use crate::mailbox::Uid;
use crate::ports::BlobId;

fn account(n: i64) -> AccountId {
    AccountId::new(n).unwrap_or_else(|_| panic!("compte"))
}

fn id(n: u64) -> GmailMessageId {
    GmailMessageId::new(n).unwrap_or_else(|| panic!("identifiant"))
}

fn gmail() -> ServerCapabilities {
    ServerCapabilities::from_imap(["IMAP4rev1", "x-gm-ext-1", "IDLE"])
}

fn plain() -> ServerCapabilities {
    ServerCapabilities::from_imap(["IMAP4rev1", "IDLE", "X-GM-EXT-2"])
}

fn pending(gmail_id: Option<GmailMessageId>) -> PendingBody {
    PendingBody {
        uid: Uid::new(7).unwrap_or_else(|| panic!("uid")),
        size: 1234,
        structure: None,
        gmail_id,
    }
}

fn blob() -> BlobId {
    BlobId::parse(&"a".repeat(64)).unwrap_or_else(|| panic!("blob"))
}

#[test]
fn identifier_is_never_zero() {
    assert_eq!(GmailMessageId::new(0), None);
    assert_eq!(id(u64::MAX).get(), u64::MAX);
    assert_eq!(
        id(1_278_455_344_230_334_865).get(),
        1_278_455_344_230_334_865
    );
}

#[test]
fn identifiers_are_used_only_when_the_extension_is_announced() {
    assert!(gmail_ids_enabled(&gmail()), "insensible à la casse");
    assert!(!gmail_ids_enabled(&plain()));
    assert!(!gmail_ids_enabled(&ServerCapabilities::default()));
}

#[test]
fn reuse_is_looked_for_only_with_the_extension_and_an_identifier() {
    let expected = BodyReuse {
        account: account(3),
        gmail_id: id(42),
        size: 1234,
    };
    assert_eq!(
        BodyReuse::for_pending(account(3), &gmail(), &pending(Some(id(42)))),
        Some(expected)
    );
    assert_eq!(
        BodyReuse::for_pending(account(3), &plain(), &pending(Some(id(42)))),
        None,
        "identifiant enregistré mais extension plus annoncée"
    );
    assert_eq!(
        BodyReuse::for_pending(account(3), &gmail(), &pending(None)),
        None
    );
}

#[test]
fn only_a_downloaded_body_of_the_same_message_in_the_same_account_is_reused() {
    let key = BodyReuse {
        account: account(3),
        gmail_id: id(42),
        size: 1234,
    };
    let full = StoredBody::Full(blob());
    let partial = StoredBody::Partial {
        header: blob(),
        parts: vec![],
        deferred: vec![PartPath::parse("2").unwrap_or_else(|| panic!("chemin"))],
    };
    let same = BodyCandidate {
        account: account(3),
        gmail_id: Some(id(42)),
        size: 1234,
        body: &full,
        provisional: false,
    };
    assert!(key.accepts(&same));
    assert!(
        key.accepts(&BodyCandidate {
            body: &partial,
            ..same
        }),
        "corps partiel : parties différées reprises telles quelles"
    );
    let refused = [
        (
            BodyCandidate {
                account: account(4),
                ..same
            },
            "autre compte",
        ),
        (
            BodyCandidate {
                gmail_id: Some(id(43)),
                ..same
            },
            "autre identifiant",
        ),
        (
            BodyCandidate {
                gmail_id: None,
                ..same
            },
            "sans identifiant",
        ),
        (BodyCandidate { size: 1235, ..same }, "taille différente"),
        (
            BodyCandidate {
                body: &StoredBody::Missing,
                ..same
            },
            "corps pas encore téléchargé",
        ),
        (
            BodyCandidate {
                provisional: true,
                ..same
            },
            "copie provisoire",
        ),
    ];
    for (candidate, why) in refused {
        assert!(!key.accepts(&candidate), "{why}");
    }
}
