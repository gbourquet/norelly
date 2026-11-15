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
//!   fois. Une connexion coupée ou bloquée avant la mise en place du
//!   chiffrement l'est de même, mais son échec définitif le dit
//!   ([`SendFailure::EncryptionInterrupted`]) : répétée, elle peut venir du
//!   réseau utilisé.
//! - Un envoi interrompu par l'arrêt de l'application (`Sending` au
//!   démarrage) a peut-être abouti : il devient un échec
//!   [`SendFailure::Uncertain`], jamais renvoyé automatiquement (pas de
//!   doublon chez les destinataires) ; l'utilisateur décide. Il en va de
//!   même d'une transmission interrompue sans réponse du serveur
//!   ([`crate::ports::SendError::Interrupted`]).

use crate::auth::AuthError;
use crate::capabilities::ServerCapabilities;
use crate::connection::{ConnectionError, TimeoutStage};
use crate::ports::SendError;

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
    /// Connexion non sûre (certificat, TLS, STARTTLS absent) : à régler
    /// par l'utilisateur, jamais contourné automatiquement.
    Security,
    /// Envoi interrompu par l'arrêt de l'application : peut-être parti.
    Uncertain,
    /// Trop d'échecs temporaires.
    TooManyAttempts,
    /// Connexion coupée ou bloquée avant la mise en place du chiffrement :
    /// réessayée comme un échec temporaire, puis échec définitif sous ce
    /// nom, car une coupure répétée à ce moment peut venir du réseau
    /// utilisé (interception). Rien n'est parti.
    EncryptionInterrupted,
    /// Connexion coupée, ou serveur muet, après l'envoi des identifiants :
    /// échec définitif, les identifiants ne sont jamais renvoyés
    /// automatiquement (un serveur peut bloquer le compte après quelques
    /// échecs) ; l'utilisateur relance.
    AuthenticationInterrupted,
}

/// Faut-il déposer une copie du message envoyé dans « Envoyés » ? Non
/// quand le serveur le fait lui-même à l'envoi SMTP : Gmail (capacité IMAP
/// `X-GM-EXT-1`), sans quoi le message y apparaîtrait deux fois.
#[must_use]
pub fn sent_copy_needed(imap: &ServerCapabilities) -> bool {
    !imap.has("X-GM-EXT-1")
}

impl SendFailure {
    /// Cause d'échec d'une erreur d'envoi.
    #[must_use]
    pub fn of(error: &SendError) -> Self {
        match error {
            SendError::Deferred(_) => Self::Temporary,
            SendError::Interrupted(_) => Self::Uncertain,
            SendError::Rejected(_) | SendError::InvalidAddress(_) => Self::Rejected,
            SendError::Connection(e) => match e {
                // Avant tout échange : hors ligne, rien n'est parti.
                ConnectionError::Network(_) | ConnectionError::Timeout(TimeoutStage::Connect) => {
                    Self::Unreachable
                }
                // Serveur joint, chiffrement jamais en place : rien n'est
                // parti, mais l'échec répété doit être nommé.
                ConnectionError::ClosedBeforeEncryption
                | ConnectionError::Timeout(
                    TimeoutStage::TlsHandshake | TimeoutStage::BeforeEncryption,
                ) => Self::EncryptionInterrupted,
                ConnectionError::Authentication(AuthError::Interrupted) => {
                    Self::AuthenticationInterrupted
                }
                ConnectionError::Timeout(TimeoutStage::Response | TimeoutStage::Write)
                | ConnectionError::Closed
                | ConnectionError::Protocol(_)
                // Trousseau verrouillé, fournisseur OAuth en panne : rien
                // n'est parti.
                | ConnectionError::Authentication(
                    AuthError::SecretStoreUnavailable { .. } | AuthError::ProviderUnavailable { .. },
                ) => Self::Temporary,
                ConnectionError::Rejected(_) => Self::Rejected,
                ConnectionError::Authentication(_) => Self::Authentication,
                ConnectionError::Certificate(_)
                | ConnectionError::Tls(_)
                | ConnectionError::StartTlsUnavailable
                | ConnectionError::StartTlsRejected(_)
                | ConnectionError::PlaintextInjection
                | ConnectionError::PlaintextRefused(_) => Self::Security,
            },
        }
    }
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

/// Échéance la plus lointaine d'un envoi différé (un an).
pub const MAX_SCHEDULE_SECS: i64 = 366 * 24 * 3600;

/// Échéance d'envoi refusée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("échéance d'envoi trop lointaine")]
pub struct ScheduleTooFar;

impl OutboxEntry {
    /// Message mis en file à `now`, à envoyer pas avant `at` : une
    /// échéance passée vaut un envoi immédiat, une échéance à plus de
    /// [`MAX_SCHEDULE_SECS`] est refusée.
    ///
    /// # Errors
    /// [`ScheduleTooFar`] au-delà de [`MAX_SCHEDULE_SECS`].
    pub fn scheduled(now: i64, at: i64) -> Result<Self, ScheduleTooFar> {
        if at.saturating_sub(now) > MAX_SCHEDULE_SECS {
            return Err(ScheduleTooFar);
        }
        Ok(Self::queued(now.max(at)))
    }

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
            SendFailure::Temporary | SendFailure::EncryptionInterrupted
                if self.attempts < MAX_SEND_ATTEMPTS =>
            {
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

/// Attente avant de présenter de nouveau les identifiants d'un compte
/// après un refus ou une interruption à l'envoi (15 min), sauf action de
/// l'utilisateur.
pub const CREDENTIALS_HOLD_SECS: i64 = 15 * 60;

/// Échéance avant laquelle la file d'un compte n'envoie plus rien, après
/// les échecs `failures` d'une tournée faite à `now` : les messages
/// suivants présenteraient les mêmes identifiants, et un serveur peut
/// bloquer le compte après quelques échecs. `None` : pas d'attente.
#[must_use]
pub fn credentials_hold<'a>(
    failures: impl IntoIterator<Item = &'a SendFailure>,
    now: i64,
) -> Option<i64> {
    failures
        .into_iter()
        .any(|f| {
            matches!(
                f,
                SendFailure::Authentication | SendFailure::AuthenticationInterrupted
            )
        })
        .then(|| now.saturating_add(CREDENTIALS_HOLD_SECS))
}

/// Même règle que [`credentials_hold`] pour une connexion (copie des
/// brouillons et des messages envoyés sur le serveur de réception) dont
/// le mot de passe a été refusé ou interrompu (un jeton OAuth ne fait pas
/// bloquer le compte : [`ConnectionError::replay_risks_lockout`]).
#[must_use]
pub fn connection_hold(
    error: &ConnectionError,
    kind: crate::credentials::CredentialKind,
    now: i64,
) -> Option<i64> {
    error
        .replay_risks_lockout(kind)
        .then(|| now.saturating_add(CREDENTIALS_HOLD_SECS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_credentials_failure_on_connection_holds_the_remote_work() {
        use crate::auth::AuthError;
        use crate::credentials::CredentialKind;
        for e in [
            ConnectionError::from(AuthError::Interrupted),
            ConnectionError::from(AuthError::Rejected {
                message: "x".into(),
            }),
        ] {
            assert_eq!(
                connection_hold(&e, CredentialKind::Password, NOW),
                Some(NOW + CREDENTIALS_HOLD_SECS),
                "{e:?}"
            );
        }
        assert_eq!(
            connection_hold(&ConnectionError::Closed, CredentialKind::Password, NOW),
            None
        );
        // Jeton OAuth : refus parfois passager chez Microsoft, sans risque
        // de blocage du compte.
        assert_eq!(
            connection_hold(
                &ConnectionError::from(AuthError::Interrupted),
                CredentialKind::OAuth2,
                NOW
            ),
            None
        );
    }

    #[test]
    fn a_credentials_failure_holds_the_queue() {
        for failure in [
            SendFailure::Authentication,
            SendFailure::AuthenticationInterrupted,
        ] {
            assert_eq!(
                credentials_hold(&[SendFailure::Temporary, failure], NOW),
                Some(NOW + CREDENTIALS_HOLD_SECS)
            );
        }
        assert_eq!(
            credentials_hold(&[SendFailure::Temporary, SendFailure::Security], NOW),
            None
        );
        assert_eq!(credentials_hold(&[], NOW), None);
    }

    const NOW: i64 = 1_000;

    fn sending() -> OutboxEntry {
        OutboxEntry::queued(NOW)
            .start()
            .unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn scheduled_sending_waits_for_its_time() {
        let e = OutboxEntry::scheduled(NOW, NOW + 3600).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            e.state,
            OutboxState::Queued {
                not_before: NOW + 3600
            }
        );
        assert!(!e.is_due(NOW + 3599));
        assert!(e.is_due(NOW + 3600));
        assert_eq!(
            OutboxEntry::scheduled(NOW, NOW - 10),
            Ok(OutboxEntry::queued(NOW)),
            "échéance passée : tout de suite"
        );
        assert_eq!(
            OutboxEntry::scheduled(NOW, NOW + MAX_SCHEDULE_SECS + 1),
            Err(ScheduleTooFar)
        );
        assert!(OutboxEntry::scheduled(NOW, i64::MAX).is_err());
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
    fn a_connection_cut_before_encryption_is_retried_then_named() {
        let mut e = OutboxEntry::queued(NOW);
        for n in 1..MAX_SEND_ATTEMPTS {
            e = e
                .start()
                .and_then(|e| e.failed(SendFailure::EncryptionInterrupted, NOW))
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(
                e.state,
                OutboxState::Queued {
                    not_before: NOW + retry_delay(n)
                },
                "essai {n} compté, comme un échec temporaire"
            );
        }
        let e = e
            .start()
            .and_then(|e| e.failed(SendFailure::EncryptionInterrupted, NOW))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            e.state,
            OutboxState::Failed(SendFailure::EncryptionInterrupted),
            "l'échec définitif dit que la coupure précède le chiffrement"
        );
    }

    #[test]
    fn cuts_before_encryption_and_during_authentication_at_send() {
        use crate::auth::AuthError;
        let conn = |e: ConnectionError| SendFailure::of(&SendError::Connection(e));
        assert_eq!(
            conn(ConnectionError::ClosedBeforeEncryption),
            SendFailure::EncryptionInterrupted
        );
        assert_eq!(
            conn(ConnectionError::Timeout(TimeoutStage::TlsHandshake)),
            SendFailure::EncryptionInterrupted,
            "serveur joint, négociation TLS bloquée : compté, jamais en attente sans fin"
        );
        assert_eq!(
            conn(ConnectionError::Timeout(TimeoutStage::BeforeEncryption)),
            SendFailure::EncryptionInterrupted,
            "serveur muet avant STARTTLS"
        );
        assert_eq!(
            conn(ConnectionError::Authentication(AuthError::Interrupted)),
            SendFailure::AuthenticationInterrupted,
            "identifiants jamais renvoyés automatiquement"
        );
    }

    #[test]
    fn permanent_failures_stop_at_once() {
        for failure in [
            SendFailure::Rejected,
            SendFailure::Authentication,
            SendFailure::AuthenticationInterrupted,
        ] {
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
    fn gmail_files_sent_messages_itself() {
        let caps = |t: &[&str]| ServerCapabilities::from_imap(t.iter().copied());
        assert!(sent_copy_needed(&caps(&["IMAP4rev1", "UIDPLUS"])));
        assert!(!sent_copy_needed(&caps(&["IMAP4rev1", "X-GM-EXT-1"])));
        assert!(!sent_copy_needed(&caps(&["x-gm-ext-1"])), "casse ignorée");
    }

    #[test]
    fn send_errors_map_to_failures() {
        use crate::auth::AuthError;
        let of = |e: SendError| SendFailure::of(&e);
        let conn = |e: ConnectionError| SendFailure::of(&SendError::Connection(e));
        assert_eq!(
            of(SendError::Deferred("451".into())),
            SendFailure::Temporary
        );
        assert_eq!(
            of(SendError::Interrupted("délai".into())),
            SendFailure::Uncertain,
            "peut-être parti : jamais renvoyé automatiquement"
        );
        assert_eq!(of(SendError::Rejected("550".into())), SendFailure::Rejected);
        assert_eq!(
            of(SendError::InvalidAddress("x".into())),
            SendFailure::Rejected
        );
        assert_eq!(
            conn(ConnectionError::Network("dns".into())),
            SendFailure::Unreachable
        );
        assert_eq!(
            conn(ConnectionError::Timeout(TimeoutStage::Connect)),
            SendFailure::Unreachable
        );
        assert_eq!(
            conn(ConnectionError::Timeout(TimeoutStage::Response)),
            SendFailure::Temporary,
            "coupure pendant l'envoi : compté"
        );
        assert_eq!(conn(ConnectionError::Closed), SendFailure::Temporary);
        assert_eq!(
            conn(ConnectionError::Protocol("x".into())),
            SendFailure::Temporary
        );
        assert_eq!(
            conn(ConnectionError::Rejected("x".into())),
            SendFailure::Rejected
        );
        assert_eq!(
            conn(ConnectionError::Authentication(AuthError::Rejected {
                message: "535".into()
            })),
            SendFailure::Authentication
        );
        assert_eq!(
            conn(ConnectionError::Authentication(
                AuthError::SecretStoreUnavailable {
                    message: "verrouillé".into()
                }
            )),
            SendFailure::Temporary,
            "trousseau verrouillé : rien n'est parti, réessayé plus tard"
        );
        assert_eq!(
            conn(ConnectionError::Authentication(
                AuthError::ProviderUnavailable {
                    message: "délai dépassé".into()
                }
            )),
            SendFailure::Temporary,
            "fournisseur injoignable : rien n'est parti, réessayé plus tard"
        );
        assert_eq!(
            conn(ConnectionError::Authentication(
                AuthError::ProviderNotConfigured
            )),
            SendFailure::Authentication,
            "fournisseur absent de cette version : pas de nouveaux essais en boucle"
        );
        for security in [
            ConnectionError::Tls("x".into()),
            ConnectionError::StartTlsUnavailable,
            ConnectionError::StartTlsRejected("x".into()),
            ConnectionError::PlaintextInjection,
        ] {
            assert_eq!(conn(security), SendFailure::Security);
        }
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
