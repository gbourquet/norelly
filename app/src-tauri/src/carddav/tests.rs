use norelly_domain::carddav::SyncReport;

use super::*;

fn url(raw: &str) -> DavUrl {
    DavUrl::parse(raw).unwrap_or_else(|e| panic!("{e}"))
}

fn code(e: &IpcError) -> (ErrorCodeDto, Option<FieldDto>) {
    (e.code, e.field)
}

#[test]
fn inputs_are_revalidated() {
    assert!(server_of("dav.example.com").is_ok());
    for bad in ["http://dav.example.com", "", "https://a:b@dav.example.com/"] {
        assert_eq!(
            server_of(bad).map_err(|e| code(&e)).err(),
            Some((ErrorCodeDto::InvalidInput, Some(FieldDto::CardDavServer))),
            "{bad}"
        );
    }
    assert!(credentials_of(" alice ", "mot de passe".into()).is_ok());
    assert_eq!(
        credentials_of("alice:x", "p".into())
            .map_err(|e| code(&e))
            .err(),
        Some((ErrorCodeDto::InvalidInput, Some(FieldDto::Username)))
    );
    assert_eq!(
        credentials_of("alice", String::new())
            .map_err(|e| code(&e))
            .err(),
        Some((ErrorCodeDto::InvalidInput, Some(FieldDto::Password)))
    );
}

#[test]
fn only_a_proposed_book_can_be_connected() {
    let services = CardDavServices::new(
        Err("tests".into()),
        SqliteCardDavStore::new(
            norelly_sqlite::Database::open_in_memory().unwrap_or_else(|e| panic!("{e}")),
        ),
    );
    let server = url("https://dav.example.com/");
    let alice = DavUsername::parse("alice").unwrap_or_else(|e| panic!("{e}"));
    let bob = DavUsername::parse("bob").unwrap_or_else(|e| panic!("{e}"));
    let book = AddressBook::new(url("https://dav.example.com/alice/contacts/"), "Contacts");
    assert!(
        services
            .proposed(&server, &alice, "https://dav.example.com/alice/contacts/")
            .is_none(),
        "aucune recherche"
    );
    services.remember(&server, &alice, std::slice::from_ref(&book));
    assert_eq!(
        services.proposed(&server, &alice, "https://dav.example.com/alice/contacts/"),
        Some(book)
    );
    // Autre carnet, autre identifiant, autre serveur : refusés.
    assert!(
        services
            .proposed(&server, &alice, "https://evil.example.net/contacts/")
            .is_none()
    );
    assert!(
        services
            .proposed(&server, &bob, "https://dav.example.com/alice/contacts/")
            .is_none()
    );
    assert!(
        services
            .proposed(
                &url("https://other.example.com/"),
                &alice,
                "https://dav.example.com/alice/contacts/"
            )
            .is_none()
    );
    // Mémoire bornée.
    for i in 0..MAX_REMEMBERED_SEARCHES {
        services.remember(&url(&format!("https://s{i}.example.com/")), &alice, &[]);
    }
    assert!(
        services
            .proposed(&server, &alice, "https://dav.example.com/alice/contacts/")
            .is_none()
    );
}

#[test]
fn failures_become_codes() {
    assert_eq!(
        remote_error(CardDavError::Unauthorized).code,
        ErrorCodeDto::CardDavUnauthorized
    );
    assert_eq!(
        remote_error(CardDavError::Timeout).code,
        ErrorCodeDto::CardDavUnreachable
    );
    let cert = remote_error(CardDavError::Certificate("UnknownIssuer".into()));
    assert_eq!(cert.code, ErrorCodeDto::CardDavCertificate);
    assert_eq!(cert.detail.as_deref(), Some("UnknownIssuer"));
    assert_eq!(
        remote_error(CardDavError::NoAddressBook).code,
        ErrorCodeDto::CardDavNoAddressBook
    );
    assert_eq!(
        remote_error(CardDavError::Server(500)).code,
        ErrorCodeDto::CardDavServerError
    );
    assert_eq!(
        sync_error(SyncError::NotConfigured).code,
        ErrorCodeDto::CardDavNotConfigured
    );
    assert_eq!(
        sync_error(SyncError::Failed(SyncFailure::Keyring)).code,
        ErrorCodeDto::SecretStore
    );
    assert_eq!(
        sync_error(SyncError::Failed(SyncFailure::Unauthorized)).code,
        ErrorCodeDto::CardDavUnauthorized
    );
}

#[test]
fn the_status_never_carries_the_password() {
    let account = CardDavAccount::new(
        url("https://dav.example.com/"),
        DavUsername::parse("alice").unwrap_or_else(|e| panic!("{e}")),
        AddressBook::new(url("https://dav.example.com/c/"), "Contacts"),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let record = CardDavSyncRecord {
        at: 1_700_000_000,
        outcome: Ok(SyncReport {
            pulled: 1,
            pushed: 2,
            skipped: 3,
        }),
    };
    let dto = status_dto(Some(&account), Some(&record), false);
    assert_eq!(
        dto.account.as_ref().map(|a| a.book_url.as_str()),
        Some("https://dav.example.com/c/")
    );
    assert_eq!(
        dto.last_sync.and_then(|s| s.report).map(|r| r.pushed),
        Some(2)
    );
    let failed = CardDavSyncRecord {
        at: 1,
        outcome: Err(SyncFailure::Network),
    };
    assert_eq!(
        status_dto(None, Some(&failed), true)
            .last_sync
            .and_then(|s| s.failure),
        Some(CardDavFailureDto::Network)
    );
}
