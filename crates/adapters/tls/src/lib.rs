//! Adaptateur TLS : configuration rustls et exceptions de certificats par
//! empreinte.
//!
//! Le vérificateur [`ExceptionAwareVerifier`] délègue d'abord à un
//! vérificateur standard (plateforme, ou racines fournies pour les tests),
//! puis confie la décision à la politique du domaine
//! ([`TlsTrustPolicy`]) : un certificat invalide n'est accepté que par une
//! exception portant sur le même hôte **et** la même empreinte SHA-256.
//! Aucun vérificateur « accepte tout » n'existe dans ce crate.
//!
//! Le nom d'hôte est toujours vérifié par le vérificateur standard ; une
//! exception mémorisée couvre l'éventuel défaut constaté pour ce certificat
//! précis. Les signatures de la poignée de main sont toujours vérifiées.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use norelly_domain::connection::{ConnectionError, TimeoutStage};
use norelly_domain::endpoint::Host;
use norelly_domain::security::{
    CertificateException, CertificateFingerprint, CertificateIssue, CertificateRejection,
    TlsTrustPolicy, TrustDecision,
};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

pub use rustls;

/// Délai par défaut de la poignée de main TLS.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Empreinte SHA-256 d'un certificat DER.
#[must_use]
pub fn fingerprint(cert: &CertificateDer<'_>) -> CertificateFingerprint {
    let digest = ring::digest::digest(&ring::digest::SHA256, cert.as_ref());
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(digest.as_ref());
    CertificateFingerprint::from_bytes(bytes)
}

/// Traduit une erreur de vérification rustls en problème du domaine.
#[must_use]
pub fn certificate_issue(error: &rustls::Error) -> CertificateIssue {
    let rustls::Error::InvalidCertificate(error) = error else {
        return CertificateIssue::Other(error.to_string());
    };
    match error {
        CertificateError::UnknownIssuer => CertificateIssue::UnknownIssuer,
        CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
            CertificateIssue::Expired
        }
        CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
            CertificateIssue::NotYetValid
        }
        CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
            CertificateIssue::HostnameMismatch
        }
        CertificateError::Revoked => CertificateIssue::Revoked,
        CertificateError::BadSignature
        | CertificateError::UnsupportedSignatureAlgorithmContext { .. }
        | CertificateError::UnsupportedSignatureAlgorithmForPublicKeyContext { .. } => {
            CertificateIssue::InvalidSignature
        }
        CertificateError::BadEncoding => CertificateIssue::Malformed,
        other => CertificateIssue::Other(format!("{other:?}")),
    }
}

/// Racines de confiance.
#[derive(Debug, Clone)]
pub enum TrustRoots {
    /// Magasin du système (Windows, Linux) via `rustls-platform-verifier`.
    Platform,
    /// Racines explicites (tests : CA de test).
    Custom(Vec<CertificateDer<'static>>),
}

/// Erreur de construction d'une configuration TLS.
#[derive(Debug, thiserror::Error)]
#[error("configuration TLS impossible : {0}")]
pub struct TlsSetupError(String);

/// Dernier rejet constaté par le vérificateur, relu après la poignée de
/// main pour produire une erreur structurée.
type RejectionSlot = Arc<Mutex<Option<CertificateRejection>>>;

/// Vérificateur qui combine la vérification standard et les exceptions
/// mémorisées (hôte + empreinte), selon [`TlsTrustPolicy`].
#[derive(Debug)]
pub struct ExceptionAwareVerifier {
    inner: Arc<dyn ServerCertVerifier>,
    host: Host,
    exceptions: Vec<CertificateException>,
    rejection: RejectionSlot,
}

impl ServerCertVerifier for ExceptionAwareVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let platform = self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        );
        let verdict = match &platform {
            Ok(_) => Ok(()),
            Err(e) => Err(certificate_issue(e)),
        };
        let fp = fingerprint(end_entity);
        match TlsTrustPolicy::evaluate(&self.host, &fp, verdict, &self.exceptions) {
            TrustDecision::Trusted => platform,
            TrustDecision::TrustedByException => {
                tracing::info!(host = %self.host, fingerprint = %fp, "certificat accepté par exception");
                Ok(ServerCertVerified::assertion())
            }
            TrustDecision::Rejected(rejection) => {
                tracing::warn!(
                    host = %self.host,
                    fingerprint = %fp,
                    issue = %rejection.issue,
                    changed = rejection.fingerprint_changed,
                    "certificat refusé"
                );
                *self
                    .rejection
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(rejection);
                match platform {
                    Err(e) => Err(e),
                    // Impossible : un certificat valide n'est jamais rejeté.
                    Ok(_) => Err(rustls::Error::InvalidCertificate(
                        CertificateError::ApplicationVerificationFailure,
                    )),
                }
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// Fabrique de connexions TLS clientes.
#[derive(Debug, Clone)]
pub struct TlsClientFactory {
    roots: TrustRoots,
    provider: Arc<CryptoProvider>,
    handshake_timeout: Duration,
}

impl TlsClientFactory {
    /// Racines du système.
    #[must_use]
    pub fn platform() -> Self {
        Self::new(TrustRoots::Platform)
    }

    #[must_use]
    pub fn new(roots: TrustRoots) -> Self {
        Self {
            roots,
            provider: Arc::new(rustls::crypto::ring::default_provider()),
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
        }
    }

    #[must_use]
    pub fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    fn standard_verifier(&self) -> Result<Arc<dyn ServerCertVerifier>, TlsSetupError> {
        match &self.roots {
            TrustRoots::Platform => {
                let verifier = rustls_platform_verifier::Verifier::new(Arc::clone(&self.provider))
                    .map_err(|e| TlsSetupError(e.to_string()))?;
                Ok(Arc::new(verifier))
            }
            TrustRoots::Custom(roots) => {
                let mut store = RootCertStore::empty();
                for root in roots {
                    store
                        .add(root.clone())
                        .map_err(|e| TlsSetupError(e.to_string()))?;
                }
                let verifier = WebPkiServerVerifier::builder_with_provider(
                    Arc::new(store),
                    Arc::clone(&self.provider),
                )
                .build()
                .map_err(|e| TlsSetupError(e.to_string()))?;
                Ok(verifier)
            }
        }
    }

    /// Configuration cliente standard (TLS 1.2 et 1.3, vérification de la
    /// plateforme, **aucune exception**), pour les clients HTTPS
    /// (autodécouverte).
    pub fn standard_client_config(&self) -> Result<ClientConfig, TlsSetupError> {
        Ok(
            ClientConfig::builder_with_provider(Arc::clone(&self.provider))
                .with_safe_default_protocol_versions()
                .map_err(|e| TlsSetupError(e.to_string()))?
                // API rustls imposée pour fournir le vérificateur de la
                // plateforme ; aucune exception n'est ajoutée.
                .dangerous()
                .with_custom_certificate_verifier(self.standard_verifier()?)
                .with_no_client_auth(),
        )
    }

    /// Configuration cliente (TLS 1.2 et 1.3) pour `host`, avec ses
    /// exceptions. Renvoie aussi l'emplacement où le vérificateur dépose un
    /// éventuel rejet.
    fn client_config(
        &self,
        host: &Host,
        exceptions: &[CertificateException],
    ) -> Result<(Arc<ClientConfig>, RejectionSlot), TlsSetupError> {
        let rejection = RejectionSlot::default();
        let verifier = ExceptionAwareVerifier {
            inner: self.standard_verifier()?,
            host: host.clone(),
            exceptions: exceptions
                .iter()
                .filter(|e| &e.host == host)
                .cloned()
                .collect(),
            rejection: Arc::clone(&rejection),
        };
        let config = ClientConfig::builder_with_provider(Arc::clone(&self.provider))
            .with_safe_default_protocol_versions()
            .map_err(|e| TlsSetupError(e.to_string()))?
            // Pas un contournement : `dangerous()` est la seule API rustls
            // pour fournir un vérificateur personnalisé ; le nôtre délègue
            // au vérificateur standard et n'accepte un défaut que par
            // exception explicite (voir ExceptionAwareVerifier).
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        Ok((Arc::new(config), rejection))
    }

    /// Négocie TLS sur `stream` pour `host`. Les erreurs de certificat sont
    /// renvoyées sous forme structurée ([`ConnectionError::Certificate`]).
    pub async fn connect<S>(
        &self,
        stream: S,
        host: &Host,
        exceptions: &[CertificateException],
    ) -> Result<TlsStream<S>, ConnectionError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let (config, rejection) = self
            .client_config(host, exceptions)
            .map_err(|e| ConnectionError::Tls(e.to_string()))?;
        let server_name = ServerName::try_from(host.as_str().to_owned())
            .map_err(|e| ConnectionError::Tls(e.to_string()))?;
        let handshake = TlsConnector::from(config).connect(server_name, stream);
        let result = tokio::time::timeout(self.handshake_timeout, handshake)
            .await
            .map_err(|_| ConnectionError::Timeout(TimeoutStage::TlsHandshake))?;
        result.map_err(|e| {
            if let Some(rejection) = rejection
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            {
                return ConnectionError::Certificate(rejection);
            }
            handshake_error(&e)
        })
    }
}

/// Erreur de poignée de main hors rejet de certificat.
fn handshake_error(error: &std::io::Error) -> ConnectionError {
    let tls = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>());
    match tls {
        Some(rustls::Error::PeerIncompatible(_)) => ConnectionError::Tls(
            "le serveur ne propose pas de version TLS ou d'algorithme compatible (TLS 1.2 minimum)"
                .to_owned(),
        ),
        Some(e) => ConnectionError::Tls(e.to_string()),
        None => match error.kind() {
            std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::BrokenPipe => ConnectionError::Closed,
            _ => ConnectionError::Network(error.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use norelly_test_support::certs::{TestCa, TestCert};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn host(name: &str) -> Host {
        Host::parse(name).unwrap_or_else(|_| panic!("hôte de test invalide"))
    }

    /// Serveur TLS d'un seul échange : renvoie « ok » après la poignée de
    /// main.
    async fn serve_once(cert: &TestCert) -> Result<u16, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let acceptor = tokio_rustls::TlsAcceptor::from(cert.server_config()?);
        tokio::spawn(async move {
            if let Ok((tcp, _)) = listener.accept().await
                && let Ok(mut tls) = acceptor.accept(tcp).await
            {
                let _ = tls.write_all(b"ok").await;
                let _ = tls.shutdown().await;
            }
        });
        Ok(port)
    }

    async fn attempt(
        factory: &TlsClientFactory,
        cert: &TestCert,
        name: &str,
        exceptions: &[CertificateException],
    ) -> Result<Result<(), ConnectionError>, Box<dyn std::error::Error>> {
        let port = serve_once(cert).await?;
        let tcp = TcpStream::connect(("127.0.0.1", port)).await?;
        Ok(match factory.connect(tcp, &host(name), exceptions).await {
            Ok(mut tls) => {
                let mut buf = [0u8; 2];
                tls.read_exact(&mut buf).await?;
                assert_eq!(&buf, b"ok");
                Ok(())
            }
            Err(e) => Err(e),
        })
    }

    fn rejection(result: Result<(), ConnectionError>) -> CertificateRejection {
        match result {
            Err(ConnectionError::Certificate(r)) => r,
            other => panic!("rejet de certificat attendu, obtenu {other:?}"),
        }
    }

    fn setup() -> Result<(TestCa, TlsClientFactory), Box<dyn std::error::Error>> {
        let ca = TestCa::new()?;
        let factory = TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()]));
        Ok((ca, factory))
    }

    #[tokio::test]
    async fn valid_certificate_is_accepted() -> TestResult {
        let (ca, factory) = setup()?;
        let cert = ca.issue(&["localhost"])?;
        attempt(&factory, &cert, "localhost", &[]).await??;
        Ok(())
    }

    #[tokio::test]
    async fn self_signed_is_rejected_with_fingerprint() -> TestResult {
        let (_, factory) = setup()?;
        let cert = TestCert::self_signed(&["localhost"])?;
        let r = rejection(attempt(&factory, &cert, "localhost", &[]).await?);
        assert_eq!(r.issue, CertificateIssue::UnknownIssuer);
        assert_eq!(r.fingerprint, fingerprint(cert.leaf()));
        assert_eq!(r.host, host("localhost"));
        assert!(!r.fingerprint_changed);
        Ok(())
    }

    #[tokio::test]
    async fn expired_is_rejected() -> TestResult {
        let (ca, factory) = setup()?;
        let cert = ca.issue_expired(&["localhost"])?;
        let r = rejection(attempt(&factory, &cert, "localhost", &[]).await?);
        assert_eq!(r.issue, CertificateIssue::Expired);
        Ok(())
    }

    #[tokio::test]
    async fn wrong_host_is_rejected() -> TestResult {
        let (ca, factory) = setup()?;
        let cert = ca.issue(&["wrong.example"])?;
        let r = rejection(attempt(&factory, &cert, "localhost", &[]).await?);
        assert_eq!(r.issue, CertificateIssue::HostnameMismatch);
        Ok(())
    }

    #[tokio::test]
    async fn exception_accepts_same_fingerprint_and_host_only() -> TestResult {
        let (_, factory) = setup()?;
        let cert = TestCert::self_signed(&["localhost"])?;
        let accepted = CertificateException {
            host: host("localhost"),
            fingerprint: fingerprint(cert.leaf()),
        };
        // Même hôte, même empreinte : accepté.
        attempt(
            &factory,
            &cert,
            "localhost",
            std::slice::from_ref(&accepted),
        )
        .await??;

        // Même empreinte mais exception pour un autre hôte : refusé.
        let other_host = CertificateException {
            host: host("127.0.0.1"),
            ..accepted.clone()
        };
        let r = rejection(attempt(&factory, &cert, "localhost", &[other_host]).await?);
        assert!(!r.fingerprint_changed);

        // Le certificat change (nouvelle clé) : l'ancienne exception ne
        // couvre pas la nouvelle empreinte.
        let renewed = TestCert::self_signed(&["localhost"])?;
        let r = rejection(attempt(&factory, &renewed, "localhost", &[accepted]).await?);
        assert!(r.fingerprint_changed, "changement d'empreinte non signalé");
        assert_eq!(r.fingerprint, fingerprint(renewed.leaf()));
        Ok(())
    }

    #[tokio::test]
    async fn exception_covers_expired_and_wrong_host_for_that_certificate() -> TestResult {
        let (ca, factory) = setup()?;
        for cert in [
            ca.issue_expired(&["localhost"])?,
            ca.issue(&["wrong.example"])?,
        ] {
            let exception = CertificateException {
                host: host("localhost"),
                fingerprint: fingerprint(cert.leaf()),
            };
            attempt(&factory, &cert, "localhost", &[exception]).await??;
        }
        Ok(())
    }

    #[tokio::test]
    async fn plaintext_peer_gives_tls_error_not_certificate() -> TestResult {
        let (_, factory) = setup()?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        tokio::spawn(async move {
            if let Ok((mut tcp, _)) = listener.accept().await {
                let _ = tcp.write_all(b"* OK pas de TLS ici\r\n").await;
            }
        });
        let tcp = TcpStream::connect(("127.0.0.1", port)).await?;
        let err = factory.connect(tcp, &host("localhost"), &[]).await;
        assert!(
            matches!(err, Err(ConnectionError::Tls(_) | ConnectionError::Closed)),
            "{err:?}"
        );
        Ok(())
    }

    #[test]
    fn platform_verifier_can_be_built() {
        assert!(TlsClientFactory::platform().standard_verifier().is_ok());
    }
}
