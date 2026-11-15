//! Cas d'usage `TestConnection` : tente une connexion authentifiée à un
//! serveur IMAP ou SMTP et renvoie un résultat structuré, prêt à être
//! présenté à l'utilisateur (sans jamais contenir le secret).

use norelly_domain::auth::AuthError;
use norelly_domain::capabilities::ServerCapabilities;
use std::time::Duration;

use norelly_domain::connection::{
    CONNECTION_TEST_BUDGET, ConnectionError, FailureCategory, TimeoutStage, next_test_attempt,
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

/// Une exception de certificat existe-t-elle pour `host` dans
/// `exceptions` ? Un stockage illisible compte comme une exception : la
/// réponse sert à refuser un conseil, jamais à accorder une confiance.
pub async fn certificate_excepted<E: CertificateExceptionStore>(
    exceptions: &E,
    host: &Host,
) -> bool {
    exceptions
        .exceptions_for(host)
        .await
        .map_or(true, |exceptions| !exceptions.is_empty())
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

    /// Exceptions de certificat consultées par ce cas d'usage.
    pub fn exceptions(&self) -> &E {
        &self.exceptions
    }

    /// Connecteurs IMAP et SMTP de ce cas d'usage (délais choisis à
    /// l'assemblage).
    pub fn connectors(&self) -> (&I, &S) {
        (&self.imap, &self.smtp)
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
        // Réseau instable ou serveur momentanément muet : de nouveaux essais,
        // chacun sur une nouvelle connexion, dans une durée totale plafonnée
        // (règle du domaine : `next_test_attempt`).
        let started = tokio::time::Instant::now();
        // Premier essai coupé au plafond : au-delà des délais de connexion
        // et de poignée de main, c'est le serveur qui ne répond plus.
        let mut result = self
            .attempt_within(params.protocol, &request, CONNECTION_TEST_BUDGET)
            .await
            .unwrap_or(Err(ConnectionError::Timeout(TimeoutStage::Response)));
        let mut made = 1;
        while let Err(error) = &result {
            let Some(next) = next_test_attempt(made, Some(error), started.elapsed()) else {
                break;
            };
            tokio::time::sleep(next.wait).await;
            made += 1;
            match self
                .attempt_within(params.protocol, &request, next.limit)
                .await
            {
                Some(attempt) => result = attempt,
                // Nouvel essai coupé au plafond : l'erreur du précédent,
                // plus précise, est gardée.
                None => break,
            }
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
        certificate_excepted(&self.exceptions, host).await
    }

    /// Un essai, abandonné au bout de `limit` (`None` : coupé, la
    /// connexion en cours est fermée avec son futur).
    async fn attempt_within(
        &self,
        protocol: Protocol,
        request: &ConnectRequest,
        limit: Duration,
    ) -> Option<Result<ServerCapabilities, ConnectionError>> {
        let deadline = tokio::time::Instant::now() + limit;
        match protocol {
            Protocol::Imap => {
                let session = match tokio::time::timeout_at(deadline, self.imap.connect(request))
                    .await
                    .ok()?
                {
                    Ok(session) => session,
                    Err(e) => return Some(Err(e)),
                };
                let capabilities = session.capabilities().clone();
                // LOGOUT sans importance : la connexion a réussi. Son échec,
                // ou un serveur muet au-delà du plafond, n'y change rien.
                let _ = tokio::time::timeout_at(deadline, session.logout()).await;
                Some(Ok(capabilities))
            }
            Protocol::Smtp => tokio::time::timeout_at(deadline, self.smtp.verify(request))
                .await
                .ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use norelly_domain::credentials::{Secret, Username};
    use norelly_domain::endpoint::{Host, Security};
    use norelly_domain::ports::{OutgoingMessage, SendError, StoreError};
    use norelly_domain::security::{
        CertificateException, CertificateFingerprint, CertificateIssue, PlaintextConsent,
    };
    use norelly_test_support::contracts::InMemoryCertificateExceptionStore;
    use norelly_test_support::fake_imap::FakeImap;
    use norelly_test_support::fakes::FakeSession;
    use std::sync::{Arc, Mutex, PoisonError};

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
        /// Durée de chaque essai (serveur lent ou muet).
        delay: Duration,
        /// Durée de LOGOUT.
        logout_delay: Duration,
    }

    impl FakeConnector {
        fn new(result: Result<ServerCapabilities, ConnectionError>) -> Self {
            Self {
                result,
                first: Outcomes::default(),
                seen: Seen::default(),
                delay: Duration::ZERO,
                logout_delay: Duration::ZERO,
            }
        }
        fn slow_logout(mut self, delay: Duration) -> Self {
            self.logout_delay = delay;
            self
        }
        fn taking(mut self, delay: Duration) -> Self {
            self.delay = delay;
            self
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
        async fn record(&self, request: &ConnectRequest) {
            self.seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request.clone());
            tokio::time::sleep(self.delay).await;
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
            self.record(request).await;
            let caps = self.next_result()?;
            Ok(
                FakeSession::new(caps, &FakeImap::new(), Some(ConnectionError::Closed))?
                    .with_logout_delay(self.logout_delay),
            )
        }
    }

    impl MailSender for FakeConnector {
        async fn verify(
            &self,
            request: &ConnectRequest,
        ) -> Result<ServerCapabilities, ConnectionError> {
            self.record(request).await;
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
            name_mismatch: false,
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

    /// Serveur muet à chaque essai (chaque essai n'échoue qu'au bout de
    /// son délai) : le test entier tient dans le plafond.
    #[tokio::test(start_paused = true)]
    async fn slow_failing_attempts_stay_within_the_budget() {
        let timeout = ConnectionError::Timeout(TimeoutStage::Response);
        let silent = FakeConnector::new(Err(timeout.clone())).taking(Duration::from_secs(30));
        let uc = TestConnection::new(
            silent.clone(),
            silent.clone(),
            InMemoryCertificateExceptionStore::default(),
        );
        let started = tokio::time::Instant::now();
        let outcome = tokio::time::timeout(
            CONNECTION_TEST_BUDGET * 4,
            uc.execute(params(Protocol::Imap, Security::Tls)),
        )
        .await;
        assert_eq!(outcome, Ok(TestConnectionOutcome::Network(timeout)));
        assert!(
            started.elapsed() <= CONNECTION_TEST_BUDGET,
            "{:?}",
            started.elapsed()
        );
        assert_eq!(
            silent.requests().len(),
            2,
            "un second essai, abandonné en cours"
        );
    }

    /// Essai abandonné au plafond après un échec précis : l'erreur précise
    /// est gardée, pas un délai dépassé inventé.
    #[tokio::test(start_paused = true)]
    async fn cut_attempt_keeps_the_previous_error() {
        let unreachable = ConnectionError::Network("hôte injoignable".into());
        let flaky = FakeConnector::new(Err(ConnectionError::Closed))
            .first(vec![Err(unreachable.clone())])
            .taking(Duration::from_secs(30));
        let uc = TestConnection::new(
            flaky.clone(),
            flaky,
            InMemoryCertificateExceptionStore::default(),
        );
        assert_eq!(
            uc.execute(params(Protocol::Imap, Security::Tls)).await,
            TestConnectionOutcome::Network(unreachable)
        );
    }

    /// Connexion réussie, serveur muet à la déconnexion : le succès compte,
    /// et le test reste dans le plafond.
    #[tokio::test(start_paused = true)]
    async fn silent_logout_does_not_spoil_a_success() {
        let caps = ServerCapabilities::from_imap(["IMAP4rev1"]);
        let imap = FakeConnector::new(Ok(caps.clone())).slow_logout(CONNECTION_TEST_BUDGET * 10);
        let uc = TestConnection::new(
            imap.clone(),
            imap.clone(),
            InMemoryCertificateExceptionStore::default(),
        );
        let started = tokio::time::Instant::now();
        assert_eq!(
            uc.execute(params(Protocol::Imap, Security::Tls)).await,
            TestConnectionOutcome::Success { capabilities: caps }
        );
        assert!(
            started.elapsed() <= CONNECTION_TEST_BUDGET,
            "{:?}",
            started.elapsed()
        );
        assert_eq!(imap.requests().len(), 1);
    }

    /// Essai qui ne finit jamais : abandonné au plafond, délai dépassé.
    #[tokio::test(start_paused = true)]
    async fn hanging_attempt_is_abandoned_at_the_budget() {
        let hanging = FakeConnector::new(Ok(ServerCapabilities::default()))
            .taking(CONNECTION_TEST_BUDGET * 10);
        let uc = TestConnection::new(
            hanging.clone(),
            hanging,
            InMemoryCertificateExceptionStore::default(),
        );
        let started = tokio::time::Instant::now();
        for protocol in [Protocol::Imap, Protocol::Smtp] {
            let outcome = tokio::time::timeout(
                CONNECTION_TEST_BUDGET * 4,
                uc.execute(params(protocol, Security::Tls)),
            )
            .await;
            assert_eq!(
                outcome,
                Ok(TestConnectionOutcome::Network(ConnectionError::Timeout(
                    TimeoutStage::Response
                ))),
                "{protocol:?}"
            );
        }
        assert!(
            started.elapsed() <= CONNECTION_TEST_BUDGET * 2,
            "{:?}",
            started.elapsed()
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

    /// Coupure après l'envoi des identifiants : un seul essai, les
    /// identifiants ne sont jamais renvoyés.
    #[tokio::test(start_paused = true)]
    async fn interrupted_authentication_is_not_replayed() {
        let interrupted = ConnectionError::from(AuthError::Interrupted);
        for protocol in [Protocol::Imap, Protocol::Smtp] {
            let (uc, imap, smtp) = use_case(Err(interrupted.clone()), Err(interrupted.clone()));
            assert_eq!(
                uc.execute(params(protocol, Security::Tls)).await,
                TestConnectionOutcome::Authentication(AuthError::Interrupted)
            );
            assert_eq!(
                imap.requests().len() + smtp.requests().len(),
                1,
                "{protocol:?}"
            );
        }
    }

    /// Coupure avant le chiffrement : aucun identifiant n'est parti, le
    /// test réessaie sur une nouvelle connexion.
    #[tokio::test(start_paused = true)]
    async fn cut_before_encryption_is_retried() {
        let caps = ServerCapabilities::from_imap(["IMAP4rev1"]);
        let flaky = FakeConnector::new(Ok(caps.clone()))
            .first(vec![Err(ConnectionError::ClosedBeforeEncryption)]);
        let uc = TestConnection::new(
            flaky.clone(),
            flaky.clone(),
            InMemoryCertificateExceptionStore::default(),
        );
        assert_eq!(
            uc.execute(params(Protocol::Imap, Security::StartTls)).await,
            TestConnectionOutcome::Success { capabilities: caps }
        );
        assert_eq!(flaky.requests().len(), 2);
    }
}
