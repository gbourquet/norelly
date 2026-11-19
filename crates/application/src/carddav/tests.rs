//! Tests de la synchronisation CardDAV, sur faux client, faux état, faux
//! carnet et vrai codec vCard.

use std::collections::BTreeMap;
use std::sync::Mutex;

use norelly_contacts_io::{ContactsIo, IcuNfc};
use norelly_domain::carddav::{AddressBook, CardDavAccount, DavUsername, SyncFailure};
use norelly_domain::contact_export::ContactSelection;
use norelly_domain::contacts::{Contact, ContactDraft};
use norelly_domain::ports::{ContactBook, ContactDirectory, SecretKey, SecretStoreError};
use norelly_test_support::carddav_contracts::{
    FakeCardDavClient, InMemoryCardDavStore, dav_url, sample_card,
};
use norelly_test_support::contacts_contracts::{InMemoryContactBook, sample_contact};

use super::*;

#[derive(Debug, Default)]
struct Keyring {
    secrets: Mutex<BTreeMap<String, String>>,
}

impl Keyring {
    fn get_sync(&self, key: &str) -> Option<String> {
        self.secrets.lock().ok()?.get(key).cloned()
    }
}

impl SecretStore for Keyring {
    async fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretStoreError> {
        Ok(self.get_sync(key.as_str()).map(Secret::new))
    }
    async fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretStoreError> {
        self.secrets
            .lock()
            .map_err(|_| SecretStoreError::Backend("verrou".into()))?
            .insert(key.as_str().to_owned(), secret.expose().to_owned());
        Ok(())
    }
    async fn delete(&self, key: &SecretKey) -> Result<(), SecretStoreError> {
        self.secrets
            .lock()
            .map_err(|_| SecretStoreError::Backend("verrou".into()))?
            .remove(key.as_str());
        Ok(())
    }
}

struct World<'a> {
    client: &'a FakeCardDavClient,
    store: &'a InMemoryCardDavStore<'a, InMemoryContactBook>,
    book: &'a InMemoryContactBook,
    keyring: &'a Keyring,
}

impl World<'_> {
    fn sync(
        &self,
    ) -> SyncCardDav<
        '_,
        FakeCardDavClient,
        InMemoryCardDavStore<'_, InMemoryContactBook>,
        InMemoryContactBook,
        Keyring,
        ContactsIo,
        IcuNfc,
    > {
        SyncCardDav {
            client: self.client,
            store: self.store,
            book: self.book,
            secrets: self.keyring,
            codec: &ContactsIo,
            normalizer: &IcuNfc,
        }
    }

    async fn run(&self, now: i64) -> Result<SyncReport, SyncError> {
        self.store.set_clock(now);
        self.sync().execute(now).await
    }

    fn remote_path(&self, name: &str) -> String {
        format!("{}{name}", self.client.book_path())
    }

    fn put_remote(&self, name: &str, data: &str) {
        let path = self.remote_path(name);
        self.client
            .model
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .put(&path, data);
    }

    fn remote(&self) -> BTreeMap<String, String> {
        self.client
            .model
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cards
            .iter()
            .map(|(k, v)| (k.clone(), v.data.clone()))
            .collect()
    }

    async fn local(&self) -> Vec<Contact> {
        self.book
            .contacts(&ContactSelection::All, 1000)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    }

    async fn configure(&self) {
        let account = CardDavAccount::new(
            dav_url("https://dav.example.com/"),
            DavUsername::parse("alice").unwrap_or_else(|e| panic!("{e}")),
            AddressBook::new(
                dav_url(&format!(
                    "https://dav.example.com{}",
                    self.client.book_path()
                )),
                "Contacts",
            ),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        ConfigureCardDav {
            store: self.store,
            secrets: self.keyring,
        }
        .execute(&account, &Secret::new("secret".into()))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    }
}

macro_rules! world {
    ($w:ident) => {
        let client = FakeCardDavClient::new("alice", "secret");
        let book = InMemoryContactBook::default();
        let store = InMemoryCardDavStore::new(&book);
        let keyring = Keyring::default();
        let $w = World {
            client: &client,
            store: &store,
            book: &book,
            keyring: &keyring,
        };
    };
}

fn names(contacts: &[Contact]) -> Vec<String> {
    let mut names: Vec<String> = contacts.iter().map(|c| c.draft().display_name()).collect();
    names.sort();
    names
}

#[tokio::test]
async fn configuring_keeps_the_password_in_the_keyring_only() {
    world!(w);
    w.configure().await;
    assert_eq!(w.keyring.get_sync(SECRET_KEY), Some("secret".into()));
    assert!(w.store.account().await.ok().flatten().is_some());
    RemoveCardDav {
        store: w.store,
        secrets: w.keyring,
    }
    .execute()
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(w.keyring.get_sync(SECRET_KEY), None);
    assert!(w.store.account().await.ok().flatten().is_none());
}

#[tokio::test]
async fn a_sync_needs_an_account_and_its_password() {
    world!(w);
    assert_eq!(w.run(10).await, Err(SyncError::NotConfigured));
    w.configure().await;
    w.keyring
        .delete(&secret_key())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        w.run(10).await,
        Err(SyncError::Failed(SyncFailure::Keyring))
    );
    let last = w.store.last_sync().await.ok().flatten();
    assert_eq!(last.map(|r| r.outcome), Some(Err(SyncFailure::Keyring)));
}

#[tokio::test]
async fn a_refused_password_is_recorded() {
    world!(w);
    w.configure().await;
    w.keyring
        .set(&secret_key(), &Secret::new("mauvais".into()))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        w.run(10).await,
        Err(SyncError::Failed(SyncFailure::Unauthorized))
    );
    assert_eq!(
        w.store.last_sync().await.ok().flatten(),
        Some(CardDavSyncRecord {
            at: 10,
            outcome: Err(SyncFailure::Unauthorized)
        })
    );
}

#[tokio::test]
async fn the_first_sync_merges_both_books_then_nothing_changes() {
    world!(w);
    w.configure().await;
    w.book
        .create(&sample_contact("Alice", &["alice@example.com"]))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.put_remote("bob.vcf", &sample_card("bob", "Bob", "bob@example.com"));

    let report = w.run(100).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!((report.pulled, report.pushed), (1, 1));
    assert_eq!(names(&w.local().await), ["Alice", "Bob"]);
    let remote = w.remote();
    assert_eq!(remote.len(), 2);
    let pushed = remote
        .values()
        .find(|v| v.contains("alice@example.com"))
        .unwrap_or_else(|| panic!("Alice absente du serveur"));
    assert!(pushed.contains("UID:"), "{pushed}");
    assert!(pushed.contains("REV:19700101T000140Z"), "{pushed}");

    // Rien n'a changé : rien n'est écrit.
    let again = w.run(200).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(again.changes(), 0, "{again:?}");
    assert_eq!(
        w.store.last_sync().await.ok().flatten().map(|r| r.at),
        Some(200)
    );
}

#[tokio::test]
async fn changes_follow_in_both_directions_and_keep_unknown_properties() {
    world!(w);
    w.configure().await;
    let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:bob\r\nFN:Bob\r\nEMAIL:bob@example.com\r\nBDAY:19800102\r\nPHOTO;ENCODING=b;TYPE=PNG:AAAA\r\nEND:VCARD\r\n";
    w.put_remote("bob.vcf", card);
    w.run(100).await.unwrap_or_else(|e| panic!("{e:?}"));

    // Modifiée sur le serveur : suivie ici, favori gardé.
    let bob = w.local().await.remove(0);
    let mut input = bob.draft().to_input();
    input.favorite = true;
    let favorite = ContactDraft::new(input).unwrap_or_else(|e| panic!("{e}"));
    w.book
        .update(bob.id(), &favorite)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.put_remote("bob.vcf", &card.replace("FN:Bob", "FN:Robert"));
    let report = w.run(200).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!((report.pulled, report.pushed), (1, 0), "{report:?}");
    let local = w.local().await.remove(0);
    assert_eq!(local.draft().display_name(), "Robert");
    assert!(local.draft().favorite(), "favori perdu");

    // L'anniversaire du serveur est repris sur la fiche.
    assert_eq!(
        local.draft().birthday().map(|b| b.iso()),
        Some("1980-01-02".into())
    );

    // Modifiée ici (formulaire pré-rempli) : suivie sur le serveur, sans
    // perdre l'anniversaire ni la photo, que le carnet ne garde pas.
    let mut input = local.draft().to_input();
    input.display_name = "Bob Martin".into();
    let edited = ContactDraft::new(input).unwrap_or_else(|e| panic!("{e}"));
    w.book
        .update(local.id(), &edited)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let report = w.run(300).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!((report.pulled, report.pushed), (0, 1), "{report:?}");
    let remote = w.remote();
    let data = &remote[&w.remote_path("bob.vcf")];
    assert!(data.contains("FN:Bob Martin"), "{data}");
    // Anniversaire écrit par le carnet (vCard 3.0, forme étendue), une fois.
    assert!(data.contains("BDAY:1980-01-02"), "{data}");
    assert_eq!(data.matches("BDAY").count(), 1, "{data}");
    assert!(data.contains("PHOTO;ENCODING=b;TYPE=PNG:AAAA"), "{data}");
    assert!(data.contains("UID:bob"), "{data}");
    assert!(data.contains("VERSION:3.0"), "{data}");
    assert_eq!(w.run(400).await.map(|r| r.changes()), Ok(0));
}

#[tokio::test]
async fn deletions_follow_in_both_directions() {
    world!(w);
    w.configure().await;
    w.put_remote("bob.vcf", &sample_card("bob", "Bob", "bob@example.com"));
    w.put_remote("eve.vcf", &sample_card("eve", "Eve", "eve@example.com"));
    w.run(100).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(names(&w.local().await), ["Bob", "Eve"]);

    // Supprimée ici : supprimée sur le serveur.
    let eve = w
        .local()
        .await
        .into_iter()
        .find(|c| c.draft().display_name() == "Eve")
        .unwrap_or_else(|| panic!("Eve"));
    w.book
        .delete(eve.id())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    // Supprimée sur le serveur : supprimée ici.
    w.client
        .model
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .cards
        .remove(&w.remote_path("bob.vcf"));
    let report = w.run(200).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!((report.pulled, report.pushed), (1, 1), "{report:?}");
    assert!(w.local().await.is_empty());
    assert!(w.remote().is_empty());
    assert_eq!(w.run(300).await.map(|r| r.changes()), Ok(0));
}

#[tokio::test]
async fn a_conflict_goes_to_the_latest_modification() {
    world!(w);
    w.configure().await;
    let card = |name: &str, rev: &str| {
        format!(
            "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:bob\r\nFN:{name}\r\nEMAIL:bob@example.com\r\nREV:{rev}\r\nEND:VCARD\r\n"
        )
    };
    w.put_remote("bob.vcf", &card("Bob", "19700101T000100Z"));
    w.run(100).await.unwrap_or_else(|e| panic!("{e:?}"));
    let bob = w.local().await.remove(0);

    // Modifiée ici à 500, sur le serveur à 400 : la fiche locale gagne.
    w.store.set_clock(500);
    w.book
        .update(bob.id(), &sample_contact("Bob ici", &["bob@example.com"]))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    // Modification constatée à 500.
    let _ = w.store.modification_times().await;
    w.put_remote("bob.vcf", &card("Bob là-bas", "19700101T000640Z"));
    w.run(1000).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(w.local().await[0].draft().display_name(), "Bob ici");
    assert!(w.remote()[&w.remote_path("bob.vcf")].contains("FN:Bob ici"));

    // Modifiée ici à 1100, sur le serveur à 1200 : la version distante gagne.
    w.store.set_clock(1100);
    w.book
        .update(
            bob.id(),
            &sample_contact("Bob ici encore", &["bob@example.com"]),
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let _ = w.store.modification_times().await;
    w.put_remote("bob.vcf", &card("Bob là-bas encore", "19700101T002000Z"));
    w.run(2000).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        w.local().await[0].draft().display_name(),
        "Bob là-bas encore"
    );
}

#[tokio::test]
async fn unusable_remote_cards_are_ignored_and_not_downloaded_again() {
    world!(w);
    w.configure().await;
    w.put_remote(
        "sans-adresse.vcf",
        "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:x\r\nFN:Sans adresse\r\nEND:VCARD\r\n",
    );
    let report = w.run(100).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(report.skipped, 1);
    assert!(w.local().await.is_empty());
    let again = w.run(200).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(again.skipped, 0, "fiche ignorée revue sans changement");
    assert_eq!(w.remote().len(), 1, "fiche distante gardée");
}

#[tokio::test]
async fn an_unreachable_server_stops_the_sync_without_writing() {
    world!(w);
    w.configure().await;
    w.book
        .create(&sample_contact("Alice", &["alice@example.com"]))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    *w.client
        .failure
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(
        norelly_domain::ports::CardDavError::Network("hors ligne".into()),
    );
    assert_eq!(
        w.run(100).await,
        Err(SyncError::Failed(SyncFailure::Network))
    );
    assert!(w.store.links().await.unwrap_or_default().is_empty());
}

#[tokio::test]
async fn discovery_lists_the_books_of_the_account() {
    world!(w);
    let books = DiscoverAddressBooks { client: w.client }
        .execute(
            &dav_url("https://dav.example.com/"),
            &norelly_test_support::carddav_contracts::credentials("alice", "secret"),
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].url().path(), w.client.book_path());
}

#[tokio::test]
async fn a_read_only_book_still_brings_its_cards() {
    world!(w);
    w.configure().await;
    w.book
        .create(&sample_contact("Alice", &["alice@example.com"]))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.put_remote("bob.vcf", &sample_card("bob", "Bob", "bob@example.com"));
    *w.client
        .read_only
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
    // Écriture refusée (403) : remise à plus tard, la synchronisation
    // continue et rapporte la fiche laissée de côté.
    let report = w.run(100).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!((report.pulled, report.pushed, report.skipped), (1, 0, 1));
    assert_eq!(names(&w.local().await), ["Alice", "Bob"]);
}

#[tokio::test]
async fn an_unreadable_remote_card_is_not_duplicated_by_a_local_contact() {
    world!(w);
    w.configure().await;
    w.book
        .create(&sample_contact("Bob", &["bob@example.com"]))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    // Nom fait d'un caractère invisible : refusée à la reprise.
    w.put_remote(
        "bob.vcf",
        "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:bob\r\nFN:B\u{200B}ob\r\nEMAIL:bob@example.com\r\nEND:VCARD\r\n",
    );
    let report = w.run(100).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(report.pushed, 0, "{report:?}");
    assert_eq!(w.remote().len(), 1, "doublon envoyé sur le serveur");
}

#[tokio::test]
async fn a_photo_from_the_server_is_ignored_and_ours_is_kept() {
    use norelly_domain::contact_import::{ContactCompletion, PhotoTarget, PlacedPhoto};
    use norelly_test_support::contacts_contracts::photo_id;

    world!(w);
    w.configure().await;
    // Photo de la fiche distante : jamais reprise telle quelle ni
    // réencodée par la synchronisation.
    let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:bob\r\nFN:Bob\r\nEMAIL:bob@example.com\r\n\
                PHOTO;ENCODING=b;TYPE=JPEG:/9j/4AAQSkZJRgABAQ==\r\nEND:VCARD\r\n";
    w.put_remote("bob.vcf", card);
    w.run(100).await.unwrap_or_else(|e| panic!("{e:?}"));
    let bob = w.local().await.remove(0);
    assert_eq!(bob.photo(), None);
    assert!(
        w.book
            .photo_ids()
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .is_empty()
    );

    // Photo donnée ici (import) : gardée quand la fiche distante change.
    w.book
        .import(
            &[],
            &[ContactCompletion::new(bob.clone(), bob.draft().clone())],
            &[],
            &[PlacedPhoto {
                target: PhotoTarget::Existing(bob.id()),
                photo: photo_id(7),
            }],
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.put_remote("bob.vcf", &card.replace("FN:Bob", "FN:Robert"));
    let report = w.run(200).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(report.pulled, 1, "{report:?}");
    let local = w.local().await.remove(0);
    assert_eq!(local.draft().display_name(), "Robert");
    assert_eq!(local.photo(), Some(&photo_id(7)), "photo perdue");

    // Modifiée ici : la fiche distante garde sa propre photo, la nôtre
    // n'est pas envoyée.
    w.book
        .update(
            local.id(),
            &sample_contact("Bob Martin", &["bob@example.com"]),
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.run(300).await.unwrap_or_else(|e| panic!("{e:?}"));
    let remote = w.remote();
    let data = &remote[&w.remote_path("bob.vcf")];
    assert!(data.contains("FN:Bob Martin"), "{data}");
    assert!(
        data.contains("PHOTO;ENCODING=b;TYPE=JPEG:/9j/4AAQSkZJRgABAQ=="),
        "{data}"
    );
    assert_eq!(data.matches("PHOTO").count(), 1, "{data}");
    let local = w.local().await.remove(0);
    assert_eq!(local.photo(), Some(&photo_id(7)));
    assert_eq!(w.run(400).await.map(|r| r.changes()), Ok(0));
}

#[tokio::test]
async fn a_contact_photo_never_leaves_for_the_server() {
    use norelly_domain::contact_import::{PhotoTarget, PlacedPhoto};
    use norelly_test_support::contacts_contracts::photo_id;

    world!(w);
    w.configure().await;
    w.book
        .import(
            &[sample_contact("Carole", &["carole@example.com"])],
            &[],
            &[],
            &[PlacedPhoto {
                target: PhotoTarget::Created(0),
                photo: photo_id(3),
            }],
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let report = w.run(100).await.unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(report.pushed, 1, "{report:?}");
    let remote = w.remote();
    assert_eq!(remote.len(), 1);
    for data in remote.values() {
        assert!(data.contains("FN:Carole"), "{data}");
        assert!(!data.contains("PHOTO"), "{data}");
    }
}
