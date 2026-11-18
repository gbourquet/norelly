//! Client CardDAV contre le faux serveur HTTPS de `test-support` (aucun
//! conteneur) : suite de contrat du port, puis serveur hostile.

use norelly_domain::carddav::{DavUrl, ETag, Href, MAX_LISTING_BYTES, MAX_REMOTE_CARDS};
use norelly_domain::ports::{CardDavClient, CardDavError};
use norelly_test_support::carddav_contracts::{carddav_client_contract, credentials, sample_card};
use norelly_test_support::carddav_server::{FakeCardDavServer, RawResponse};
use norelly_test_support::certs::TestCa;
use norelly_tls::{TlsClientFactory, TrustRoots};
use norelly_webdav::WebDavClient;

async fn setup() -> (TestCa, FakeCardDavServer, WebDavClient) {
    let ca = TestCa::new().unwrap_or_else(|e| panic!("{e}"));
    let server = FakeCardDavServer::start(&ca, "alice", "mot-de-passe")
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let client = WebDavClient::new(&TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])))
        .unwrap_or_else(|e| panic!("{e}"));
    (ca, server, client)
}

fn url(raw: &str) -> DavUrl {
    DavUrl::parse(raw).unwrap_or_else(|e| panic!("{raw} : {e}"))
}

fn multistatus(body: &str) -> RawResponse {
    RawResponse {
        status: 207,
        headers: vec![("Content-Type".into(), "application/xml".into())],
        body: format!("<d:multistatus xmlns:d=\"DAV:\" xmlns:card=\"urn:ietf:params:xml:ns:carddav\">{body}</d:multistatus>").into_bytes(),
    }
}

#[tokio::test]
async fn the_client_honours_the_contract() {
    let (_ca, server, client) = setup().await;
    let result = carddav_client_contract(
        &client,
        &url(&server.url()),
        &server.book_path(),
        "alice",
        "mot-de-passe",
    )
    .await;
    assert_eq!(result, Ok(()));
}

#[tokio::test]
async fn the_book_address_can_be_given_directly() {
    let (_ca, server, client) = setup().await;
    let book = url(&format!(
        "{}{}",
        server.url().trim_end_matches('/'),
        server.book_path()
    ));
    let books = client
        .address_books(&book, &credentials("alice", "mot-de-passe"))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].url(), &book);
    assert_eq!(books[0].name(), "Contacts");
}

#[tokio::test]
async fn a_server_certificate_from_an_unknown_authority_is_refused() {
    let (_ca, server, _) = setup().await;
    // Racines de la plateforme : la CA de test n'en fait pas partie.
    let client = WebDavClient::new(&TlsClientFactory::new(TrustRoots::Custom(vec![
        TestCa::new().unwrap_or_else(|e| panic!("{e}")).root(),
    ])))
    .unwrap_or_else(|e| panic!("{e}"));
    let result = client
        .address_books(&url(&server.url()), &credentials("alice", "mot-de-passe"))
        .await;
    assert!(
        matches!(result, Err(CardDavError::Certificate(_))),
        "{result:?}"
    );
    assert!(
        server.requests().is_empty(),
        "requête envoyée sans TLS vérifié"
    );
}

#[tokio::test]
async fn redirects_and_resources_outside_the_server_are_not_followed() {
    let (_ca, server, client) = setup().await;
    let creds = credentials("alice", "mot-de-passe");
    // Redirection vers un autre hôte : pas suivie, l'adresse racine est
    // essayée ensuite.
    server.override_response(
        "PROPFIND",
        "/.well-known/carddav",
        RawResponse {
            status: 301,
            headers: vec![("Location".into(), "https://evil.example.net/dav/".into())],
            body: Vec::new(),
        },
    );
    let books = client
        .address_books(&url(&server.url()), &creds)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(books.len(), 1);

    // Fiches citées hors du serveur : omises.
    let book = books[0].url().clone();
    server.override_response(
        "PROPFIND",
        &server.book_path(),
        multistatus(&format!(
            "<d:response><d:href>{0}</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>\
             <d:response><d:href>https://evil.example.net/a.vcf</d:href><d:propstat><d:prop><d:getetag>\"1\"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>\
             <d:response><d:href>{0}b.vcf</d:href><d:propstat><d:prop><d:getetag>\"2\"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>\
             <d:response><d:href>{0}sans-etag.vcf</d:href><d:propstat><d:prop/><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
            server.book_path()
        )),
    );
    let listing = client
        .entries(&book, &creds)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.omitted, 2, "réponses omises comptées");
    assert_eq!(
        listing.entries[0].href.as_str(),
        format!("{}b.vcf", server.book_path())
    );
}

#[tokio::test]
async fn oversized_or_hostile_answers_are_refused() {
    let (_ca, server, client) = setup().await;
    let creds = credentials("alice", "mot-de-passe");
    let book = url(&format!(
        "{}{}",
        server.url().trim_end_matches('/'),
        server.book_path()
    ));

    // Liste trop volumineuse.
    server.override_response(
        "PROPFIND",
        &server.book_path(),
        RawResponse {
            status: 207,
            headers: Vec::new(),
            body: vec![b' '; MAX_LISTING_BYTES + 1],
        },
    );
    assert_eq!(
        client.entries(&book, &creds).await,
        Err(CardDavError::TooLarge)
    );

    // Trop de fiches.
    let many = "<d:response><d:href>/x.vcf</d:href></d:response>".repeat(MAX_REMOTE_CARDS + 2);
    server.override_response("PROPFIND", &server.book_path(), multistatus(&many));
    assert_eq!(
        client.entries(&book, &creds).await,
        Err(CardDavError::TooLarge)
    );

    // Entité externe.
    server.override_response(
        "PROPFIND",
        &server.book_path(),
        RawResponse {
            status: 207,
            headers: Vec::new(),
            body: b"<?xml version=\"1.0\"?><!DOCTYPE m [<!ENTITY x SYSTEM \"file:///etc/passwd\">]><m>&x;</m>".to_vec(),
        },
    );
    assert!(matches!(
        client.entries(&book, &creds).await,
        Err(CardDavError::Protocol(_))
    ));

    // Erreur du serveur.
    server.override_response(
        "PROPFIND",
        &server.book_path(),
        RawResponse {
            status: 503,
            headers: Vec::new(),
            body: Vec::new(),
        },
    );
    assert_eq!(
        client.entries(&book, &creds).await,
        Err(CardDavError::Server(503))
    );
}

#[tokio::test]
async fn a_card_without_etag_after_writing_is_still_written() {
    let (_ca, server, client) = setup().await;
    server.set_etag_on_put(false);
    let creds = credentials("alice", "mot-de-passe");
    let book = url(&format!(
        "{}{}",
        server.url().trim_end_matches('/'),
        server.book_path()
    ));
    let href =
        Href::parse(&format!("{}n.vcf", server.book_path())).unwrap_or_else(|e| panic!("{e}"));
    let created = client
        .create(
            &book,
            &creds,
            &href,
            &sample_card("n", "N", "n@example.com"),
        )
        .await;
    assert_eq!(created, Ok(None));
    assert_eq!(
        client.entries(&book, &creds).await.map(|e| e.entries.len()),
        Ok(1)
    );
    let stale = ETag::parse("\"x\"").unwrap_or_else(|| panic!("etag"));
    assert_eq!(
        client.delete(&book, &creds, &href, &stale).await,
        Err(CardDavError::PreconditionFailed)
    );
}

#[tokio::test]
async fn the_password_waits_for_the_server_to_ask_for_it() {
    let (_ca, server, client) = setup().await;
    let books = client
        .address_books(&url(&server.url()), &credentials("alice", "mot-de-passe"))
        .await;
    assert!(books.is_ok(), "{books:?}");
    let requests = server.requests();
    // Première requête sans identifiants ; le serveur les demande (401,
    // Basic) : toutes les suivantes les portent.
    assert!(!requests[0].authorized, "{requests:?}");
    assert!(requests[1..].iter().all(|r| r.authorized), "{requests:?}");
}

#[tokio::test]
async fn a_server_that_never_asks_never_receives_the_password() {
    let (_ca, server, client) = setup().await;
    let not_found = RawResponse {
        status: 404,
        headers: Vec::new(),
        body: Vec::new(),
    };
    server.override_response("PROPFIND", "/.well-known/carddav", not_found.clone());
    server.override_response("PROPFIND", "/", not_found);
    let books = client
        .address_books(&url(&server.url()), &credentials("alice", "mot-de-passe"))
        .await;
    assert_eq!(books, Err(CardDavError::NoAddressBook));
    let requests = server.requests();
    assert!(!requests.is_empty());
    assert!(requests.iter().all(|r| !r.authorized), "{requests:?}");
}

#[tokio::test]
async fn resources_outside_the_book_are_neither_listed_nor_written() {
    let (_ca, server, client) = setup().await;
    let creds = credentials("alice", "mot-de-passe");
    let book = url(&format!(
        "{}{}",
        server.url().trim_end_matches('/'),
        server.book_path()
    ));
    server.override_response(
        "PROPFIND",
        &server.book_path(),
        multistatus(&format!(
            "<d:response><d:href>/dav/calendars/alice/perso/evt.ics</d:href><d:propstat><d:prop><d:getetag>\"1\"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>\
             <d:response><d:href>{0}a.vcf</d:href><d:propstat><d:prop><d:getetag>\"2\"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
            server.book_path()
        )),
    );
    let listing = client
        .entries(&book, &creds)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.omitted, 1, "réponse hors du carnet comptée");
    let outside =
        Href::parse("/dav/calendars/alice/perso/evt.ics").unwrap_or_else(|e| panic!("{e}"));
    let tag = ETag::parse("\"1\"").unwrap_or_else(|| panic!("etag"));
    assert!(matches!(
        client.delete(&book, &creds, &outside, &tag).await,
        Err(CardDavError::Protocol(_))
    ));
    assert!(matches!(
        client.update(&book, &creds, &outside, &tag, "x").await,
        Err(CardDavError::Protocol(_))
    ));
    assert!(server.requests().iter().all(|r| r.path != outside.as_str()));
}
