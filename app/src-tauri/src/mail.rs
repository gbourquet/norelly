//! Messagerie au point d'assemblage : acteurs de synchronisation par compte
//! (démarrés au lancement et à l'ajout d'un compte, arrêtés à sa
//! suppression), événements vers l'interface, rendu des messages.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use norelly_application::account_actor::{self, AccountHandle, ActorClient, ActorConfig};
use norelly_application::accounts::connect_request;
use norelly_application::reading::{
    DownloadBody, DownloadPart, LoadedBody, LoadedMessage, ReadError, ReadMessage,
};
use norelly_application::sync::SyncEvent;
use norelly_blobs::FsBlobStore;
use norelly_domain::account::{Account, AccountId};
use norelly_domain::connection::{ConnectionError, TimeoutStage};
use norelly_domain::endpoint::Protocol;
use norelly_domain::fetch_policy::{BodyFetch, FetchPolicy, PartPath};
use norelly_domain::mailbox::{MailboxName, Uid};
use norelly_domain::ports::{FetchedBody, MessageId, RemoteBodyFetcher};
use norelly_imap::ImapConnector;
use norelly_render::{PartContent, Protocols, RenderRequest, Rendered, Source};
use norelly_sqlite::mail::SqliteMailStore;
use norelly_sqlite::preferences::SqliteReadingPreferences;
use norelly_sqlite::{Database, SqliteCertificateExceptionStore};
use tauri::{AppHandle, Runtime};
use tauri_specta::Event;

use crate::ipc::SyncEventDto;
use crate::remote_images::RemoteImageProxy;
use crate::state::AppSecretStore;

/// Délai maximal d'un téléchargement à la demande.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(120);
/// Nombre de messages dont la liste d'images distantes est gardée.
const RENDER_CACHE: usize = 32;

/// Services de messagerie partagés par les commandes et les protocoles.
#[derive(Debug)]
pub struct MailServices {
    pub store: Arc<SqliteMailStore>,
    pub blobs: Arc<FsBlobStore>,
    pub prefs: SqliteReadingPreferences,
    pub protocols: Protocols,
    /// `None` si le client HTTPS n'a pas pu être construit (images
    /// distantes alors indisponibles).
    pub proxy: Option<RemoteImageProxy>,
    /// Dossier privé des pièces jointes ouvertes.
    pub open_dir: PathBuf,
    imap: Arc<ImapConnector>,
    actors: tokio::sync::Mutex<BTreeMap<AccountId, AccountHandle>>,
    clients: Mutex<BTreeMap<AccountId, ActorClient>>,
    /// Images distantes autorisées des derniers messages rendus :
    /// (message, adresses dans l'ordre des index `remote-img://`).
    remote: Mutex<VecDeque<(MessageId, Vec<String>)>>,
}

impl MailServices {
    #[must_use]
    pub fn new(
        db: &Database,
        blobs: FsBlobStore,
        imap: ImapConnector,
        proxy: Option<RemoteImageProxy>,
        open_dir: PathBuf,
    ) -> Self {
        Self {
            store: Arc::new(SqliteMailStore::new(db.clone())),
            blobs: Arc::new(blobs),
            prefs: SqliteReadingPreferences::new(db.clone()),
            protocols: Protocols::for_platform(),
            proxy,
            open_dir,
            imap: Arc::new(imap),
            actors: tokio::sync::Mutex::new(BTreeMap::new()),
            clients: Mutex::new(BTreeMap::new()),
            remote: Mutex::new(VecDeque::new()),
        }
    }

    /// Démarre la synchronisation d'un compte (sans effet si elle tourne
    /// déjà). Les événements sont relayés à l'interface.
    pub async fn start<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        account: &Account,
        secrets: &AppSecretStore,
        exceptions: &SqliteCertificateExceptionStore,
    ) -> Result<(), String> {
        let mut actors = self.actors.lock().await;
        if actors.contains_key(&account.id()) {
            return Ok(());
        }
        let request = connect_request(account, Protocol::Imap, secrets, exceptions)
            .await
            .map_err(|e| e.to_string())?;
        let handle = account_actor::spawn(
            account.id(),
            request,
            Arc::clone(&self.imap),
            Arc::clone(&self.store),
            Arc::clone(&self.blobs),
            ActorConfig::default(),
        );
        let mut events = handle.subscribe();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if let Some(dto) = SyncEventDto::from_event(&event) {
                            let _ = dto.emit(&app);
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
        Ok(())
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
    }

    /// Demande une synchronisation complète d'un compte.
    pub async fn sync_now(&self, account: AccountId) {
        if let Some(handle) = self.actors.lock().await.get(&account) {
            handle.sync_now();
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
        };
        match download.execute(id).await {
            Ok(_) => self.reader().load(id).await,
            Err(e) => {
                tracing::warn!(error = %e, "corps indisponible");
                Ok(Some(loaded))
            }
        }
    }

    /// Rend un message ; mémorise ses images distantes autorisées.
    pub async fn render(&self, id: MessageId) -> Result<Option<Rendered>, ReadError> {
        let Some(loaded) = self.load(id).await? else {
            return Ok(None);
        };
        let rendered = with_source(&loaded.body, |source| {
            norelly_render::render(&RenderRequest {
                message: id.get(),
                source,
                structure: loaded.location.structure.as_ref(),
                remote: loaded.remote,
                prefer_plain_text: loaded.prefer_plain_text,
                protocols: &self.protocols,
            })
        });
        let Some(rendered) = rendered else {
            return Ok(Some(Rendered {
                document: norelly_render::document(
                    "<p>Le contenu de ce message n'est pas encore disponible.</p>",
                ),
                mode: norelly_render::BodyMode::Empty,
                has_html: false,
                remote_images: Vec::new(),
                blocked_images: 0,
                attachments: Vec::new(),
            }));
        };
        let mut remote = self.remote.lock().unwrap_or_else(PoisonError::into_inner);
        remote.retain(|(m, _)| *m != id);
        remote.push_back((id, rendered.remote_images.clone()));
        while remote.len() > RENDER_CACHE {
            remote.pop_front();
        }
        Ok(Some(rendered))
    }

    /// Adresse de l'image distante n° `index` du message, si elle a été
    /// autorisée lors du dernier rendu.
    #[must_use]
    pub fn remote_image(&self, id: MessageId, index: usize) -> Option<String> {
        self.remote
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(m, _)| *m == id)
            .and_then(|(_, urls)| urls.get(index).cloned())
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
            SyncEvent::Online { account } => Self::Online {
                account: id(*account)?,
            },
            SyncEvent::Offline {
                account, reason, ..
            } => Self::Offline {
                account: id(*account)?,
                reason: reason.clone(),
            },
        })
    }
}
