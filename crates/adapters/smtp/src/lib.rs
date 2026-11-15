//! Adaptateur SMTP : implémentation du port `MailSender` avec `lettre`.
//!
//! `lettre` 0.11 n'accepte pas de `rustls::ClientConfig` fourni par
//! l'appelant (les champs de `TlsParameters` sont privés) : impossible d'y
//! brancher notre vérificateur à exceptions par empreinte. On utilise donc
//! son API publique `AsyncSmtpConnection::connect_with_transport`, qui
//! accepte un flux déjà établi :
//! - **465 (TLS implicite)** : TCP puis TLS par `norelly-tls`, flux remis à
//!   `lettre` qui lit la bannière et envoie `EHLO` ;
//! - **587 (STARTTLS)** : bannière, `EHLO` et `STARTTLS` faits ici (décision
//!   du domaine, rejet des données injectées avant la poignée de main), puis
//!   TLS par `norelly-tls`. Comme le serveur n'envoie pas de nouvelle
//!   bannière après STARTTLS, le flux remis à `lettre` commence par une
//!   bannière `220` locale (jamais envoyée sur le réseau).
//!
//! `lettre` envoie l'unique `EHLO` du canal final (un second serait refusé
//! par certains serveurs, dont GMX) ; les capacités du domaine en sont
//! reprises (mécanismes `AUTH`, `8BITMIME`, `SMTPUTF8`, `STARTTLS`).
//! Il gère ensuite `AUTH` (sauf XOAUTH2, mené ici : voir `xoauth2`),
//! `MAIL`/`RCPT`/`DATA` et `QUIT`. Il
//! est compilé sans backend TLS ni journalisation.

mod stream;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use lettre::address::{Address, Envelope};
use lettre::transport::smtp::authentication::{Credentials as LettreCredentials, Mechanism};
use lettre::transport::smtp::client::AsyncSmtpConnection;
use lettre::transport::smtp::extension::{ClientId, Extension, ServerInfo};
use lettre::transport::smtp::response::Severity;
use norelly_domain::auth::{
    AuthError, AuthMechanism, XOAuth2Authenticator, select_smtp_auth_mechanism,
};
use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::connection::{
    ConnectionError, ConnectionPhase, PASSWORD_AUTH_TIMEOUT, TimeoutStage, authentication_timeout,
    server_text,
};
use norelly_domain::credentials::Credentials;
use norelly_domain::endpoint::{Security, ServerEndpoint};
use norelly_domain::ports::{
    Authenticator, ConnectRequest, MailSender, OutgoingMessage, SendError,
};
use norelly_domain::security::{StartTlsDecision, decide_starttls};
use norelly_tls::TlsClientFactory;
use tokio::net::TcpStream;
use zeroize::Zeroizing;

use crate::stream::{LettreStream, PlainSession};

/// Nom annoncé dans `EHLO` : littéral d'adresse neutre, pour ne pas
/// divulguer le nom de la machine.
fn hello() -> ClientId {
    ClientId::Ipv4(Ipv4Addr::LOCALHOST)
}

/// Capacités du domaine d'après la réponse `EHLO` analysée par `lettre`
/// (mécanismes d'authentification et extensions qu'il connaît).
fn capabilities_of(info: &ServerInfo) -> ServerCapabilities {
    let mut lines = Vec::new();
    let mechanisms: Vec<&str> = [
        (Mechanism::Plain, "PLAIN"),
        (Mechanism::Login, "LOGIN"),
        (Mechanism::Xoauth2, "XOAUTH2"),
    ]
    .into_iter()
    .filter(|(m, _)| info.supports_auth_mechanism(*m))
    .map(|(_, name)| name)
    .collect();
    if !mechanisms.is_empty() {
        lines.push(format!("AUTH {}", mechanisms.join(" ")));
    }
    for (extension, name) in [
        (Extension::EightBitMime, "8BITMIME"),
        (Extension::SmtpUtfEight, "SMTPUTF8"),
        (Extension::StartTls, "STARTTLS"),
    ] {
        if info.supports_feature(extension) {
            lines.push(name.to_owned());
        }
    }
    ServerCapabilities::from_smtp_ehlo(lines)
}

/// Bannière locale insérée devant le flux après STARTTLS (voir la
/// documentation du module).
const LOCAL_GREETING: &[u8] = b"220 norelly STARTTLS local\r\n";

/// Attente au plus de la réponse à `QUIT`.
const QUIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Durée totale de l'ouverture d'une connexion (TCP, TLS, salutation,
/// authentification, capacités).
const OPEN_TOTAL: Duration = Duration::from_secs(120);

/// Délais d'attente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    pub connect: Duration,
    /// Chaque échange pendant l'ouverture : bannière, `EHLO`, `STARTTLS`,
    /// authentification, puis `QUIT`. Un serveur sain y répond en quelques
    /// secondes.
    pub open: Duration,
    /// Plancher du délai de l'authentification par mot de passe
    /// (`authentication_timeout`).
    pub password_auth: Duration,
    /// Durée totale de l'ouverture : un serveur qui envoie un octet de
    /// temps en temps ne la prolonge pas.
    pub open_total: Duration,
    /// Transmission d'un message (augmentée selon sa taille).
    pub response: Duration,
}

impl Timeouts {
    /// Test de connexion, qui réessaie aussitôt sur une nouvelle
    /// connexion : un serveur muet est abandonné tôt.
    pub const CONNECTION_TEST: Self = Self {
        connect: Duration::from_secs(20),
        open: Duration::from_secs(15),
        password_auth: PASSWORD_AUTH_TIMEOUT,
        open_total: OPEN_TOTAL,
        response: Duration::from_secs(15),
    };
}

impl Default for Timeouts {
    /// Envoi : l'ouverture garde de la marge pour un serveur lent à
    /// authentifier.
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(20),
            open: Duration::from_secs(30),
            password_auth: PASSWORD_AUTH_TIMEOUT,
            open_total: OPEN_TOTAL,
            response: Duration::from_secs(60),
        }
    }
}

/// Envoi SMTP.
#[derive(Debug, Clone)]
pub struct SmtpSender {
    tls: TlsClientFactory,
    timeouts: Timeouts,
    /// Tests : adresse jointe à la place de l'hôte du point de connexion
    /// (faux serveur local se présentant sous le nom d'un fournisseur).
    #[cfg(test)]
    test_address: Option<SocketAddr>,
}

/// Connexion SMTP authentifiée, capacités relues sur le canal final.
struct Session {
    conn: AsyncSmtpConnection,
    capabilities: ServerCapabilities,
}

fn lettre_error(e: &lettre::transport::smtp::Error) -> ConnectionError {
    if e.is_timeout() {
        ConnectionError::Timeout(TimeoutStage::Response)
    } else if e.is_response() || e.is_permanent() || e.is_transient() {
        ConnectionError::Protocol(server_text(e.to_string().as_bytes()))
    } else if connection_cut(e) {
        ConnectionError::Closed
    } else {
        ConnectionError::Network(server_text(e.to_string().as_bytes()))
    }
}

/// Vrai si l'erreur d'entrée-sortie est une connexion coupée par le
/// serveur ou le réseau (réinitialisée, fermée) : le serveur avait été
/// joint.
pub(crate) fn is_cut(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::NotConnected
    )
}

/// Erreur d'entrée-sortie sur une connexion établie.
pub(crate) fn io_failure(e: &std::io::Error) -> ConnectionError {
    if is_cut(e) {
        ConnectionError::Closed
    } else {
        ConnectionError::Network(e.to_string())
    }
}

/// Vrai si l'erreur vient d'une connexion coupée par le serveur ou le
/// réseau (réinitialisée, fermée) : le serveur avait été joint.
fn connection_cut(e: &lettre::transport::smtp::Error) -> bool {
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            return is_cut(io);
        }
        source = cause.source();
    }
    false
}

/// Refus du serveur pendant l'authentification (code SMTP) : refus des
/// identifiants ; autre échec : erreur de connexion.
fn auth_failure(e: &lettre::transport::smtp::Error) -> ConnectionError {
    if e.is_permanent() || e.is_transient() {
        AuthError::Rejected {
            message: server_text(e.to_string().as_bytes()),
        }
        .into()
    } else {
        lettre_error(e)
    }
}

/// Ligne SASL `<préfixe><réponse en base64>\r\n`, effacée de la mémoire à
/// sa libération ; la copie que fait `lettre` pour l'envoyer ne l'est pas.
fn sasl_line(prefix: &str, response: &[u8]) -> Zeroizing<String> {
    let mut line = Zeroizing::new(String::with_capacity(
        prefix.len() + response.len().div_ceil(3) * 4 + 2,
    ));
    line.push_str(prefix);
    BASE64.encode_string(response, &mut line);
    line.push_str("\r\n");
    line
}

/// Échange SASL XOAUTH2, mené ici plutôt que par `lettre` : à un jeton
/// refusé, le serveur répond par un défi `334` portant l'erreur (JSON), et
/// attend une réponse vide avant le refus final ; `lettre` n'y répond pas.
/// Les réponses viennent de [`XOAuth2Authenticator`] ; un défi de trop est
/// annulé par `*` (RFC 4954).
async fn xoauth2(
    conn: &mut AsyncSmtpConnection,
    credentials: &Credentials,
) -> Result<(), ConnectionError> {
    let authenticator = XOAuth2Authenticator;
    let initial = authenticator.respond(credentials, 0, b"")?;
    let mut reply = conn
        .command(&*sasl_line("AUTH XOAUTH2 ", initial.expose()))
        .await;
    let mut step = 1;
    loop {
        let response = reply.map_err(|e| auth_failure(&e))?;
        if !response.has_code(334) {
            return if response.code().severity == Severity::PositiveCompletion {
                Ok(())
            } else {
                Err(ConnectionError::Protocol(server_text(
                    response.code().to_string().as_bytes(),
                )))
            };
        }
        match authenticator.respond(credentials, step, b"") {
            Ok(answer) => {
                reply = conn.command(&*sasl_line("", answer.expose())).await;
            }
            Err(e) => {
                // Réponse au `*` sans importance : l'échange a échoué.
                let _ = conn.command("*\r\n").await;
                return Err(e.into());
            }
        }
        step += 1;
    }
}

impl SmtpSender {
    #[must_use]
    pub fn new(tls: TlsClientFactory) -> Self {
        Self {
            tls,
            timeouts: Timeouts::default(),
            #[cfg(test)]
            test_address: None,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn connecting_to(mut self, address: SocketAddr) -> Self {
        self.test_address = Some(address);
        self
    }

    #[must_use]
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Délais d'attente.
    #[must_use]
    pub fn timeouts(&self) -> Timeouts {
        self.timeouts
    }

    /// Fabrique TLS utilisée.
    #[must_use]
    pub fn tls(&self) -> &TlsClientFactory {
        &self.tls
    }

    /// Échange de l'ouverture (ou `QUIT`), borné par le délai d'ouverture.
    async fn timed<T, E>(
        &self,
        future: impl Future<Output = Result<T, E>>,
        map: impl FnOnce(E) -> ConnectionError,
    ) -> Result<T, ConnectionError> {
        self.timed_within(self.timeouts.open, future, map).await
    }

    async fn timed_within<T, E>(
        &self,
        limit: Duration,
        future: impl Future<Output = Result<T, E>>,
        map: impl FnOnce(E) -> ConnectionError,
    ) -> Result<T, ConnectionError> {
        tokio::time::timeout(limit, future)
            .await
            .map_err(|_| ConnectionError::Timeout(TimeoutStage::Response))?
            .map_err(map)
    }

    async fn tcp(
        &self,
        endpoint: &ServerEndpoint,
    ) -> Result<(TcpStream, SocketAddr), ConnectionError> {
        let connect = async {
            #[cfg(test)]
            if let Some(address) = self.test_address {
                return TcpStream::connect(address).await;
            }
            TcpStream::connect((endpoint.host().as_str(), endpoint.port())).await
        };
        let tcp = tokio::time::timeout(self.timeouts.connect, connect)
            .await
            .map_err(|_| ConnectionError::Timeout(TimeoutStage::Connect))?
            .map_err(|e| ConnectionError::Network(e.to_string()))?;
        let peer = tcp
            .peer_addr()
            .map_err(|e| ConnectionError::Network(e.to_string()))?;
        Ok((tcp, peer))
    }

    /// Connexion, TLS, EHLO ; capacités relues sur le canal final.
    async fn open(&self, request: &ConnectRequest) -> Result<Session, ConnectionError> {
        request.plaintext.authorize(&request.endpoint)?;
        request.authorize_credentials()?;
        let endpoint = &request.endpoint;
        let exceptions = &request.certificate_exceptions;
        tracing::debug!(endpoint = %endpoint, "connexion SMTP");
        let (tcp, peer) = self.tcp(endpoint).await?;

        let stream = if endpoint.security() == Security::Tls {
            let tls = self.tls.connect(tcp, endpoint.host(), exceptions).await?;
            LettreStream::tls(tls, peer, Vec::new())
        } else {
            // Échange en clair : avant le chiffrement si STARTTLS est exigé.
            let plaintext = |e| ConnectionPhase::plaintext_opening(endpoint.security()).classify(e);
            let mut plain = PlainSession::new(tcp, self.timeouts.open);
            plain.greeting().await.map_err(plaintext)?;
            let caps = plain.ehlo().await.map_err(plaintext)?;
            match decide_starttls(endpoint.security(), caps.starttls())? {
                StartTlsDecision::Upgrade => {
                    tracing::debug!(endpoint = %endpoint, "passage en TLS par STARTTLS");
                    let tcp = plain.starttls().await.map_err(plaintext)?;
                    let tls = self.tls.connect(tcp, endpoint.host(), exceptions).await?;
                    LettreStream::tls(tls, peer, LOCAL_GREETING.to_vec())
                }
                StartTlsDecision::StayPlain | StartTlsDecision::AlreadyEncrypted => {
                    tracing::warn!(endpoint = %endpoint, "connexion SMTP en clair (consentie)");
                    LettreStream::plain(plain.into_tcp()?, peer, LOCAL_GREETING.to_vec())
                }
            }
        };

        // `connect_with_transport` lit la bannière et envoie lui-même
        // `EHLO` : un second `EHLO` est refusé par certains serveurs (GMX :
        // « 503 Bad sequence of commands »). Les capacités sont reprises de
        // ce que `lettre` a lu.
        let conn = self
            .timed(
                AsyncSmtpConnection::connect_with_transport(Box::new(stream), &hello()),
                |e| lettre_error(&e),
            )
            .await?;
        let capabilities = capabilities_of(conn.server_info());
        Ok(Session { conn, capabilities })
    }

    /// Ouvre et authentifie, dans la durée totale d'ouverture. Atteinte
    /// pendant l'authentification, elle l'interrompt
    /// ([`ConnectionPhase::Authentication`]).
    async fn open_authenticated(
        &self,
        request: &ConnectRequest,
    ) -> Result<Session, ConnectionError> {
        let authenticating = AtomicBool::new(false);
        tokio::time::timeout(
            self.timeouts.open_total,
            self.authenticate(request, &authenticating),
        )
        .await
        .map_err(|_| {
            let phase = if authenticating.load(Ordering::Relaxed) {
                ConnectionPhase::Authentication
            } else {
                ConnectionPhase::BeforeAuthentication
            };
            phase.classify(ConnectionError::Timeout(TimeoutStage::Response))
        })?
    }

    /// `authenticating` : vrai dès l'envoi des identifiants.
    async fn authenticate(
        &self,
        request: &ConnectRequest,
        authenticating: &AtomicBool,
    ) -> Result<Session, ConnectionError> {
        let mut session = self.open(request).await?;
        let kind = request.credentials.kind();
        let mechanism = match select_smtp_auth_mechanism(&session.capabilities, kind)? {
            AuthMechanism::Plain => Mechanism::Plain,
            AuthMechanism::Login => Mechanism::Login,
            AuthMechanism::XOAuth2 => Mechanism::Xoauth2,
            other => return Err(AuthError::Unsupported(other).into()),
        };
        let credentials = &request.credentials;
        tracing::debug!(user = %credentials.username(), mechanism = ?mechanism, "authentification SMTP");
        let limit = authentication_timeout(
            credentials.kind(),
            self.timeouts.open,
            self.timeouts.password_auth,
        );
        // Coupure ou silence une fois les identifiants envoyés :
        // authentification interrompue, jamais rejouée.
        let during_auth = |e| ConnectionPhase::Authentication.classify(e);
        authenticating.store(true, Ordering::Relaxed);
        if mechanism == Mechanism::Xoauth2 {
            self.timed_within(limit, xoauth2(&mut session.conn, credentials), |e| e)
                .await
                .map_err(during_auth)?;
            return Ok(session);
        }
        let lettre_credentials = LettreCredentials::new(
            credentials.username().as_str().to_owned(),
            credentials.secret().expose().to_owned(),
        );
        self.timed_within(
            limit,
            session.conn.auth(&[mechanism], &lettre_credentials),
            |e| auth_failure(&e),
        )
        .await
        .map_err(during_auth)?;
        Ok(session)
    }

    async fn quit(&self, mut session: Session) {
        // Échec du QUIT sans conséquence : le travail est fait.
        // Attente courte : la connexion est fermée de toute façon.
        let limit = self.timeouts.open.min(QUIT_TIMEOUT);
        let _ = self
            .timed_within(limit, session.conn.quit(), |e| lettre_error(&e))
            .await;
    }
}

impl MailSender for SmtpSender {
    async fn verify(
        &self,
        request: &ConnectRequest,
    ) -> Result<ServerCapabilities, ConnectionError> {
        let session = self.open_authenticated(request).await?;
        let capabilities = session.capabilities.clone();
        self.quit(session).await;
        tracing::debug!(endpoint = %request.endpoint, "connexion SMTP vérifiée");
        Ok(capabilities)
    }

    async fn send(
        &self,
        request: &ConnectRequest,
        message: &OutgoingMessage,
    ) -> Result<(), SendError> {
        let parse = |raw: &str| {
            raw.parse::<Address>()
                .map_err(|_| SendError::InvalidAddress(server_text(raw.as_bytes())))
        };
        let from = parse(&message.from)?;
        let recipients = message
            .recipients
            .iter()
            .map(|r| parse(r))
            .collect::<Result<Vec<_>, _>>()?;
        let envelope = Envelope::new(Some(from), recipients)
            .map_err(|e| SendError::InvalidAddress(e.to_string()))?;

        let mut session = self.open_authenticated(request).await?;
        // Délai de la transmission proportionnel à la taille (≥ 64 Kio/s) :
        // un message de 34 Mio ne doit pas expirer sur une liaison lente.
        let budget = self.timeouts.response
            + Duration::from_secs(
                u64::try_from(message.raw.len() / (64 * 1024)).unwrap_or(u64::MAX),
            );
        let sent = tokio::time::timeout(budget, session.conn.send(&envelope, &message.raw)).await;
        match sent {
            // Réponse du serveur : refus certain, rien n'est parti.
            Ok(Err(e)) if e.is_transient() => {
                Err(SendError::Deferred(server_text(e.to_string().as_bytes())))
            }
            Ok(Err(e)) if e.is_permanent() => {
                Err(SendError::Rejected(server_text(e.to_string().as_bytes())))
            }
            // Coupure ou délai dépassé sans réponse : le serveur a peut-être
            // accepté le message ; ne pas le renvoyer automatiquement.
            Err(_) => Err(SendError::Interrupted("délai dépassé".to_owned())),
            Ok(Err(e)) => Err(SendError::Interrupted(server_text(
                e.to_string().as_bytes(),
            ))),
            Ok(Ok(_)) => {
                self.quit(session).await;
                tracing::debug!(endpoint = %request.endpoint, "message envoyé");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests;
