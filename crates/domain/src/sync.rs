//! Rapprochement de synchronisation d'une boîte (pur, sans E/S).
//!
//! Déroulé, piloté par le cas d'usage `SyncAccount` :
//!
//! 1. [`select_request`] : paramètres du `SELECT` d'après l'état local
//!    (QRESYNC si l'état est complet et que le serveur le permet) ;
//! 2. [`plan`] : d'après la réponse au `SELECT` ([`SelectedMailbox`]),
//!    choix de la stratégie ([`SyncPlan`]) :
//!    - première synchronisation ou UIDVALIDITY changée : complète ;
//!    - QRESYNC : changements déjà reçus au `SELECT` (`VANISHED (EARLIER)`
//!      et `FETCH`) ;
//!    - CONDSTORE : drapeaux modifiés depuis le dernier `HIGHESTMODSEQ`,
//!      puis liste des UID si des suppressions sont possibles ;
//!    - sans extension : comparaison des drapeaux et des UID de tous les
//!      messages ;
//!    - rien à faire si `HIGHESTMODSEQ` et `UIDNEXT` n'ont pas bougé ;
//! 3. l'adaptateur interroge le serveur selon [`SyncPlan::queries`] et
//!    remplit un [`ServerReport`] ;
//! 4. [`reconcile`] : changements à appliquer localement ([`SyncOutcome`]),
//!    à enregistrer d'un bloc avec le nouvel état ([`MailboxSyncState`]).
//!
//! Propriétés vérifiées par les tests (`proptest`) : après une
//! synchronisation, les messages locaux sont exactement ceux du serveur,
//! avec leurs drapeaux (convergence) ; une deuxième synchronisation sans
//! changement côté serveur ne change rien (idempotence) ; appliquer deux
//! fois le même résultat donne le même état.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::mailbox::{Flags, ModSeq, Uid, UidValidity};

/// État de synchronisation enregistré pour une boîte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxSyncState {
    /// `None` : jamais synchronisée.
    pub uid_validity: Option<UidValidity>,
    /// `HIGHESTMODSEQ` du serveur à la dernière synchronisation complète ;
    /// `None` si inconnu ou si la dernière synchronisation a été
    /// interrompue (on repassera alors par la comparaison des UID).
    pub highest_modseq: Option<ModSeq>,
    /// `UIDNEXT` à la dernière synchronisation complète.
    pub uid_next: Option<Uid>,
}

/// Vue locale d'une boîte : état de synchronisation et messages connus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalMailbox {
    pub state: MailboxSyncState,
    pub messages: BTreeMap<Uid, Flags>,
}

/// Extensions utilisables sur la session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncExtensions {
    pub condstore: bool,
    /// QRESYNC activé (`ENABLE QRESYNC` accepté) ; implique CONDSTORE.
    pub qresync: bool,
}

/// Paramètres QRESYNC du `SELECT` (RFC 7162, 3.2.5).
///
/// Volontairement sans liste d'UID connus : avec elle, le serveur ne
/// signale que les changements parmi ces UID (RFC 7162, 3.2.5.1), donc pas
/// les nouveaux messages. Sans elle, il signale tout ce qui a changé
/// depuis `modseq`, nouveaux messages compris ; les `VANISHED` d'UID
/// inconnus localement sont ignorés par [`reconcile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QresyncParams {
    pub uid_validity: UidValidity,
    pub modseq: ModSeq,
}

/// Paramètres du `SELECT` (ou `EXAMINE`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectRequest {
    /// `(CONDSTORE)` : active les MODSEQ sur cette boîte.
    pub condstore: bool,
    pub qresync: Option<QresyncParams>,
}

/// Paramètres du `SELECT` d'après l'état local.
#[must_use]
pub fn select_request(local: &LocalMailbox, ext: SyncExtensions) -> SelectRequest {
    let qresync = match (
        ext.qresync,
        local.state.uid_validity,
        local.state.highest_modseq,
    ) {
        (true, Some(uid_validity), Some(modseq)) => Some(QresyncParams {
            uid_validity,
            modseq,
        }),
        _ => None,
    };
    SelectRequest {
        condstore: ext.condstore && qresync.is_none(),
        qresync,
    }
}

/// Réponse au `SELECT`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectedMailbox {
    pub uid_validity: Option<UidValidity>,
    pub uid_next: Option<Uid>,
    /// `HIGHESTMODSEQ` ; `None` si absent ou `NOMODSEQ`.
    pub highest_modseq: Option<ModSeq>,
    /// `EXISTS`.
    pub exists: u32,
    /// Avec QRESYNC : changements depuis le MODSEQ fourni (`FETCH`).
    pub changed: Vec<FlagUpdate>,
    /// Avec QRESYNC : `VANISHED (EARLIER)`.
    pub vanished: BTreeSet<Uid>,
}

/// Drapeaux d'un message reçus du serveur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagUpdate {
    pub uid: Uid,
    pub flags: Flags,
    pub modseq: Option<ModSeq>,
}

/// Erreur de rapprochement : réponse du serveur inexploitable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SyncError {
    #[error("le serveur n'a pas fourni d'UIDVALIDITY")]
    MissingUidValidity,
    #[error("réponse du serveur incohérente : {0}")]
    Inconsistent(&'static str),
}

/// Stratégie de synchronisation d'une boîte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPlan {
    /// Aucun changement (CONDSTORE : `HIGHESTMODSEQ` et `UIDNEXT`
    /// identiques).
    UpToDate,
    /// Tout recharger ; `discard_local` : les messages locaux sont caducs
    /// (UIDVALIDITY changée).
    Full { discard_local: bool },
    /// QRESYNC : les changements sont dans la réponse au `SELECT`.
    Qresync,
    /// CONDSTORE : `UID FETCH 1:* (FLAGS) (CHANGEDSINCE m)`.
    Condstore { changed_since: ModSeq },
    /// Sans extension (ou état incomplet) : drapeaux de tous les messages.
    UidCompare,
}

/// Requête au serveur nécessaire à un plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerQuery {
    /// `UID FETCH 1:* (UID FLAGS [MODSEQ])` : drapeaux de tous les
    /// messages (donne aussi la liste complète des UID).
    AllFlags,
    /// `UID FETCH 1:* (UID FLAGS MODSEQ) (CHANGEDSINCE m)`.
    ChangedFlags(ModSeq),
}

impl SyncPlan {
    /// Requête de drapeaux à envoyer ; `None` si la réponse au `SELECT`
    /// suffit.
    #[must_use]
    pub fn query(self) -> Option<ServerQuery> {
        match self {
            Self::UpToDate | Self::Qresync => None,
            Self::Full { .. } | Self::UidCompare => Some(ServerQuery::AllFlags),
            Self::Condstore { changed_since } => Some(ServerQuery::ChangedFlags(changed_since)),
        }
    }
}

/// Choix de la stratégie d'après la réponse au `SELECT`.
pub fn plan(
    local: &LocalMailbox,
    request: &SelectRequest,
    selected: &SelectedMailbox,
) -> Result<SyncPlan, SyncError> {
    let Some(server_validity) = selected.uid_validity else {
        return Err(SyncError::MissingUidValidity);
    };
    match local.state.uid_validity {
        None => {
            return Ok(SyncPlan::Full {
                discard_local: !local.messages.is_empty(),
            });
        }
        Some(v) if v != server_validity => {
            return Ok(SyncPlan::Full {
                discard_local: true,
            });
        }
        Some(_) => {}
    }
    let (Some(server_modseq), Some(local_modseq)) =
        (selected.highest_modseq, local.state.highest_modseq)
    else {
        return Ok(SyncPlan::UidCompare);
    };
    // MODSEQ en recul (restauration du serveur) : on ne peut rien déduire.
    if server_modseq < local_modseq {
        return Ok(SyncPlan::UidCompare);
    }
    if request.qresync.is_some() {
        return Ok(SyncPlan::Qresync);
    }
    if !request.condstore {
        return Ok(SyncPlan::UidCompare);
    }
    if server_modseq == local_modseq
        && selected.uid_next == local.state.uid_next
        && selected.exists as usize == local.messages.len()
    {
        return Ok(SyncPlan::UpToDate);
    }
    Ok(SyncPlan::Condstore {
        changed_since: local_modseq,
    })
}

/// Réponses du serveur aux requêtes du plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerReport {
    /// Drapeaux reçus : tous ([`ServerQuery::AllFlags`]) ou seulement les
    /// modifiés.
    pub flags: Vec<FlagUpdate>,
    /// Liste complète des UID présents (`UID SEARCH ALL`), demandée par
    /// [`needs_uid_list`] avec CONDSTORE.
    pub all_uids: Option<BTreeSet<Uid>>,
}

/// Avec CONDSTORE sans QRESYNC, les suppressions ne sont pas signalées :
/// faut-il demander la liste complète des UID ? Non si le nombre de
/// messages du serveur (`EXISTS`) est exactement le nombre local plus les
/// nouveaux reçus (aucun message n'a alors disparu).
#[must_use]
pub fn needs_uid_list(
    local: &LocalMailbox,
    selected: &SelectedMailbox,
    changed: &[FlagUpdate],
) -> bool {
    let new: BTreeSet<Uid> = changed
        .iter()
        .map(|u| u.uid)
        .filter(|uid| !local.messages.contains_key(uid))
        .collect();
    selected.exists as usize != local.messages.len() + new.len()
}

/// Message à récupérer (nouveau localement).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    pub uid: Uid,
    pub flags: Flags,
    pub modseq: Option<ModSeq>,
}

/// Changements à appliquer localement, d'un bloc.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncOutcome {
    /// Supprimer tous les messages locaux de la boîte avant le reste.
    pub reset: bool,
    /// Messages disparus du serveur.
    pub removed: BTreeSet<Uid>,
    /// Messages connus dont les drapeaux ont changé.
    pub flag_changes: BTreeMap<Uid, (Flags, Option<ModSeq>)>,
    /// Messages à récupérer, par UID croissant.
    pub new: Vec<NewMessage>,
    /// État à enregistrer une fois tous les nouveaux messages récupérés.
    pub state: MailboxSyncState,
}

impl SyncOutcome {
    /// Vrai si rien ne change localement (hors état).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.reset
            && self.removed.is_empty()
            && self.flag_changes.is_empty()
            && self.new.is_empty()
    }

    /// État à enregistrer tant que des nouveaux messages restent à
    /// récupérer : UIDVALIDITY connue, mais ni `HIGHESTMODSEQ` ni
    /// `UIDNEXT`, pour qu'une synchronisation interrompue reprenne par
    /// comparaison des UID.
    #[must_use]
    pub fn interim_state(&self) -> MailboxSyncState {
        MailboxSyncState {
            uid_validity: self.state.uid_validity,
            highest_modseq: None,
            uid_next: None,
        }
    }
}

/// Changements locaux d'après le plan et les réponses du serveur.
pub fn reconcile(
    local: &LocalMailbox,
    selected: &SelectedMailbox,
    plan: SyncPlan,
    report: &ServerReport,
) -> Result<SyncOutcome, SyncError> {
    let uid_validity = selected.uid_validity.ok_or(SyncError::MissingUidValidity)?;
    let mut outcome = SyncOutcome::default();
    let reset = matches!(
        plan,
        SyncPlan::Full {
            discard_local: true
        }
    );
    outcome.reset = reset;
    let known = |uid: &Uid| !reset && local.messages.contains_key(uid);

    // Dernière valeur reçue pour chaque UID (un serveur peut répéter un
    // FETCH) ; ignore les UID au-delà de UIDNEXT, signe d'une réponse
    // incohérente.
    let mut received: BTreeMap<Uid, (Flags, Option<ModSeq>)> = BTreeMap::new();
    let changes = match plan {
        SyncPlan::Qresync => selected.changed.as_slice(),
        _ => report.flags.as_slice(),
    };
    for update in changes {
        if selected.uid_next.is_some_and(|next| update.uid >= next) {
            continue;
        }
        received.insert(update.uid, (update.flags.clone(), update.modseq));
    }

    // Ensemble des UID présents sur le serveur, quand il est connu.
    let present: Option<BTreeSet<Uid>> = match plan {
        SyncPlan::Full { .. } | SyncPlan::UidCompare => Some(received.keys().copied().collect()),
        SyncPlan::Condstore { .. } => report.all_uids.as_ref().map(|all| {
            all.iter()
                .copied()
                .filter(|uid| selected.uid_next.is_none_or(|next| *uid < next))
                .collect()
        }),
        SyncPlan::UpToDate | SyncPlan::Qresync => None,
    };

    // Suppressions.
    if !reset {
        match &present {
            Some(present) => {
                outcome.removed = local
                    .messages
                    .keys()
                    .filter(|uid| !present.contains(uid))
                    .copied()
                    .collect();
            }
            None if plan == SyncPlan::Qresync => {
                outcome.removed = selected
                    .vanished
                    .iter()
                    .filter(|uid| local.messages.contains_key(uid))
                    .copied()
                    .collect();
            }
            None => {}
        }
    }

    // Drapeaux modifiés et nouveaux messages.
    for (uid, (flags, modseq)) in received {
        if outcome.removed.contains(&uid) {
            continue;
        }
        if plan == SyncPlan::Qresync && selected.vanished.contains(&uid) {
            continue;
        }
        if let Some(present) = &present
            && !present.contains(&uid)
        {
            continue;
        }
        if known(&uid) {
            if local.messages.get(&uid) != Some(&flags) {
                outcome.flag_changes.insert(uid, (flags, modseq));
            }
        } else {
            outcome.new.push(NewMessage { uid, flags, modseq });
        }
    }

    // UIDNEXT enregistré : celui du serveur, ou à défaut le plus grand UID
    // connu plus un.
    let max_uid = local
        .messages
        .keys()
        .filter(|_| !reset)
        .chain(outcome.new.iter().map(|n| &n.uid))
        .copied()
        .max();
    let uid_next = selected.uid_next.or_else(|| max_uid.and_then(Uid::next));
    outcome.state = MailboxSyncState {
        uid_validity: Some(uid_validity),
        highest_modseq: selected.highest_modseq,
        uid_next,
    };
    Ok(outcome)
}

/// Applique un résultat à une vue locale (référence pour les faux et les
/// tests ; le stockage réel fait la même chose en SQL).
pub fn apply(local: &mut LocalMailbox, outcome: &SyncOutcome) {
    if outcome.reset {
        local.messages.clear();
    }
    for uid in &outcome.removed {
        local.messages.remove(uid);
    }
    for (uid, (flags, _)) in &outcome.flag_changes {
        local.messages.insert(*uid, flags.clone());
    }
    for new in &outcome.new {
        local.messages.insert(new.uid, new.flags.clone());
    }
    local.state = outcome.state;
}

// ---------------------------------------------------------------------------
// Reconnexion
// ---------------------------------------------------------------------------

/// Attente exponentielle entre deux tentatives de reconnexion, bornée.
/// Le hasard (gigue) est fourni par l'appelant pour garder le calcul pur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    /// Première attente, en millisecondes.
    pub base_ms: u64,
    /// Attente maximale, en millisecondes.
    pub max_ms: u64,
    attempt: u32,
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new(1_000, 5 * 60 * 1_000)
    }
}

impl Backoff {
    #[must_use]
    pub const fn new(base_ms: u64, max_ms: u64) -> Self {
        Self {
            base_ms,
            max_ms,
            attempt: 0,
        }
    }

    /// Attente avant la prochaine tentative : `base * 2^n`, plafonnée,
    /// réduite d'une gigue `jitter` dans `[0, 1]` (jusqu'à moitié de
    /// l'attente), puis compte la tentative.
    pub fn next_delay_ms(&mut self, jitter: f64) -> u64 {
        let factor = 1u64.checked_shl(self.attempt.min(32)).unwrap_or(u64::MAX);
        let raw = self.base_ms.saturating_mul(factor).min(self.max_ms);
        self.attempt = self.attempt.saturating_add(1);
        let jitter = if jitter.is_finite() {
            jitter.clamp(0.0, 1.0)
        } else {
            0.0
        };
        // Conversion bornée : la gigue retire au plus la moitié de raw.
        let cut = ((raw as f64) * 0.5 * jitter) as u64;
        raw - cut.min(raw / 2)
    }

    /// Nombre de tentatives depuis le dernier succès.
    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempt
    }

    /// Connexion réussie : on repart de l'attente minimale.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

/// Durée maximale d'un IDLE avant renouvellement : les serveurs peuvent
/// fermer une connexion inactive après 30 minutes (RFC 2177) ; on
/// renouvelle avant 29 minutes, avec de la marge.
pub const IDLE_RENEWAL_SECS: u64 = 25 * 60;

#[cfg(test)]
mod tests;
