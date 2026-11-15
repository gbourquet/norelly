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
//!    `AUTHENTICATE LOGIN` ; `AUTHENTICATE XOAUTH2` seulement pour un jeton
//!    OAuth), puis capacités relues.
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
use norelly_domain::connection::{
    ConnectionError, ConnectionPhase, OpeningPhase, PASSWORD_AUTH_TIMEOUT, TimeoutStage,
    authentication_timeout,
};
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

/// Durée totale de l'ouverture d'une connexion (TCP, TLS, salutation,
/// authentification, capacités).
const OPEN_TOTAL: Duration = Duration::from_secs(120);

/// Délais d'attente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// Établissement TCP.
    pub connect: Duration,
    /// Chaque lecture ou écriture pendant l'ouverture : salutation,
    /// STARTTLS, authentification et capacités relues ensuite. Un serveur
    /// sain y répond en quelques secondes ; un serveur muet à ce stade
    /// (même juste après l'authentification) répond en général à une
    /// nouvelle connexion.
    pub open: Duration,
    /// Plancher du délai de l'authentification par mot de passe
    /// (`authentication_timeout`).
    pub password_auth: Duration,
    /// Durée totale de l'ouverture : un serveur qui envoie un octet de
    /// temps en temps ne la prolonge pas.
    pub open_total: Duration,
    /// Chaque lecture ou écriture une fois la session ouverte (grosses
    /// boîtes, messages volumineux).
    pub response: Duration,
}

impl Timeouts {
    /// Test de connexion, qui réessaie aussitôt sur une nouvelle
    /// connexion : un serveur muet est abandonné tôt. Jusqu'à la fin du
    /// chiffrement, un essai tient dans le plafond du test (`CONNECTION_TEST_BUDGET`).
    pub const CONNECTION_TEST: Self = Self {
        connect: Duration::from_secs(15),
        open: Duration::from_secs(15),
        password_auth: PASSWORD_AUTH_TIMEOUT,
        open_total: OPEN_TOTAL,
        response: Duration::from_secs(15),
    };
}

impl Default for Timeouts {
    /// Synchronisation : l'acteur du compte réessaie lui-même, avec une
    /// attente croissante ; l'ouverture garde de la marge pour un serveur
    /// lent à authentifier.
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

/// Ouvre des sessions IMAP authentifiées.
#[derive(Debug, Clone)]
pub struct ImapConnector {
    tls: TlsClientFactory,
    timeouts: Timeouts,
    /// Capacités ignorées après authentification (voir
    /// [`ImapConnector::hiding_capabilities`]).
    hidden: Vec<String>,
    /// Tests : adresse jointe à la place de l'hôte du point de connexion
    /// (faux serveur local se présentant sous le nom d'un fournisseur).
    #[cfg(test)]
    test_address: Option<std::net::SocketAddr>,
}

impl ImapConnector {
    #[must_use]
    pub fn new(tls: TlsClientFactory) -> Self {
        Self {
            tls,
            timeouts: Timeouts::default(),
            hidden: Vec::new(),
            #[cfg(test)]
            test_address: None,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn connecting_to(mut self, address: std::net::SocketAddr) -> Self {
        self.test_address = Some(address);
        self
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

    async fn tcp(&self, endpoint: &ServerEndpoint) -> Result<TcpStream, ConnectionError> {
        let connect = async {
            #[cfg(test)]
            if let Some(address) = self.test_address {
                return TcpStream::connect(address).await;
            }
            TcpStream::connect((endpoint.host().as_str(), endpoint.port())).await
        };
        tokio::time::timeout(self.timeouts.connect, connect)
            .await
            .map_err(|_| ConnectionError::Timeout(TimeoutStage::Connect))?
            .map_err(|e| ConnectionError::Network(e.to_string()))
    }

    /// Connexion chiffrée (ou en clair consenti), salutation lue, avant
    /// authentification. Renvoie aussi les capacités valables sur ce canal
    /// et `true` si le serveur a pré-authentifié la session. `opening` suit
    /// la phase de l'ouverture (voir `connect`).
    async fn open(
        &self,
        request: &ConnectRequest,
        opening: &OpeningPhase,
    ) -> Result<(Connection, ServerCapabilities, bool), ConnectionError> {
        let endpoint = &request.endpoint;
        let exceptions = &request.certificate_exceptions;
        // Délai d'ouverture jusqu'aux capacités relues après
        // l'authentification (voir `connect`).
        let timeout = self.timeouts.open;
        let tcp = self.tcp(endpoint).await?;

        if endpoint.security() == Security::Tls {
            opening.enter(ConnectionPhase::BeforeEncryption);
            let tls = self.tls.connect(tcp, endpoint.host(), exceptions).await?;
            opening.enter(ConnectionPhase::BeforeAuthentication);
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

        // Échange en clair : avant le chiffrement si STARTTLS est exigé ;
        // borné en entier par le délai d'ouverture (un serveur qui distille
        // ses réponses ne le prolonge pas).
        let phase = ConnectionPhase::plaintext_opening(endpoint.security());
        opening.enter(phase);
        let conn = Connection::new(
            Transport::Plain(tcp),
            true,
            timeout,
            PRE_TLS_MAX_RESPONSE_SIZE,
        );
        let exchange = Self::plaintext_exchange(conn, endpoint);
        let outcome = tokio::time::timeout(timeout, exchange)
            .await
            .map_err(|_| phase.classify(ConnectionError::Timeout(TimeoutStage::Response)))?
            .map_err(|e| phase.classify(e))?;
        match outcome {
            Plaintext::PreAuth(conn, caps) => Ok((conn, caps, true)),
            Plaintext::Upgrade(tcp) => {
                opening.enter(ConnectionPhase::BeforeEncryption);
                let tls = self.tls.connect(tcp, endpoint.host(), exceptions).await?;
                opening.enter(ConnectionPhase::BeforeAuthentication);
                let mut conn = Connection::new(
                    Transport::Tls(Box::new(tls)),
                    false,
                    timeout,
                    MAX_RESPONSE_SIZE,
                );
                let caps = conn.capability().await?;
                Ok((conn, caps, false))
            }
            Plaintext::StayPlain(conn, caps) => {
                opening.enter(ConnectionPhase::BeforeAuthentication);
                Ok((conn, caps, false))
            }
        }
    }

    /// Salutation, capacités et `STARTTLS` en clair.
    async fn plaintext_exchange(
        mut conn: Connection,
        endpoint: &ServerEndpoint,
    ) -> Result<Plaintext, ConnectionError> {
        let greeting = conn.read_greeting().await?;
        if greeting.kind == GreetingKind::PreAuth {
            // Session déjà authentifiée en clair : STARTTLS n'est plus
            // possible (RFC 3501, 6.2.1). Refus sauf clair consenti.
            if endpoint.security() == Security::StartTls {
                return Err(ConnectionError::StartTlsUnavailable);
            }
            let caps = conn.capability().await?;
            return Ok(Plaintext::PreAuth(conn, caps));
        }
        // Capacités en clair : uniquement pour décider de STARTTLS.
        let plain_caps = match greeting_capabilities(&greeting) {
            Some(caps) => caps,
            None => conn.capability().await?,
        };
        match decide_starttls(endpoint.security(), plain_caps.starttls())? {
            StartTlsDecision::Upgrade => {
                tracing::debug!(endpoint = %endpoint, "passage en TLS par STARTTLS");
                Ok(Plaintext::Upgrade(conn.starttls().await?))
            }
            StartTlsDecision::StayPlain | StartTlsDecision::AlreadyEncrypted => {
                // Le canal garde la limite de réponse d'avant TLS (64 Kio)
                // jusqu'à l'authentification ; elle est relevée ensuite
                // (voir `connect`).
                tracing::warn!(endpoint = %endpoint, "connexion IMAP en clair (consentie)");
                Ok(Plaintext::StayPlain(conn, plain_caps))
            }
        }
    }
}

/// Issue de l'échange en clair.
enum Plaintext {
    /// Session pré-authentifiée en clair (consentie), capacités.
    PreAuth(Connection, ServerCapabilities),
    /// `STARTTLS` accepté : flux TCP prêt pour la poignée de main.
    Upgrade(TcpStream),
    /// Clair consenti, capacités.
    StayPlain(Connection, ServerCapabilities),
}

impl RemoteMailboxConnector for ImapConnector {
    type Session = ImapSession;

    async fn connect(&self, request: &ConnectRequest) -> Result<ImapSession, ConnectionError> {
        request.plaintext.authorize(&request.endpoint)?;
        request.authorize_credentials()?;
        tracing::debug!(endpoint = %request.endpoint, "connexion IMAP");
        // Durée totale atteinte : erreur de la phase en cours (pendant
        // l'authentification, celle-ci est interrompue).
        let opening = OpeningPhase::default();
        tokio::time::timeout(
            self.timeouts.open_total,
            self.open_session(request, &opening),
        )
        .await
        .map_err(|_| opening.deadline_reached())?
    }
}

impl ImapConnector {
    /// Ouverture complète : connexion, authentification, capacités.
    async fn open_session(
        &self,
        request: &ConnectRequest,
        opening: &OpeningPhase,
    ) -> Result<ImapSession, ConnectionError> {
        let (mut conn, caps, preauth) = self.open(request, opening).await?;
        if !preauth {
            let credentials = &request.credentials;
            conn.set_timeout(authentication_timeout(
                credentials.kind(),
                self.timeouts.open,
                self.timeouts.password_auth,
            ));
            // Coupure, `BYE` ou silence une fois les identifiants envoyés :
            // authentification interrompue, jamais rejouée.
            let during_auth = |e| ConnectionPhase::Authentication.classify(e);
            let method = select_imap_auth_method(&caps, credentials.kind())?;
            opening.enter(ConnectionPhase::Authentication);
            match method {
                AuthMethod::ImapLogin => {
                    tracing::debug!(user = %credentials.username(), "authentification par LOGIN");
                    conn.login(credentials).await.map_err(during_auth)?;
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
                    .await
                    .map_err(during_auth)?;
                }
            }
            opening.enter(ConnectionPhase::BeforeAuthentication);
            conn.set_timeout(self.timeouts.open);
        }
        conn.mark_authenticated();
        let capabilities = conn.capability().await?.without(&self.hidden);
        // Session ouverte : le délai de réponse laisse le temps aux
        // commandes longues (grosses boîtes).
        conn.set_timeout(self.timeouts.response);
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
            selected: false,
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
    /// Une boîte est sélectionnée par Norelly (à quitter avant de la
    /// renommer ou de la supprimer).
    selected: bool,
}

impl ImapSession {
    /// Quitte la boîte sélectionnée : certains serveurs (Dovecot) coupent
    /// la session si elle est renommée ou supprimée. `UNSELECT` si
    /// annoncé ; sinon `EXAMINE INBOX`, jamais `CLOSE` (qui effacerait les
    /// messages marqués supprimés).
    async fn leave_selected(&mut self) -> Result<(), ConnectionError> {
        if !self.selected {
            return Ok(());
        }
        if self.capabilities.has("UNSELECT") {
            self.conn.unselect().await?;
        } else {
            self.conn
                .examine(&MailboxName::inbox(), &SelectRequest::default())
                .await?;
        }
        self.selected = false;
        self.selected_condstore = false;
        Ok(())
    }
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
        // Un SELECT refusé laisse la session sans boîte sélectionnée.
        self.selected = false;
        let selected = self.conn.examine(mailbox, &request).await?;
        self.selected = true;
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
        self.selected = false;
        let selected = self.conn.select_rw(mailbox, condstore).await?;
        self.selected = true;
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

    async fn rename_mailbox(
        &mut self,
        from: &MailboxName,
        to: &MailboxName,
    ) -> Result<(), ConnectionError> {
        self.leave_selected().await?;
        self.conn.rename(from, to).await
    }

    async fn delete_mailbox(&mut self, mailbox: &MailboxName) -> Result<(), ConnectionError> {
        self.leave_selected().await?;
        self.conn.delete(mailbox).await
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
