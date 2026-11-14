//! Cas d'usage `TestConnection` : tente une connexion authentifiée à un
//! serveur IMAP ou SMTP et renvoie un résultat structuré, prêt à être
//! présenté à l'utilisateur (sans jamais contenir le secret).

use norelly_domain::auth::AuthError;
use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::connection::{
    CONNECTION_ATTEMPTS, CONNECTION_RETRY_DELAY, ConnectionError, FailureCategory,
};
use norelly_domain::credentials::Credentials;
use norelly_domain::endpoint::{Host, Protocol, ServerEndpoint};
use norelly_domain::ports::{
    CertificateExceptionStore, ConnectRequest, MailSender, RemoteMailboxConnector,
    RemoteMailboxSession,
};
use norelly_domain::security::{CertificateRejection, PlaintextPolicy};

/// Paramètres d'un test de connexion.
#[derive(Debug, Clone)]
pub struct TestConnectionParams {
    pub protocol: Protocol,
    pub endpoint: ServerEndpoint,
    pub credentials: Credentials,
    pub plaintext: PlaintextPolicy,
}

/// Résultat d'un test de connexion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestConnectionOutcome {
    /// Connexion chiffrée (ou clair consenti) et authentification réussies.
    Success { capabilities: ServerCapabilities },
    /// Certificat refusé : l'utilisateur peut, en connaissance de cause,
    /// ajouter une exception (hôte + empreinte) si
    /// [`CertificateRejection::can_be_excepted`].
    Certificate(CertificateRejection),
    /// Identifiants refusés ou méthode d'authentification impossible.
    Authentication(AuthError),
    /// Serveur injoignable, délai dépassé, connexion coupée.
    Network(ConnectionError),
    /// Protection déclenchée : clair refusé, STARTTLS absent ou refusé,
    /// injection détectée, négociation TLS impossible.
    Security(ConnectionError),
    /// Serveur non conforme.
    Protocol(ConnectionError),
    /// Erreur interne (stockage des exceptions…).
    Internal(String),
}

impl From<ConnectionError> for TestConnectionOutcome {
    fn from(error: ConnectionError) -> Self {
        match (error.category(), error) {
            (_, ConnectionError::Certificate(rejection)) => Self::Certificate(rejection),
            (_, ConnectionError::Authentication(auth)) => Self::Authentication(auth),
            (FailureCategory::Network, e) => Self::Network(e),
            (FailureCategory::Protocol, e) => Self::Protocol(e),
            (
                FailureCategory::Security
                | FailureCategory::Certificate
                | FailureCategory::Authentication,
                e,
            ) => Self::Security(e),
        }
    }
}

/// Cas d'usage : tester une connexion.
#[derive(Debug)]
pub struct TestConnection<I, S, E> {
    imap: I,
    smtp: S,
    exceptions: E,
}

impl<I, S, E> TestConnection<I, S, E>
where
    I: RemoteMailboxConnector,
    S: MailSender,
    E: CertificateExceptionStore,
{
    pub fn new(imap: I, smtp: S, exceptions: E) -> Self {
        Self {
            imap,
            smtp,
            exceptions,
        }
    }

    pub async fn execute(&self, params: TestConnectionParams) -> TestConnectionOutcome {
        // Refus du clair avant toute E/S.
        if let Err(refused) = params.plaintext.authorize(&params.endpoint) {
            return ConnectionError::from(refused).into();
        }
        let certificate_exceptions =
            match self.exceptions.exceptions_for(params.endpoint.host()).await {
                Ok(exceptions) => exceptions,
                Err(e) => return TestConnectionOutcome::Internal(e.to_string()),
            };
        let request = ConnectRequest {
            endpoint: params.endpoint,
            credentials: params.credentials,
            certificate_exceptions,
            plaintext: params.plaintext,
        };
        let mut result = self.attempt(params.protocol, &request).await;
        // Réseau instable ou serveur momentanément muet : de nouveaux essais,
        // chacun sur une nouvelle connexion.
        for _ in 1..CONNECTION_ATTEMPTS {
            match &result {
                Err(e) if e.is_transient() => {}
                _ => break,
            }
            tokio::time::sleep(CONNECTION_RETRY_DELAY).await;
            result = self.attempt(params.protocol, &request).await;
        }
        match result {
            Ok(capabilities) => TestConnectionOutcome::Success { capabilities },
            Err(e) => e.into(),
        }
    }

    /// Une exception de certificat existe-t-elle pour `host` ? Un stockage
    /// illisible compte comme une exception : la réponse sert à refuser un
    /// conseil, jamais à accorder une confiance.
    pub async fn has_certificate_exception(&self, host: &Host) -> bool {
        self.exceptions
            .exceptions_for(host)
            .await
            .map_or(true, |exceptions| !exceptions.is_empty())
    }

    async fn attempt(
        &self,
        protocol: Protocol,
        request: &ConnectRequest,
    ) -> Result<ServerCapabilities, ConnectionError> {
        match protocol {
            Protocol::Imap => {
                let session = self.imap.connect(request).await?;
                let capabilities = session.capabilities().clone();
                // Échec de LOGOUT sans importance : la connexion a réussi.
                let _ = session.logout().await;
                Ok(capabilities)
            }
            Protocol::Smtp => self.smtp.verify(request).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use norelly_domain::connection::TimeoutStage;
    use norelly_domain::credentials::{Secret, Username};
    use norelly_domain::endpoint::{Host, Security};
    use norelly_domain::ports::{OutgoingMessage, SendError, StoreError};
    use norelly_domain::security::{
        CertificateException, CertificateFingerprint, CertificateIssue, PlaintextConsent,
    };
    use norelly_test_support::contracts::InMemoryCertificateExceptionStore;
    use norelly_test_support::fake_imap::FakeImap;
    use norelly_test_support::fakes::FakeSession;

    use super::*;

    type Seen = Arc<Mutex<Vec<ConnectRequest>>>;

    type Outcomes =
        Arc<Mutex<std::collections::VecDeque<Result<ServerCapabilities, ConnectionError>>>>;

    /// Faux connecteur : renvoie d'abord les résultats de `first`, un par
    /// connexion, puis un résultat fixé ; mémorise les requêtes.
    #[derive(Debug, Clone)]
    struct FakeConnector {
        result: Result<ServerCapabilities, ConnectionError>,
        first: Outcomes,
        seen: Seen,
    }

    impl FakeConnector {
        fn new(result: Result<ServerCapabilities, ConnectionError>) -> Self {
            Self {
                result,
                first: Outcomes::default(),
                seen: Seen::default(),
            }
        }
        fn first(self, results: Vec<Result<ServerCapabilities, ConnectionError>>) -> Self {
            self.first
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(results);
            self
        }
        fn next_result(&self) -> Result<ServerCapabilities, ConnectionError> {
            self.first
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or_else(|| self.result.clone())
        }
        fn record(&self, request: &ConnectRequest) {
            self.seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request.clone());
        }
        fn requests(&self) -> Vec<ConnectRequest> {
            self.seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    impl RemoteMailboxConnector for FakeConnector {
        type Session = FakeSession;
        async fn connect(&self, request: &ConnectRequest) -> Result<FakeSession, ConnectionError> {
            self.record(request);
            let caps = self.next_result()?;
            FakeSession::new(caps, &FakeImap::new(), Some(ConnectionError::Closed))
        }
    }

    impl MailSender for FakeConnector {
        async fn verify(
            &self,
            request: &ConnectRequest,
        ) -> Result<ServerCapabilities, ConnectionError> {
            self.record(request);
            self.next_result()
        }
        async fn send(&self, _: &ConnectRequest, _: &OutgoingMessage) -> Result<(), SendError> {
            Ok(())
        }
    }

    fn host(name: &str) -> Host {
        Host::parse(name).unwrap_or_else(|_| panic!("hôte"))
    }

    fn params(protocol: Protocol, security: Security) -> TestConnectionParams {
        TestConnectionParams {
            protocol,
            endpoint: ServerEndpoint::new(host("mail.example.com"), 993, security)
                .unwrap_or_else(|_| panic!("point de connexion")),
            credentials: Credentials::new(
                Username::parse("alice").unwrap_or_else(|_| panic!("utilisateur")),
                Secret::new("pw".into()),
            )
            .unwrap_or_else(|_| panic!("identifiants")),
            plaintext: PlaintextPolicy::deny_all(),
        }
    }

    fn rejection() -> CertificateRejection {
        CertificateRejection {
            host: host("mail.example.com"),
            fingerprint: CertificateFingerprint::from_bytes([1; 32]),
            issue: CertificateIssue::UnknownIssuer,
            fingerprint_changed: false,
        }
    }

    fn use_case(
        imap: Result<ServerCapabilities, ConnectionError>,
        smtp: Result<ServerCapabilities, ConnectionError>,
    ) -> (
        TestConnection<FakeConnector, FakeConnector, InMemoryCertificateExceptionStore>,
        FakeConnector,
        FakeConnector,
    ) {
        let (i, s) = (FakeConnector::new(imap), FakeConnector::new(smtp));
        (
            TestConnection::new(
                i.clone(),
                s.clone(),
                InMemoryCertificateExceptionStore::default(),
            ),
            i,
            s,
        )
    }

    #[tokio::test]
    async fn success_reports_capabilities_even_if_logout_fails() {
        let caps = ServerCapabilities::from_imap(["IMAP4rev1", "IDLE"]);
        let (uc, imap, smtp) = use_case(Ok(caps.clone()), Err(ConnectionError::Closed));
        assert_eq!(
            uc.execute(params(Protocol::Imap, Security::Tls)).await,
            TestConnectionOutcome::Success { capabilities: caps }
        );
        assert_eq!(imap.requests().len(), 1);
        assert!(smtp.requests().is_empty());
    }

    #[tokio::test]
    async fn smtp_uses_the_mail_sender() {
        let caps = ServerCapabilities::from_smtp_ehlo(["AUTH PLAIN"]);
        let (uc, imap, smtp) = use_case(Err(ConnectionError::Closed), Ok(caps.clone()));
        assert_eq!(
            uc.execute(params(Protocol::Smtp, Security::StartTls)).await,
            TestConnectionOutcome::Success { capabilities: caps }
        );
        assert!(imap.requests().is_empty());
        assert_eq!(smtp.requests().len(), 1);
    }

    #[tokio::test]
    async fn failures_are_classified() {
        let cases = [
            (
                ConnectionError::Certificate(rejection()),
                TestConnectionOutcome::Certificate(rejection()),
            ),
            (
                AuthError::LoginDisabled.into(),
                TestConnectionOutcome::Authentication(AuthError::LoginDisabled),
            ),
            (
                ConnectionError::Timeout(TimeoutStage::Connect),
                TestConnectionOutcome::Network(ConnectionError::Timeout(TimeoutStage::Connect)),
            ),
            (
                ConnectionError::Network("refusé".into()),
                TestConnectionOutcome::Network(ConnectionError::Network("refusé".into())),
            ),
            (
                ConnectionError::StartTlsUnavailable,
                TestConnectionOutcome::Security(ConnectionError::StartTlsUnavailable),
            ),
            (
                ConnectionError::PlaintextInjection,
                TestConnectionOutcome::Security(ConnectionError::PlaintextInjection),
            ),
            (
                ConnectionError::Tls("x".into()),
                TestConnectionOutcome::Security(ConnectionError::Tls("x".into())),
            ),
            (
                ConnectionError::Protocol("x".into()),
                TestConnectionOutcome::Protocol(ConnectionError::Protocol("x".into())),
            ),
        ];
        for (error, expected) in cases {
            let (uc, _, _) = use_case(Err(error), Err(ConnectionError::Closed));
            assert_eq!(
                uc.execute(params(Protocol::Imap, Security::Tls)).await,
                expected
            );
        }
    }

    #[tokio::test]
    async fn plaintext_without_consent_is_refused_before_any_connection() {
        let (uc, imap, _) = use_case(
            Ok(ServerCapabilities::default()),
            Err(ConnectionError::Closed),
        );
        let outcome = uc.execute(params(Protocol::Imap, Security::Plain)).await;
        assert!(
            matches!(
                outcome,
                TestConnectionOutcome::Security(ConnectionError::PlaintextRefused(_))
            ),
            "{outcome:?}"
        );
        assert!(
            imap.requests().is_empty(),
            "connexion tentée malgré le refus"
        );
    }

    #[tokio::test]
    async fn plaintext_with_consent_is_attempted() {
        let (uc, imap, _) = use_case(
            Ok(ServerCapabilities::default()),
            Err(ConnectionError::Closed),
        );
        let mut p = params(Protocol::Imap, Security::Plain);
        p.plaintext = PlaintextPolicy::deny_all().with_consent(PlaintextConsent {
            host: host("mail.example.com"),
            port: 993,
        });
        assert!(matches!(
            uc.execute(p).await,
            TestConnectionOutcome::Success { .. }
        ));
        assert_eq!(imap.requests().len(), 1);
    }

    #[tokio::test]
    async fn stored_exceptions_for_the_host_are_passed_to_the_adapter()
    -> Result<(), Box<dyn std::error::Error>> {
        let (uc, imap, _) = use_case(
            Ok(ServerCapabilities::default()),
            Err(ConnectionError::Closed),
        );
        let mine = CertificateException {
            host: host("mail.example.com"),
            fingerprint: CertificateFingerprint::from_bytes([7; 32]),
        };
        let other = CertificateException {
            host: host("other.example.com"),
            fingerprint: CertificateFingerprint::from_bytes([8; 32]),
        };
        uc.exceptions.add(&mine).await?;
        uc.exceptions.add(&other).await?;
        uc.execute(params(Protocol::Imap, Security::Tls)).await;
        let requests = imap.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].certificate_exceptions, vec![mine]);
        Ok(())
    }

    /// Stockage des exceptions illisible.
    struct BrokenExceptions;

    impl CertificateExceptionStore for BrokenExceptions {
        async fn exceptions_for(&self, _: &Host) -> Result<Vec<CertificateException>, StoreError> {
            Err(StoreError("base verrouillée".into()))
        }

        async fn add(&self, _: &CertificateException) -> Result<(), StoreError> {
            Err(StoreError("base verrouillée".into()))
        }

        async fn remove(&self, _: &CertificateException) -> Result<(), StoreError> {
            Err(StoreError("base verrouillée".into()))
        }
    }

    #[tokio::test]
    async fn certificate_exception_is_looked_up_per_host() -> Result<(), Box<dyn std::error::Error>>
    {
        let (uc, _, _) = use_case(Err(ConnectionError::Closed), Err(ConnectionError::Closed));
        assert!(
            !uc.has_certificate_exception(&host("mail.example.com"))
                .await
        );
        uc.exceptions
            .add(&CertificateException {
                host: host("other.example.com"),
                fingerprint: CertificateFingerprint::from_bytes([8; 32]),
            })
            .await?;
        assert!(
            !uc.has_certificate_exception(&host("mail.example.com"))
                .await
        );
        assert!(
            uc.has_certificate_exception(&host("other.example.com"))
                .await
        );
        Ok(())
    }

    #[tokio::test]
    async fn unreadable_exception_store_counts_as_an_exception() {
        let connector = FakeConnector::new(Err(ConnectionError::Closed));
        let uc = TestConnection::new(connector.clone(), connector, BrokenExceptions);
        assert!(uc.has_certificate_exception(&host("imap.gmail.com")).await);
    }

    #[tokio::test(start_paused = true)]
    async fn transient_failure_is_retried_on_a_new_connection() {
        let caps = ServerCapabilities::from_imap(["IMAP4rev1"]);
        for protocol in [Protocol::Imap, Protocol::Smtp] {
            let flaky = FakeConnector::new(Ok(caps.clone())).first(vec![
                Err(ConnectionError::Timeout(TimeoutStage::TlsHandshake)),
                Err(ConnectionError::Closed),
            ]);
            let uc = TestConnection::new(
                flaky.clone(),
                flaky.clone(),
                InMemoryCertificateExceptionStore::default(),
            );
            assert_eq!(
                uc.execute(params(protocol, Security::Tls)).await,
                TestConnectionOutcome::Success {
                    capabilities: caps.clone()
                },
                "{protocol:?}"
            );
            assert_eq!(flaky.requests().len(), 3, "{protocol:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn persistent_transient_failure_gives_up_after_the_last_attempt() {
        let timeout = ConnectionError::Timeout(TimeoutStage::Response);
        let (uc, imap, _) = use_case(Err(timeout.clone()), Err(ConnectionError::Closed));
        assert_eq!(
            uc.execute(params(Protocol::Imap, Security::Tls)).await,
            TestConnectionOutcome::Network(timeout)
        );
        assert_eq!(
            imap.requests().len(),
            norelly_domain::connection::CONNECTION_ATTEMPTS
        );
    }

    #[tokio::test(start_paused = true)]
    async fn refusal_is_not_retried() {
        let refused = ConnectionError::from(AuthError::Rejected {
            message: "mauvais mot de passe".into(),
        });
        let (uc, imap, _) = use_case(Err(refused), Err(ConnectionError::Closed));
        assert!(matches!(
            uc.execute(params(Protocol::Imap, Security::Tls)).await,
            TestConnectionOutcome::Authentication(_)
        ));
        assert_eq!(imap.requests().len(), 1);
    }
}
