//! Acteur de compte : deux connexions IMAP par compte.
//!
//! - **connexion de commandes** : synchronisation complète au démarrage
//!   puis périodiquement, synchronisation d'une boîte à la demande (IDLE,
//!   interface) ;
//! - **connexion IDLE** sur la boîte de réception : renouvelée avant
//!   29 minutes ([`IDLE_RENEWAL_SECS`]) ; chaque changement signalé
//!   déclenche la synchronisation de la boîte de réception sur la
//!   connexion de commandes.
//!
//! Chaque connexion perdue est rouverte avec une attente exponentielle
//! bornée ([`Backoff`]), remise à zéro après un succès. Les événements
//! ([`SyncEvent`]) sont diffusés à tous les abonnés.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use norelly_domain::account::AccountId;
use norelly_domain::fetch_policy::FetchPolicy;
use norelly_domain::mailbox::{MailboxId, MailboxName};
use norelly_domain::ports::{
    BlobStore, ConnectRequest, IdleOutcome, MailStore, RemoteMailboxConnector,
    RemoteMailboxSession, StoredMailbox,
};
use norelly_domain::sync::{Backoff, IDLE_RENEWAL_SECS, SelectRequest, SyncExtensions};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;

use crate::sync::{SyncAccount, SyncEvent, SyncFailure};

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

#[derive(Debug)]
enum Command {
    SyncAll,
    SyncMailbox(MailboxName),
}

/// Poignée d'un acteur en cours d'exécution.
#[derive(Debug)]
pub struct AccountHandle {
    account: AccountId,
    commands: mpsc::Sender<Command>,
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

    /// Arrête l'acteur et attend la fin de ses tâches.
    pub async fn shutdown(self) {
        let _ = self.stop.send(true);
        for task in self.tasks {
            let _ = task.await;
        }
    }
}

/// Gigue dans `[0, 1]`, tirée de l'horloge (pas besoin d'aléa
/// cryptographique : il s'agit d'étaler les reconnexions).
fn jitter() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    f64::from(nanos % 1000) / 1000.0
}

/// Dépendances partagées par les deux tâches.
struct Shared<C, M, B> {
    account: AccountId,
    request: ConnectRequest,
    connector: Arc<C>,
    store: Arc<M>,
    blobs: Arc<B>,
    config: ActorConfig,
    events: broadcast::Sender<SyncEvent>,
}

impl<C, M, B> Shared<C, M, B>
where
    C: RemoteMailboxConnector,
    M: MailStore,
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
        let delay = backoff.next_delay_ms(jitter());
        tracing::warn!(account = %self.account, error = %failure, delay_ms = delay, "connexion perdue");
        self.emit(SyncEvent::Offline {
            account: self.account,
            reason: failure.to_string(),
            retry_in_ms: delay,
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
        let mailboxes = self
            .use_case()
            .sync_all(session, ext, |id| changed.push(id))
            .await?;
        if *known != mailboxes {
            *known = mailboxes;
            self.emit(SyncEvent::FoldersChanged {
                account: self.account,
            });
        }
        for mailbox in changed {
            self.emit(SyncEvent::FolderChanged {
                account: self.account,
                mailbox,
            });
        }
        Ok(())
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
        if let Err(e) = self.full_sync(session, ext, known).await {
            return Some(e);
        }
        backoff.reset();
        self.emit(SyncEvent::Online {
            account: self.account,
        });
        let every = self.config.full_sync_every;
        let mut next_full = tokio::time::Instant::now() + every;
        loop {
            let command = tokio::select! {
                _ = stop.changed() => return None,
                () = tokio::time::sleep_until(next_full) => Some(Command::SyncAll),
                c = commands.recv() => c,
            };
            let command = command?;
            // Regroupe les demandes en attente.
            let mut all = matches!(command, Command::SyncAll);
            let mut names = BTreeMap::new();
            if let Command::SyncMailbox(name) = command {
                names.insert(name, ());
            }
            while let Ok(more) = commands.try_recv() {
                match more {
                    Command::SyncAll => all = true,
                    Command::SyncMailbox(name) => {
                        names.insert(name, ());
                    }
                }
            }
            let result = if all {
                next_full = tokio::time::Instant::now() + every;
                self.full_sync(session, ext, known).await
            } else {
                self.sync_named(session, ext, known, names.into_keys())
                    .await
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

    async fn sync_named(
        &self,
        session: &mut C::Session,
        ext: SyncExtensions,
        known: &mut Vec<StoredMailbox>,
        names: impl Iterator<Item = MailboxName>,
    ) -> Result<(), SyncFailure> {
        for name in names {
            let Some(id): Option<MailboxId> =
                known.iter().find(|m| m.mailbox.name == name).map(|m| m.id)
            else {
                // Boîte inconnue (liste pas encore lue) : synchronisation
                // complète.
                return self.full_sync(session, ext, known).await;
            };
            let report = self
                .use_case()
                .sync_mailbox(session, ext, id, &name)
                .await?;
            if report.changed() {
                self.emit(SyncEvent::FolderChanged {
                    account: self.account,
                    mailbox: id,
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

/// Démarre l'acteur d'un compte.
pub fn spawn<C, M, B>(
    account: AccountId,
    request: ConnectRequest,
    connector: Arc<C>,
    store: Arc<M>,
    blobs: Arc<B>,
    config: ActorConfig,
) -> AccountHandle
where
    C: RemoteMailboxConnector + 'static,
    C::Session: 'static,
    M: MailStore + 'static,
    B: BlobStore + 'static,
{
    let (events, _) = broadcast::channel(256);
    let (commands_tx, commands_rx) = mpsc::channel(64);
    let (stop_tx, stop_rx) = watch::channel(false);
    let shared = Arc::new(Shared {
        account,
        request,
        connector,
        store,
        blobs,
        config,
        events: events.clone(),
    });
    let tasks = vec![
        tokio::spawn(Arc::clone(&shared).command_loop(commands_rx, stop_rx.clone())),
        tokio::spawn(shared.idle_loop(commands_tx.clone(), stop_rx)),
    ];
    AccountHandle {
        account,
        commands: commands_tx,
        events,
        stop: stop_tx,
        tasks,
    }
}

#[cfg(test)]
mod tests;
