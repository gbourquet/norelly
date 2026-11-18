//! Suites de contrat de la synchronisation CardDAV (ADR-014) et faux en
//! mémoire :
//!
//! - [`carddav_client_contract`] : client CardDAV ([`CardDavClient`]),
//!   joué contre [`FakeCardDavClient`] et contre l'adaptateur `webdav`
//!   branché sur le faux serveur HTTPS ([`crate::carddav_server`]) ;
//! - [`carddav_store_contract`] : état de la synchronisation
//!   ([`CardDavStore`]), joué contre [`InMemoryCardDavStore`] et SQLite.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use norelly_domain::carddav::{
    AddressBook, CardDavAccount, CardUid, DavCredentials, DavUrl, DavUsername, ETag, Href,
    LinkState, RemoteCard, RemoteEntry, SyncFailure, SyncLink, SyncReport, new_card_href,
};
use norelly_domain::contact_export::ContactSelection;
use norelly_domain::contacts::{ContactDraft, ContactId};
use norelly_domain::credentials::Secret;
use norelly_domain::ports::{
    CardDavClient, CardDavError, CardDavStore, CardDavSyncRecord, ContactBook, ContactDirectory,
    ContactStoreError, StoreError,
};

use crate::carddav_server::{CardDavModel, ModelError};
use crate::contacts_contracts::sample_contact;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Adresse de test.
#[must_use]
pub fn dav_url(raw: &str) -> DavUrl {
    DavUrl::parse(raw).unwrap_or_else(|e| panic!("{raw} : {e}"))
}

/// Identifiants de test.
#[must_use]
pub fn credentials(user: &str, password: &str) -> DavCredentials {
    DavCredentials {
        username: DavUsername::parse(user).unwrap_or_else(|e| panic!("{e}")),
        password: Secret::new(password.to_owned()),
    }
}

/// Fiche vCard de test, d'UID `uid`.
#[must_use]
pub fn sample_card(uid: &str, name: &str, address: &str) -> String {
    format!(
        "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:{uid}\r\nFN:{name}\r\nEMAIL;TYPE=INTERNET:{address}\r\nEND:VCARD\r\n"
    )
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Faux client CardDAV : lit et écrit directement le carnet en mémoire du
/// faux serveur, avec les mêmes règles (ETag, identifiants).
#[derive(Debug)]
pub struct FakeCardDavClient {
    pub model: Arc<Mutex<CardDavModel>>,
    user: String,
    password: String,
    book_path: String,
    uids: AtomicU64,
    /// Erreur imposée à toute requête (serveur injoignable…).
    pub failure: Mutex<Option<CardDavError>>,
}

impl FakeCardDavClient {
    #[must_use]
    pub fn new(user: &str, password: &str) -> Self {
        Self {
            model: Arc::new(Mutex::new(CardDavModel::default())),
            user: user.to_owned(),
            password: password.to_owned(),
            book_path: format!("/dav/books/{user}/contacts/"),
            uids: AtomicU64::new(0),
            failure: Mutex::new(None),
        }
    }

    /// Chemin du carnet.
    #[must_use]
    pub fn book_path(&self) -> &str {
        &self.book_path
    }

    fn check(&self, credentials: &DavCredentials) -> Result<(), CardDavError> {
        if let Some(e) = lock(&self.failure).clone() {
            return Err(e);
        }
        if credentials.username.as_str() == self.user
            && credentials.password.expose() == self.password
        {
            Ok(())
        } else {
            Err(CardDavError::Unauthorized)
        }
    }

    fn in_book(&self, book: &DavUrl) -> Result<(), CardDavError> {
        if book.path() == self.book_path {
            Ok(())
        } else {
            Err(CardDavError::NotFound)
        }
    }
}

fn model_error(e: ModelError) -> CardDavError {
    match e {
        ModelError::Precondition => CardDavError::PreconditionFailed,
        ModelError::NotFound => CardDavError::NotFound,
    }
}

fn etag(raw: &str) -> ETag {
    ETag::parse(raw).unwrap_or_else(|| panic!("ETag {raw}"))
}

impl CardDavClient for FakeCardDavClient {
    async fn address_books(
        &self,
        server: &DavUrl,
        credentials: &DavCredentials,
    ) -> Result<Vec<AddressBook>, CardDavError> {
        self.check(credentials)?;
        let book = server
            .join(&self.book_path)
            .ok_or(CardDavError::NoAddressBook)?;
        Ok(vec![AddressBook::new(book, "Contacts")])
    }

    async fn entries(
        &self,
        book: &DavUrl,
        credentials: &DavCredentials,
    ) -> Result<Vec<RemoteEntry>, CardDavError> {
        self.check(credentials)?;
        self.in_book(book)?;
        Ok(lock(&self.model)
            .cards
            .iter()
            .filter_map(|(path, card)| {
                Some(RemoteEntry {
                    href: Href::parse(path).ok()?,
                    etag: ETag::parse(&card.etag)?,
                })
            })
            .collect())
    }

    async fn cards(
        &self,
        book: &DavUrl,
        credentials: &DavCredentials,
        hrefs: &[Href],
    ) -> Result<Vec<RemoteCard>, CardDavError> {
        self.check(credentials)?;
        self.in_book(book)?;
        let model = lock(&self.model);
        Ok(hrefs
            .iter()
            .filter_map(|href| {
                let card = model.cards.get(href.as_str())?;
                Some(RemoteCard {
                    href: href.clone(),
                    etag: ETag::parse(&card.etag)?,
                    data: card.data.clone(),
                })
            })
            .collect())
    }

    async fn create(
        &self,
        book: &DavUrl,
        credentials: &DavCredentials,
        href: &Href,
        card: &str,
    ) -> Result<Option<ETag>, CardDavError> {
        self.check(credentials)?;
        self.in_book(book)?;
        let tag = lock(&self.model)
            .put_if(href.as_str(), None, card)
            .map_err(model_error)?;
        Ok(Some(etag(&tag)))
    }

    async fn update(
        &self,
        book: &DavUrl,
        credentials: &DavCredentials,
        href: &Href,
        expected: &ETag,
        card: &str,
    ) -> Result<Option<ETag>, CardDavError> {
        self.check(credentials)?;
        self.in_book(book)?;
        let tag = lock(&self.model)
            .put_if(href.as_str(), Some(expected.as_str()), card)
            .map_err(model_error)?;
        Ok(Some(etag(&tag)))
    }

    async fn delete(
        &self,
        book: &DavUrl,
        credentials: &DavCredentials,
        href: &Href,
        expected: &ETag,
    ) -> Result<(), CardDavError> {
        self.check(credentials)?;
        self.in_book(book)?;
        match lock(&self.model).delete_if(href.as_str(), expected.as_str()) {
            Ok(()) | Err(ModelError::NotFound) => Ok(()),
            Err(e) => Err(model_error(e)),
        }
    }

    fn new_uid(&self) -> CardUid {
        let n = self.uids.fetch_add(1, Ordering::Relaxed) + 1;
        CardUid::parse(&format!("fake-{n}")).unwrap_or_else(|| panic!("uid"))
    }
}

/// Contrat du client : `server` est la racine du serveur, dont le carnet
/// `book_path` (vide au départ) appartient à `user` (mot de passe
/// `password`).
pub async fn carddav_client_contract<C: CardDavClient>(
    client: &C,
    server: &DavUrl,
    book_path: &str,
    user: &str,
    password: &str,
) -> Result<(), CardDavError> {
    let ok = credentials(user, password);

    // Mot de passe refusé.
    assert_eq!(
        client
            .address_books(server, &credentials(user, "mauvais"))
            .await,
        Err(CardDavError::Unauthorized)
    );

    // Découverte depuis la racine.
    let books = client.address_books(server, &ok).await?;
    let book = books
        .iter()
        .find(|b| b.url().path() == book_path)
        .unwrap_or_else(|| panic!("carnet {book_path} absent : {books:?}"))
        .url()
        .clone();
    assert!(book.same_origin(server));
    assert!(client.entries(&book, &ok).await?.is_empty(), "carnet vide");

    // Création.
    let uid = client.new_uid();
    assert_ne!(uid, client.new_uid(), "UID aléatoires");
    let href = new_card_href(&book, &uid);
    let data = sample_card(uid.as_str(), "Alice", "alice@example.com");
    let created = client.create(&book, &ok, &href, &data).await?;
    assert_eq!(
        client.create(&book, &ok, &href, &data).await,
        Err(CardDavError::PreconditionFailed),
        "création sur une fiche existante"
    );
    let entries = client.entries(&book, &ok).await?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].href, href);
    let first = entries[0].etag.clone();
    if let Some(created) = created {
        assert_eq!(created, first);
    }

    // Lecture.
    let cards = client
        .cards(
            &book,
            &ok,
            &[
                href.clone(),
                Href::parse(&format!("{book_path}absente.vcf")).unwrap_or_else(|e| panic!("{e}")),
            ],
        )
        .await?;
    assert_eq!(cards.len(), 1, "fiche absente omise");
    assert_eq!(cards[0].href, href);
    assert_eq!(cards[0].etag, first);
    assert!(cards[0].data.contains("alice@example.com"));

    // Modification conditionnelle.
    let stale = etag("\"perimee\"");
    let changed = sample_card(uid.as_str(), "Alice M.", "alice@example.com");
    assert_eq!(
        client.update(&book, &ok, &href, &stale, &changed).await,
        Err(CardDavError::PreconditionFailed)
    );
    client.update(&book, &ok, &href, &first, &changed).await?;
    let second = client.entries(&book, &ok).await?[0].etag.clone();
    assert_ne!(second, first);

    // Suppression conditionnelle ; déjà absente : succès.
    assert_eq!(
        client.delete(&book, &ok, &href, &first).await,
        Err(CardDavError::PreconditionFailed)
    );
    client.delete(&book, &ok, &href, &second).await?;
    assert!(client.entries(&book, &ok).await?.is_empty());
    client.delete(&book, &ok, &href, &second).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// État de la synchronisation
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct StoreState {
    account: Option<CardDavAccount>,
    links: BTreeMap<Href, SyncLink>,
    record: Option<CardDavSyncRecord>,
    /// Contenu et date de modification constatés de chaque contact.
    seen: HashMap<ContactId, (ContactDraft, i64)>,
}

/// Faux état de synchronisation, sur un faux carnet : les modifications
/// et suppressions des contacts sont constatées (datées de l'horloge du
/// faux, [`Self::set_clock`]) à la lecture suivante.
#[derive(Debug)]
pub struct InMemoryCardDavStore<'a, B> {
    book: &'a B,
    clock: AtomicI64,
    state: Mutex<StoreState>,
}

impl<'a, B: ContactDirectory> InMemoryCardDavStore<'a, B> {
    #[must_use]
    pub fn new(book: &'a B) -> Self {
        Self {
            book,
            clock: AtomicI64::new(1),
            state: Mutex::new(StoreState::default()),
        }
    }

    /// Heure des modifications constatées désormais.
    pub fn set_clock(&self, now: i64) {
        self.clock.store(now, Ordering::Relaxed);
    }

    /// Date de modification imposée à un contact.
    pub fn set_modified(&self, id: ContactId, at: i64) {
        if let Some(entry) = lock(&self.state).seen.get_mut(&id) {
            entry.1 = at;
        }
    }

    async fn observe(&self) -> Result<(), StoreError> {
        let contacts = self
            .book
            .contacts(&ContactSelection::All, usize::MAX)
            .await?;
        let now = self.clock.load(Ordering::Relaxed);
        let mut state = lock(&self.state);
        let present: HashMap<ContactId, &ContactDraft> =
            contacts.iter().map(|c| (c.id(), c.draft())).collect();
        state.seen.retain(|id, _| present.contains_key(id));
        for (id, draft) in &present {
            match state.seen.get(id) {
                Some((known, _)) if known == *draft => {}
                _ => {
                    state.seen.insert(*id, ((*draft).clone(), now));
                }
            }
        }
        for link in state.links.values_mut() {
            if let LinkState::Linked(id) = link.state
                && !present.contains_key(&id)
            {
                link.state = LinkState::Deleted { at: now };
            }
        }
        Ok(())
    }
}

impl<B: ContactDirectory> CardDavStore for InMemoryCardDavStore<'_, B> {
    async fn account(&self) -> Result<Option<CardDavAccount>, StoreError> {
        Ok(lock(&self.state).account.clone())
    }

    async fn save_account(&self, account: &CardDavAccount) -> Result<(), StoreError> {
        let mut state = lock(&self.state);
        let same_book = state
            .account
            .as_ref()
            .is_some_and(|a| a.book().url() == account.book().url());
        if !same_book {
            state.links.clear();
            state.record = None;
        }
        state.account = Some(account.clone());
        Ok(())
    }

    async fn remove_account(&self) -> Result<(), StoreError> {
        let mut state = lock(&self.state);
        state.account = None;
        state.links.clear();
        state.record = None;
        Ok(())
    }

    async fn links(&self) -> Result<Vec<SyncLink>, StoreError> {
        self.observe().await?;
        Ok(lock(&self.state).links.values().cloned().collect())
    }

    async fn save_link(&self, link: &SyncLink) -> Result<(), StoreError> {
        let mut state = lock(&self.state);
        if let LinkState::Linked(id) = link.state
            && state
                .links
                .values()
                .any(|l| l.href != link.href && l.state == LinkState::Linked(id))
        {
            return Err(StoreError("contact déjà relié".into()));
        }
        state.links.insert(link.href.clone(), link.clone());
        Ok(())
    }

    async fn forget_link(&self, href: &Href) -> Result<(), StoreError> {
        lock(&self.state).links.remove(href);
        Ok(())
    }

    async fn modification_times(&self) -> Result<Vec<(ContactId, i64)>, StoreError> {
        self.observe().await?;
        Ok(lock(&self.state)
            .seen
            .iter()
            .map(|(id, (_, at))| (*id, *at))
            .collect())
    }

    async fn record_sync(&self, record: &CardDavSyncRecord) -> Result<(), StoreError> {
        lock(&self.state).record = Some(record.clone());
        Ok(())
    }

    async fn last_sync(&self) -> Result<Option<CardDavSyncRecord>, StoreError> {
        Ok(lock(&self.state).record.clone())
    }
}

fn account(book: &str) -> CardDavAccount {
    let server = dav_url("https://dav.example.com/");
    CardDavAccount::new(
        server,
        DavUsername::parse("alice").unwrap_or_else(|e| panic!("{e}")),
        AddressBook::new(
            dav_url(&format!("https://dav.example.com{book}")),
            "Contacts",
        ),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

fn link(path: &str, tag: &str, state: LinkState) -> SyncLink {
    SyncLink {
        href: Href::parse(path).unwrap_or_else(|e| panic!("{e}")),
        etag: etag(tag),
        state,
        synced: format!("synced {path}"),
        remote: format!("remote {path}"),
    }
}

/// Contrat de l'état de synchronisation (carnet et état vides au départ),
/// sur le carnet `book` dont il constate les modifications.
pub async fn carddav_store_contract<S, B>(store: &S, book: &B) -> Result<(), ContactStoreError>
where
    S: CardDavStore,
    B: ContactBook + ContactDirectory,
{
    assert_eq!(store.account().await?, None);
    assert!(store.links().await?.is_empty());
    assert_eq!(store.last_sync().await?, None);

    // Compte.
    let first = account("/dav/alice/contacts/");
    store.save_account(&first).await?;
    assert_eq!(store.account().await?, Some(first.clone()));

    // Ressources connues.
    let alice = book
        .create(&sample_contact("Alice", &["alice@example.com"]))
        .await?;
    let bob = book
        .create(&sample_contact("Bob", &["bob@example.com"]))
        .await?;
    let a = link(
        "/dav/alice/contacts/a.vcf",
        "\"1\"",
        LinkState::Linked(alice.id()),
    );
    let b = link(
        "/dav/alice/contacts/b.vcf",
        "\"1\"",
        LinkState::Linked(bob.id()),
    );
    let ignored = link("/dav/alice/contacts/i.vcf", "\"1\"", LinkState::Ignored);
    let gone = link(
        "/dav/alice/contacts/d.vcf",
        "\"1\"",
        LinkState::Deleted { at: 42 },
    );
    for l in [&a, &b, &ignored, &gone] {
        store.save_link(l).await?;
    }
    let mut links = store.links().await?;
    links.sort_by(|x, y| x.href.cmp(&y.href));
    assert_eq!(links, [a.clone(), b.clone(), gone.clone(), ignored.clone()]);
    // Remplacée.
    let a2 = SyncLink {
        etag: etag("\"2\""),
        synced: "nouveau".into(),
        ..a.clone()
    };
    store.save_link(&a2).await?;
    assert!(store.links().await?.contains(&a2));
    assert!(!store.links().await?.contains(&a));
    // Un contact n'est relié qu'à une ressource.
    let twice = link(
        "/dav/alice/contacts/a-bis.vcf",
        "\"1\"",
        LinkState::Linked(alice.id()),
    );
    assert!(store.save_link(&twice).await.is_err());

    // Dates de modification.
    let times: HashMap<ContactId, i64> = store.modification_times().await?.into_iter().collect();
    assert_eq!(times.len(), 2);
    let alice_before = times.get(&alice.id()).copied().unwrap_or(i64::MIN);
    assert!(alice_before > 0);
    book.update(
        alice.id(),
        &sample_contact("Alice M.", &["alice@example.com"]),
    )
    .await?;
    let times: HashMap<ContactId, i64> = store.modification_times().await?.into_iter().collect();
    assert!(times.get(&alice.id()).copied().unwrap_or(i64::MIN) >= alice_before);

    // Contact supprimé : sa ressource passe à « supprimée », datée.
    book.delete(bob.id()).await?;
    let b_now = store
        .links()
        .await?
        .into_iter()
        .find(|l| l.href == b.href)
        .unwrap_or_else(|| panic!("ressource perdue"));
    match b_now.state {
        LinkState::Deleted { at } => assert!(at > 0, "date de suppression"),
        other => panic!("ressource toujours {other:?}"),
    }
    assert_eq!(b_now.synced, b.synced, "contenu gardé");
    assert_eq!(store.modification_times().await?.len(), 1);

    // Oubliée.
    store.forget_link(&gone.href).await?;
    store.forget_link(&gone.href).await?;
    assert!(store.links().await?.iter().all(|l| l.href != gone.href));

    // Dernière synchronisation.
    let done = CardDavSyncRecord {
        at: 1_700_000_000,
        outcome: Ok(SyncReport {
            pulled: 1,
            pushed: 2,
            skipped: 3,
        }),
    };
    store.record_sync(&done).await?;
    assert_eq!(store.last_sync().await?, Some(done));
    let failed = CardDavSyncRecord {
        at: 1_700_000_100,
        outcome: Err(SyncFailure::Unauthorized),
    };
    store.record_sync(&failed).await?;
    assert_eq!(store.last_sync().await?, Some(failed.clone()));

    // Même carnet : rien n'est oublié ; autre carnet : tout l'est.
    store.save_account(&first).await?;
    assert!(!store.links().await?.is_empty());
    assert_eq!(store.last_sync().await?, Some(failed));
    store.save_account(&account("/dav/alice/autre/")).await?;
    assert!(store.links().await?.is_empty());
    assert_eq!(store.last_sync().await?, None);

    // Compte retiré : état oublié, contacts gardés.
    store.save_link(&a2).await?;
    store.remove_account().await?;
    assert_eq!(store.account().await?, None);
    assert!(store.links().await?.is_empty());
    assert!(book.get(alice.id()).await?.is_some());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts_contracts::InMemoryContactBook;

    #[tokio::test]
    async fn the_fake_client_honours_the_contract() {
        let client = FakeCardDavClient::new("alice", "secret");
        let book = client.book_path().to_owned();
        let result = carddav_client_contract(
            &client,
            &dav_url("https://dav.example.com/"),
            &book,
            "alice",
            "secret",
        )
        .await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn the_fake_store_honours_the_contract() {
        let book = InMemoryContactBook::default();
        let store = InMemoryCardDavStore::new(&book);
        let result = carddav_store_contract(&store, &book).await;
        assert_eq!(result, Ok(()));
    }
}
