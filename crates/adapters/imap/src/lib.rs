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
mod nesting;
mod session;
mod utf7;

use std::collections::BTreeSet;
use std::time::Duration;

use imap_next::imap_types::flag::Flag;
use imap_next::imap_types::response::GreetingKind;
use norelly_domain::actions::{ActionFlag, FlagChange, StoreOutcome};
use norelly_domain::auth::{AuthMethod, authenticator_for, select_imap_auth_method};
use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::compose::draft::RemoteCopy;
use norelly_domain::connection::{ConnectionError, TimeoutStage};
use norelly_domain::endpoint::{Security, ServerEndpoint};
use norelly_domain::fetch_policy::BodyFetch;
use norelly_domain::mailbox::{Flags, ListedMailbox, MailboxName, MessageMeta, ModSeq, Uid};
use norelly_domain::ports::{
    ConnectRequest, FetchedBody, IdleOutcome, RemoteMailboxConnector, RemoteMailboxSession,
};
use norelly_domain::security::{StartTlsDecision, decide_starttls};
use norelly_domain::sync::{
    FlagUpdate, SelectRequest, SelectedMailbox, ServerQuery, SyncExtensions,
};
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
    /// Capacités ignorées après authentification (voir
    /// [`ImapConnector::hiding_capabilities`]).
    hidden: Vec<String>,
}

impl ImapConnector {
    #[must_use]
    pub fn new(tls: TlsClientFactory) -> Self {
        Self {
            tls,
            timeouts: Timeouts::default(),
            hidden: Vec::new(),
        }
    }

    /// Ignore des capacités annoncées par le serveur après
    /// l'authentification (par exemple `MOVE`, `UIDPLUS`) : contournement
    /// d'un serveur défaillant, et tests des stratégies de repli.
    #[must_use]
    pub fn hiding_capabilities(mut self, tokens: &[&str]) -> Self {
        self.hidden = tokens.iter().map(|t| (*t).to_owned()).collect();
        self
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
                // Le canal garde la limite de réponse d'avant TLS (64 Kio)
                // jusqu'à l'authentification ; elle est relevée ensuite
                // (voir `connect`).
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
        conn.mark_authenticated();
        let capabilities = conn.capability().await?.without(&self.hidden);
        if request.endpoint.security() == Security::Plain {
            // Canal en clair consenti, authentifié : il transporte désormais
            // des messages (limite d'avant TLS levée).
            conn.raise_response_limit();
        }
        tracing::debug!(endpoint = %request.endpoint, "session IMAP ouverte");
        Ok(ImapSession {
            conn,
            capabilities,
            extensions: SyncExtensions::default(),
            selected_condstore: false,
        })
    }
}

/// Session IMAP authentifiée.
#[derive(Debug)]
pub struct ImapSession {
    conn: Connection,
    capabilities: ServerCapabilities,
    extensions: SyncExtensions,
    /// CONDSTORE actif sur la boîte sélectionnée (MODSEQ demandés).
    selected_condstore: bool,
}

impl RemoteMailboxSession for ImapSession {
    fn capabilities(&self) -> &ServerCapabilities {
        &self.capabilities
    }

    async fn enable_sync_extensions(&mut self) -> Result<SyncExtensions, ConnectionError> {
        let condstore = self.capabilities.has("CONDSTORE") || self.capabilities.has("QRESYNC");
        let qresync = self.capabilities.has("QRESYNC");
        if !self.capabilities.has("ENABLE") {
            // Sans ENABLE, QRESYNC est inutilisable ; CONDSTORE reste
            // activable par le paramètre de SELECT.
            self.extensions = SyncExtensions {
                condstore,
                qresync: false,
            };
            return Ok(self.extensions);
        }
        self.extensions = self.conn.enable_sync(condstore, qresync).await?;
        Ok(self.extensions)
    }

    async fn list_mailboxes(&mut self) -> Result<Vec<ListedMailbox>, ConnectionError> {
        self.conn.list().await
    }

    async fn select(
        &mut self,
        mailbox: &MailboxName,
        request: &SelectRequest,
    ) -> Result<SelectedMailbox, ConnectionError> {
        // Les paramètres non activés ne sont jamais envoyés.
        let mut request = request.clone();
        if !self.extensions.qresync {
            request.qresync = None;
        }
        request.condstore &= self.extensions.condstore;
        let selected = self.conn.examine(mailbox, &request).await?;
        self.selected_condstore = request.condstore || request.qresync.is_some();
        Ok(selected)
    }

    async fn fetch_flags(
        &mut self,
        query: ServerQuery,
    ) -> Result<Vec<FlagUpdate>, ConnectionError> {
        if matches!(query, ServerQuery::ChangedFlags(_)) && !self.selected_condstore {
            return Err(ConnectionError::Protocol(
                "CHANGEDSINCE sans CONDSTORE".to_owned(),
            ));
        }
        self.conn.fetch_flags(query, self.selected_condstore).await
    }

    async fn search_all_uids(&mut self) -> Result<BTreeSet<Uid>, ConnectionError> {
        self.conn.search_all().await
    }

    async fn fetch_meta(&mut self, uids: &[Uid]) -> Result<Vec<MessageMeta>, ConnectionError> {
        self.conn.fetch_meta(uids, self.selected_condstore).await
    }

    async fn fetch_body(
        &mut self,
        uid: Uid,
        fetch: &BodyFetch,
    ) -> Result<Option<FetchedBody>, ConnectionError> {
        self.conn.fetch_body(uid, fetch).await
    }

    async fn select_writable(
        &mut self,
        mailbox: &MailboxName,
    ) -> Result<SelectedMailbox, ConnectionError> {
        let condstore = self.extensions.condstore;
        let selected = self.conn.select_rw(mailbox, condstore).await?;
        self.selected_condstore = condstore;
        Ok(selected)
    }

    async fn message_flags(&mut self, uid: Uid) -> Result<Option<Flags>, ConnectionError> {
        self.conn.flags_of(uid).await
    }

    async fn store_flag(
        &mut self,
        uid: Uid,
        change: FlagChange,
        unchanged_since: Option<ModSeq>,
    ) -> Result<StoreOutcome, ConnectionError> {
        let flag = match change.flag {
            ActionFlag::Seen => Flag::Seen,
            ActionFlag::Flagged => Flag::Flagged,
        };
        // UNCHANGEDSINCE n'a de sens qu'avec CONDSTORE actif sur la boîte.
        let unchanged_since = unchanged_since.filter(|_| self.selected_condstore);
        self.conn
            .store(uid, flag, change.value, unchanged_since)
            .await
    }

    async fn mark_deleted(&mut self, uid: Uid) -> Result<(), ConnectionError> {
        self.conn
            .store(uid, Flag::Deleted, true, None)
            .await
            .map(|_| ())
    }

    async fn move_to(&mut self, uid: Uid, to: &MailboxName) -> Result<(), ConnectionError> {
        if !self.capabilities.has("MOVE") {
            return Err(ConnectionError::Protocol("MOVE non annoncé".to_owned()));
        }
        self.conn.transfer(uid, to, false).await
    }

    async fn create_mailbox(&mut self, mailbox: &MailboxName) -> Result<(), ConnectionError> {
        self.conn.create(mailbox).await
    }

    async fn copy_to(&mut self, uid: Uid, to: &MailboxName) -> Result<(), ConnectionError> {
        self.conn.transfer(uid, to, true).await
    }

    async fn expunge_uid(&mut self, uid: Uid) -> Result<(), ConnectionError> {
        if !self.capabilities.has("UIDPLUS") {
            return Err(ConnectionError::Protocol("UIDPLUS non annoncé".to_owned()));
        }
        self.conn.expunge(Some(uid)).await.map(|_| ())
    }

    async fn append(
        &mut self,
        mailbox: &MailboxName,
        raw: &[u8],
        draft: bool,
    ) -> Result<Option<RemoteCopy>, ConnectionError> {
        self.conn.append(mailbox, raw, draft).await
    }

    async fn expunge_all(&mut self) -> Result<u32, ConnectionError> {
        self.conn.expunge(None).await
    }

    async fn deleted_uids(&mut self) -> Result<BTreeSet<Uid>, ConnectionError> {
        self.conn.search_deleted().await
    }

    async fn idle(&mut self, max: Duration) -> Result<IdleOutcome, ConnectionError> {
        let supported = self.capabilities.has("IDLE");
        self.conn.idle_or_poll(max, supported).await
    }

    async fn logout(mut self) -> Result<(), ConnectionError> {
        self.conn.logout().await
    }
}

#[cfg(test)]
mod tests;
