//! Adaptateur IMAP : implémentation du port `RemoteMailboxConnector` avec
//! `imap-next` (machine d'états sans E/S) et notre pilote d'E/S.
//!
//! Séquence de connexion :
//! 1. clair refusé sans consentement ([`PlaintextPolicy`], vérifié à nouveau
//!    ici) ;
//! 2. TCP, puis TLS implicite, ou salutation en clair suivie de STARTTLS
//!    (obligatoire s'il est annoncé, erreur s'il manque en mode STARTTLS) ;
//! 3. capacités relues sur le canal chiffré ;
//! 4. authentification (`AUTHENTICATE PLAIN`, `LOGIN` sauf `LOGINDISABLED`,
//!    `AUTHENTICATE LOGIN`), puis capacités relues.
//!
//! [`PlaintextPolicy`]: norelly_domain::security::PlaintextPolicy

mod connection;

use std::time::Duration;

use imap_next::imap_types::response::GreetingKind;
use norelly_domain::auth::{AuthMethod, authenticator_for, select_imap_auth_method};
use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::connection::{ConnectionError, TimeoutStage};
use norelly_domain::endpoint::{Security, ServerEndpoint};
use norelly_domain::ports::{ConnectRequest, RemoteMailboxConnector, RemoteMailboxSession};
use norelly_domain::security::{StartTlsDecision, decide_starttls};
use norelly_tls::TlsClientFactory;
use tokio::net::TcpStream;

use crate::connection::{
    Connection, MAX_RESPONSE_SIZE, PRE_TLS_MAX_RESPONSE_SIZE, Transport, greeting_capabilities,
};

/// Délais d'attente.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// Établissement TCP.
    pub connect: Duration,
    /// Chaque lecture ou écriture.
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

/// Ouvre des sessions IMAP authentifiées.
#[derive(Debug, Clone)]
pub struct ImapConnector {
    tls: TlsClientFactory,
    timeouts: Timeouts,
}

impl ImapConnector {
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

    async fn tcp(&self, endpoint: &ServerEndpoint) -> Result<TcpStream, ConnectionError> {
        let address = (endpoint.host().as_str(), endpoint.port());
        tokio::time::timeout(self.timeouts.connect, TcpStream::connect(address))
            .await
            .map_err(|_| ConnectionError::Timeout(TimeoutStage::Connect))?
            .map_err(|e| ConnectionError::Network(e.to_string()))
    }

    /// Connexion chiffrée (ou en clair consenti), salutation lue, avant
    /// authentification. Renvoie aussi les capacités valables sur ce canal
    /// et `true` si le serveur a pré-authentifié la session.
    async fn open(
        &self,
        request: &ConnectRequest,
    ) -> Result<(Connection, ServerCapabilities, bool), ConnectionError> {
        let endpoint = &request.endpoint;
        let exceptions = &request.certificate_exceptions;
        let timeout = self.timeouts.response;
        let tcp = self.tcp(endpoint).await?;

        if endpoint.security() == Security::Tls {
            let tls = self.tls.connect(tcp, endpoint.host(), exceptions).await?;
            let mut conn = Connection::new(
                Transport::Tls(Box::new(tls)),
                true,
                timeout,
                MAX_RESPONSE_SIZE,
            );
            let greeting = conn.read_greeting().await?;
            let preauth = greeting.kind == GreetingKind::PreAuth;
            let caps = match greeting_capabilities(&greeting) {
                Some(caps) => caps,
                None => conn.capability().await?,
            };
            return Ok((conn, caps, preauth));
        }

        let mut conn = Connection::new(
            Transport::Plain(tcp),
            true,
            timeout,
            PRE_TLS_MAX_RESPONSE_SIZE,
        );
        let greeting = conn.read_greeting().await?;
        if greeting.kind == GreetingKind::PreAuth {
            // Session déjà authentifiée en clair : STARTTLS n'est plus
            // possible (RFC 3501, 6.2.1). Refus sauf clair consenti.
            if endpoint.security() == Security::StartTls {
                return Err(ConnectionError::StartTlsUnavailable);
            }
            let caps = conn.capability().await?;
            return Ok((conn, caps, true));
        }
        // Capacités en clair : uniquement pour décider de STARTTLS.
        let plain_caps = match greeting_capabilities(&greeting) {
            Some(caps) => caps,
            None => conn.capability().await?,
        };
        match decide_starttls(endpoint.security(), plain_caps.starttls())? {
            StartTlsDecision::Upgrade => {
                tracing::debug!(endpoint = %endpoint, "passage en TLS par STARTTLS");
                let tcp = conn.starttls().await?;
                let tls = self.tls.connect(tcp, endpoint.host(), exceptions).await?;
                let mut conn = Connection::new(
                    Transport::Tls(Box::new(tls)),
                    false,
                    timeout,
                    MAX_RESPONSE_SIZE,
                );
                let caps = conn.capability().await?;
                Ok((conn, caps, false))
            }
            StartTlsDecision::StayPlain | StartTlsDecision::AlreadyEncrypted => {
                // Le canal garde la limite de réponse d'avant TLS (64 Kio) :
                // suffisant pour l'authentification ; à relever quand les
                // sessions en clair consenti transporteront des messages
                // (phase 4).
                tracing::warn!(endpoint = %endpoint, "connexion IMAP en clair (consentie)");
                Ok((conn, plain_caps, false))
            }
        }
    }
}

impl RemoteMailboxConnector for ImapConnector {
    type Session = ImapSession;

    async fn connect(&self, request: &ConnectRequest) -> Result<ImapSession, ConnectionError> {
        request.plaintext.authorize(&request.endpoint)?;
        tracing::debug!(endpoint = %request.endpoint, "connexion IMAP");
        let (mut conn, caps, preauth) = self.open(request).await?;
        if !preauth {
            let credentials = &request.credentials;
            match select_imap_auth_method(&caps)? {
                AuthMethod::ImapLogin => {
                    tracing::debug!(user = %credentials.username(), "authentification par LOGIN");
                    conn.login(credentials).await?;
                }
                AuthMethod::Sasl(mechanism) => {
                    tracing::debug!(
                        user = %credentials.username(),
                        mechanism = %mechanism,
                        "authentification SASL"
                    );
                    let authenticator = authenticator_for(&mechanism)?;
                    conn.authenticate(
                        authenticator.as_ref(),
                        credentials,
                        caps.sasl_initial_response(),
                    )
                    .await?;
                }
            }
        }
        let capabilities = conn.capability().await?;
        tracing::debug!(endpoint = %request.endpoint, "session IMAP ouverte");
        Ok(ImapSession { conn, capabilities })
    }
}

/// Session IMAP authentifiée.
#[derive(Debug)]
pub struct ImapSession {
    conn: Connection,
    capabilities: ServerCapabilities,
}

impl RemoteMailboxSession for ImapSession {
    fn capabilities(&self) -> &ServerCapabilities {
        &self.capabilities
    }

    async fn logout(mut self) -> Result<(), ConnectionError> {
        self.conn.logout().await
    }
}

#[cfg(test)]
mod tests;
