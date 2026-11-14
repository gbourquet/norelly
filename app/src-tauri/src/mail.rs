//! Messagerie au point d'assemblage : acteurs de synchronisation par compte
//! (démarrés au lancement et à l'ajout d'un compte, arrêtés à sa
//! suppression), événements vers l'interface, rendu des messages.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use norelly_application::account_actor::{self, AccountHandle, ActorClient, ActorConfig};
use norelly_application::accounts::connect_request;
use norelly_application::compose_worker::{self, ComposeEvent, ComposeHandle, ComposeWorker};
use norelly_application::folders::{FolderChange, FolderFailure};
use norelly_application::reading::{
    DownloadBody, DownloadPart, LoadedBody, LoadedMessage, ReadError, ReadMessage,
};
use norelly_application::sync::{SyncEvent, SyncFailure};
use norelly_blobs::FsBlobStore;
use norelly_domain::account::{Account, AccountId};
use norelly_domain::compose::draft::Recipient;
use norelly_domain::connection::{ConnectionError, TimeoutStage, server_text};
use norelly_domain::endpoint::Protocol;
use norelly_domain::fetch_policy::{BodyFetch, FetchPolicy, PartPath};
use norelly_domain::mailbox::{MailboxName, Uid};
use norelly_domain::ports::ConnectRequest;
use norelly_domain::ports::{FetchedBody, MessageId, RemoteBodyFetcher, SettingsStore};
use norelly_domain::reading::RemoteContent;
use norelly_keyring::SystemSecretStore;
use norelly_render::compose::MimeComposer;
use norelly_render::{PartContent, Protocols, RenderNotice, RenderRequest, Rendered, Source};
use norelly_sqlite::compose::SqliteComposeStore;
use norelly_sqlite::mail::SqliteMailStore;
use norelly_sqlite::preferences::{SqliteReadingPreferences, SqliteSettingsStore};
use norelly_sqlite::{Database, SqliteCertificateExceptionStore};
use tauri::{AppHandle, Runtime};
use tauri_specta::Event;

use crate::ipc::{ComposeEventDto, FailureKindDto, SyncEventDto, connection_failure_of};
use crate::remote_images::RemoteImageProxy;
use crate::state::AppSecretStore;

/// Délai maximal d'un téléchargement à la demande.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(120);
/// Délai maximal d'une modification de dossier (reconnexion comprise).
pub const FOLDER_CHANGE_TIMEOUT: Duration = Duration::from_secs(30);
/// Nombre de rendus gardés en cache.
const RENDER_CACHE: usize = 32;
/// Durée maximale d'un rendu.
const RENDER_TIMEOUT: Duration = Duration::from_secs(10);

/// Clé d'un rendu : message, décision sur les images, texte brut.
type RenderKey = (MessageId, RemoteContent, bool);

fn placeholder(notice: RenderNotice) -> Rendered {
    Rendered {
        notice: Some(notice),
        document: norelly_render::document(""),
        mode: norelly_render::BodyMode::Empty,
        has_html: false,
        remote_images: Vec::new(),
        blocked_images: 0,
        attachments: Vec::new(),
    }
}

/// Services de messagerie partagés par les commandes et les protocoles.
#[derive(Debug)]
pub struct MailServices {
    pub store: Arc<SqliteMailStore>,
    pub blobs: Arc<FsBlobStore>,
    pub prefs: SqliteReadingPreferences,
    /// Paramètres de l'application.
    pub settings: SqliteSettingsStore,
    pub protocols: Protocols,
    /// `None` si le client HTTPS n'a pas pu être construit (images
    /// distantes alors indisponibles).
    pub proxy: Option<RemoteImageProxy>,
    /// Dossier privé des pièces jointes ouvertes.
    pub open_dir: PathBuf,
    imap: Arc<crate::state::AppImap>,
    smtp: Arc<crate::state::AppSmtp>,
    /// Brouillons et file d'envoi.
    pub compose: Arc<SqliteComposeStore>,
    composer: Arc<MimeComposer>,
    workers: tokio::sync::Mutex<BTreeMap<AccountId, ComposeHandle>>,
    /// Derniers fichiers déposés sur la fenêtre (jeton, chemins venus de
    /// l'événement natif, jamais de l'interface).
    dropped: Mutex<Option<(u32, Vec<PathBuf>)>>,
    actors: tokio::sync::Mutex<BTreeMap<AccountId, AccountHandle>>,
    clients: Mutex<BTreeMap<AccountId, ActorClient>>,
    /// Derniers rendus : (message, décision images, texte brut) → rendu.
    rendered: Mutex<VecDeque<(RenderKey, Arc<Rendered>)>>,
}

impl MailServices {
    #[must_use]
    pub fn new(
        db: &Database,
        blobs: FsBlobStore,
        imap: crate::state::AppImap,
        smtp: crate::state::AppSmtp,
        proxy: Option<RemoteImageProxy>,
        open_dir: PathBuf,
    ) -> Self {
        Self {
            store: Arc::new(SqliteMailStore::new(db.clone())),
            blobs: Arc::new(blobs),
            prefs: SqliteReadingPreferences::new(db.clone()),
            settings: SqliteSettingsStore::new(db.clone()),
            protocols: Protocols::for_platform(),
            proxy,
            open_dir,
            imap: Arc::new(imap),
            smtp: Arc::new(smtp),
            compose: Arc::new(SqliteComposeStore::new(db.clone())),
            composer: Arc::new(MimeComposer),
            workers: tokio::sync::Mutex::new(BTreeMap::new()),
            dropped: Mutex::new(None),
            actors: tokio::sync::Mutex::new(BTreeMap::new()),
            clients: Mutex::new(BTreeMap::new()),
            rendered: Mutex::new(VecDeque::new()),
        }
    }

    /// Démarre la synchronisation d'un compte (sans effet si elle tourne
    /// déjà). Les événements sont relayés à l'interface.
    pub async fn start<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        account: &Account,
        secrets: &AppSecretStore<SystemSecretStore>,
        exceptions: &SqliteCertificateExceptionStore,
    ) -> Result<(), String> {
        let mut actors = self.actors.lock().await;
        if actors.contains_key(&account.id()) {
            return Ok(());
        }
        let request = connect_request(account, Protocol::Imap, secrets, exceptions)
            .await
            .map_err(|e| e.to_string())?;
        let settings = self.settings.settings().await.unwrap_or_default();
        let handle = account_actor::spawn(
            account.id(),
            request.clone(),
            Arc::clone(&self.imap),
            Arc::clone(&self.store),
            Arc::clone(&self.blobs),
            Arc::new(norelly_render::search_text::MimeTextExtractor),
            ActorConfig {
                policy: settings.fetch_policy(),
                ..ActorConfig::default()
            },
        );
        let mut events = handle.subscribe();
        let relay = app.clone();
        let settings_store = self.settings.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match events.recv().await {
                    // Arrivées : filtrées selon les paramètres du moment.
                    Ok(SyncEvent::NewMail { account, mail, .. }) => {
                        // Paramètres illisibles : notification sans
                        // contenu (échec fermé sur la confidentialité).
                        let current = settings_store.settings().await.unwrap_or(
                            norelly_domain::settings::Settings {
                                notification_privacy: true,
                                ..norelly_domain::settings::Settings::default()
                            },
                        );
                        if let Some(dto) = crate::settings::new_mail_event(account, mail, &current)
                        {
                            let _ = dto.emit(&relay);
                        }
                    }
                    Ok(event) => {
                        if let Some(dto) = SyncEventDto::from_event(&event) {
                            let _ = dto.emit(&relay);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(account.id(), handle.client());
        actors.insert(account.id(), handle);
        drop(actors);
        self.start_compose(app, account, request, secrets, exceptions)
            .await
    }

    /// Démarre le travailleur de rédaction d'un compte (envoi, copies).
    async fn start_compose<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        account: &Account,
        imap: ConnectRequest,
        secrets: &AppSecretStore<SystemSecretStore>,
        exceptions: &SqliteCertificateExceptionStore,
    ) -> Result<(), String> {
        let mut workers = self.workers.lock().await;
        if workers.contains_key(&account.id()) {
            return Ok(());
        }
        let smtp = connect_request(account, Protocol::Smtp, secrets, exceptions)
            .await
            .map_err(|e| e.to_string())?;
        let handle = compose_worker::spawn(ComposeWorker {
            account: account.id(),
            from: Recipient::new(account.display_name().as_str(), account.address().clone()),
            imap,
            smtp,
            connector: Arc::clone(&self.imap),
            sender: Arc::clone(&self.smtp),
            store: Arc::clone(&self.compose),
            folders: Arc::clone(&self.store),
            blobs: Arc::clone(&self.blobs),
            composer: Arc::clone(&self.composer),
        });
        let mut events = handle.subscribe();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(ComposeEvent::OutboxChanged { account }) => {
                        if let Ok(account) = u32::try_from(account.get()) {
                            let _ = ComposeEventDto::OutboxChanged { account }.emit(&app);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        workers.insert(account.id(), handle);
        Ok(())
    }

    /// Demande un passage du travailleur de rédaction d'un compte.
    pub async fn compose_soon(&self, account: AccountId) {
        if let Some(handle) = self.workers.lock().await.get(&account) {
            handle.trigger();
        }
    }

    /// Mémorise des fichiers déposés sur la fenêtre ; renvoie leur jeton.
    pub fn remember_dropped(&self, paths: Vec<PathBuf>) -> u32 {
        let mut dropped = self.dropped.lock().unwrap_or_else(PoisonError::into_inner);
        let token = dropped
            .as_ref()
            .map_or(1, |(t, _)| t.wrapping_add(1).max(1));
        *dropped = Some((token, paths));
        token
    }

    /// Fichiers déposés désignés par `token` (une seule fois).
    pub fn take_dropped(&self, token: u32) -> Option<Vec<PathBuf>> {
        let mut dropped = self.dropped.lock().unwrap_or_else(PoisonError::into_inner);
        match dropped.take() {
            Some((t, paths)) if t == token => Some(paths),
            other => {
                *dropped = other;
                None
            }
        }
    }

    /// Arrête la synchronisation d'un compte.
    pub async fn stop(&self, account: AccountId) {
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&account);
        let handle = self.actors.lock().await.remove(&account);
        if let Some(handle) = handle {
            handle.shutdown().await;
        }
        let worker = self.workers.lock().await.remove(&account);
        if let Some(worker) = worker {
            worker.shutdown().await;
        }
    }

    /// Demande une synchronisation complète d'un compte.
    pub async fn sync_now(&self, account: AccountId) {
        if let Some(handle) = self.actors.lock().await.get(&account) {
            handle.sync_now();
        }
    }

    /// Crée, renomme, déplace ou supprime un dossier par la connexion de
    /// commandes du compte (au plus [`FOLDER_CHANGE_TIMEOUT`]) ; renvoie le
    /// nom du dossier créé, renommé ou déplacé (nouveau nom), ou supprimé.
    pub async fn change_folder(
        &self,
        account: AccountId,
        change: FolderChange,
    ) -> Result<MailboxName, FolderFailure> {
        let client = self
            .clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&account)
            .cloned()
            .ok_or(FolderFailure::Connection(ConnectionError::Closed))?;
        tokio::time::timeout(FOLDER_CHANGE_TIMEOUT, client.change_folder(change))
            .await
            .map_err(|_| {
                FolderFailure::Connection(ConnectionError::Timeout(TimeoutStage::Response))
            })?
    }

    /// Signale de nouvelles opérations dans la file d'un compte.
    pub async fn replay_soon(&self, account: AccountId) {
        if let Some(handle) = self.actors.lock().await.get(&account) {
            handle.replay_soon();
        }
    }

    fn reader(&self) -> ReadMessage<'_, SqliteMailStore, FsBlobStore, SqliteReadingPreferences> {
        ReadMessage {
            locator: &self.store,
            blobs: &self.blobs,
            prefs: &self.prefs,
        }
    }

    /// Message chargé ; son corps est téléchargé s'il manque encore.
    pub async fn load(&self, id: MessageId) -> Result<Option<LoadedMessage>, ReadError> {
        let Some(loaded) = self.reader().load(id).await? else {
            return Ok(None);
        };
        if loaded.body != LoadedBody::Missing {
            return Ok(Some(loaded));
        }
        let download = DownloadBody {
            locator: &*self.store,
            blobs: &*self.blobs,
            store: &*self.store,
            fetcher: self,
            policy: FetchPolicy::default(),
            text: std::sync::Arc::new(norelly_render::search_text::MimeTextExtractor),
        };
        match download.execute(id).await {
            Ok(_) => self.reader().load(id).await,
            Err(e) => {
                tracing::warn!(error = %e, "corps indisponible");
                Ok(Some(loaded))
            }
        }
    }

    /// Rend un message (hors des workers async, durée bornée) ; le rendu
    /// est mis en cache par message et par décision (images, texte brut),
    /// avec la liste de ses images distantes autorisées et de ses parties
    /// `cid:`.
    pub async fn render(&self, id: MessageId) -> Result<Option<Arc<Rendered>>, ReadError> {
        let Some(loaded) = self.load(id).await? else {
            return Ok(None);
        };
        let key = (id, loaded.remote, loaded.prefer_plain_text);
        let body_ready = loaded.body != LoadedBody::Missing;
        if let Some(hit) = self
            .rendered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(k, _)| *k == key)
        {
            return Ok(Some(Arc::clone(&hit.1)));
        }
        let protocols = self.protocols.clone();
        let task = tokio::task::spawn_blocking(move || {
            with_source(&loaded.body, |source| {
                norelly_render::render(&RenderRequest {
                    message: id.get(),
                    source,
                    structure: loaded.location.structure.as_ref(),
                    remote: loaded.remote,
                    prefer_plain_text: loaded.prefer_plain_text,
                    protocols: &protocols,
                })
            })
        });
        let rendered = match tokio::time::timeout(RENDER_TIMEOUT, task).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(ReadError::Download(format!("rendu interrompu : {e}"))),
            Err(_) => Some(placeholder(RenderNotice::TooLong)),
        };
        let rendered =
            Arc::new(rendered.unwrap_or_else(|| placeholder(RenderNotice::NotAvailable)));
        if body_ready {
            let mut cache = self.rendered.lock().unwrap_or_else(PoisonError::into_inner);
            cache.retain(|(k, _)| k.0 != id);
            cache.push_back((key, Arc::clone(&rendered)));
            while cache.len() > RENDER_CACHE {
                cache.pop_front();
            }
        }
        Ok(Some(rendered))
    }

    fn cached(&self, id: MessageId) -> Option<Arc<Rendered>> {
        self.rendered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(k, _)| k.0 == id)
            .map(|(_, r)| Arc::clone(r))
    }

    /// Adresse de l'image distante n° `index` du message, si elle a été
    /// autorisée lors du dernier rendu.
    #[must_use]
    pub fn remote_image(&self, id: MessageId, index: usize) -> Option<String> {
        self.cached(id)
            .and_then(|r| r.remote_images.get(index).cloned())
    }

    /// Partie `cid:` d'un message rendu (protocole `mailpart://`) : seules
    /// les parties à `Content-ID` du dernier rendu de ce message sont
    /// servies, et jamais téléchargées à cette occasion.
    pub async fn inline_part(
        &self,
        id: MessageId,
        path: &PartPath,
    ) -> Result<Option<PartContent>, ReadError> {
        let listed = self.cached(id).is_some_and(|r| {
            r.attachments
                .iter()
                .any(|a| a.content_id.is_some() && a.available && &a.path == path)
        });
        if !listed {
            return Ok(None);
        }
        let Some(loaded) = self.reader().load(id).await? else {
            return Ok(None);
        };
        let path = path.clone();
        tokio::task::spawn_blocking(move || {
            with_source(&loaded.body, |s| norelly_render::part_content(&s, &path)).flatten()
        })
        .await
        .map_err(|e| ReadError::Download(e.to_string()))
    }

    /// Contenu décodé d'une partie ; une partie différée est d'abord
    /// téléchargée.
    pub async fn part(
        &self,
        id: MessageId,
        path: &PartPath,
    ) -> Result<Option<PartContent>, ReadError> {
        let Some(loaded) = self.load(id).await? else {
            return Ok(None);
        };
        if let LoadedBody::Partial { deferred, .. } = &loaded.body
            && deferred.contains(path)
        {
            DownloadPart {
                locator: &*self.store,
                blobs: &*self.blobs,
                store: &*self.store,
                fetcher: self,
            }
            .execute(id, path)
            .await?;
            let Some(reloaded) = self.reader().load(id).await? else {
                return Ok(None);
            };
            return Ok(
                with_source(&reloaded.body, |s| norelly_render::part_content(&s, path)).flatten(),
            );
        }
        Ok(with_source(&loaded.body, |s| norelly_render::part_content(&s, path)).flatten())
    }
}

/// Appelle `f` avec la source de rendu d'un corps chargé (`None` si le
/// corps est absent).
fn with_source<T>(body: &LoadedBody, f: impl FnOnce(Source<'_>) -> T) -> Option<T> {
    match body {
        LoadedBody::Missing => None,
        LoadedBody::Full(raw) => Some(f(Source::Full(raw))),
        LoadedBody::Partial {
            header,
            parts,
            deferred,
        } => Some(f(Source::Partial {
            header,
            parts: parts
                .iter()
                .map(|(p, b)| (p.clone(), b.as_slice()))
                .collect(),
            deferred: deferred.clone(),
        })),
    }
}

impl RemoteBodyFetcher for MailServices {
    async fn fetch(
        &self,
        account: AccountId,
        mailbox: &MailboxName,
        uid: Uid,
        fetch: &BodyFetch,
    ) -> Result<Option<FetchedBody>, ConnectionError> {
        let client = self
            .clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&account)
            .cloned()
            .ok_or(ConnectionError::Closed)?;
        tokio::time::timeout(
            FETCH_TIMEOUT,
            client.fetch_body(mailbox.clone(), uid, fetch.clone()),
        )
        .await
        .map_err(|_| ConnectionError::Timeout(TimeoutStage::Response))?
    }
}

impl SyncEventDto {
    fn from_event(event: &SyncEvent) -> Option<Self> {
        let id = |a: AccountId| u32::try_from(a.get()).ok();
        Some(match event {
            SyncEvent::FolderChanged { account, mailbox } => Self::FolderChanged {
                account: id(*account)?,
                mailbox: u32::try_from(mailbox.get()).ok()?,
            },
            SyncEvent::FoldersChanged { account } => Self::FoldersChanged {
                account: id(*account)?,
            },
            // Relayé à part, selon les paramètres de notification.
            SyncEvent::NewMail { .. } => return None,
            SyncEvent::SyncProgress {
                account,
                done,
                total,
            } => Self::SyncProgress {
                account: id(*account)?,
                done: u32::try_from(*done).unwrap_or(u32::MAX),
                total: u32::try_from(*total).unwrap_or(u32::MAX),
            },
            SyncEvent::Online { account } => Self::Online {
                account: id(*account)?,
            },
            SyncEvent::Offline {
                account, failure, ..
            } => {
                let (failure, detail) = sync_failure_of(failure);
                Self::Offline {
                    account: id(*account)?,
                    failure,
                    detail: detail.map(|d| server_text(d.as_bytes())),
                }
            }
        })
    }
}

/// Catégorie et détail technique brut d'un échec de synchronisation.
fn sync_failure_of(failure: &SyncFailure) -> (FailureKindDto, Option<String>) {
    match failure {
        SyncFailure::Connection(e) => connection_failure_of(e),
        SyncFailure::Store(_) => (FailureKindDto::Storage, None),
        SyncFailure::Reconcile { mailbox, .. } => (FailureKindDto::Protocol, Some(mailbox.clone())),
    }
}

#[cfg(test)]
mod sync_event_tests {
    use super::*;

    #[test]
    fn sync_progress_reaches_the_interface() {
        let Ok(account) = AccountId::new(1) else {
            panic!("identifiant valide refusé");
        };
        let dto = SyncEventDto::from_event(&SyncEvent::SyncProgress {
            account,
            done: 3,
            total: 61,
        });
        assert_eq!(
            serde_json::to_value(dto).unwrap_or_default(),
            serde_json::json!({ "kind": "sync_progress", "account": 1, "done": 3, "total": 61 })
        );
    }
}
