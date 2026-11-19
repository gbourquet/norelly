//! Commandes IPC de la synchronisation CardDAV du carnet (ADR-014) :
//! chercher les carnets d'un serveur, relier le carnet choisi, voir
//! l'état, synchroniser, déconnecter.
//!
//! Toute entrée est revalidée par le domaine. Le carnet relié doit être
//! l'un de ceux que la dernière recherche a proposés pour ce serveur et
//! cet identifiant : l'interface ne peut pas désigner une autre adresse.
//! Le mot de passe va au trousseau ; il ne revient jamais à l'interface.

use std::sync::{Mutex, PoisonError};

use norelly_application::carddav::{
    CardDavSetupError, ConfigureCardDav, DiscoverAddressBooks, RemoveCardDav, SyncCardDav,
    SyncError,
};
use norelly_domain::carddav::{
    AddressBook, CardDavAccount, DavCredentials, DavUrl, DavUsername, SyncFailure, SyncReport,
    validate_password,
};
use norelly_domain::credentials::Secret;
use norelly_domain::ports::{CardDavError, CardDavStore, CardDavSyncRecord, StoreError};
use norelly_sqlite::carddav::SqliteCardDavStore;
use norelly_webdav::WebDavClient;
use tauri::State;

use crate::ipc::{
    CardDavAccountDto, CardDavBookDto, CardDavFailureDto, CardDavReportDto, CardDavStatusDto,
    CardDavSyncDto, ErrorCodeDto, FieldDto, IpcError, date_dto,
};
use crate::state::AppState;

/// Carnets proposés par une recherche, au plus ce nombre de recherches
/// retenues.
const MAX_REMEMBERED_SEARCHES: usize = 4;

/// Recherche retenue : (serveur, identifiant) et carnets proposés.
type Search = ((String, String), Vec<AddressBook>);

/// Services CardDAV de l'application.
#[derive(Debug)]
pub struct CardDavServices {
    /// Client HTTPS, ou la raison de son absence (TLS indisponible).
    client: Result<WebDavClient, String>,
    pub store: SqliteCardDavStore,
    /// Une synchronisation à la fois.
    syncing: tokio::sync::Mutex<()>,
    /// Dernières recherches : (serveur, identifiant) et carnets proposés.
    searches: Mutex<Vec<Search>>,
}

impl CardDavServices {
    #[must_use]
    pub fn new(client: Result<WebDavClient, String>, store: SqliteCardDavStore) -> Self {
        Self {
            client,
            store,
            syncing: tokio::sync::Mutex::new(()),
            searches: Mutex::new(Vec::new()),
        }
    }

    fn client(&self) -> Result<&WebDavClient, CardDavError> {
        self.client
            .as_ref()
            .map_err(|e| CardDavError::Network(e.clone()))
    }

    fn remember(&self, server: &DavUrl, username: &DavUsername, books: &[AddressBook]) {
        let key = (server.to_string(), username.as_str().to_owned());
        let mut searches = self.searches.lock().unwrap_or_else(PoisonError::into_inner);
        searches.retain(|(k, _)| k != &key);
        searches.push((key, books.to_vec()));
        let excess = searches.len().saturating_sub(MAX_REMEMBERED_SEARCHES);
        searches.drain(..excess);
    }

    /// Carnet `url` proposé par la dernière recherche sur ce serveur pour
    /// cet identifiant.
    fn proposed(&self, server: &DavUrl, username: &DavUsername, url: &str) -> Option<AddressBook> {
        let key = (server.to_string(), username.as_str().to_owned());
        self.searches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(k, _)| k == &key)
            .and_then(|(_, books)| books.iter().find(|b| b.url().to_string() == url).cloned())
    }
}

/// Adresse du serveur revalidée.
pub(crate) fn server_of(raw: &str) -> Result<DavUrl, IpcError> {
    DavUrl::parse(raw).map_err(|_| IpcError::invalid(FieldDto::CardDavServer))
}

/// Identifiants revalidés.
pub(crate) fn credentials_of(username: &str, password: String) -> Result<DavCredentials, IpcError> {
    let username =
        DavUsername::parse(username.trim()).map_err(|_| IpcError::invalid(FieldDto::Username))?;
    let password = Secret::new(password);
    validate_password(&password).map_err(|_| IpcError::invalid(FieldDto::Password))?;
    Ok(DavCredentials { username, password })
}

/// Échec d'un échange avec le serveur, en code.
pub(crate) fn remote_error(e: CardDavError) -> IpcError {
    match e {
        CardDavError::Unauthorized => IpcError::new(ErrorCodeDto::CardDavUnauthorized),
        CardDavError::Network(detail) => {
            IpcError::new(ErrorCodeDto::CardDavUnreachable).with_detail(detail)
        }
        CardDavError::Timeout => IpcError::new(ErrorCodeDto::CardDavUnreachable),
        CardDavError::Certificate(detail) => {
            IpcError::new(ErrorCodeDto::CardDavCertificate).with_detail(detail)
        }
        CardDavError::NotFound | CardDavError::NoAddressBook => {
            IpcError::new(ErrorCodeDto::CardDavNoAddressBook)
        }
        other => IpcError::new(ErrorCodeDto::CardDavServerError).with_detail(other),
    }
}

/// Échec de l'enregistrement du compte, en code.
pub(crate) fn setup_error(e: CardDavSetupError) -> IpcError {
    match e {
        CardDavSetupError::Keyring(e) => {
            tracing::warn!(error = %e, "trousseau");
            IpcError::new(ErrorCodeDto::SecretStore)
        }
        CardDavSetupError::Store(e) => IpcError::storage(e),
    }
}

pub(crate) fn failure_dto(failure: SyncFailure) -> CardDavFailureDto {
    match failure {
        SyncFailure::Unauthorized => CardDavFailureDto::Unauthorized,
        SyncFailure::Network => CardDavFailureDto::Network,
        SyncFailure::Certificate => CardDavFailureDto::Certificate,
        SyncFailure::NotFound => CardDavFailureDto::NotFound,
        SyncFailure::TooLarge => CardDavFailureDto::TooLarge,
        SyncFailure::Server | SyncFailure::Protocol => CardDavFailureDto::Server,
        SyncFailure::Storage => CardDavFailureDto::Storage,
        SyncFailure::Keyring => CardDavFailureDto::Keyring,
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

pub(crate) fn report_dto(report: SyncReport) -> CardDavReportDto {
    CardDavReportDto {
        pulled: count(report.pulled),
        pushed: count(report.pushed),
        skipped: count(report.skipped),
    }
}

/// Échec d'une synchronisation, en code (les mêmes que la recherche des
/// carnets).
pub(crate) fn sync_error(e: SyncError) -> IpcError {
    let failure = match e {
        SyncError::NotConfigured => return IpcError::new(ErrorCodeDto::CardDavNotConfigured),
        SyncError::Failed(failure) => failure,
    };
    IpcError::new(match failure {
        SyncFailure::Unauthorized => ErrorCodeDto::CardDavUnauthorized,
        SyncFailure::Network => ErrorCodeDto::CardDavUnreachable,
        SyncFailure::Certificate => ErrorCodeDto::CardDavCertificate,
        SyncFailure::NotFound => ErrorCodeDto::CardDavNoAddressBook,
        SyncFailure::TooLarge => ErrorCodeDto::CardDavTooLarge,
        SyncFailure::Server | SyncFailure::Protocol => ErrorCodeDto::CardDavServerError,
        SyncFailure::Storage => ErrorCodeDto::Storage,
        SyncFailure::Keyring => ErrorCodeDto::SecretStore,
    })
}

pub(crate) fn status_dto(
    account: Option<&CardDavAccount>,
    last: Option<&CardDavSyncRecord>,
    syncing: bool,
) -> CardDavStatusDto {
    CardDavStatusDto {
        account: account.map(|a| CardDavAccountDto {
            server: a.server().to_string(),
            username: a.username().as_str().to_owned(),
            book_url: a.book().url().to_string(),
            book_name: a.book().name().to_owned(),
        }),
        last_sync: last.map(|r| CardDavSyncDto {
            at: date_dto(r.at),
            report: r.outcome.ok().map(report_dto),
            failure: r.outcome.err().map(failure_dto),
        }),
        syncing,
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

async fn status(state: &AppState) -> Result<CardDavStatusDto, StoreError> {
    let carddav = &state.carddav;
    let account = carddav.store.account().await?;
    let last = carddav.store.last_sync().await?;
    let syncing = carddav.syncing.try_lock().is_err();
    Ok(status_dto(account.as_ref(), last.as_ref(), syncing))
}

/// Synchronise le carnet, une fois à la fois (`None` si une autre
/// synchronisation est en cours).
pub(crate) async fn sync_now(state: &AppState) -> Option<Result<SyncReport, SyncError>> {
    let carddav = &state.carddav;
    let _guard = carddav.syncing.try_lock().ok()?;
    let Ok(client) = carddav.client() else {
        return Some(Err(SyncError::Failed(SyncFailure::Network)));
    };
    Some(
        SyncCardDav {
            client,
            store: &carddav.store,
            book: &state.contacts,
            secrets: &state.secrets,
            codec: &state.imports.codec,
            normalizer: &state.imports.nfc,
        }
        .execute(now())
        .await,
    )
}

/// Carnets du compte sur le serveur `server` (adresse, ou nom de domaine
/// seul : la découverte commence alors par l'adresse bien connue).
#[tauri::command]
#[specta::specta]
pub async fn find_carddav_books(
    state: State<'_, AppState>,
    server: String,
    username: String,
    password: String,
) -> Result<Vec<CardDavBookDto>, IpcError> {
    let server = server_of(&server)?;
    let credentials = credentials_of(&username, password)?;
    let books = DiscoverAddressBooks {
        client: state.carddav.client().map_err(remote_error)?,
    }
    .execute(&server, &credentials)
    .await
    .map_err(remote_error)?;
    state
        .carddav
        .remember(&server, &credentials.username, &books);
    Ok(books
        .iter()
        .map(|b| CardDavBookDto {
            url: b.url().to_string(),
            name: b.name().to_owned(),
        })
        .collect())
}

/// Relie le carnet `book_url` (proposé par la dernière recherche) ; le
/// mot de passe va au trousseau.
#[tauri::command]
#[specta::specta]
pub async fn connect_carddav(
    state: State<'_, AppState>,
    server: String,
    username: String,
    password: String,
    book_url: String,
) -> Result<CardDavStatusDto, IpcError> {
    let server = server_of(&server)?;
    let credentials = credentials_of(&username, password)?;
    // Pas pendant une synchronisation : elle écrirait les ressources de
    // l'ancien carnet sur l'état du nouveau.
    let guard = state
        .carddav
        .syncing
        .try_lock()
        .map_err(|_| IpcError::new(ErrorCodeDto::CardDavSyncBusy))?;
    let book = state
        .carddav
        .proposed(&server, &credentials.username, &book_url)
        .ok_or_else(|| IpcError::invalid(FieldDto::CardDavBook))?;
    let account = CardDavAccount::new(server, credentials.username, book)
        .map_err(|_| IpcError::invalid(FieldDto::CardDavBook))?;
    ConfigureCardDav {
        store: &state.carddav.store,
        secrets: &state.secrets,
    }
    .execute(&account, &credentials.password)
    .await
    .map_err(setup_error)?;
    drop(guard);
    status(&state).await.map_err(IpcError::storage)
}

/// Compte relié et dernière synchronisation.
#[tauri::command]
#[specta::specta]
pub async fn carddav_status(state: State<'_, AppState>) -> Result<CardDavStatusDto, IpcError> {
    status(&state).await.map_err(IpcError::storage)
}

/// Synchronise maintenant.
#[tauri::command]
#[specta::specta]
pub async fn sync_carddav(state: State<'_, AppState>) -> Result<CardDavReportDto, IpcError> {
    match sync_now(&state).await {
        None => Err(IpcError::new(ErrorCodeDto::CardDavSyncBusy)),
        Some(result) => result.map(report_dto).map_err(sync_error),
    }
}

/// Déconnecte le carnet distant : compte et mot de passe oubliés, contacts
/// gardés.
#[tauri::command]
#[specta::specta]
pub async fn disconnect_carddav(state: State<'_, AppState>) -> Result<(), IpcError> {
    let _guard = state
        .carddav
        .syncing
        .try_lock()
        .map_err(|_| IpcError::new(ErrorCodeDto::CardDavSyncBusy))?;
    RemoveCardDav {
        store: &state.carddav.store,
        secrets: &state.secrets,
    }
    .execute()
    .await
    .map_err(setup_error)
}

#[cfg(test)]
mod tests;
