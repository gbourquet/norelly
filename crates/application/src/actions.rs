//! Cas d'usage des actions sur les messages (phase 6) : application
//! optimiste et file durable ([`PerformAction`]), annulation
//! ([`UndoActions`]), rejeu sur le serveur ([`ReplayActions`]).
//!
//! Les règles (sémantique, fusion, conflits, stratégies) sont dans
//! `norelly_domain::actions` ; ce module ne fait qu'orchestrer les ports.

use std::collections::{BTreeMap, BTreeSet};

use norelly_domain::account::AccountId;
use norelly_domain::actions::{
    ActionError, Coalesce, Conflict, FolderRef, MAX_ACTION_TARGETS, MAX_REPLAY_ATTEMPTS,
    MessageAction, MoveStrategy, OpKind, PendingOp, ReplayDecision, ServerView, StoreOutcome,
    after_modified, coalesce, decide, expunge_is_safe, expunge_strategy, move_strategy, resolve,
};
use norelly_domain::connection::ConnectionError;
use norelly_domain::mailbox::{MailboxId, MailboxName, Uid};
use norelly_domain::ports::{
    MessageId, PendingOpId, PendingOpStore, RemoteMailboxSession, StoreError,
};

/// Échec d'une action.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionFailure {
    #[error(transparent)]
    Invalid(#[from] ActionError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Bilan d'une action.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionReport {
    /// Opérations enregistrées (annulables par [`UndoActions`]).
    pub recorded: Vec<PendingOpId>,
    /// Comptes dont la file a changé (rejeu à demander).
    pub accounts: BTreeSet<AccountId>,
    /// Messages sans effet : introuvables, déjà dans l'état voulu, ou
    /// boîte jamais synchronisée.
    pub skipped: usize,
}

/// Cas d'usage : appliquer une action à des messages.
#[derive(Debug)]
pub struct PerformAction<'a, S> {
    pub store: &'a S,
}

impl<S: PendingOpStore> PerformAction<'_, S> {
    /// Applique `action` à `messages` (instant `now`, secondes Unix) :
    /// effet local immédiat, opération enregistrée pour le rejeu. Une
    /// action impossible (pas de Corbeille…) n'en applique aucune.
    pub async fn execute(
        &self,
        messages: &[MessageId],
        action: MessageAction,
        now: i64,
    ) -> Result<ActionReport, ActionFailure> {
        if messages.len() > MAX_ACTION_TARGETS {
            return Err(ActionError::TooManyTargets.into());
        }
        let unique: BTreeSet<MessageId> = messages.iter().copied().collect();
        let mut folders: BTreeMap<AccountId, (Vec<FolderRef>, Option<MailboxId>)> = BTreeMap::new();
        // D'abord tout résoudre : une erreur n'applique rien.
        let mut planned = Vec::new();
        let mut report = ActionReport::default();
        for id in unique {
            let Some(target) = self.store.action_target(id).await? else {
                report.skipped += 1;
                continue;
            };
            let (list, fallback) = match folders.entry(target.account) {
                std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::btree_map::Entry::Vacant(e) => {
                    let list = self.store.action_folders(target.account).await?;
                    let fallback = self.store.archive_fallback(target.account).await?;
                    e.insert((list, fallback))
                }
            };
            let effect = resolve(action, &target.mailbox, list, *fallback)?;
            match PendingOp::new(&effect, &target, now) {
                Some(op) => planned.push(op),
                None => report.skipped += 1,
            }
        }
        for op in planned {
            let queue = self.store.pending(op.account).await?;
            let ops: Vec<PendingOp> = queue.iter().map(|(_, o)| o.clone()).collect();
            match coalesce(&ops, &op) {
                Coalesce::Push => report.recorded.push(self.store.record(&op).await?),
                Coalesce::Cancel(i) => {
                    // Déjà réservée par le rejeu : trop tard pour fusionner,
                    // la nouvelle opération est enregistrée à la suite.
                    let cancelled = match queue.get(i) {
                        Some((existing, _)) => self.store.cancel(*existing).await?,
                        None => false,
                    };
                    if !cancelled {
                        report.recorded.push(self.store.record(&op).await?);
                    }
                }
            }
            report.accounts.insert(op.account);
        }
        Ok(report)
    }
}

/// Cas d'usage : annuler des opérations encore en attente (dernière
/// action). Renvoie le nombre d'opérations annulées ; une opération déjà
/// rejouée, ou en cours de rejeu (réservée), n'est plus annulable.
#[derive(Debug)]
pub struct UndoActions<'a, S> {
    pub store: &'a S,
}

impl<S: PendingOpStore> UndoActions<'_, S> {
    pub async fn execute(&self, ops: &[PendingOpId]) -> Result<usize, StoreError> {
        let mut undone = 0;
        for id in ops {
            if self.store.cancel(*id).await? {
                undone += 1;
            }
        }
        Ok(undone)
    }
}

/// Bilan d'un rejeu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplayReport {
    /// Opérations exécutées (ou déjà acquises sur le serveur).
    pub applied: usize,
    /// Opérations abandonnées (conflit ou refus du serveur), état local
    /// rétabli.
    pub dropped: Vec<(PendingOpId, Option<Conflict>)>,
    /// Boîtes à resynchroniser (sources et destinations).
    pub touched: BTreeSet<MailboxName>,
}

/// Cas d'usage : rejouer les opérations dues d'un compte sur une session
/// ouverte. Une erreur de connexion interrompt le rejeu : les opérations
/// restantes attendent la reconnexion.
#[derive(Debug)]
pub struct ReplayActions<'a, S> {
    pub account: AccountId,
    pub store: &'a S,
}

/// Résultat de l'exécution d'une opération.
enum Executed {
    Done,
    Dropped(Option<Conflict>),
}

impl<S: PendingOpStore> ReplayActions<'_, S> {
    pub async fn execute<R: RemoteMailboxSession>(
        &self,
        session: &mut R,
        now: i64,
    ) -> Result<ReplayReport, ReplayFailure> {
        let mut report = ReplayReport::default();
        let mut existing: Option<BTreeSet<MailboxName>> = None;
        for (id, op) in self.store.pending(self.account).await? {
            if !op.is_due(now) {
                continue;
            }
            // Réservée avant tout envoi : l'utilisateur ne peut plus
            // l'annuler, et chaque essai est compté (même si la session
            // tombe pendant l'envoi).
            let Some(attempts) = self.store.claim(id).await? else {
                continue;
            };
            if attempts > MAX_REPLAY_ATTEMPTS {
                tracing::warn!(account = %self.account, attempts, "opération abandonnée après trop d'essais");
                self.store.revert(id).await?;
                report.touched.insert(op.mailbox.clone());
                report.dropped.push((id, Some(Conflict::TooManyAttempts)));
                continue;
            }
            if matches!(op.kind, OpKind::Move { .. }) && existing.is_none() {
                let listed = session.list_mailboxes().await?;
                existing = Some(listed.into_iter().map(|m| m.name).collect());
            }
            let outcome = match self.replay_one(session, &op, existing.as_ref()).await {
                Ok(outcome) => outcome,
                // Refus du serveur (`NO`) : l'opération ne passera pas,
                // l'état local est rétabli ; la session reste utilisable.
                Err(ConnectionError::Rejected(reason)) => {
                    tracing::warn!(account = %self.account, %reason, "opération refusée par le serveur");
                    Executed::Dropped(None)
                }
                Err(e) => return Err(e.into()),
            };
            report.touched.insert(op.mailbox.clone());
            if let OpKind::Move { to } = &op.kind {
                report.touched.insert(to.clone());
            }
            match outcome {
                Executed::Done => {
                    self.store.complete(id).await?;
                    report.applied += 1;
                }
                Executed::Dropped(conflict) => {
                    tracing::info!(account = %self.account, ?conflict, "opération abandonnée");
                    self.store.revert(id).await?;
                    report.dropped.push((id, conflict));
                }
            }
        }
        Ok(report)
    }

    async fn replay_one<R: RemoteMailboxSession>(
        &self,
        session: &mut R,
        op: &PendingOp,
        existing: Option<&BTreeSet<MailboxName>>,
    ) -> Result<Executed, ConnectionError> {
        let selected = match session.select_writable(&op.mailbox).await {
            Ok(s) => Some(s),
            // Boîte source disparue.
            Err(ConnectionError::Rejected(_)) => None,
            Err(e) => return Err(e),
        };
        let flags = match &selected {
            Some(_) => session.message_flags(op.uid).await?,
            None => None,
        };
        let view = ServerView {
            uid_validity: selected.and_then(|s| s.uid_validity),
            flags,
            target_exists: match &op.kind {
                OpKind::Move { to } => existing.is_some_and(|e| e.contains(to)),
                _ => true,
            },
        };
        match decide(op, &view) {
            ReplayDecision::AlreadyDone => return Ok(Executed::Done),
            ReplayDecision::Drop(conflict) => return Ok(Executed::Dropped(Some(conflict))),
            ReplayDecision::Apply => {}
        }
        match &op.kind {
            OpKind::Flag {
                change,
                unchanged_since,
                ..
            } => {
                if session
                    .store_flag(op.uid, *change, *unchanged_since)
                    .await?
                    == StoreOutcome::Modified
                {
                    let current = session.message_flags(op.uid).await?;
                    let Some(current) = current else {
                        return Ok(Executed::Dropped(Some(Conflict::MessageGone)));
                    };
                    if after_modified(*change, &current) == ReplayDecision::Apply {
                        session.store_flag(op.uid, *change, None).await?;
                    }
                }
            }
            OpKind::Move { to } => match move_strategy(session.capabilities()) {
                MoveStrategy::Move => session.move_to(op.uid, to).await?,
                strategy => {
                    session.copy_to(op.uid, to).await?;
                    remove(session, op.uid, strategy).await?;
                }
            },
            OpKind::Expunge => {
                let strategy = expunge_strategy(session.capabilities());
                remove(session, op.uid, strategy).await?;
            }
        }
        Ok(Executed::Done)
    }
}

/// Marque `uid` `\Deleted` puis le supprime selon `strategy`. Sans
/// UIDPLUS, l'`EXPUNGE` n'a lieu que si aucun autre message n'est marqué :
/// sinon le message reste `\Deleted` (masqué localement) jusqu'au prochain
/// `EXPUNGE` d'un client.
async fn remove<R: RemoteMailboxSession>(
    session: &mut R,
    uid: Uid,
    strategy: MoveStrategy,
) -> Result<(), ConnectionError> {
    session.mark_deleted(uid).await?;
    match strategy {
        MoveStrategy::Move | MoveStrategy::CopyUidExpunge => session.expunge_uid(uid).await,
        MoveStrategy::CopyGuardedExpunge => {
            let deleted = session.deleted_uids().await?;
            if expunge_is_safe(&BTreeSet::from([uid]), &deleted) {
                session.expunge_all().await
            } else {
                tracing::warn!(
                    others = deleted.len().saturating_sub(1),
                    "EXPUNGE différé : d'autres messages sont marqués \\Deleted"
                );
                Ok(())
            }
        }
    }
}

/// Échec d'un rejeu.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplayFailure {
    #[error(transparent)]
    Connection(#[from] ConnectionError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[cfg(test)]
mod tests;
