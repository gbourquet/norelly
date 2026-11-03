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
use std::time::Duration;

use norelly_domain::account::AccountId;
use norelly_domain::compose::draft::{REMOTE_SAVE_INTERVAL_SECS, Recipient};
use norelly_domain::compose::outbox::OutboxState;
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
        let delivery = DeliverOutbox {
            account: self.account,
            outbox: &*self.store,
            sender: &*self.sender,
        }
        .execute(&self.smtp, now)
        .await?;
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
        if has_sent || drafts_due || trash {
            let mut session = self.connector.connect(&self.imap).await?;
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
        tick.next_wake = [tick.delivery.next_attempt, next_draft]
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

/// Poignée d'un travailleur en cours d'exécution.
#[derive(Debug)]
pub struct ComposeHandle {
    trigger: Arc<Notify>,
    events: broadcast::Sender<ComposeEvent>,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl ComposeHandle {
    /// Demande un passage (nouveau message en file, brouillon enregistré).
    pub fn trigger(&self) {
        self.trigger.notify_one();
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
    let trigger = Arc::new(Notify::new());
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
    trigger: Arc<Notify>,
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
    loop {
        if *stop.borrow() {
            return;
        }
        let now = unix_now();
        let wait = tokio::select! {
            _ = stop.changed() => return,
            r = worker.run_once(now) => match r {
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
                    tracing::warn!(%account, error = %e, "rédaction : nouvel essai plus tard");
                    IDLE_TICK
                }
            },
        };
        tokio::select! {
            _ = stop.changed() => return,
            () = trigger.notified() => {}
            () = tokio::time::sleep(wait) => {}
        }
    }
}

#[cfg(test)]
mod tests;
