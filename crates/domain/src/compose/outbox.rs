//! File d'envoi : états d'un message à envoyer, nouvelles tentatives et
//! échec définitif.
//!
//! ```text
//! Queued ──start──▶ Sending ──succeeded──▶ Sent
//!   ▲                  │
//!   └──failed (temporaire, injoignable)
//!                      └──failed (refus, authentification, trop d'essais)──▶ Failed
//! ```
//!
//! - Hors ligne (serveur injoignable), l'essai n'est pas compté : l'envoi
//!   est simplement différé jusqu'au retour du réseau.
//! - Un échec temporaire (réponse SMTP 4xx, coupure en cours d'envoi) est
//!   retenté avec une attente croissante, au plus [`MAX_SEND_ATTEMPTS`]
//!   fois.
//! - Un envoi interrompu par l'arrêt de l'application (`Sending` au
//!   démarrage) a peut-être abouti : il devient un échec
//!   [`SendFailure::Uncertain`], jamais renvoyé automatiquement (pas de
//!   doublon chez les destinataires) ; l'utilisateur décide.

/// Essais comptés avant l'échec définitif.
pub const MAX_SEND_ATTEMPTS: u32 = 8;
/// Attente avant le premier nouvel essai, doublée ensuite.
pub const RETRY_BASE_SECS: i64 = 30;
/// Attente maximale entre deux essais.
pub const RETRY_MAX_SECS: i64 = 3600;

/// Cause d'un échec d'envoi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendFailure {
    /// Serveur injoignable (hors ligne, DNS, connexion refusée).
    Unreachable,
    /// Échec temporaire : réponse 4xx, coupure pendant l'envoi.
    Temporary,
    /// Refus définitif du serveur (5xx : destinataire refusé, message
    /// trop gros…).
    Rejected,
    /// Identifiants refusés : à corriger par l'utilisateur.
    Authentication,
    /// Envoi interrompu par l'arrêt de l'application : peut-être parti.
    Uncertain,
    /// Trop d'échecs temporaires.
    TooManyAttempts,
}

/// État d'un message de la file d'envoi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxState {
    /// En attente d'envoi, pas avant `not_before` (secondes Unix).
    Queued {
        not_before: i64,
    },
    /// Envoi en cours (réservé).
    Sending,
    Sent,
    /// Échec définitif : l'utilisateur peut relancer ([`OutboxEntry::retry`]).
    Failed(SendFailure),
}

/// Message de la file d'envoi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboxEntry {
    pub state: OutboxState,
    /// Essais comptés (les essais hors ligne ne comptent pas).
    pub attempts: u32,
}

/// Transition impossible depuis l'état courant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("transition impossible depuis {0:?}")]
pub struct InvalidTransition(pub OutboxState);

/// Attente avant l'essai suivant le `attempts`-ième.
#[must_use]
pub fn retry_delay(attempts: u32) -> i64 {
    let doublings = attempts.saturating_sub(1).min(20);
    RETRY_BASE_SECS
        .saturating_mul(1_i64 << doublings)
        .min(RETRY_MAX_SECS)
}

impl OutboxEntry {
    /// Message mis en file à l'instant `now`.
    #[must_use]
    pub fn queued(now: i64) -> Self {
        Self {
            state: OutboxState::Queued { not_before: now },
            attempts: 0,
        }
    }

    /// Peut-il partir à l'instant `now` ?
    #[must_use]
    pub fn is_due(&self, now: i64) -> bool {
        matches!(self.state, OutboxState::Queued { not_before } if now >= not_before)
    }

    /// Début d'un essai : réservé, essai compté.
    pub fn start(self) -> Result<Self, InvalidTransition> {
        match self.state {
            OutboxState::Queued { .. } => Ok(Self {
                state: OutboxState::Sending,
                attempts: self.attempts.saturating_add(1),
            }),
            state => Err(InvalidTransition(state)),
        }
    }

    /// Envoi accepté par le serveur.
    pub fn succeeded(self) -> Result<Self, InvalidTransition> {
        match self.state {
            OutboxState::Sending => Ok(Self {
                state: OutboxState::Sent,
                ..self
            }),
            state => Err(InvalidTransition(state)),
        }
    }

    /// Échec de l'essai en cours, à l'instant `now`.
    pub fn failed(self, failure: SendFailure, now: i64) -> Result<Self, InvalidTransition> {
        if self.state != OutboxState::Sending {
            return Err(InvalidTransition(self.state));
        }
        let queued = |attempts: u32, delay: i64| Self {
            state: OutboxState::Queued {
                not_before: now.saturating_add(delay),
            },
            attempts,
        };
        Ok(match failure {
            // Hors ligne : l'essai ne compte pas.
            SendFailure::Unreachable => queued(self.attempts.saturating_sub(1), RETRY_BASE_SECS),
            SendFailure::Temporary if self.attempts < MAX_SEND_ATTEMPTS => {
                queued(self.attempts, retry_delay(self.attempts))
            }
            SendFailure::Temporary => Self {
                state: OutboxState::Failed(SendFailure::TooManyAttempts),
                ..self
            },
            failure => Self {
                state: OutboxState::Failed(failure),
                ..self
            },
        })
    }

    /// Au démarrage : un envoi resté en cours est incertain.
    #[must_use]
    pub fn recovered(self) -> Self {
        match self.state {
            OutboxState::Sending => Self {
                state: OutboxState::Failed(SendFailure::Uncertain),
                ..self
            },
            _ => self,
        }
    }

    /// Relance par l'utilisateur après un échec définitif.
    pub fn retry(self, now: i64) -> Result<Self, InvalidTransition> {
        match self.state {
            OutboxState::Failed(_) => Ok(Self::queued(now)),
            state => Err(InvalidTransition(state)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000;

    fn sending() -> OutboxEntry {
        OutboxEntry::queued(NOW)
            .start()
            .unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn a_queued_message_is_sent() {
        let e = OutboxEntry::queued(NOW);
        assert!(e.is_due(NOW));
        assert!(!e.is_due(NOW - 1));
        let e = e.start().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((e.state, e.attempts), (OutboxState::Sending, 1));
        assert!(!e.is_due(NOW), "réservé");
        let e = e.succeeded().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(e.state, OutboxState::Sent);
        assert!(!e.is_due(NOW + 10_000));
    }

    #[test]
    fn offline_attempts_are_not_counted() {
        let e = sending()
            .failed(SendFailure::Unreachable, NOW)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(e.attempts, 0);
        assert_eq!(
            e.state,
            OutboxState::Queued {
                not_before: NOW + RETRY_BASE_SECS
            }
        );
        // Hors ligne indéfiniment : jamais d'échec définitif.
        let mut e = e;
        for n in 0..100 {
            e = e
                .start()
                .and_then(|e| e.failed(SendFailure::Unreachable, NOW + n))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        assert!(matches!(e.state, OutboxState::Queued { .. }));
    }

    #[test]
    fn temporary_failures_back_off_then_give_up() {
        assert_eq!(retry_delay(1), RETRY_BASE_SECS);
        assert_eq!(retry_delay(2), 2 * RETRY_BASE_SECS);
        assert_eq!(retry_delay(3), 4 * RETRY_BASE_SECS);
        assert_eq!(retry_delay(30), RETRY_MAX_SECS);
        assert_eq!(retry_delay(u32::MAX), RETRY_MAX_SECS);
        let mut e = OutboxEntry::queued(NOW);
        for n in 1..MAX_SEND_ATTEMPTS {
            e = e
                .start()
                .and_then(|e| e.failed(SendFailure::Temporary, NOW))
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(
                e.state,
                OutboxState::Queued {
                    not_before: NOW + retry_delay(n)
                }
            );
        }
        let e = e
            .start()
            .and_then(|e| e.failed(SendFailure::Temporary, NOW))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(e.state, OutboxState::Failed(SendFailure::TooManyAttempts));
    }

    #[test]
    fn permanent_failures_stop_at_once() {
        for failure in [SendFailure::Rejected, SendFailure::Authentication] {
            let e = sending()
                .failed(failure, NOW)
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(e.state, OutboxState::Failed(failure));
            assert!(!e.is_due(NOW + 1_000_000));
        }
    }

    #[test]
    fn an_interrupted_send_is_uncertain_and_never_resent_automatically() {
        let e = sending().recovered();
        assert_eq!(e.state, OutboxState::Failed(SendFailure::Uncertain));
        assert!(!e.is_due(NOW + 1_000_000));
        let queued = OutboxEntry::queued(NOW);
        assert_eq!(queued.recovered(), queued, "en attente : inchangé");
        // L'utilisateur relance : nouvel essai, compteur remis à zéro.
        let e = e.retry(NOW + 5).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(e, OutboxEntry::queued(NOW + 5));
    }

    #[test]
    fn impossible_transitions_are_refused() {
        let queued = OutboxEntry::queued(NOW);
        assert!(queued.succeeded().is_err());
        assert!(queued.failed(SendFailure::Temporary, NOW).is_err());
        assert!(queued.retry(NOW).is_err());
        assert!(sending().start().is_err(), "déjà réservé");
        let sent = sending().succeeded().unwrap_or_else(|e| panic!("{e}"));
        assert!(sent.start().is_err());
        assert!(sent.retry(NOW).is_err(), "déjà parti");
        assert!(sent.failed(SendFailure::Temporary, NOW).is_err());
    }
}
