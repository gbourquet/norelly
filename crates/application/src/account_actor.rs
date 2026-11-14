//! Acteur de compte : deux connexions IMAP par compte.
//!
//! - **connexion de commandes** : synchronisation complète au démarrage
//!   puis périodiquement, synchronisation d'une boîte à la demande (IDLE,
//!   interface), corps téléchargés à la demande (pièces jointes
//!   différées) ;
//! - **connexion IDLE** sur la boîte de réception : renouvelée avant
//!   29 minutes ([`IDLE_RENEWAL_SECS`]) ; chaque changement signalé
//!   déclenche la synchronisation de la boîte de réception sur la
//!   connexion de commandes.
//!
//! La connexion de commandes rejoue aussi la file des actions
//! ([`ReplayActions`]) : à chaque ouverture de session (reconnexion), sur
//! demande ([`AccountHandle::replay_soon`]) et à l'échéance de la fenêtre
//! d'annulation de la prochaine opération ; les boîtes touchées sont
//! ensuite resynchronisées.
//!
//! Chaque connexion perdue est rouverte avec une attente exponentielle
//! bornée ([`Backoff`]), remise à zéro après un succès. Les événements
//! ([`SyncEvent`]) sont diffusés à tous les abonnés.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use norelly_domain::account::AccountId;
use norelly_domain::connection::ConnectionError;
use norelly_domain::fetch_policy::{BodyFetch, FetchPolicy};
use norelly_domain::mailbox::{MailboxName, MailboxRole, RemoteMailbox, Uid};
use norelly_domain::ports::{
    BlobStore, ConnectRequest, FetchedBody, IdleOutcome, MailStore, PendingOpStore,
    RemoteMailboxConnector, RemoteMailboxSession, SearchTextExtractor, StoredMailbox,
};
use norelly_domain::sync::{Backoff, IDLE_RENEWAL_SECS, SelectRequest, SyncExtensions};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::actions::{FollowFolder, ReplayActions, ReplayFailure};
use crate::folders::{FolderChange, FolderFailure};
use crate::sync::{SyncAccount, SyncEvent, SyncFailure, SyncStep};

/// Réglages de l'acteur.
#[derive(Debug, Clone, Copy)]
pub struct ActorConfig {
    /// Durée maximale d'un IDLE avant renouvellement.
    pub idle_renewal: Duration,
    /// Intervalle des synchronisations complètes.
    pub full_sync_every: Duration,
    /// Attente entre deux tentatives de connexion.
    pub backoff: Backoff,
    pub policy: FetchPolicy,
}

impl Default for ActorConfig {
    fn default() -> Self {
        Self {
            idle_renewal: Duration::from_secs(IDLE_RENEWAL_SECS),
            full_sync_every: Duration::from_secs(10 * 60),
            backoff: Backoff::default(),
            policy: FetchPolicy::default(),
        }
    }
}

/// Nouvel essai du rejeu après une erreur de stockage.
const REPLAY_RETRY: Duration = Duration::from_secs(30);

/// Délai laissé à chaque tâche pour s'arrêter proprement.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Réponse à une demande de corps.
type FetchReply = oneshot::Sender<Result<Option<FetchedBody>, ConnectionError>>;

/// Réponse à une modification de dossier.
type FolderReply = oneshot::Sender<Result<MailboxName, FolderFailure>>;

#[derive(Debug)]
enum Command {
    SyncAll,
    /// Rejouer les opérations dues de la file (et planifier les autres).
    Replay,
    SyncMailbox(MailboxName),
    /// Corps (ou parties différées) d'un message, à la demande.
    Fetch {
        mailbox: MailboxName,
        uid: Uid,
        fetch: BodyFetch,
        reply: FetchReply,
    },
    /// Création, renommage ou suppression d'un dossier, à la demande.
    Folder {
        change: FolderChange,
        reply: FolderReply,
    },
}

/// Poignée d'un acteur en cours d'exécution.
#[derive(Debug)]
pub struct AccountHandle {
    account: AccountId,
    commands: mpsc::Sender<Command>,
    halted: Halted,
    events: broadcast::Sender<SyncEvent>,
    stop: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl AccountHandle {
    #[must_use]
    pub fn account(&self) -> AccountId {
        self.account
    }

    /// Abonnement aux événements de synchronisation.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<SyncEvent> {
        self.events.subscribe()
    }

    /// Demande une synchronisation complète (sans attendre).
    pub fn sync_now(&self) {
        let _ = self.commands.try_send(Command::SyncAll);
    }

    /// Signale de nouvelles opérations dans la file : elles seront
    /// rejouées à leur échéance (sans attendre).
    pub fn replay_soon(&self) {
        let _ = self.commands.try_send(Command::Replay);
    }

    /// Client de l'acteur (clonable), pour lui adresser des demandes sans
    /// garder la poignée.
    #[must_use]
    pub fn client(&self) -> ActorClient {
        ActorClient {
            commands: self.commands.clone(),
            halted: Arc::clone(&self.halted),
        }
    }

    /// Voir [`ActorClient::fetch_body`].
    pub async fn fetch_body(
        &self,
        mailbox: MailboxName,
        uid: Uid,
        fetch: BodyFetch,
    ) -> Result<Option<FetchedBody>, ConnectionError> {
        self.client().fetch_body(mailbox, uid, fetch).await
    }

    /// Arrête l'acteur et attend la fin de ses tâches.
    /// Arrête l'acteur : les synchronisations en cours sont interrompues ;
    /// une tâche qui ne s'arrête pas d'elle-même dans [`SHUTDOWN_GRACE`]
    /// est abandonnée.
    pub async fn shutdown(self) {
        let _ = self.stop.send(true);
        for mut task in self.tasks {
            if tokio::time::timeout(SHUTDOWN_GRACE, &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

/// Client d'un acteur en cours d'exécution.
#[derive(Debug, Clone)]
pub struct ActorClient {
    commands: mpsc::Sender<Command>,
    halted: Halted,
}

/// Échec définitif qui a arrêté l'acteur
/// ([`SyncFailure::retry_is_useless`]), partagé avec ses clients.
type Halted = Arc<std::sync::Mutex<Option<ConnectionError>>>;

impl ActorClient {
    /// Erreur d'une demande que l'acteur ne recevra plus : la cause de son
    /// arrêt si elle est connue, sinon une connexion coupée.
    fn unreachable(&self) -> ConnectionError {
        self.halted
            .lock()
            .ok()
            .and_then(|h| h.clone())
            .unwrap_or(ConnectionError::Closed)
    }

    /// Télécharge un corps par la connexion de commandes (attend la
    /// reconnexion si elle est coupée ; à borner par l'appelant).
    pub async fn fetch_body(
        &self,
        mailbox: MailboxName,
        uid: Uid,
        fetch: BodyFetch,
    ) -> Result<Option<FetchedBody>, ConnectionError> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(Command::Fetch {
                mailbox,
                uid,
                fetch,
                reply,
            })
            .await
            .map_err(|_| self.unreachable())?;
        answer.await.map_err(|_| self.unreachable())?
    }

    /// Crée le dossier `label` (sous `parent`, ou au premier niveau) ;
    /// voir [`Self::change_folder`].
    pub async fn create_folder(
        &self,
        label: String,
        parent: Option<MailboxName>,
    ) -> Result<MailboxName, FolderFailure> {
        self.change_folder(FolderChange::Create { label, parent })
            .await
    }

    /// Renomme le dossier `folder` en `label`, à la même place ; renvoie
    /// son nouveau nom. Le dossier et ses sous-dossiers sont aussitôt
    /// resynchronisés sous leur nouveau nom.
    pub async fn rename_folder(
        &self,
        folder: MailboxName,
        label: String,
    ) -> Result<MailboxName, FolderFailure> {
        self.change_folder(FolderChange::Rename { folder, label })
            .await
    }

    /// Déplace le dossier `folder` sous `parent` (ou au premier niveau) ;
    /// renvoie son nouveau nom. Comme pour un renommage, le dossier et ses
    /// sous-dossiers sont aussitôt resynchronisés sous leur nouveau nom.
    pub async fn move_folder(
        &self,
        folder: MailboxName,
        parent: Option<MailboxName>,
    ) -> Result<MailboxName, FolderFailure> {
        self.change_folder(FolderChange::Move { folder, parent })
            .await
    }

    /// Supprime le dossier `folder` et ses messages.
    pub async fn delete_folder(&self, folder: MailboxName) -> Result<MailboxName, FolderFailure> {
        self.change_folder(FolderChange::Delete { folder }).await
    }

    /// Modifie un dossier par la connexion de commandes, puis relit la
    /// liste des dossiers (événement [`SyncEvent::FoldersChanged`]).
    /// Attend la reconnexion si elle est coupée ; à borner par l'appelant.
    pub async fn change_folder(&self, change: FolderChange) -> Result<MailboxName, FolderFailure> {
        let closed = || FolderFailure::Connection(self.unreachable());
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(Command::Folder { change, reply })
            .await
            .map_err(|_| closed())?;
        answer.await.map_err(|_| closed())?
    }
}

/// `mailbox` est le dossier `folder` ou l'un de ses sous-dossiers.
fn within(mailbox: &RemoteMailbox, folder: &MailboxName) -> bool {
    mailbox.name == *folder
        || mailbox.delimiter.is_some_and(|d| {
            mailbox
                .name
                .as_str()
                .strip_prefix(folder.as_str())
                .is_some_and(|rest| rest.starts_with(d))
        })
}

/// Gigue dans `[0, 1]`, tirée de l'horloge (pas besoin d'aléa
/// cryptographique : il s'agit d'étaler les reconnexions).
fn jitter_seed() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos())
}

/// Instant présent, en secondes Unix.
#[must_use]
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl From<ReplayFailure> for SyncFailure {
    fn from(e: ReplayFailure) -> Self {
        match e {
            ReplayFailure::Connection(e) => Self::Connection(e),
            ReplayFailure::Store(e) => Self::Store(e),
        }
    }
}

fn jitter() -> f64 {
    f64::from(jitter_seed() % 1000) / 1000.0
}

/// Dépendances partagées par les deux tâches.
struct Shared<C, M, B> {
    account: AccountId,
    request: ConnectRequest,
    connector: Arc<C>,
    store: Arc<M>,
    blobs: Arc<B>,
    text: Arc<dyn SearchTextExtractor>,
    config: ActorConfig,
    events: broadcast::Sender<SyncEvent>,
    halted: Halted,
}

/// Intervalle minimal entre deux signaux à l'interface pendant un tour
/// complet (rechargement, progression).
const LIVE_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

/// Cadence des signaux d'un tour complet.
#[derive(Default)]
struct LiveRefresh {
    refreshed: Option<tokio::time::Instant>,
    progressed: Option<tokio::time::Instant>,
}

impl LiveRefresh {
    /// Vrai si un rechargement peut être signalé maintenant.
    fn refresh_due(&mut self) -> bool {
        Self::due(&mut self.refreshed)
    }

    /// Vrai si la progression `done` peut être signalée maintenant (le
    /// départ l'est toujours).
    fn progress_due(&mut self, done: usize) -> bool {
        Self::due(&mut self.progressed) || done == 0
    }

    fn due(last: &mut Option<tokio::time::Instant>) -> bool {
        let now = tokio::time::Instant::now();
        if last.is_some_and(|t| now.duration_since(t) < LIVE_REFRESH_INTERVAL) {
            return false;
        }
        *last = Some(now);
        true
    }
}

impl<C, M, B> Shared<C, M, B>
where
    C: RemoteMailboxConnector,
    M: MailStore + PendingOpStore,
    B: BlobStore,
{
    fn emit(&self, event: SyncEvent) {
        let _ = self.events.send(event);
    }

    /// Connexion et activation des extensions.
    async fn open(&self) -> Result<(C::Session, SyncExtensions), SyncFailure> {
        let mut session = self.connector.connect(&self.request).await?;
        let ext = session.enable_sync_extensions().await?;
        Ok((session, ext))
    }

    /// Attend avant une nouvelle tentative ; `false` si l'acteur s'arrête.
    async fn wait_retry(
        &self,
        backoff: &mut Backoff,
        failure: &SyncFailure,
        stop: &mut watch::Receiver<bool>,
    ) -> bool {
        if failure.retry_is_useless() {
            // L'échec reste affiché et rendu aux demandes suivantes ;
            // l'acteur repart au prochain démarrage.
            tracing::warn!(account = %self.account, error = %failure, "synchronisation arrêtée");
            if let (SyncFailure::Connection(e), Ok(mut halted)) = (failure, self.halted.lock()) {
                *halted = Some(e.clone());
            }
            self.emit(SyncEvent::Offline {
                account: self.account,
                failure: failure.clone(),
                retry_in_ms: None,
            });
            return false;
        }
        let delay = backoff.next_delay_ms(jitter());
        tracing::warn!(account = %self.account, error = %failure, delay_ms = delay, "connexion perdue");
        self.emit(SyncEvent::Offline {
            account: self.account,
            failure: failure.clone(),
            retry_in_ms: Some(delay),
        });
        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(delay)) => !*stop.borrow(),
            _ = stop.changed() => false,
        }
    }

    fn use_case(&self) -> SyncAccount<'_, M, B> {
        SyncAccount {
            account: self.account,
            store: &self.store,
            blobs: &self.blobs,
            policy: self.config.policy,
            text: Arc::clone(&self.text),
        }
    }

    /// Boucle de la connexion de commandes.
    async fn command_loop(
        self: Arc<Self>,
        mut commands: mpsc::Receiver<Command>,
        mut stop: watch::Receiver<bool>,
    ) {
        let mut backoff = self.config.backoff;
        let mut known: Vec<StoredMailbox> = Vec::new();
        loop {
            if *stop.borrow() {
                return;
            }
            let failure = match self.open().await {
                Err(e) => e,
                Ok((mut session, ext)) => {
                    let failure = self
                        .serve(
                            &mut session,
                            ext,
                            &mut commands,
                            &mut stop,
                            &mut backoff,
                            &mut known,
                        )
                        .await;
                    match failure {
                        None => {
                            let _ = session.logout().await;
                            return;
                        }
                        Some(e) => e,
                    }
                }
            };
            if !self.wait_retry(&mut backoff, &failure, &mut stop).await {
                return;
            }
        }
    }

    /// Synchronisation complète ; événements pour les dossiers modifiés.
    async fn full_sync(
        &self,
        session: &mut C::Session,
        ext: SyncExtensions,
        known: &mut Vec<StoredMailbox>,
    ) -> Result<(), SyncFailure> {
        let mut changed = Vec::new();
        let mut arrivals = Vec::new();
        let account = self.account;
        let mut live = LiveRefresh::default();
        let mut finished = None;
        // Dossiers, en-têtes et progression s'affichent pendant le tour,
        // regroupés (au plus un signal par `LIVE_REFRESH_INTERVAL`) : un
        // compte aux milliers de dossiers ne submerge pas l'interface.
        self.use_case()
            .sync_all_observed(session, ext, |step| match step {
                SyncStep::Folders(all) => {
                    if known.as_slice() != all {
                        *known = all.to_vec();
                        self.emit(SyncEvent::FoldersChanged { account });
                    }
                }
                // La fin est annoncée après la mise à jour finale (plus bas).
                SyncStep::Progress { done, total } if done == total => {
                    finished = Some((done, total));
                }
                SyncStep::Progress { done, total } => {
                    if live.progress_due(done) {
                        self.emit(SyncEvent::SyncProgress {
                            account,
                            done,
                            total,
                        });
                    }
                }
                SyncStep::Headers(_) => {
                    if live.refresh_due() {
                        self.emit(SyncEvent::FoldersChanged { account });
                    }
                }
                SyncStep::MailboxDone(m, report) => {
                    if live.refresh_due() {
                        self.emit(SyncEvent::FoldersChanged { account });
                    }
                    changed.push(m.id);
                    if m.mailbox.role == MailboxRole::Inbox
                        && let Some(mail) = &report.arrivals
                    {
                        arrivals.push((m.id, mail.clone()));
                    }
                }
            })
            .await?;
        // L'état du serveur vient d'être rapporté : les actions encore en
        // attente gardent leur effet local, et les copies provisoires des
        // déplacements déjà rejoués laissent la place aux vrais messages.
        self.store.reapply(self.account).await?;
        self.store.purge_placeholders(self.account, None).await?;
        for mailbox in changed {
            self.emit(SyncEvent::FolderChanged {
                account: self.account,
                mailbox,
            });
        }
        for (mailbox, mail) in arrivals {
            self.emit(SyncEvent::NewMail {
                account: self.account,
                mailbox,
                mail,
            });
        }
        // Dernier événement du tour : s'il faut en perdre en route (relais
        // en retard), c'est lui qui reste.
        if let Some((done, total)) = finished {
            self.emit(SyncEvent::SyncProgress {
                account,
                done,
                total,
            });
        }
        Ok(())
    }

    /// Rejoue les opérations dues, resynchronise les boîtes touchées et
    /// renvoie l'échéance de la prochaine opération en attente.
    async fn replay(
        &self,
        session: &mut C::Session,
        ext: SyncExtensions,
        known: &mut Vec<StoredMailbox>,
    ) -> Result<Option<tokio::time::Instant>, SyncFailure> {
        let now = unix_now();
        let report = ReplayActions {
            account: self.account,
            store: &*self.store,
        }
        .execute(session, now)
        .await?;
        if !report.touched.is_empty() {
            self.sync_named(session, ext, known, report.touched.iter().cloned())
                .await?;
            // Effets locaux rétablis (opération abandonnée) : l'interface
            // doit relire ces boîtes même si le serveur n'a pas changé.
            for name in &report.touched {
                if let Some(m) = known.iter().find(|m| m.mailbox.name == *name) {
                    self.emit(SyncEvent::FolderChanged {
                        account: self.account,
                        mailbox: m.id,
                    });
                }
            }
        }
        let next = self
            .store
            .pending(self.account)
            .await?
            .iter()
            .map(|(_, op)| op.not_before)
            .min();
        Ok(next.map(|at| {
            let wait = u64::try_from(at.saturating_sub(now)).unwrap_or(0);
            // Une demi-seconde de marge : l'opération sera due à coup sûr.
            tokio::time::Instant::now() + Duration::from_millis(wait * 1000 + 500)
        }))
    }

    /// Sert les commandes sur une session ouverte. `None` : arrêt demandé ;
    /// `Some(échec)` : session à rouvrir.
    async fn serve(
        &self,
        session: &mut C::Session,
        ext: SyncExtensions,
        commands: &mut mpsc::Receiver<Command>,
        stop: &mut watch::Receiver<bool>,
        backoff: &mut Backoff,
        known: &mut Vec<StoredMailbox>,
    ) -> Option<SyncFailure> {
        // Interruptible : l'arrêt abandonne la synchronisation (la session
        // est alors inutilisable et seulement fermée).
        // Actions faites hors ligne : rejouées avant la synchronisation,
        // qui verra donc leur résultat.
        let mut next_replay = tokio::select! {
            _ = stop.changed() => return None,
            r = self.replay(session, ext, known) => match r {
                Ok(next) => next,
                Err(e) if e.is_connection() => return Some(e),
                Err(e) => {
                    tracing::error!(account = %self.account, error = %e, "rejeu des actions");
                    Some(tokio::time::Instant::now() + REPLAY_RETRY)
                }
            },
        };
        let first = tokio::select! {
            _ = stop.changed() => return None,
            r = self.full_sync(session, ext, known) => r,
        };
        if let Err(e) = first {
            return Some(e);
        }
        backoff.reset();
        self.emit(SyncEvent::Online {
            account: self.account,
        });
        let every = self.config.full_sync_every;
        let mut next_full = tokio::time::Instant::now() + every;
        loop {
            let replay_at = next_replay.unwrap_or(next_full);
            let command = tokio::select! {
                _ = stop.changed() => return None,
                () = tokio::time::sleep_until(next_full) => Some(Command::SyncAll),
                () = tokio::time::sleep_until(replay_at), if next_replay.is_some() => {
                    Some(Command::Replay)
                }
                c = commands.recv() => c,
            };
            let command = command?;
            // Regroupe les demandes en attente ; les corps demandés par
            // l'interface passent en premier.
            let mut all = false;
            let mut replay = false;
            let mut names = BTreeMap::new();
            let mut fetches = Vec::new();
            let mut folders = Vec::new();
            for command in
                std::iter::once(command).chain(std::iter::from_fn(|| commands.try_recv().ok()))
            {
                match command {
                    Command::SyncAll => all = true,
                    Command::Replay => replay = true,
                    Command::SyncMailbox(name) => {
                        names.insert(name, ());
                    }
                    Command::Fetch {
                        mailbox,
                        uid,
                        fetch,
                        reply,
                    } => fetches.push((mailbox, uid, fetch, reply)),
                    Command::Folder { change, reply } => folders.push((change, reply)),
                }
            }
            for (mailbox, uid, fetch, reply) in fetches {
                let result = tokio::select! {
                    _ = stop.changed() => return None,
                    r = fetch_on(session, &mailbox, uid, &fetch) => r,
                };
                let failure = match &result {
                    Err(e) if !matches!(e, ConnectionError::Rejected(_)) => Some(e.clone()),
                    _ => None,
                };
                let _ = reply.send(result);
                if let Some(e) = failure {
                    return Some(e.into());
                }
            }
            for (change, reply) in folders {
                // Demande abandonnée (délai de l'interface dépassé) : rien
                // n'est modifié, l'utilisateur a déjà vu l'échec.
                if reply.is_closed() {
                    continue;
                }
                let (result, failure) = tokio::select! {
                    _ = stop.changed() => return None,
                    r = self.change_folder(session, known, &change) => r,
                };
                // Dossier renommé ou déplacé : relu aussitôt sous son
                // nouveau nom, avec ses sous-dossiers (le cache local de
                // l'ancien nom est oublié avec la liste).
                if let (FolderChange::Rename { .. } | FolderChange::Move { .. }, Ok(renamed)) =
                    (&change, &result)
                {
                    for m in known.iter().filter(|m| within(&m.mailbox, renamed)) {
                        names.insert(m.mailbox.name.clone(), ());
                    }
                }
                let _ = reply.send(result);
                if let Some(e) = failure {
                    return Some(e);
                }
            }
            if replay || all {
                let result = tokio::select! {
                    _ = stop.changed() => return None,
                    r = self.replay(session, ext, known) => r,
                };
                match result {
                    Ok(next) => next_replay = next,
                    Err(e) if e.is_connection() => return Some(e),
                    Err(e) => {
                        // Erreur de stockage : nouvel essai plus tard, sans
                        // boucler.
                        tracing::error!(account = %self.account, error = %e, "rejeu des actions");
                        next_replay = Some(tokio::time::Instant::now() + REPLAY_RETRY);
                    }
                }
            }
            if !all && names.is_empty() {
                continue;
            }
            if all {
                next_full = tokio::time::Instant::now() + every;
            }
            let result = tokio::select! {
                _ = stop.changed() => return None,
                r = async {
                    if all {
                        self.full_sync(session, ext, known).await
                    } else {
                        self.sync_named(session, ext, known, names.into_keys()).await
                    }
                } => r,
            };
            match result {
                Ok(()) => {}
                Err(e) if e.is_connection() => return Some(e),
                // Erreur de stockage : journalisée ; la session reste
                // utilisable et la prochaine synchronisation réessaiera.
                Err(e) => tracing::error!(account = %self.account, error = %e, "synchronisation"),
            }
        }
    }

    /// Modifie un dossier puis relit la liste des dossiers. Renvoie la
    /// réponse à transmettre et, si la connexion est perdue, l'échec qui
    /// impose de la rouvrir : un dossier modifié reste annoncé comme tel
    /// même si la relecture échoue. Une erreur de stockage pendant la
    /// relecture est seulement journalisée (la prochaine synchronisation
    /// rattrapera la liste).
    async fn change_folder(
        &self,
        session: &mut C::Session,
        known: &mut Vec<StoredMailbox>,
        change: &FolderChange,
    ) -> (Result<MailboxName, FolderFailure>, Option<SyncFailure>) {
        let changed = match change.execute(session).await {
            Ok(changed) => changed,
            Err(FolderFailure::Connection(e)) if !matches!(e, ConnectionError::Rejected(_)) => {
                return (Err(FolderFailure::Connection(e.clone())), Some(e.into()));
            }
            Err(e) => return (Err(e), None),
        };
        tracing::info!(account = %self.account, "{}", change.kind());
        // Dossier renommé ou déplacé : les actions en attente qui le visent
        // suivent son nouveau nom (séparateur de la liste d'avant).
        if let FolderChange::Rename { folder, .. } | FolderChange::Move { folder, .. } = change {
            let delimiter = known
                .iter()
                .find(|m| m.mailbox.name == *folder)
                .and_then(|m| m.mailbox.delimiter);
            let follow = FollowFolder {
                account: self.account,
                store: &*self.store,
            };
            if let Err(e) = follow.execute(folder, &changed, delimiter).await {
                // Le dossier est bien changé sur le serveur : l'échec du
                // stockage est journalisé, les actions concernées seront
                // abandonnées au rejeu (état local rétabli).
                tracing::error!(account = %self.account, error = %e, "actions en attente non suivies");
            }
        }
        let failure = match self.use_case().sync_folders(session).await {
            Ok(mailboxes) => {
                if *known != mailboxes {
                    *known = mailboxes;
                    self.emit(SyncEvent::FoldersChanged {
                        account: self.account,
                    });
                }
                None
            }
            Err(e) if e.is_connection() => Some(e),
            Err(e) => {
                tracing::error!(account = %self.account, error = %e, "liste des dossiers");
                None
            }
        };
        (Ok(changed), failure)
    }

    async fn sync_named(
        &self,
        session: &mut C::Session,
        ext: SyncExtensions,
        known: &mut Vec<StoredMailbox>,
        names: impl Iterator<Item = MailboxName>,
    ) -> Result<(), SyncFailure> {
        for name in names {
            let Some((id, role)) = known
                .iter()
                .find(|m| m.mailbox.name == name)
                .map(|m| (m.id, m.mailbox.role))
            else {
                // Boîte inconnue (liste pas encore lue) : synchronisation
                // complète.
                return self.full_sync(session, ext, known).await;
            };
            let report = self
                .use_case()
                .sync_mailbox(session, ext, id, &name)
                .await?;
            self.store.reapply(self.account).await?;
            // Avant l'événement : l'interface ne voit ni trou ni doublon.
            let purged = self
                .store
                .purge_placeholders(self.account, Some(id))
                .await?;
            if report.changed() || purged > 0 {
                self.emit(SyncEvent::FolderChanged {
                    account: self.account,
                    mailbox: id,
                });
            }
            if role == MailboxRole::Inbox
                && let Some(mail) = report.arrivals
            {
                self.emit(SyncEvent::NewMail {
                    account: self.account,
                    mailbox: id,
                    mail,
                });
            }
        }
        Ok(())
    }

    /// Boucle de la connexion IDLE sur la boîte de réception.
    async fn idle_loop(
        self: Arc<Self>,
        commands: mpsc::Sender<Command>,
        mut stop: watch::Receiver<bool>,
    ) {
        let mut backoff = self.config.backoff;
        loop {
            if *stop.borrow() {
                return;
            }
            let failure = match self.open().await {
                Err(e) => e,
                Ok((mut session, ext)) => {
                    let request = SelectRequest {
                        condstore: ext.condstore,
                        qresync: None,
                    };
                    match session.select(&MailboxName::inbox(), &request).await {
                        Err(e) => e.into(),
                        Ok(_) => {
                            backoff.reset();
                            // Changements survenus avant l'IDLE : une
                            // synchronisation de rattrapage.
                            let _ = commands.try_send(Command::SyncMailbox(MailboxName::inbox()));
                            match self
                                .idle_until_failure(&mut session, &commands, &mut stop)
                                .await
                            {
                                None => {
                                    let _ = session.logout().await;
                                    return;
                                }
                                Some(e) => e,
                            }
                        }
                    }
                }
            };
            if !self.wait_retry(&mut backoff, &failure, &mut stop).await {
                return;
            }
        }
    }

    async fn idle_until_failure(
        &self,
        session: &mut C::Session,
        commands: &mpsc::Sender<Command>,
        stop: &mut watch::Receiver<bool>,
    ) -> Option<SyncFailure> {
        loop {
            let outcome = tokio::select! {
                _ = stop.changed() => return None,
                r = session.idle(self.config.idle_renewal) => r,
            };
            match outcome {
                Ok(IdleOutcome::Changed) => {
                    tracing::debug!(account = %self.account, "changement signalé par IDLE");
                    let _ = commands.try_send(Command::SyncMailbox(MailboxName::inbox()));
                }
                // Renouvellement : nouvel IDLE.
                Ok(IdleOutcome::Timeout) => {}
                Err(e) => return Some(e.into()),
            }
        }
    }
}

/// Sélectionne la boîte (lecture seule) puis télécharge le corps.
async fn fetch_on<S: RemoteMailboxSession>(
    session: &mut S,
    mailbox: &MailboxName,
    uid: Uid,
    fetch: &BodyFetch,
) -> Result<Option<FetchedBody>, ConnectionError> {
    let request = SelectRequest {
        condstore: false,
        qresync: None,
    };
    session.select(mailbox, &request).await?;
    session.fetch_body(uid, fetch).await
}

/// Démarre l'acteur d'un compte.
pub fn spawn<C, M, B>(
    account: AccountId,
    request: ConnectRequest,
    connector: Arc<C>,
    store: Arc<M>,
    blobs: Arc<B>,
    text: Arc<dyn SearchTextExtractor>,
    config: ActorConfig,
) -> AccountHandle
where
    C: RemoteMailboxConnector + 'static,
    C::Session: 'static,
    M: MailStore + PendingOpStore + 'static,
    B: BlobStore + 'static,
{
    let (events, _) = broadcast::channel(256);
    let (commands_tx, commands_rx) = mpsc::channel(64);
    let (stop_tx, stop_rx) = watch::channel(false);
    let halted: Halted = Arc::default();
    let shared = Arc::new(Shared {
        account,
        request,
        connector,
        store,
        blobs,
        text,
        config,
        events: events.clone(),
        halted: Arc::clone(&halted),
    });
    let tasks = vec![
        tokio::spawn(Arc::clone(&shared).command_loop(commands_rx, stop_rx.clone())),
        tokio::spawn(shared.idle_loop(commands_tx.clone(), stop_rx)),
    ];
    AccountHandle {
        account,
        commands: commands_tx,
        halted,
        events,
        stop: stop_tx,
        tasks,
    }
}

#[cfg(test)]
mod tests;
