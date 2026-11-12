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
//! Il gère ensuite `AUTH`, `MAIL`/`RCPT`/`DATA` et `QUIT`. Il
//! est compilé sans backend TLS ni journalisation.

mod stream;

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use lettre::address::{Address, Envelope};
use lettre::transport::smtp::authentication::{Credentials as LettreCredentials, Mechanism};
use lettre::transport::smtp::client::AsyncSmtpConnection;
use lettre::transport::smtp::extension::{ClientId, Extension, ServerInfo};
use norelly_domain::auth::{AuthError, AuthMechanism, select_smtp_auth_mechanism};
use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::connection::{ConnectionError, TimeoutStage, server_text};
use norelly_domain::endpoint::{Security, ServerEndpoint};
use norelly_domain::ports::{ConnectRequest, MailSender, OutgoingMessage, SendError};
use norelly_domain::security::{StartTlsDecision, decide_starttls};
use norelly_tls::TlsClientFactory;
use tokio::net::TcpStream;

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

/// Délais d'attente.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub connect: Duration,
    /// Chaque échange commande/réponse.
    pub response: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(20),
            response: Duration::from_secs(60),
        }
    }
}

/// Envoi SMTP.
#[derive(Debug, Clone)]
pub struct SmtpSender {
    tls: TlsClientFactory,
    timeouts: Timeouts,
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

/// Vrai si l'erreur vient d'une connexion coupée par le serveur ou le
/// réseau (réinitialisée, fermée) : le serveur avait été joint.
fn connection_cut(e: &lettre::transport::smtp::Error) -> bool {
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            return matches!(
                io.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::NotConnected
            );
        }
        source = cause.source();
    }
    false
}

impl SmtpSender {
    #[must_use]
    pub fn new(tls: TlsClientFactory) -> Self {
        Self {
            tls,
            timeouts: Timeouts::default(),
        }
    }

    #[must_use]
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    async fn timed<T, E>(
        &self,
        future: impl Future<Output = Result<T, E>>,
        map: impl FnOnce(E) -> ConnectionError,
    ) -> Result<T, ConnectionError> {
        tokio::time::timeout(self.timeouts.response, future)
            .await
            .map_err(|_| ConnectionError::Timeout(TimeoutStage::Response))?
            .map_err(map)
    }

    async fn tcp(
        &self,
        endpoint: &ServerEndpoint,
    ) -> Result<(TcpStream, SocketAddr), ConnectionError> {
        let address = (endpoint.host().as_str(), endpoint.port());
        let tcp = tokio::time::timeout(self.timeouts.connect, TcpStream::connect(address))
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
            let mut plain = PlainSession::new(tcp, self.timeouts.response);
            plain.greeting().await?;
            let caps = plain.ehlo().await?;
            match decide_starttls(endpoint.security(), caps.starttls())? {
                StartTlsDecision::Upgrade => {
                    tracing::debug!(endpoint = %endpoint, "passage en TLS par STARTTLS");
                    let tcp = plain.starttls().await?;
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

    /// Ouvre et authentifie.
    async fn open_authenticated(
        &self,
        request: &ConnectRequest,
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
        let lettre_credentials = LettreCredentials::new(
            credentials.username().as_str().to_owned(),
            credentials.secret().expose().to_owned(),
        );
        self.timed(session.conn.auth(&[mechanism], &lettre_credentials), |e| {
            if e.is_permanent() || e.is_transient() {
                AuthError::Rejected {
                    message: server_text(e.to_string().as_bytes()),
                }
                .into()
            } else {
                lettre_error(&e)
            }
        })
        .await?;
        Ok(session)
    }

    async fn quit(&self, mut session: Session) {
        // Échec du QUIT sans conséquence : le travail est fait.
        let _ = self.timed(session.conn.quit(), |e| lettre_error(&e)).await;
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
