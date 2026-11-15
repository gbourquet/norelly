//! Travailleur de rédaction, un par compte : envoie la file (SMTP), puis
//! dépose les copies dans « Envoyés » et synchronise les brouillons sur
//! une connexion IMAP courte, ouverte seulement s'il y a du travail.
//!
//! Au démarrage, les envois restés « en cours » deviennent incertains
//! ([`RecoverOutbox`]). Ensuite, un passage ([`ComposeWorker::run_once`])
//! a lieu à chaque demande ([`ComposeHandle::trigger`]), à l'échéance du
//! prochain essai ou de la prochaine copie de brouillon, et au plus tard
//! toutes les [`IDLE_TICK`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use norelly_domain::account::AccountId;
use norelly_domain::compose::draft::{REMOTE_SAVE_INTERVAL_SECS, Recipient};
use norelly_domain::compose::outbox::{OutboxState, connection_hold, credentials_hold};
use norelly_domain::ports::{
    BlobStore, ConnectRequest, DraftStore, MailSender, MessageComposer, OutboxStore,
    PendingOpStore, RemoteMailboxConnector, RemoteMailboxSession,
};
use tokio::sync::{Notify, broadcast, watch};
use tokio::task::JoinHandle;

use crate::account_actor::{SHUTDOWN_GRACE, unix_now};
use crate::compose::{
    DeliverOutbox, DeliveryReport, FileSentCopies, RecoverOutbox, RemoteComposeFailure, SyncDrafts,
};

/// Passage au plus tard toutes les `IDLE_TICK`, même sans demande.
pub const IDLE_TICK: Duration = Duration::from_secs(60);

/// Événement de la rédaction, relayé à l'interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeEvent {
    /// La file d'envoi ou les brouillons d'un compte ont changé.
    OutboxChanged { account: AccountId },
}

/// Échéances avant lesquelles un passage ne présente plus les
/// identifiants d'un serveur, après un refus ou une interruption
/// ([`credentials_hold`], [`connection_hold`]) ; levées par une action de
/// l'utilisateur.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Holds {
    /// Envoi (SMTP).
    pub send: Option<i64>,
    /// Travail sur le serveur de réception (copies dans « Envoyés »,
    /// brouillons).
    pub imap: Option<i64>,
}

/// Bilan d'un passage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tick {
    pub delivery: DeliveryReport,
    /// Une connexion IMAP a été ouverte.
    pub imap_used: bool,
    /// Copies déposées dans « Envoyés ».
    pub filed: usize,
    /// Brouillons copiés sur le serveur.
    pub drafts_saved: usize,
    /// Prochain passage utile (secondes Unix).
    pub next_wake: Option<i64>,
}

/// Dépendances d'un travailleur.
#[derive(Debug)]
pub struct ComposeWorker<C, S, St, F, B, M> {
    pub account: AccountId,
    pub from: Recipient,
    pub imap: ConnectRequest,
    pub smtp: ConnectRequest,
    pub connector: Arc<C>,
    pub sender: Arc<S>,
    /// Brouillons et file d'envoi.
    pub store: Arc<St>,
    /// Dossiers du compte (rôles `\Sent`, `\Drafts`).
    pub folders: Arc<F>,
    pub blobs: Arc<B>,
    pub composer: Arc<M>,
}

impl<C, S, St, F, B, M> ComposeWorker<C, S, St, F, B, M>
where
    C: RemoteMailboxConnector,
    S: MailSender,
    St: DraftStore + OutboxStore,
    F: PendingOpStore,
    B: BlobStore,
    M: MessageComposer,
{
    /// Un passage à l'instant `now`.
    pub async fn run_once(&self, now: i64) -> Result<Tick, RemoteComposeFailure> {
        self.run_once_after(now, &mut Holds::default()).await
    }

    /// Un passage à l'instant `now`, en respectant les attentes qui suivent
    /// des identifiants refusés ou interrompus ([`Holds`]).
    pub async fn run_once_after(
        &self,
        now: i64,
        holds: &mut Holds,
    ) -> Result<Tick, RemoteComposeFailure> {
        let held = |hold: Option<i64>| hold.filter(|until| *until > now);
        let delivery = match held(holds.send) {
            Some(until) => DeliveryReport {
                next_attempt: Some(until),
                ..DeliveryReport::default()
            },
            None => {
                DeliverOutbox {
                    account: self.account,
                    outbox: &*self.store,
                    sender: &*self.sender,
                }
                .execute(&self.smtp, now)
                .await?
            }
        };
        // Posée avant le travail distant : son échec ne doit pas la perdre.
        if let Some(until) = credentials_hold(delivery.failed.iter().map(|(_, f)| f), now) {
            holds.send = Some(until);
        }
        let mut tick = Tick {
            delivery,
            ..Tick::default()
        };
        let has_sent = self
            .store
            .outbox(self.account)
            .await?
            .iter()
            .any(|i| i.entry.state == OutboxState::Sent);
        let drafts = self.store.drafts(self.account).await?;
        let drafts_due = drafts.iter().any(|d| d.sync.remote_save_due(now));
        let trash = !self.store.remote_trash(self.account).await?.is_empty();
        let imap_held = held(holds.imap);
        if (has_sent || drafts_due || trash) && imap_held.is_none() {
            let kind = self.imap.credentials.kind();
            let mut session = self.connector.connect(&self.imap).await.inspect_err(|e| {
                if let Some(until) = connection_hold(e, kind, now) {
                    holds.imap = Some(until);
                }
            })?;
            tick.imap_used = true;
            let result = self.remote_work(&mut session, now, &mut tick).await;
            let _ = session.logout().await;
            result?;
        }
        // Prochaine copie de brouillon : modifié, copié trop récemment.
        let next_draft = self
            .store
            .drafts(self.account)
            .await?
            .iter()
            .filter(|d| d.sync.remote_revision != Some(d.sync.revision))
            .filter_map(|d| d.sync.last_remote_save)
            .map(|last| last.saturating_add(REMOTE_SAVE_INTERVAL_SECS))
            .filter(|at| *at > now)
            .min();
        tick.next_wake = [tick.delivery.next_attempt, next_draft, imap_held]
            .into_iter()
            .flatten()
            .min();
        Ok(tick)
    }

    async fn remote_work(
        &self,
        session: &mut C::Session,
        now: i64,
        tick: &mut Tick,
    ) -> Result<(), RemoteComposeFailure> {
        tick.filed = FileSentCopies {
            account: self.account,
            outbox: &*self.store,
            folders: &*self.folders,
        }
        .execute(session)
        .await?;
        tick.drafts_saved = SyncDrafts {
            account: self.account,
            drafts: &*self.store,
            folders: &*self.folders,
            blobs: &*self.blobs,
            composer: &*self.composer,
        }
        .execute(session, &self.from, now)
        .await?
        .saved;
        Ok(())
    }
}

/// Demande de passage ; `by_user` : demandé par l'utilisateur (lève
/// l'attente qui suit un échec des identifiants).
#[derive(Debug, Default)]
struct Wake {
    notify: Notify,
    by_user: AtomicBool,
}

/// Poignée d'un travailleur en cours d'exécution.
#[derive(Debug)]
pub struct ComposeHandle {
    trigger: Arc<Wake>,
    events: broadcast::Sender<ComposeEvent>,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl ComposeHandle {
    /// Demande un passage (nouveau message en file, brouillon enregistré).
    pub fn trigger(&self) {
        self.trigger.notify.notify_one();
    }

    /// Demande un passage à l'initiative de l'utilisateur (envoi,
    /// « Réessayer ») : lève aussi l'attente qui suit des identifiants
    /// refusés ou interrompus.
    pub fn retry_now(&self) {
        self.trigger.by_user.store(true, Ordering::Relaxed);
        self.trigger.notify.notify_one();
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ComposeEvent> {
        self.events.subscribe()
    }

    /// Arrête le travailleur (un envoi en cours est abandonné au-delà de
    /// [`SHUTDOWN_GRACE`] ; il sera jugé incertain au prochain démarrage).
    pub async fn shutdown(self) {
        let _ = self.stop.send(true);
        let mut task = self.task;
        if tokio::time::timeout(SHUTDOWN_GRACE, &mut task)
            .await
            .is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }
}

/// Démarre le travailleur d'un compte.
pub fn spawn<C, S, St, F, B, M>(worker: ComposeWorker<C, S, St, F, B, M>) -> ComposeHandle
where
    C: RemoteMailboxConnector + 'static,
    S: MailSender + 'static,
    St: DraftStore + OutboxStore + 'static,
    F: PendingOpStore + 'static,
    B: BlobStore + 'static,
    M: MessageComposer + 'static,
    C::Session: Send,
{
    let trigger = Arc::new(Wake::default());
    let (events, _) = broadcast::channel(64);
    let (stop, stop_rx) = watch::channel(false);
    let task = tokio::spawn(run(worker, Arc::clone(&trigger), events.clone(), stop_rx));
    ComposeHandle {
        trigger,
        events,
        stop,
        task,
    }
}

async fn run<C, S, St, F, B, M>(
    worker: ComposeWorker<C, S, St, F, B, M>,
    trigger: Arc<Wake>,
    events: broadcast::Sender<ComposeEvent>,
    mut stop: watch::Receiver<bool>,
) where
    C: RemoteMailboxConnector,
    S: MailSender,
    St: DraftStore + OutboxStore,
    F: PendingOpStore,
    B: BlobStore,
    M: MessageComposer,
{
    let account = worker.account;
    match (RecoverOutbox {
        account,
        outbox: &*worker.store,
    })
    .execute()
    .await
    {
        Ok(0) => {}
        Ok(_) => {
            let _ = events.send(ComposeEvent::OutboxChanged { account });
        }
        Err(e) => tracing::error!(%account, error = %e, "reprise de la file d'envoi"),
    }
    // Ni envoi ni travail distant avant ces échéances après des identifiants refusés ou
    // interrompus ; levée par une action de l'utilisateur (réveil).
    let mut holds = Holds::default();
    loop {
        if *stop.borrow() {
            return;
        }
        let now = unix_now();
        let wait = tokio::select! {
            _ = stop.changed() => return,
            r = worker.run_once_after(now, &mut holds) => match r {
                Ok(tick) => {
                    if !tick.delivery.sent.is_empty()
                        || !tick.delivery.failed.is_empty()
                        || tick.filed > 0
                    {
                        let _ = events.send(ComposeEvent::OutboxChanged { account });
                    }
                    tick.next_wake
                        .map(|at| Duration::from_secs(u64::try_from(at.saturating_sub(now)).unwrap_or(0)))
                        .map_or(IDLE_TICK, |d| d.min(IDLE_TICK))
                }
                Err(e) => {
                    // Un envoi a pu avoir lieu avant l'échec du travail
                    // distant.
                    let _ = events.send(ComposeEvent::OutboxChanged { account });
                    tracing::warn!(%account, error = %e, "rédaction : nouvel essai plus tard");
                    IDLE_TICK
                }
            },
        };
        tokio::select! {
            _ = stop.changed() => return,
            () = trigger.notify.notified() => {
                if trigger.by_user.swap(false, Ordering::Relaxed) {
                    holds = Holds::default();
                }
            }
            () = tokio::time::sleep(wait) => {}
        }
    }
}

#[cfg(test)]
mod tests;
