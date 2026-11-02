//! Cas d'usage `SyncAccount` : synchronisation d'un compte, boîte par
//! boîte, selon le rapprochement du domaine ([`norelly_domain::sync`]).
//!
//! Pour chaque boîte :
//! 1. `SELECT` (en lecture seule) avec les paramètres CONDSTORE/QRESYNC
//!    issus de l'état local ;
//! 2. plan, requêtes de drapeaux (et liste des UID si nécessaire),
//!    rapprochement ;
//! 3. enregistrement par lots : le premier lot porte suppressions,
//!    changements de drapeaux et premiers nouveaux messages ; chaque lot
//!    intermédiaire enregistre un état sans `HIGHESTMODSEQ`, de sorte
//!    qu'une interruption reprend par comparaison des UID ; le dernier
//!    lot enregistre l'état final ;
//! 4. corps des messages selon la politique de rapatriement, du plus
//!    récent au plus ancien, enregistrés un à un (reprise naturelle après
//!    interruption).
//!
//! Chaque boîte modifiée produit un événement [`SyncEvent::FolderChanged`].

use norelly_domain::account::AccountId;
use norelly_domain::connection::ConnectionError;
use norelly_domain::fetch_policy::{BodyFetch, FetchPolicy};
use norelly_domain::mailbox::{MailboxId, MailboxName, classify_mailboxes};
use norelly_domain::ports::{
    BlobStore, FetchedBody, MAX_FETCH_BATCH, MailStore, PendingBody, RemoteMailboxSession,
    StoreError, StoredBody, StoredMailbox, StoredMessage, SyncBatch,
};
use norelly_domain::sync::{
    ServerQuery, ServerReport, SyncError, SyncExtensions, SyncPlan, needs_uid_list, plan,
    reconcile, select_request,
};

/// Nombre de corps téléchargés par tour de [`SyncAccount::fetch_bodies`].
const BODY_BATCH: usize = 50;

/// Événement de synchronisation, diffusé à l'interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncEvent {
    /// Le contenu ou les compteurs d'un dossier ont changé.
    FolderChanged {
        account: AccountId,
        mailbox: MailboxId,
    },
    /// La liste des dossiers a changé (ou a été lue pour la première fois).
    FoldersChanged { account: AccountId },
    /// Connexion établie (après une coupure éventuelle).
    Online { account: AccountId },
    /// Connexion perdue ; nouvelle tentative prévue. L'échec est transmis
    /// structuré : l'interface en tire un message localisé (ADR-011).
    Offline {
        account: AccountId,
        failure: SyncFailure,
        retry_in_ms: u64,
    },
}

/// Échec d'une synchronisation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SyncFailure {
    #[error(transparent)]
    Connection(#[from] ConnectionError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("boîte « {mailbox} » : {error}")]
    Reconcile { mailbox: String, error: SyncError },
}

impl SyncFailure {
    /// Vrai si la connexion est à rouvrir (erreur réseau ou protocole) ;
    /// faux pour une commande simplement refusée (`NO`).
    #[must_use]
    pub fn is_connection(&self) -> bool {
        matches!(self, Self::Connection(e) if !matches!(e, ConnectionError::Rejected(_)))
    }
}

/// Bilan de la synchronisation d'une boîte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MailboxReport {
    pub plan: Option<SyncPlanKind>,
    pub new: usize,
    pub removed: usize,
    pub flag_changes: usize,
    pub bodies: usize,
}

impl MailboxReport {
    /// Vrai si la boîte a changé localement.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.new + self.removed + self.flag_changes + self.bodies > 0
    }
}

/// Stratégie employée (bilan, journal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPlanKind {
    UpToDate,
    Full,
    Qresync,
    Condstore,
    UidCompare,
}

impl From<SyncPlan> for SyncPlanKind {
    fn from(p: SyncPlan) -> Self {
        match p {
            SyncPlan::UpToDate => Self::UpToDate,
            SyncPlan::Full { .. } => Self::Full,
            SyncPlan::Qresync => Self::Qresync,
            SyncPlan::Condstore { .. } => Self::Condstore,
            SyncPlan::UidCompare => Self::UidCompare,
        }
    }
}

/// Cas d'usage : synchroniser un compte sur une session ouverte.
#[derive(Debug)]
pub struct SyncAccount<'a, M, B> {
    pub account: AccountId,
    pub store: &'a M,
    pub blobs: &'a B,
    pub policy: FetchPolicy,
}

impl<M: MailStore, B: BlobStore> SyncAccount<'_, M, B> {
    /// Liste des boîtes : lue sur le serveur, classée, enregistrée.
    pub async fn sync_folders<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
    ) -> Result<Vec<StoredMailbox>, SyncFailure> {
        let listed = session.list_mailboxes().await?;
        let mailboxes = classify_mailboxes(listed);
        Ok(self
            .store
            .replace_mailboxes(self.account, &mailboxes)
            .await?)
    }

    /// Synchronise toutes les boîtes sélectionnables ; `on_change` est
    /// appelé pour chaque boîte modifiée. Une boîte en erreur de
    /// rapprochement est sautée (journalisée) ; une erreur de connexion ou
    /// de stockage interrompt tout.
    pub async fn sync_all<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
        ext: SyncExtensions,
        mut on_change: impl FnMut(MailboxId) + Send,
    ) -> Result<Vec<StoredMailbox>, SyncFailure> {
        let mailboxes = self.sync_folders(session).await?;
        for m in mailboxes.iter().filter(|m| m.mailbox.selectable) {
            match self.sync_mailbox(session, ext, m.id, &m.mailbox.name).await {
                Ok(report) if report.changed() => on_change(m.id),
                Ok(_) => {}
                Err(SyncFailure::Reconcile { mailbox, error }) => {
                    tracing::warn!(%mailbox, %error, "boîte ignorée");
                }
                // Boîte refusée par le serveur (droits, boîte disparue
                // entre LIST et SELECT) : les autres sont synchronisées.
                Err(SyncFailure::Connection(ConnectionError::Rejected(reason))) => {
                    tracing::warn!(mailbox = %m.mailbox.name, %reason, "boîte refusée");
                }
                Err(e) => return Err(e),
            }
        }
        Ok(mailboxes)
    }

    /// Synchronise une boîte.
    pub async fn sync_mailbox<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
        ext: SyncExtensions,
        mailbox: MailboxId,
        name: &MailboxName,
    ) -> Result<MailboxReport, SyncFailure> {
        let reconcile_error = |error| SyncFailure::Reconcile {
            mailbox: name.to_string(),
            error,
        };
        let local = self.store.local_mailbox(mailbox).await?;
        let request = select_request(&local, ext);
        let selected = session.select(name, &request).await?;
        let plan = plan(&local, &request, &selected).map_err(reconcile_error)?;
        let mut report = ServerReport::default();
        if let Some(query) = plan.query() {
            report.flags = session.fetch_flags(query).await?;
            if matches!(query, ServerQuery::ChangedFlags(_))
                && needs_uid_list(&local, &selected, &report.flags)
            {
                report.all_uids = Some(session.search_all_uids().await?);
            }
        }
        let outcome = reconcile(&local, &selected, plan, &report).map_err(reconcile_error)?;
        let mut summary = MailboxReport {
            plan: Some(plan.into()),
            new: 0,
            removed: outcome.removed.len(),
            flag_changes: outcome.flag_changes.len(),
            bodies: 0,
        };
        tracing::debug!(
            mailbox = %name,
            plan = ?summary.plan,
            new = outcome.new.len(),
            removed = summary.removed,
            flags = summary.flag_changes,
            "rapprochement"
        );

        if !outcome.is_empty() || outcome.state != local.state {
            let chunks: Vec<&[norelly_domain::sync::NewMessage]> =
                outcome.new.chunks(MAX_FETCH_BATCH).collect();
            let count = chunks.len().max(1);
            for index in 0..count {
                let chunk = chunks.get(index).copied().unwrap_or_default();
                let uids: Vec<_> = chunk.iter().map(|n| n.uid).collect();
                let metas = if uids.is_empty() {
                    Vec::new()
                } else {
                    session.fetch_meta(&uids).await?
                };
                summary.new += metas.len();
                let first = index == 0;
                let last = index + 1 == count;
                let batch = SyncBatch {
                    reset: first && outcome.reset,
                    removed: if first {
                        outcome.removed.iter().copied().collect()
                    } else {
                        Vec::new()
                    },
                    flag_changes: if first {
                        outcome
                            .flag_changes
                            .iter()
                            .map(|(uid, (flags, modseq))| norelly_domain::sync::FlagUpdate {
                                uid: *uid,
                                flags: flags.clone(),
                                modseq: *modseq,
                            })
                            .collect()
                    } else {
                        Vec::new()
                    },
                    new: metas
                        .into_iter()
                        .map(|meta| StoredMessage {
                            meta,
                            body: StoredBody::Missing,
                        })
                        .collect(),
                    state: if last {
                        outcome.state
                    } else {
                        outcome.interim_state()
                    },
                };
                self.store.apply_sync(mailbox, &batch).await?;
            }
        }
        summary.bodies = self.fetch_bodies(session, mailbox).await?;
        Ok(summary)
    }

    /// Télécharge les corps manquants de la boîte sélectionnée, du plus
    /// récent au plus ancien. Renvoie le nombre de corps enregistrés.
    pub async fn fetch_bodies<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
        mailbox: MailboxId,
    ) -> Result<usize, SyncFailure> {
        let mut done = 0;
        loop {
            let pending = self.store.missing_bodies(mailbox, BODY_BATCH).await?;
            if pending.is_empty() {
                return Ok(done);
            }
            let mut progressed = false;
            for PendingBody {
                uid,
                size,
                structure,
            } in pending
            {
                let fetch = self.policy.decide(size, structure.as_ref());
                let body = match session.fetch_body(uid, &fetch).await {
                    // Disparu entre-temps (ou réponse écartée) : la
                    // prochaine synchronisation le retirera ou réessaiera.
                    Ok(None) => continue,
                    // Refusé par le serveur pour ce seul message.
                    Err(ConnectionError::Rejected(reason)) => {
                        tracing::warn!(%uid, %reason, "corps refusé par le serveur");
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                    Ok(Some(body)) => body,
                };
                let body = store_fetched(self.blobs, self.account, body, fetch).await?;
                self.store.set_body(mailbox, uid, &body).await?;
                done += 1;
                progressed = true;
            }
            // Seuls restent des messages disparus du serveur : la
            // prochaine synchronisation les retirera.
            if !progressed {
                return Ok(done);
            }
        }
    }
}

/// Enregistre un corps téléchargé dans les blobs ; renvoie sa forme
/// stockée (parties différées reprises de la demande).
pub async fn store_fetched<B: BlobStore>(
    blobs: &B,
    account: AccountId,
    body: FetchedBody,
    fetch: BodyFetch,
) -> Result<StoredBody, StoreError> {
    Ok(match body {
        FetchedBody::Full(raw) => StoredBody::Full(blobs.put(account, raw).await?),
        FetchedBody::Partial { header, parts } => {
            let header = blobs.put(account, header).await?;
            let mut stored = Vec::with_capacity(parts.len());
            for (path, bytes) in parts {
                stored.push((path, blobs.put(account, bytes).await?));
            }
            let deferred = match fetch {
                BodyFetch::Partial { deferred, .. } => deferred,
                BodyFetch::Full => Vec::new(),
            };
            StoredBody::Partial {
                header,
                parts: stored,
                deferred,
            }
        }
    })
}

#[cfg(test)]
mod tests;
