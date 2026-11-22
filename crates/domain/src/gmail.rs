//! Messages Gmail présents dans plusieurs dossiers (ADR-016).
//!
//! Gmail expose le même message dans « Tous les messages » et dans chaque
//! dossier d'étiquette, sous un identifiant unique et stable,
//! `X-GM-MSGID` (extension `X-GM-EXT-1`). Un corps déjà téléchargé sous
//! le même identifiant est réutilisé plutôt que téléchargé à nouveau.
//!
//! Garde-fous : l'identifiant n'est demandé et pris en compte que si la
//! session annonce `X-GM-EXT-1` ; la réutilisation reste dans le compte
//! et exige la même taille ; une copie provisoire (action en attente)
//! ne cède pas son corps.

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use crate::account::AccountId;
use crate::capabilities::ServerCapabilities;
use crate::ports::{PendingBody, StoredBody};

/// Capacité IMAP des extensions Gmail.
pub const GMAIL_EXTENSION: &str = "X-GM-EXT-1";

/// Identifiant Gmail d'un message (`X-GM-MSGID`), jamais nul.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GmailMessageId(NonZeroU64);

impl GmailMessageId {
    /// `None` pour zéro (aucun message Gmail n'a cet identifiant).
    #[must_use]
    pub fn new(raw: u64) -> Option<Self> {
        NonZeroU64::new(raw).map(Self)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Vrai si l'identifiant Gmail est à demander et à prendre en compte sur
/// cette session (`X-GM-EXT-1` annoncé).
#[must_use]
pub fn gmail_ids_enabled(capabilities: &ServerCapabilities) -> bool {
    capabilities.has(GMAIL_EXTENSION)
}

/// Recherche d'un corps réutilisable pour un message encore sans corps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyReuse {
    /// Compte du message : seul compte où chercher.
    pub account: AccountId,
    pub gmail_id: GmailMessageId,
    /// `RFC822.SIZE` du message : le candidat doit avoir la même.
    pub size: u32,
}

/// Message candidat à céder son corps.
#[derive(Debug, Clone, Copy)]
pub struct BodyCandidate<'a> {
    pub account: AccountId,
    pub gmail_id: Option<GmailMessageId>,
    pub size: u32,
    pub body: &'a StoredBody,
    /// Copie provisoire créée par une action en attente.
    pub provisional: bool,
}

impl BodyReuse {
    /// Recherche pour `pending`, du compte `account` ; `None` si la session
    /// n'annonce pas `X-GM-EXT-1` ou si le message n'a pas d'identifiant.
    #[must_use]
    pub fn for_pending(
        account: AccountId,
        capabilities: &ServerCapabilities,
        pending: &PendingBody,
    ) -> Option<Self> {
        if !gmail_ids_enabled(capabilities) {
            return None;
        }
        Some(Self {
            account,
            gmail_id: pending.gmail_id?,
            size: pending.size,
        })
    }

    /// Vrai si `candidate` peut céder son corps : même compte, même
    /// identifiant, même taille, corps déjà téléchargé, pas une copie
    /// provisoire.
    #[must_use]
    pub fn accepts(&self, candidate: &BodyCandidate<'_>) -> bool {
        candidate.account == self.account
            && candidate.gmail_id == Some(self.gmail_id)
            && candidate.size == self.size
            && *candidate.body != StoredBody::Missing
            && !candidate.provisional
    }
}

#[cfg(test)]
mod tests;
