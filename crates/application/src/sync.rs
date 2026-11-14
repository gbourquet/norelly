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

use std::sync::Arc;
use std::time::Duration;

use norelly_domain::account::AccountId;
use norelly_domain::connection::ConnectionError;
use norelly_domain::fetch_policy::MimePart;
use norelly_domain::fetch_policy::{BodyFetch, FetchPolicy};
use norelly_domain::mailbox::{MailboxId, MailboxName, classify_mailboxes};
use norelly_domain::notify::{NewMail, new_mail};
use norelly_domain::ports::{
    BlobStore, FetchedBody, MAX_FETCH_BATCH, MailStore, PendingBody, PendingOpStore,
    RemoteMailboxSession, SearchTextExtractor, StoreError, StoredBody, StoredMailbox,
    StoredMessage, SyncBatch,
};
use norelly_domain::sync::{
    ServerQuery, ServerReport, SyncError, SyncExtensions, SyncPlan, needs_uid_list, plan,
    reconcile, select_request,
};

/// Étape d'une synchronisation complète, signalée au fil de l'eau : l'écran
/// suit sans attendre la fin du tour.
#[derive(Debug)]
pub enum SyncStep<'a> {
    /// Liste des dossiers lue et enregistrée.
    Folders(&'a [StoredMailbox]),
    /// `done` dossiers sélectionnables traités sur `total` (0 au départ).
    Progress { done: usize, total: usize },
    /// Un lot d'en-têtes (ou de suppressions, de drapeaux) vient d'être
    /// enregistré dans ce dossier ; les corps viennent ensuite.
    Headers(&'a StoredMailbox),
    /// Dossier terminé (corps compris) et modifié.
    MailboxDone(&'a StoredMailbox, &'a MailboxReport),
}

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
    /// La liste des dossiers a changé (ou a été lue pour la première fois),
    /// ou, pendant un tour complet, le contenu de dossiers du compte (signal
    /// regroupé : l'interface relit dossiers et liste).
    FoldersChanged { account: AccountId },
    /// Nouveaux messages non lus arrivés dans la boîte de réception d'un
    /// compte (notification ; contenu filtré ensuite selon les
    /// préférences).
    NewMail {
        account: AccountId,
        mailbox: MailboxId,
        mail: NewMail,
    },
    /// Connexion établie (après une coupure éventuelle).
    Online { account: AccountId },
    /// Synchronisation complète en cours : `done` dossiers traités sur
    /// `total` (terminée quand `done == total`).
    SyncProgress {
        account: AccountId,
        done: usize,
        total: usize,
    },
    /// Connexion perdue ; nouvelle tentative prévue. L'échec est transmis
    /// structuré : l'interface en tire un message localisé (ADR-011).
    Offline {
        account: AccountId,
        failure: SyncFailure,
        /// Attente avant le nouvel essai ; `None` : aucun nouvel essai (la
        /// synchronisation s'arrête, l'échec reste affiché).
        retry_in_ms: Option<u64>,
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
    /// Vrai si un nouvel essai ne peut pas aboutir
    /// ([`ConnectionError::retry_is_useless`]).
    #[must_use]
    pub fn retry_is_useless(&self) -> bool {
        matches!(self, Self::Connection(e) if e.retry_is_useless())
    }

    /// Vrai si la connexion est à rouvrir (erreur réseau ou protocole) ;
    /// faux pour une commande simplement refusée (`NO`).
    #[must_use]
    pub fn is_connection(&self) -> bool {
        matches!(self, Self::Connection(e) if !matches!(e, ConnectionError::Rejected(_)))
    }
}

/// Bilan de la synchronisation d'une boîte.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MailboxReport {
    /// Nouveaux messages non lus à signaler (jamais à la première
    /// synchronisation de la boîte).
    pub arrivals: Option<NewMail>,
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
    /// Texte cherchable des corps téléchargés (extrait hors du fil
    /// asynchrone, en temps borné).
    pub text: Arc<dyn SearchTextExtractor>,
}

impl<M: MailStore + PendingOpStore, B: BlobStore> SyncAccount<'_, M, B> {
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
        mut on_change: impl FnMut(&StoredMailbox, &MailboxReport) + Send,
    ) -> Result<Vec<StoredMailbox>, SyncFailure> {
        self.sync_all_observed(session, ext, |step| {
            if let SyncStep::MailboxDone(m, report) = step {
                on_change(m, report);
            }
        })
        .await
    }

    /// [`Self::sync_all`], chaque étape signalée à `observe`
    /// ([`SyncStep`]).
    pub async fn sync_all_observed<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
        ext: SyncExtensions,
        mut observe: impl FnMut(SyncStep<'_>) + Send,
    ) -> Result<Vec<StoredMailbox>, SyncFailure> {
        let mailboxes = self.sync_folders(session).await?;
        observe(SyncStep::Folders(&mailboxes));
        let selectable: Vec<&StoredMailbox> =
            mailboxes.iter().filter(|m| m.mailbox.selectable).collect();
        let total = selectable.len();
        observe(SyncStep::Progress { done: 0, total });
        for (index, m) in selectable.into_iter().enumerate() {
            let result = self
                .sync_mailbox_observed(session, ext, m.id, &m.mailbox.name, &mut || {
                    observe(SyncStep::Headers(m));
                })
                .await;
            match result {
                Ok(report) if report.changed() => observe(SyncStep::MailboxDone(m, &report)),
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
            observe(SyncStep::Progress {
                done: index + 1,
                total,
            });
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
        self.sync_mailbox_observed(session, ext, mailbox, name, &mut || {})
            .await
    }

    /// [`Self::sync_mailbox`] ; `on_headers` est appelé après chaque lot
    /// enregistré (en-têtes, suppressions, drapeaux), avant les corps.
    pub async fn sync_mailbox_observed<S: RemoteMailboxSession>(
        &self,
        session: &mut S,
        ext: SyncExtensions,
        mailbox: MailboxId,
        name: &MailboxName,
        on_headers: &mut (dyn FnMut() + Send),
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
        // Boîte jamais synchronisée, ou repartie de zéro (UIDVALIDITY
        // changée) : ses messages ne sont pas des arrivées.
        let first_sync = local.state.uid_validity.is_none() || outcome.reset;
        let mut summary = MailboxReport {
            arrivals: None,
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
                if let Some(batch) = new_mail(first_sync, &metas) {
                    summary.arrivals = Some(match summary.arrivals.take() {
                        Some(earlier) => earlier.merge(batch),
                        None => batch,
                    });
                }
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
                let visible = batch.reset
                    || !batch.new.is_empty()
                    || !batch.removed.is_empty()
                    || !batch.flag_changes.is_empty();
                self.store.apply_sync(mailbox, &batch).await?;
                if visible {
                    // Les actions encore en attente gardent leur effet local
                    // dans ce que l'écran va relire (le lot vient d'écrire
                    // l'état du serveur).
                    self.store.reapply(self.account).await?;
                    on_headers();
                }
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
                let text = extract_text(
                    &self.text,
                    &body,
                    structure.as_ref(),
                    TEXT_EXTRACTION_TIMEOUT,
                )
                .await;
                let body = store_fetched(self.blobs, self.account, body, fetch).await?;
                self.store
                    .set_body(mailbox, uid, &body, Some(&text))
                    .await?;
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

/// Délai maximal de l'extraction du texte cherchable d'un corps (message
/// hostile) ; au-delà, le message est indexé sans son texte.
pub const TEXT_EXTRACTION_TIMEOUT: Duration = Duration::from_secs(5);

/// Texte cherchable d'un corps, extrait hors du fil asynchrone, en au plus
/// `timeout` (vide au-delà).
pub async fn extract_text(
    extractor: &Arc<dyn SearchTextExtractor>,
    body: &FetchedBody,
    structure: Option<&MimePart>,
    timeout: Duration,
) -> String {
    let extractor = Arc::clone(extractor);
    let body = body.clone();
    let structure = structure.cloned();
    let task =
        tokio::task::spawn_blocking(move || extractor.searchable_text(&body, structure.as_ref()));
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "extraction du texte interrompue");
            String::new()
        }
        Err(_) => {
            tracing::warn!("extraction du texte trop longue : message indexé sans son texte");
            String::new()
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
