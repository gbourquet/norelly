//! Actions sur les messages (phase 6) : sémantique, application optimiste,
//! file d'opérations durable (`pending_ops`) et règles de rejeu.
//!
//! Déroulé, piloté par le cas d'usage `PerformAction` :
//!
//! 1. [`resolve`] traduit l'action de l'utilisateur ([`MessageAction`]) en
//!    effet ([`Effect`]) selon le dossier du message :
//!    - « supprimer » déplace vers la Corbeille ; depuis la Corbeille, la
//!      suppression est définitive ;
//!    - « archiver » déplace vers le dossier `\Archive`, ou à défaut vers
//!      le dossier de repli choisi par l'utilisateur ;
//! 2. [`PendingOp::new`] en fait une opération durable, enregistrée avec
//!    l'état local nécessaire à son annulation ; [`local_effect`] est
//!    appliqué tout de suite à la base (application optimiste : drapeau
//!    changé, ou message masqué) ;
//! 3. après la fenêtre d'annulation ([`UNDO_WINDOW_SECS`]), l'opération
//!    est rejouée sur le serveur dès qu'il est joignable. [`decide`]
//!    tranche les conflits (UIDVALIDITY changée, message disparu, dossier
//!    de destination supprimé) : l'opération est alors abandonnée et
//!    l'état local rétabli ([`revert_effect`]) ; la synchronisation
//!    suivante fait foi.
//!
//! Stratégies serveur : `MOVE` si annoncé ; sinon `COPY`, `STORE \Deleted`
//! et `UID EXPUNGE` (UIDPLUS) ; sinon `EXPUNGE`, seulement si aucun autre
//! message de la boîte n'est marqué `\Deleted` ([`expunge_is_safe`]) :
//! sans quoi des messages supprimés par un autre client partiraient aussi.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::account::AccountId;
use crate::capabilities::ServerCapabilities;
use crate::conversation::ConversationScope;
use crate::mailbox::{Flags, MailboxId, MailboxName, MailboxRole, ModSeq, Uid, UidValidity};
use crate::ports::MessageId;

/// Délai avant le rejeu d'une opération : l'utilisateur peut l'annuler
/// pendant ce temps sans qu'elle ait touché le serveur.
pub const UNDO_WINDOW_SECS: i64 = 5;

/// Nombre maximal de messages visés par une action (sélection multiple).
pub const MAX_ACTION_TARGETS: usize = 1000;

/// Drapeau modifiable par une action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionFlag {
    Seen,
    Flagged,
}

impl ActionFlag {
    /// Valeur du drapeau dans `flags`.
    #[must_use]
    pub fn get(self, flags: &Flags) -> bool {
        match self {
            Self::Seen => flags.seen,
            Self::Flagged => flags.flagged,
        }
    }

    pub fn set(self, flags: &mut Flags, value: bool) {
        match self {
            Self::Seen => flags.seen = value,
            Self::Flagged => flags.flagged = value,
        }
    }

    /// Jeton IMAP.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Seen => "\\Seen",
            Self::Flagged => "\\Flagged",
        }
    }
}

/// Ajout ou retrait d'un drapeau (`STORE +FLAGS` / `-FLAGS`) : les autres
/// drapeaux du message ne sont jamais touchés.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FlagChange {
    pub flag: ActionFlag,
    pub value: bool,
}

/// Action demandée par l'utilisateur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageAction {
    /// Lu (`true`) ou non lu.
    MarkSeen(bool),
    /// Suivi (drapeau) ou non.
    MarkFlagged(bool),
    /// Déplacer vers un dossier du même compte.
    Move(MailboxId),
    Delete,
    Archive,
}

/// Dossier d'un compte, tel que connu localement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRef {
    pub id: MailboxId,
    pub name: MailboxName,
    pub role: MailboxRole,
    pub selectable: bool,
}

/// Effet d'une action sur un message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    SetFlag(FlagChange),
    MoveTo(FolderRef),
    /// Suppression définitive (depuis la Corbeille).
    Expunge,
}

/// Action impossible.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    #[error("aucun dossier Corbeille sur ce compte")]
    NoTrash,
    #[error("aucun dossier d'archives sur ce compte")]
    NoArchive,
    #[error("dossier de destination inconnu")]
    UnknownFolder,
    #[error("le message est déjà dans ce dossier")]
    SameFolder,
    #[error("dossier de destination non sélectionnable")]
    NotSelectable,
    #[error("trop de messages visés")]
    TooManyTargets,
}

/// Effet de `action` sur un message du dossier `source`. `folders` :
/// dossiers du compte ; `archive_fallback` : dossier de repli choisi par
/// l'utilisateur quand le compte n'a pas de dossier `\Archive`.
pub fn resolve(
    action: MessageAction,
    source: &FolderRef,
    folders: &[FolderRef],
    archive_fallback: Option<MailboxId>,
) -> Result<Effect, ActionError> {
    let target = |folder: Option<&FolderRef>, missing: ActionError| {
        let folder = folder.ok_or(missing)?;
        if folder.id == source.id {
            return Err(ActionError::SameFolder);
        }
        if !folder.selectable {
            return Err(ActionError::NotSelectable);
        }
        Ok(Effect::MoveTo(folder.clone()))
    };
    let by_role = |role: MailboxRole| folders.iter().find(|f| f.role == role && f.selectable);
    match action {
        MessageAction::MarkSeen(value) => Ok(Effect::SetFlag(FlagChange {
            flag: ActionFlag::Seen,
            value,
        })),
        MessageAction::MarkFlagged(value) => Ok(Effect::SetFlag(FlagChange {
            flag: ActionFlag::Flagged,
            value,
        })),
        MessageAction::Move(id) => target(
            folders.iter().find(|f| f.id == id),
            ActionError::UnknownFolder,
        ),
        MessageAction::Delete if source.role == MailboxRole::Trash => Ok(Effect::Expunge),
        MessageAction::Delete => target(by_role(MailboxRole::Trash), ActionError::NoTrash),
        MessageAction::Archive => {
            let archive = by_role(MailboxRole::Archive)
                .or_else(|| archive_fallback.and_then(|id| folders.iter().find(|f| f.id == id)));
            target(archive, ActionError::NoArchive)
        }
    }
}

/// Message d'une conversation, tous dossiers et exemplaires compris
/// (sélection des messages visés par une action sur une conversation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub id: MessageId,
    pub mailbox: MailboxId,
    pub role: MailboxRole,
    pub flags: Flags,
    pub date: i64,
}

/// Messages visés par une action sur une conversation affichée dans
/// `scope` :
/// - lu / non lu : toute la conversation, tous dossiers ;
/// - suivi : le message le plus récent de la portée (sans effet si l'un
///   d'eux l'est déjà) ; plus suivi : tous les messages suivis ;
/// - déplacer, supprimer, archiver : les messages de la portée seulement
///   (les réponses rangées dans « Envoyés » ne bougent pas).
#[must_use]
pub fn conversation_targets(
    action: MessageAction,
    members: &[Member],
    scope: &ConversationScope,
) -> Vec<MessageId> {
    let in_scope = |m: &&Member| match scope {
        ConversationScope::Mailbox(id) => m.mailbox == *id,
        ConversationScope::Unified(role) => m.role == *role,
    };
    match action {
        MessageAction::MarkSeen(value) => members
            .iter()
            .filter(|m| m.flags.seen != value)
            .map(|m| m.id)
            .collect(),
        MessageAction::MarkFlagged(true) => {
            if members.iter().filter(in_scope).any(|m| m.flags.flagged) {
                return Vec::new();
            }
            members
                .iter()
                .filter(in_scope)
                .max_by_key(|m| (m.date, m.id))
                .map(|m| vec![m.id])
                .unwrap_or_default()
        }
        MessageAction::MarkFlagged(false) => members
            .iter()
            .filter(|m| m.flags.flagged)
            .map(|m| m.id)
            .collect(),
        MessageAction::Move(_) | MessageAction::Delete | MessageAction::Archive => {
            members.iter().filter(in_scope).map(|m| m.id).collect()
        }
    }
}

/// Opération sur le serveur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpKind {
    /// `previous` : valeur locale avant l'action (annulation) ;
    /// `unchanged_since` : `HIGHESTMODSEQ` connu de la boîte au moment de
    /// l'action (`STORE … (UNCHANGEDSINCE …)`).
    Flag {
        change: FlagChange,
        previous: bool,
        unchanged_since: Option<ModSeq>,
    },
    Move {
        to: MailboxName,
    },
    Expunge,
}

/// Opération en attente de rejeu (table `pending_ops`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOp {
    pub account: AccountId,
    /// Message local visé (application et annulation optimistes).
    pub message: MessageId,
    /// Dossier source, UIDVALIDITY et UID au moment de l'action.
    pub mailbox: MailboxName,
    pub uid_validity: UidValidity,
    pub uid: Uid,
    pub kind: OpKind,
    /// Rejeu au plus tôt (secondes Unix) : fin de la fenêtre d'annulation.
    pub not_before: i64,
}

/// Message visé, tel que connu localement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalMessage {
    pub id: MessageId,
    pub account: AccountId,
    pub mailbox: FolderRef,
    pub uid: Uid,
    pub flags: Flags,
    /// État de synchronisation de sa boîte.
    pub uid_validity: Option<UidValidity>,
    pub highest_modseq: Option<ModSeq>,
}

impl PendingOp {
    /// Opération d'un effet sur un message ; `None` si l'effet est déjà
    /// acquis (drapeau déjà dans l'état voulu) ou si la boîte n'a jamais
    /// été synchronisée (pas d'UIDVALIDITY : l'UID n'aurait pas de sens).
    #[must_use]
    pub fn new(effect: &Effect, message: &LocalMessage, now: i64) -> Option<Self> {
        let kind = match effect {
            Effect::SetFlag(change) => {
                let previous = change.flag.get(&message.flags);
                if previous == change.value {
                    return None;
                }
                OpKind::Flag {
                    change: *change,
                    previous,
                    unchanged_since: message.highest_modseq,
                }
            }
            Effect::MoveTo(folder) => OpKind::Move {
                to: folder.name.clone(),
            },
            Effect::Expunge => OpKind::Expunge,
        };
        Some(Self {
            account: message.account,
            message: message.id,
            mailbox: message.mailbox.name.clone(),
            uid_validity: message.uid_validity?,
            uid: message.uid,
            kind,
            not_before: now.saturating_add(UNDO_WINDOW_SECS),
        })
    }

    /// Vrai si l'opération peut être rejouée à l'instant `now`.
    #[must_use]
    pub fn is_due(&self, now: i64) -> bool {
        now >= self.not_before
    }
}

/// Changement local (application optimiste ou annulation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalEffect {
    /// Donne au drapeau la valeur indiquée.
    SetFlag {
        message: MessageId,
        flag: ActionFlag,
        value: bool,
    },
    /// Masque le message (déplacé ou supprimé, en attente du serveur).
    Hide { message: MessageId },
    /// Le rend de nouveau visible.
    Show { message: MessageId },
}

/// Changement local appliqué dès l'action.
#[must_use]
pub fn local_effect(op: &PendingOp) -> LocalEffect {
    match &op.kind {
        OpKind::Flag { change, .. } => LocalEffect::SetFlag {
            message: op.message,
            flag: change.flag,
            value: change.value,
        },
        OpKind::Move { .. } | OpKind::Expunge => LocalEffect::Hide {
            message: op.message,
        },
    }
}

/// Changement local qui annule [`local_effect`] (annulation par
/// l'utilisateur ou opération abandonnée).
#[must_use]
pub fn revert_effect(op: &PendingOp) -> LocalEffect {
    match &op.kind {
        OpKind::Flag {
            change, previous, ..
        } => LocalEffect::SetFlag {
            message: op.message,
            flag: change.flag,
            value: *previous,
        },
        OpKind::Move { .. } | OpKind::Expunge => LocalEffect::Show {
            message: op.message,
        },
    }
}

/// Fusion d'une nouvelle opération avec la file existante.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coalesce {
    /// Ajouter la nouvelle opération.
    Push,
    /// Les deux s'annulent (drapeau remis à sa valeur d'origine avant
    /// rejeu) : retirer l'opération n° `index`, ne rien ajouter.
    Cancel(usize),
}

/// Décide de la fusion de `new` avec les opérations en attente `queue`.
#[must_use]
pub fn coalesce(queue: &[PendingOp], new: &PendingOp) -> Coalesce {
    let OpKind::Flag { change, .. } = &new.kind else {
        return Coalesce::Push;
    };
    let found = queue.iter().position(|op| {
        op.message == new.message
            && matches!(&op.kind, OpKind::Flag { change: c, previous, .. }
                if c.flag == change.flag && *previous == change.value)
    });
    found.map_or(Coalesce::Push, Coalesce::Cancel)
}

/// Résultat d'un `STORE` conditionnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOutcome {
    Stored,
    /// Refusé : le message a changé depuis `UNCHANGEDSINCE` (`MODIFIED`).
    Modified,
}

/// Stratégie de déplacement selon les capacités du serveur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveStrategy {
    /// `UID MOVE` (RFC 6851).
    Move,
    /// `UID COPY`, `UID STORE +FLAGS (\Deleted)`, `UID EXPUNGE` (UIDPLUS).
    CopyUidExpunge,
    /// `UID COPY`, `UID STORE +FLAGS (\Deleted)`, puis `EXPUNGE` seulement
    /// si [`expunge_is_safe`].
    CopyGuardedExpunge,
}

#[must_use]
pub fn move_strategy(capabilities: &ServerCapabilities) -> MoveStrategy {
    if capabilities.has("MOVE") {
        MoveStrategy::Move
    } else {
        expunge_strategy(capabilities)
    }
}

/// Stratégie de suppression définitive (sans `MOVE`).
#[must_use]
pub fn expunge_strategy(capabilities: &ServerCapabilities) -> MoveStrategy {
    if capabilities.has("UIDPLUS") {
        MoveStrategy::CopyUidExpunge
    } else {
        MoveStrategy::CopyGuardedExpunge
    }
}

/// Garde-fou d'un `EXPUNGE` sans UIDPLUS : il supprime tous les messages
/// marqués `\Deleted` de la boîte. Autorisé seulement si ce sont
/// exactement ceux que l'on veut supprimer.
#[must_use]
pub fn expunge_is_safe(ours: &BTreeSet<Uid>, deleted_on_server: &BTreeSet<Uid>) -> bool {
    deleted_on_server.is_subset(ours)
}

/// État du serveur au moment du rejeu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerView {
    /// UIDVALIDITY de la boîte source (`None` : boîte disparue).
    pub uid_validity: Option<UidValidity>,
    /// Drapeaux du message (`None` : message disparu).
    pub flags: Option<Flags>,
    /// La boîte de destination existe (déplacement).
    pub target_exists: bool,
}

/// Raison de l'abandon d'une opération.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// Boîte source disparue ou recréée (UIDVALIDITY changée) : l'UID ne
    /// désigne plus le même message.
    UidValidityChanged,
    /// Message supprimé entre-temps (par un autre client).
    MessageGone,
    /// Dossier de destination supprimé entre-temps.
    TargetMissing,
}

/// Décision de rejeu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayDecision {
    /// Exécuter l'opération.
    Apply,
    /// Rien à faire : le serveur est déjà dans l'état voulu.
    AlreadyDone,
    /// Abandon : rétablir l'état local, la synchronisation fait foi.
    Drop(Conflict),
}

/// Décide du rejeu d'une opération d'après l'état du serveur.
#[must_use]
pub fn decide(op: &PendingOp, view: &ServerView) -> ReplayDecision {
    if view.uid_validity != Some(op.uid_validity) {
        return ReplayDecision::Drop(Conflict::UidValidityChanged);
    }
    let Some(flags) = &view.flags else {
        return ReplayDecision::Drop(Conflict::MessageGone);
    };
    match &op.kind {
        OpKind::Flag { change, .. } if change.flag.get(flags) == change.value => {
            ReplayDecision::AlreadyDone
        }
        OpKind::Move { .. } if !view.target_exists => ReplayDecision::Drop(Conflict::TargetMissing),
        _ => ReplayDecision::Apply,
    }
}

/// Après un `STORE … (UNCHANGEDSINCE …)` refusé (`MODIFIED`) : le message
/// a changé sur le serveur depuis la dernière synchronisation. L'action de
/// l'utilisateur est plus récente que l'état qu'il voyait ; elle ne touche
/// qu'un drapeau (`+FLAGS`/`-FLAGS`), donc aucun autre changement
/// concurrent n'est écrasé : elle est réappliquée sans condition, sauf si
/// le serveur a déjà la valeur voulue.
#[must_use]
pub fn after_modified(change: FlagChange, server_flags: &Flags) -> ReplayDecision {
    if change.flag.get(server_flags) == change.value {
        ReplayDecision::AlreadyDone
    } else {
        ReplayDecision::Apply
    }
}

#[cfg(test)]
mod tests;
