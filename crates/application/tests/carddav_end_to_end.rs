//! Synchronisation CardDAV de bout en bout : vrai client HTTPS, vraie base
//! SQLite, vrai codec, contre le faux serveur CardDAV de `test-support`
//! (aucun conteneur).

use std::collections::BTreeMap;
use std::sync::Mutex;

use norelly_application::carddav::{ConfigureCardDav, DiscoverAddressBooks, SyncCardDav};
use norelly_contacts_io::{ContactsIo, IcuNfc};
use norelly_domain::carddav::{CardDavAccount, DavUrl, DavUsername};
use norelly_domain::contact_export::ContactSelection;
use norelly_domain::credentials::Secret;
use norelly_domain::ports::{
    CardDavStore, ContactBook, ContactDirectory, SecretKey, SecretStore, SecretStoreError,
};
use norelly_sqlite::Database;
use norelly_sqlite::carddav::SqliteCardDavStore;
use norelly_sqlite::contacts::SqliteContactBook;
use norelly_test_support::carddav_contracts::{credentials, sample_card};
use norelly_test_support::carddav_server::FakeCardDavServer;
use norelly_test_support::certs::TestCa;
use norelly_test_support::contacts_contracts::sample_contact;
use norelly_tls::{TlsClientFactory, TrustRoots};
use norelly_webdav::WebDavClient;

#[derive(Debug, Default)]
struct Keyring(Mutex<BTreeMap<String, String>>);

impl SecretStore for Keyring {
    async fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretStoreError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| SecretStoreError::Backend("verrou".into()))?
            .get(key.as_str())
            .cloned()
            .map(Secret::new))
    }
    async fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretStoreError> {
        self.0
            .lock()
            .map_err(|_| SecretStoreError::Backend("verrou".into()))?
            .insert(key.as_str().to_owned(), secret.expose().to_owned());
        Ok(())
    }
    async fn delete(&self, key: &SecretKey) -> Result<(), SecretStoreError> {
        self.0
            .lock()
            .map_err(|_| SecretStoreError::Backend("verrou".into()))?
            .remove(key.as_str());
        Ok(())
    }
}

#[tokio::test]
async fn a_book_synchronises_over_https_with_sqlite() {
    let ca = TestCa::new().unwrap_or_else(|e| panic!("{e}"));
    let server = FakeCardDavServer::start(&ca, "alice", "mot-de-passe")
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let client = WebDavClient::new(&TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])))
        .unwrap_or_else(|e| panic!("{e}"));
    let db =
        Database::open_in_memory(norelly_contacts_io::IcuNfc).unwrap_or_else(|e| panic!("{e}"));
    let book = SqliteContactBook::new(db.clone());
    let store = SqliteCardDavStore::new(db);
    let keyring = Keyring::default();

    // Découverte puis enregistrement du compte.
    let root = DavUrl::parse(&server.url()).unwrap_or_else(|e| panic!("{e}"));
    let books = DiscoverAddressBooks { client: &client }
        .execute(&root, &credentials("alice", "mot-de-passe"))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let account = CardDavAccount::new(
        root,
        DavUsername::parse("alice").unwrap_or_else(|e| panic!("{e}")),
        books[0].clone(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    ConfigureCardDav {
        store: &store,
        secrets: &keyring,
    }
    .execute(&account, &Secret::new("mot-de-passe".into()))
    .await
    .unwrap_or_else(|e| panic!("{e}"));

    let alice = book
        .create(&sample_contact("Alice", &["alice@example.com"]))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let bob_path = format!("{}bob.vcf", server.book_path());
    server
        .model()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .put(&bob_path, &sample_card("bob", "Bob", "bob@example.com"));

    let sync = SyncCardDav {
        client: &client,
        store: &store,
        book: &book,
        secrets: &keyring,
        codec: &ContactsIo,
        normalizer: &IcuNfc,
    };
    let report = sync
        .execute(1_700_000_000)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!((report.pulled, report.pushed), (1, 1), "{report:?}");
    let local = book
        .contacts(&ContactSelection::All, 10)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(local.len(), 2);
    assert_eq!(server.model().lock().map(|m| m.cards.len()).unwrap_or(0), 2);

    // Rien n'a changé : rien n'est écrit.
    let again = sync
        .execute(1_700_000_100)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(again.changes(), 0, "{again:?}");

    // Modifiée ici : écrite sur le serveur.
    book.update(
        alice.id(),
        &sample_contact("Alice Martin", &["alice@example.com"]),
    )
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    let pushed = sync
        .execute(1_700_000_200)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!((pushed.pulled, pushed.pushed), (0, 1), "{pushed:?}");
    let remote = server
        .model()
        .lock()
        .map(|m| m.cards.values().map(|c| c.data.clone()).collect::<Vec<_>>())
        .unwrap_or_default();
    assert!(
        remote.iter().any(|d| d.contains("FN:Alice Martin")),
        "{remote:?}"
    );
    assert_eq!(
        sync.execute(1_700_000_300).await.map(|r| r.changes()),
        Ok(0)
    );
}

/// Valeurs piégées venues du serveur (fiche, nom de ressource, UID) :
/// enregistrées telles quelles, sans toucher au schéma ni aux autres
/// lignes (requêtes en paramètres liés).
#[tokio::test]
async fn hostile_server_values_reach_the_database_as_plain_data() {
    let ca = TestCa::new().unwrap_or_else(|e| panic!("{e}"));
    let server = FakeCardDavServer::start(&ca, "alice", "mot-de-passe")
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let client = WebDavClient::new(&TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()])))
        .unwrap_or_else(|e| panic!("{e}"));
    let db =
        Database::open_in_memory(norelly_contacts_io::IcuNfc).unwrap_or_else(|e| panic!("{e}"));
    let book = SqliteContactBook::new(db.clone());
    let store = SqliteCardDavStore::new(db.clone());
    let keyring = Keyring::default();
    let root = DavUrl::parse(&server.url()).unwrap_or_else(|e| panic!("{e}"));
    let books = DiscoverAddressBooks { client: &client }
        .execute(&root, &credentials("alice", "mot-de-passe"))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let account = CardDavAccount::new(
        root,
        DavUsername::parse("alice").unwrap_or_else(|e| panic!("{e}")),
        books[0].clone(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    ConfigureCardDav {
        store: &store,
        secrets: &keyring,
    }
    .execute(&account, &Secret::new("mot-de-passe".into()))
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    let kept = book
        .create(&sample_contact("Témoin", &["temoin@example.com"]))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let schema_before = db.schema().await.unwrap_or_else(|e| panic!("{e}"));

    let evil = "'); DROP TABLE contacts;--";
    for (name, data) in [
        (
            "a%27b.vcf",
            format!(
                "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:x'y\r\nFN:{evil}\r\nNOTE:\" OR 1=1 --\r\nEMAIL:piege@example.com\r\nEND:VCARD\r\n"
            ),
        ),
        (
            "sans-adresse.vcf",
            format!("BEGIN:VCARD\r\nVERSION:3.0\r\nFN:{evil}\r\nEND:VCARD\r\n"),
        ),
    ] {
        server
            .model()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .put(&format!("{}{name}", server.book_path()), &data);
    }
    let sync = SyncCardDav {
        client: &client,
        store: &store,
        book: &book,
        secrets: &keyring,
        codec: &ContactsIo,
        normalizer: &IcuNfc,
    };
    let report = sync
        .execute(1_700_000_000)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!((report.pulled, report.skipped), (1, 1), "{report:?}");

    assert_eq!(
        db.schema().await.unwrap_or_else(|e| panic!("{e}")),
        schema_before
    );
    assert!(
        db.integrity_problems()
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .is_empty()
    );
    let local = book
        .contacts(&ContactSelection::All, 10)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(local.len(), 2, "{local:?}");
    assert!(local.iter().any(|c| c.id() == kept.id()), "témoin perdu");
    assert!(local.iter().any(|c| c.draft().display_name() == evil));
    let links = store.links().await.unwrap_or_else(|e| panic!("{e}"));
    assert!(
        links
            .iter()
            .any(|l| l.href.as_str() == format!("{}a%27b.vcf", server.book_path()))
    );
}
