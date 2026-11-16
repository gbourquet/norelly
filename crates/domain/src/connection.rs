//! Erreurs de connexion structurées, communes à IMAP et SMTP.

use serde::{Deserialize, Serialize};

use crate::auth::AuthError;
use crate::security::{CertificateRejection, PlaintextRefused, StartTlsUnavailable};

/// Longueur maximale d'un texte fourni par le serveur dans une erreur.
const MAX_SERVER_TEXT: usize = 200;

/// Rend affichable un texte fourni par un serveur (non fiable) : caractères
/// de contrôle remplacés, longueur bornée.
#[must_use]
pub fn server_text(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out: String = text
        .chars()
        .map(|c| {
            if c.is_control() || is_format_char(c) {
                ' '
            } else {
                c
            }
        })
        .take(MAX_SERVER_TEXT)
        .collect();
    if text.chars().count() > MAX_SERVER_TEXT {
        out.push('…');
    }
    out.trim().to_owned()
}

/// Caractères de mise en forme invisibles ou bidirectionnels (catégorie
/// Unicode Cf usuelle) : ils permettent de maquiller un texte affiché
/// (U+202E retourne la suite du texte).
pub(crate) fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// Étape pendant laquelle un délai a expiré.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeoutStage {
    Connect,
    TlsHandshake,
    /// Serveur muet pendant l'échange en clair qui précède STARTTLS : le
    /// chiffrement n'a pas pu être mis en place.
    BeforeEncryption,
    Response,
    Write,
}

/// Échec d'une connexion à un serveur.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum ConnectionError {
    #[error("erreur réseau : {0}")]
    Network(String),
    #[error("délai dépassé ({0:?})")]
    Timeout(TimeoutStage),
    #[error("le serveur a fermé la connexion")]
    Closed,
    /// Connexion coupée (fermée ou réinitialisée) avant la fin de la mise
    /// en place du chiffrement : échange en clair qui précède STARTTLS ou
    /// poignée de main TLS. Aucun identifiant n'est parti ; une coupure
    /// répétée à ce moment peut venir du réseau utilisé (interception).
    #[error("connexion coupée avant la mise en place du chiffrement")]
    ClosedBeforeEncryption,
    #[error("certificat refusé pour {} : {}", .0.host, .0.issue)]
    Certificate(CertificateRejection),
    #[error("échec de la négociation TLS : {0}")]
    Tls(String),
    #[error("le serveur n'annonce pas STARTTLS : connexion interrompue (aucun repli en clair)")]
    StartTlsUnavailable,
    #[error("le serveur a refusé STARTTLS : {0}")]
    StartTlsRejected(String),
    #[error(
        "données reçues avant la négociation TLS : connexion interrompue (tentative d'injection)"
    )]
    PlaintextInjection,
    #[error(transparent)]
    PlaintextRefused(#[from] PlaintextRefused),
    #[error(transparent)]
    Authentication(#[from] AuthError),
    #[error("réponse du serveur invalide : {0}")]
    Protocol(String),
    /// Commande refusée (`NO`) : la session reste utilisable.
    #[error("commande refusée par le serveur : {0}")]
    Rejected(String),
}

impl From<StartTlsUnavailable> for ConnectionError {
    fn from(_: StartTlsUnavailable) -> Self {
        Self::StartTlsUnavailable
    }
}

/// Catégorie d'échec, pour l'affichage et le diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureCategory {
    /// Serveur injoignable, délai, coupure.
    Network,
    /// Certificat refusé (exception possible) ou négociation TLS impossible.
    Certificate,
    /// Identifiants refusés ou méthode d'authentification impossible.
    Authentication,
    /// Protection de sécurité déclenchée (clair refusé, STARTTLS absent,
    /// injection).
    Security,
    /// Serveur non conforme.
    Protocol,
}

impl ConnectionError {
    /// Vrai si la commande qui a reçu cette erreur n'a certainement pas
    /// été appliquée par le serveur : seul un refus (`NO`) le prouve. Une
    /// coupure, un délai ou une réponse illisible (dont `BAD`, que
    /// l'adaptateur ne distingue pas d'une réponse mal formée) peuvent
    /// survenir après son application : son résultat est inconnu.
    #[must_use]
    pub fn leaves_command_unapplied(&self) -> bool {
        matches!(self, Self::Rejected(_))
    }

    /// Vrai si un nouvel essai ne peut pas aboutir avec cette version de
    /// l'application (fournisseur OAuth dont elle n'a pas les identifiants
    /// client) : la synchronisation s'arrête au lieu de réessayer.
    #[must_use]
    pub fn retry_is_useless(&self) -> bool {
        matches!(
            self,
            Self::Authentication(crate::auth::AuthError::ProviderNotConfigured)
        )
    }

    #[must_use]
    pub fn category(&self) -> FailureCategory {
        match self {
            Self::Network(_) | Self::Timeout(_) | Self::Closed | Self::ClosedBeforeEncryption => {
                FailureCategory::Network
            }
            Self::Certificate(_) | Self::Tls(_) => FailureCategory::Certificate,
            Self::Authentication(_) => FailureCategory::Authentication,
            Self::StartTlsUnavailable
            | Self::StartTlsRejected(_)
            | Self::PlaintextInjection
            | Self::PlaintextRefused(_) => FailureCategory::Security,
            Self::Protocol(_) | Self::Rejected(_) => FailureCategory::Protocol,
        }
    }
    /// Vrai si l'échec est passager (serveur muet, connexion coupée, réseau
    /// instable) : une nouvelle connexion a des chances d'aboutir. Un refus
    /// du serveur, un certificat ou une protection ne se réessaient pas, ni
    /// une coupure après l'envoi des identifiants
    /// ([`AuthError::Interrupted`]) : seuls les échecs qui précèdent leur
    /// envoi le sont.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Network(_) | Self::Timeout(_) | Self::Closed | Self::ClosedBeforeEncryption
        )
    }

    /// Vrai si le serveur a été joint puis s'est tu ou a coupé la connexion,
    /// par opposition à un serveur introuvable ou qui refuse la connexion.
    /// Réseau instable le plus souvent ; un port ou un chiffrement faux
    /// produit aussi ce silence.
    #[must_use]
    pub fn is_unresponsive(&self) -> bool {
        matches!(
            self,
            Self::Timeout(
                TimeoutStage::TlsHandshake
                    | TimeoutStage::BeforeEncryption
                    | TimeoutStage::Response
                    | TimeoutStage::Write
            ) | Self::Closed
                | Self::ClosedBeforeEncryption
        )
    }

    /// Vrai si un compte dont les serveurs sont imposés par le fournisseur
    /// (OAuth) peut être ajouté malgré cet échec du test d'envoi, la
    /// réception ayant réussi : le serveur d'envoi a été joint puis s'est tu
    /// ou a coupé la connexion une fois le canal chiffré (réseau instable).
    /// L'envoi sera vérifié au premier message. Un serveur introuvable, un
    /// refus ou une protection bloquent l'ajout, de même qu'une coupure ou
    /// une négociation bloquée avant la fin du chiffrement : c'est ce que
    /// produirait un réseau qui intercepte la connexion. Un jeton présenté
    /// une fois sur le canal chiffré, puis coupé ou resté sans réponse
    /// ([`AuthError::Interrupted`]), ne bloque pas l'ajout : il n'est pas
    /// renvoyé. Pour des serveurs saisis, un port ou un chiffrement faux
    /// produit le même silence : cette règle ne s'y applique pas.
    #[must_use]
    pub fn send_check_can_wait(&self) -> bool {
        let before_encryption = matches!(
            self,
            Self::ClosedBeforeEncryption
                | Self::Timeout(TimeoutStage::TlsHandshake | TimeoutStage::BeforeEncryption)
        );
        (self.is_unresponsive() && !before_encryption)
            || matches!(self, Self::Authentication(AuthError::Interrupted))
    }

    /// Identifiants refusés ou authentification interrompue : une
    /// reconnexion les renverrait au serveur, qui peut bloquer le compte
    /// après quelques échecs.
    #[must_use]
    pub fn credentials_failed(&self) -> bool {
        matches!(
            self,
            Self::Authentication(AuthError::Rejected { .. } | AuthError::Interrupted)
        )
    }

    /// Vrai si une reconnexion rapide risque de faire bloquer le compte :
    /// mot de passe refusé ou interrompu ([`Self::credentials_failed`]). Un
    /// jeton OAuth ne déclenche pas ces blocages.
    #[must_use]
    pub fn replay_risks_lockout(&self, kind: crate::credentials::CredentialKind) -> bool {
        kind == crate::credentials::CredentialKind::Password && self.credentials_failed()
    }
}

/// Phase courante d'une ouverture de connexion, lue par le délai total qui
/// l'enveloppe.
#[derive(Debug, Default)]
pub struct OpeningPhase(std::sync::atomic::AtomicU8);

impl OpeningPhase {
    /// L'ouverture entre dans `phase`.
    pub fn enter(&self, phase: ConnectionPhase) {
        let code = match phase {
            ConnectionPhase::BeforeAuthentication => 0,
            ConnectionPhase::BeforeEncryption => 1,
            ConnectionPhase::Authentication => 2,
        };
        self.0.store(code, std::sync::atomic::Ordering::Relaxed);
    }

    /// Phase courante (au départ : rien d'envoyé, ni chiffrement exigé).
    #[must_use]
    pub fn current(&self) -> ConnectionPhase {
        match self.0.load(std::sync::atomic::Ordering::Relaxed) {
            1 => ConnectionPhase::BeforeEncryption,
            2 => ConnectionPhase::Authentication,
            _ => ConnectionPhase::BeforeAuthentication,
        }
    }

    /// Erreur du délai total d'ouverture atteint dans la phase courante.
    #[must_use]
    pub fn deadline_reached(&self) -> ConnectionError {
        self.current()
            .classify(ConnectionError::Timeout(TimeoutStage::Response))
    }
}

/// Phase d'une connexion pendant laquelle un échec survient : elle décide
/// de ce que signifie une coupure ou un silence du serveur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionPhase {
    /// Serveur joint, chiffrement pas encore en place : échange en clair
    /// qui précède STARTTLS, poignée de main TLS.
    BeforeEncryption,
    /// Canal établi (chiffré, ou en clair consenti), identifiants pas
    /// encore envoyés : salutation, capacités.
    BeforeAuthentication,
    /// Identifiants envoyés (`LOGIN`, `AUTHENTICATE`, `AUTH`), réponse du
    /// serveur attendue.
    Authentication,
}

impl ConnectionPhase {
    /// Phase de l'échange en clair qui précède l'authentification : avant
    /// le chiffrement si STARTTLS est exigé ; sur un serveur en clair
    /// consenti, il n'y a pas de chiffrement à attendre. En TLS implicite,
    /// il n'y a pas d'échange en clair.
    #[must_use]
    pub fn plaintext_opening(security: crate::endpoint::Security) -> Self {
        match security {
            crate::endpoint::Security::StartTls => Self::BeforeEncryption,
            crate::endpoint::Security::Tls | crate::endpoint::Security::Plain => {
                Self::BeforeAuthentication
            }
        }
    }

    /// Erreur d'une connexion survenue pendant cette phase :
    /// - avant le chiffrement, une coupure par le serveur ou le réseau
    ///   (fermeture ou réinitialisation, indifféremment) devient
    ///   [`ConnectionError::ClosedBeforeEncryption`], et un silence
    ///   [`TimeoutStage::BeforeEncryption`] ; un réseau perdu (hors ligne,
    ///   hôte injoignable) reste une erreur réseau ;
    /// - pendant l'authentification, une coupure, un silence ou un réseau
    ///   perdu devient [`AuthError::Interrupted`], qui n'est pas rejoué :
    ///   les identifiants ont pu arriver ;
    /// - tout autre échec est gardé tel quel.
    #[must_use]
    pub fn classify(self, error: ConnectionError) -> ConnectionError {
        match (self, error) {
            (Self::BeforeEncryption, ConnectionError::Closed) => {
                ConnectionError::ClosedBeforeEncryption
            }
            (
                Self::BeforeEncryption,
                ConnectionError::Timeout(TimeoutStage::Response | TimeoutStage::Write),
            ) => ConnectionError::Timeout(TimeoutStage::BeforeEncryption),
            (Self::Authentication, error) if error.is_transient() => AuthError::Interrupted.into(),
            (_, error) => error,
        }
    }
}

/// Essais d'une connexion dont l'échec est passager
/// ([`ConnectionError::is_transient`]), le premier compris.
pub const CONNECTION_ATTEMPTS: usize = 3;

/// Attente entre deux essais d'une connexion.
pub const CONNECTION_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

/// Durée totale au plus du test d'un serveur (réception ou envoi), essais
/// et attentes compris : au-delà, un débutant croit l'application bloquée.
pub const CONNECTION_TEST_BUDGET: std::time::Duration = std::time::Duration::from_secs(45);

/// Durée minimale laissée à un nouvel essai : un essai plus court
/// échouerait par manque de temps et masquerait l'erreur du précédent.
pub const MIN_ATTEMPT_DURATION: std::time::Duration = std::time::Duration::from_secs(5);

/// Délai minimal de l'échange d'authentification par mot de passe : plus
/// long que les retards qu'appliquent les serveurs à un refus (pénalité
/// croissante après des échecs répétés), pour que le refus soit lu et non
/// pris pour un silence passager, qui renverrait le mot de passe.
pub const PASSWORD_AUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Délai de chaque lecture pendant l'échange d'authentification : le délai
/// d'ouverture pour un jeton OAuth (un serveur muet est abandonné tôt), au
/// moins `password_floor` pour un mot de passe. Dans les deux cas,
/// l'expiration n'est pas rejouée ([`ConnectionPhase::Authentication`]).
#[must_use]
pub fn authentication_timeout(
    kind: crate::credentials::CredentialKind,
    opening: std::time::Duration,
    password_floor: std::time::Duration,
) -> std::time::Duration {
    match kind {
        crate::credentials::CredentialKind::Password => opening.max(password_floor),
        crate::credentials::CredentialKind::OAuth2 => opening,
    }
}

/// Prochain essai d'un test de connexion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextAttempt {
    /// Attente avant l'essai.
    pub wait: std::time::Duration,
    /// Durée accordée à l'essai, au-delà de laquelle il est abandonné.
    pub limit: std::time::Duration,
}

/// Nouvel essai d'un test de connexion, d'après le nombre d'essais déjà
/// faits (au moins un : le premier dispose de tout le plafond), l'erreur du
/// dernier (`None` : essai réussi) et le temps écoulé depuis le début du
/// test. `None` : le test s'arrête sur le dernier résultat.
#[must_use]
pub fn next_test_attempt(
    made: usize,
    last_error: Option<&ConnectionError>,
    elapsed: std::time::Duration,
) -> Option<NextAttempt> {
    if made >= CONNECTION_ATTEMPTS || !last_error.is_some_and(ConnectionError::is_transient) {
        return None;
    }
    let limit = CONNECTION_TEST_BUDGET
        .saturating_sub(elapsed)
        .saturating_sub(CONNECTION_RETRY_DELAY);
    (limit >= MIN_ATTEMPT_DURATION).then_some(NextAttempt {
        wait: CONNECTION_RETRY_DELAY,
        limit,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_a_refusal_proves_a_command_was_not_applied() {
        assert!(ConnectionError::Rejected("NO".into()).leaves_command_unapplied());
        // Coupure, délai, réponse illisible ou `BAD` : le serveur a pu
        // appliquer la commande avant que la réponse se perde.
        for e in [
            ConnectionError::Closed,
            ConnectionError::Timeout(TimeoutStage::Response),
            ConnectionError::Network("x".into()),
            ConnectionError::Protocol("x".into()),
        ] {
            assert!(!e.leaves_command_unapplied(), "{e:?}");
        }
    }

    #[test]
    fn retrying_is_useless_only_when_this_version_lacks_the_provider() {
        use crate::auth::AuthError;
        assert!(
            ConnectionError::Authentication(AuthError::ProviderNotConfigured).retry_is_useless()
        );
        for e in [
            ConnectionError::Network("x".into()),
            ConnectionError::Authentication(AuthError::ReauthRequired),
            ConnectionError::Authentication(AuthError::Rejected {
                message: "x".into(),
            }),
            ConnectionError::Authentication(AuthError::ProviderUnavailable {
                message: "x".into(),
            }),
        ] {
            assert!(!e.retry_is_useless(), "{e:?}");
        }
    }

    use super::*;

    #[test]
    fn server_text_is_sanitized_and_bounded() {
        assert_eq!(
            server_text(b"Bad\r\npassword\x1b[31m"),
            "Bad  password [31m"
        );
        let long = server_text(&[b'a'; 1000]);
        assert_eq!(long.chars().count(), MAX_SERVER_TEXT + 1);
        assert!(long.ends_with('…'));
        assert_eq!(server_text(&[0xff, b'o', b'k']), "\u{fffd}ok");
        assert_eq!(
            server_text("mot\u{202E}gnp.exe\u{200B}\u{2066}x\u{FEFF}".as_bytes()),
            "mot gnp.exe  x"
        );
    }

    #[test]
    fn categories() {
        assert_eq!(ConnectionError::Closed.category(), FailureCategory::Network);
        assert_eq!(
            ConnectionError::Timeout(TimeoutStage::Connect).category(),
            FailureCategory::Network
        );
        assert_eq!(
            ConnectionError::PlaintextInjection.category(),
            FailureCategory::Security
        );
        assert_eq!(
            ConnectionError::from(StartTlsUnavailable).category(),
            FailureCategory::Security
        );
        assert_eq!(
            ConnectionError::from(AuthError::LoginDisabled).category(),
            FailureCategory::Authentication
        );
        assert_eq!(
            ConnectionError::Tls("x".into()).category(),
            FailureCategory::Certificate
        );
        assert_eq!(
            ConnectionError::Protocol("x".into()).category(),
            FailureCategory::Protocol
        );
    }

    #[test]
    fn transient_failures_are_retried() {
        for e in [
            ConnectionError::Network("connexion réinitialisée".into()),
            ConnectionError::Timeout(TimeoutStage::Connect),
            ConnectionError::Timeout(TimeoutStage::TlsHandshake),
            ConnectionError::Timeout(TimeoutStage::Response),
            ConnectionError::Timeout(TimeoutStage::Write),
            ConnectionError::Closed,
        ] {
            assert!(e.is_transient(), "{e:?}");
        }
        for e in [
            ConnectionError::from(AuthError::LoginDisabled),
            ConnectionError::Tls("x".into()),
            ConnectionError::from(StartTlsUnavailable),
            ConnectionError::PlaintextInjection,
            ConnectionError::Protocol("x".into()),
            ConnectionError::Rejected("x".into()),
        ] {
            assert!(!e.is_transient(), "{e:?}");
        }
    }

    mod phases {
        use super::super::*;
        use crate::endpoint::Security;

        /// Coupures et silences du serveur.
        fn cuts_and_silences() -> [ConnectionError; 3] {
            [
                ConnectionError::Closed,
                ConnectionError::Timeout(TimeoutStage::Response),
                ConnectionError::Timeout(TimeoutStage::Write),
            ]
        }

        /// Échecs qu'aucune phase ne requalifie.
        fn other_failures() -> [ConnectionError; 5] {
            [
                ConnectionError::from(AuthError::Rejected {
                    message: "x".into(),
                }),
                ConnectionError::Protocol("x".into()),
                ConnectionError::Rejected("x".into()),
                ConnectionError::Tls("x".into()),
                ConnectionError::PlaintextInjection,
            ]
        }

        #[test]
        fn a_cut_or_silence_after_the_credentials_is_an_interrupted_authentication() {
            // Réseau perdu une fois les identifiants partis : ils ont pu
            // arriver, un nouvel essai les renverrait.
            let lost = ConnectionError::Network("réseau injoignable".into());
            for e in cuts_and_silences().into_iter().chain([lost]) {
                let classified = ConnectionPhase::Authentication.classify(e.clone());
                assert_eq!(
                    classified,
                    ConnectionError::from(AuthError::Interrupted),
                    "{e:?}"
                );
            }
        }

        #[test]
        fn no_failure_during_authentication_stays_transient() {
            for stage in [
                TimeoutStage::Connect,
                TimeoutStage::TlsHandshake,
                TimeoutStage::BeforeEncryption,
                TimeoutStage::Response,
                TimeoutStage::Write,
            ] {
                let e = ConnectionError::Timeout(stage);
                assert!(
                    !ConnectionPhase::Authentication.classify(e).is_transient(),
                    "{stage:?}"
                );
            }
            for e in [
                ConnectionError::Closed,
                ConnectionError::ClosedBeforeEncryption,
                ConnectionError::Network("x".into()),
            ] {
                assert!(
                    !ConnectionPhase::Authentication
                        .classify(e.clone())
                        .is_transient(),
                    "{e:?}"
                );
            }
        }

        #[test]
        fn an_interrupted_authentication_is_never_replayed() {
            let interrupted = ConnectionError::from(AuthError::Interrupted);
            assert!(!interrupted.is_transient());
            assert!(!interrupted.retry_is_useless());
            assert!(interrupted.credentials_failed());
            assert_eq!(interrupted.category(), FailureCategory::Authentication);
            assert_eq!(
                next_test_attempt(1, Some(&interrupted), std::time::Duration::ZERO),
                None
            );
        }

        #[test]
        fn an_interrupted_token_lets_the_oauth_account_wait_for_its_first_send() {
            // Serveur d'envoi coupé après le jeton, sur un canal chiffré :
            // rien n'est rejoué, l'envoi sera vérifié au premier message.
            assert!(ConnectionError::from(AuthError::Interrupted).send_check_can_wait());
        }

        #[test]
        fn only_a_password_risks_a_lockout_when_replayed() {
            use crate::credentials::CredentialKind;
            let interrupted = ConnectionError::from(AuthError::Interrupted);
            assert!(interrupted.replay_risks_lockout(CredentialKind::Password));
            // Jeton OAuth (déjà rafraîchi) : Microsoft refuse parfois une
            // session de façon passagère, la reconnexion ne doit pas
            // attendre un quart d'heure.
            assert!(!interrupted.replay_risks_lockout(CredentialKind::OAuth2));
            assert!(!ConnectionError::Closed.replay_risks_lockout(CredentialKind::Password));
        }

        #[test]
        fn only_a_refusal_or_an_interruption_is_a_credentials_failure() {
            assert!(
                ConnectionError::from(AuthError::Rejected {
                    message: "x".into()
                })
                .credentials_failed()
            );
            for e in [
                ConnectionError::Closed,
                ConnectionError::from(AuthError::LoginDisabled),
                ConnectionError::from(AuthError::ProviderUnavailable {
                    message: "x".into(),
                }),
                ConnectionError::Rejected("x".into()),
            ] {
                assert!(!e.credentials_failed(), "{e:?}");
            }
        }

        #[test]
        fn a_cut_or_silence_before_encryption_is_named_as_such() {
            assert_eq!(
                ConnectionPhase::BeforeEncryption.classify(ConnectionError::Closed),
                ConnectionError::ClosedBeforeEncryption
            );
            for e in [
                ConnectionError::Timeout(TimeoutStage::Response),
                ConnectionError::Timeout(TimeoutStage::Write),
            ] {
                assert_eq!(
                    ConnectionPhase::BeforeEncryption.classify(e.clone()),
                    ConnectionError::Timeout(TimeoutStage::BeforeEncryption),
                    "{e:?}"
                );
            }
            // Réseau perdu (hors ligne, hôte injoignable) : pas une coupure
            // par le serveur ni le signe d'une interception.
            let lost = ConnectionError::Network("réseau injoignable".into());
            assert_eq!(
                ConnectionPhase::BeforeEncryption.classify(lost.clone()),
                lost
            );
            let handshake = ConnectionError::Timeout(TimeoutStage::TlsHandshake);
            assert_eq!(
                ConnectionPhase::BeforeEncryption.classify(handshake.clone()),
                handshake
            );
        }

        #[test]
        fn a_cut_before_encryption_is_retried_but_never_tolerated() {
            for e in [
                ConnectionError::ClosedBeforeEncryption,
                ConnectionError::Timeout(TimeoutStage::BeforeEncryption),
                ConnectionError::Timeout(TimeoutStage::TlsHandshake),
            ] {
                assert!(e.is_transient(), "aucun identifiant n'est parti : {e:?}");
                assert!(e.is_unresponsive(), "{e:?}");
                assert_eq!(e.category(), FailureCategory::Network);
                assert!(!e.send_check_can_wait(), "{e:?}");
            }
        }

        #[test]
        fn other_phases_and_failures_are_kept() {
            for e in cuts_and_silences() {
                assert_eq!(ConnectionPhase::BeforeAuthentication.classify(e.clone()), e);
            }
            for phase in [
                ConnectionPhase::BeforeEncryption,
                ConnectionPhase::BeforeAuthentication,
                ConnectionPhase::Authentication,
            ] {
                for e in other_failures() {
                    assert_eq!(phase.classify(e.clone()), e, "{phase:?}");
                }
            }
        }

        #[test]
        fn only_starttls_has_a_plaintext_exchange_before_encryption() {
            assert_eq!(
                ConnectionPhase::plaintext_opening(Security::StartTls),
                ConnectionPhase::BeforeEncryption
            );
            for security in [Security::Tls, Security::Plain] {
                assert_eq!(
                    ConnectionPhase::plaintext_opening(security),
                    ConnectionPhase::BeforeAuthentication
                );
            }
        }

        #[test]
        fn the_total_opening_delay_is_classified_by_the_current_phase() {
            let opening = OpeningPhase::default();
            assert_eq!(
                opening.deadline_reached(),
                ConnectionError::Timeout(TimeoutStage::Response)
            );
            opening.enter(ConnectionPhase::BeforeEncryption);
            assert_eq!(
                opening.deadline_reached(),
                ConnectionError::Timeout(TimeoutStage::BeforeEncryption)
            );
            opening.enter(ConnectionPhase::Authentication);
            assert_eq!(
                opening.deadline_reached(),
                ConnectionError::from(AuthError::Interrupted)
            );
            opening.enter(ConnectionPhase::BeforeAuthentication);
            assert_eq!(
                opening.deadline_reached(),
                ConnectionError::Timeout(TimeoutStage::Response)
            );
        }
    }

    #[test]
    fn password_authentication_outlasts_a_delayed_refusal() {
        use crate::credentials::CredentialKind;
        use std::time::Duration;
        let short = Duration::from_secs(15);
        assert_eq!(
            authentication_timeout(CredentialKind::OAuth2, short, PASSWORD_AUTH_TIMEOUT),
            short
        );
        assert_eq!(
            authentication_timeout(CredentialKind::Password, short, PASSWORD_AUTH_TIMEOUT),
            PASSWORD_AUTH_TIMEOUT
        );
        let long = Duration::from_secs(60);
        assert_eq!(
            authentication_timeout(CredentialKind::Password, long, PASSWORD_AUTH_TIMEOUT),
            long
        );
    }

    mod test_attempts {
        use std::time::Duration;

        use super::super::*;

        const TRANSIENT: ConnectionError = ConnectionError::Timeout(TimeoutStage::Response);

        #[test]
        fn transient_failure_is_retried_within_what_remains() {
            let elapsed = Duration::from_secs(16);
            assert_eq!(
                next_test_attempt(1, Some(&TRANSIENT), elapsed),
                Some(NextAttempt {
                    wait: CONNECTION_RETRY_DELAY,
                    limit: CONNECTION_TEST_BUDGET - elapsed - CONNECTION_RETRY_DELAY,
                })
            );
        }

        #[test]
        fn success_or_refusal_ends_the_test() {
            assert_eq!(next_test_attempt(1, None, Duration::from_secs(1)), None);
            let refused = ConnectionError::from(AuthError::LoginDisabled);
            assert_eq!(
                next_test_attempt(1, Some(&refused), Duration::from_secs(1)),
                None
            );
        }

        #[test]
        fn attempts_are_counted() {
            assert!(
                next_test_attempt(CONNECTION_ATTEMPTS - 1, Some(&TRANSIENT), Duration::ZERO)
                    .is_some()
            );
            assert_eq!(
                next_test_attempt(CONNECTION_ATTEMPTS, Some(&TRANSIENT), Duration::ZERO),
                None
            );
        }

        #[test]
        fn no_retry_without_enough_time_left() {
            let just_enough =
                CONNECTION_TEST_BUDGET - CONNECTION_RETRY_DELAY - MIN_ATTEMPT_DURATION;
            assert_eq!(
                next_test_attempt(1, Some(&TRANSIENT), just_enough).map(|a| a.limit),
                Some(MIN_ATTEMPT_DURATION)
            );
            let too_late = just_enough + Duration::from_millis(1);
            assert_eq!(next_test_attempt(1, Some(&TRANSIENT), too_late), None);
            assert_eq!(
                next_test_attempt(1, Some(&TRANSIENT), CONNECTION_TEST_BUDGET * 2),
                None
            );
        }

        #[test]
        fn the_whole_test_fits_in_the_budget() {
            // Premier essai muet pendant 16 s, puis essais qui durent chacun
            // tout le temps accordé : la somme des attentes et des essais ne
            // dépasse jamais le plafond.
            let mut elapsed = Duration::from_secs(16);
            let mut made = 1;
            while let Some(next) = next_test_attempt(made, Some(&TRANSIENT), elapsed) {
                elapsed += next.wait + next.limit;
                made += 1;
            }
            assert_eq!(made, 2);
            assert!(elapsed <= CONNECTION_TEST_BUDGET, "{elapsed:?}");
        }
    }

    #[test]
    fn unresponsive_server_was_reached() {
        for e in [
            ConnectionError::Timeout(TimeoutStage::TlsHandshake),
            ConnectionError::Timeout(TimeoutStage::Response),
            ConnectionError::Timeout(TimeoutStage::Write),
            ConnectionError::Closed,
        ] {
            assert!(e.is_unresponsive(), "{e:?}");
        }
        for e in [
            ConnectionError::Network("nom inconnu".into()),
            ConnectionError::Timeout(TimeoutStage::Connect),
            ConnectionError::from(AuthError::LoginDisabled),
        ] {
            assert!(!e.is_unresponsive(), "{e:?}");
        }
    }

    #[test]
    fn only_a_silent_send_server_lets_the_account_be_added() {
        for e in [
            ConnectionError::Timeout(TimeoutStage::Response),
            ConnectionError::Timeout(TimeoutStage::Write),
            ConnectionError::Closed,
            ConnectionError::from(AuthError::Interrupted),
        ] {
            assert!(e.send_check_can_wait(), "{e:?}");
        }
        for e in [
            // Avant la fin du chiffrement : interception possible.
            ConnectionError::Timeout(TimeoutStage::TlsHandshake),
            ConnectionError::Timeout(TimeoutStage::BeforeEncryption),
            ConnectionError::ClosedBeforeEncryption,
            ConnectionError::Network("nom inconnu".into()),
            ConnectionError::Timeout(TimeoutStage::Connect),
            ConnectionError::from(AuthError::LoginDisabled),
            ConnectionError::from(StartTlsUnavailable),
            ConnectionError::Tls("x".into()),
            ConnectionError::Protocol("x".into()),
            ConnectionError::Rejected("x".into()),
            ConnectionError::StartTlsRejected("454".into()),
            ConnectionError::PlaintextInjection,
            ConnectionError::from(PlaintextRefused {
                host: crate::endpoint::Host::parse("smtp.example.com")
                    .unwrap_or_else(|_| panic!("hôte")),
                port: 587,
            }),
            ConnectionError::from(AuthError::Rejected {
                message: "535 5.7.8".into(),
            }),
            ConnectionError::Certificate(crate::security::CertificateRejection {
                host: crate::endpoint::Host::parse("smtp.example.com")
                    .unwrap_or_else(|_| panic!("hôte")),
                fingerprint: crate::security::CertificateFingerprint::from_bytes([1; 32]),
                issue: crate::security::CertificateIssue::UnknownIssuer,
                fingerprint_changed: false,
                name_mismatch: false,
            }),
        ] {
            assert!(!e.send_check_can_wait(), "{e:?}");
        }
    }
}
