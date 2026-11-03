//! Faux adaptateurs pour les tests des cas d'usage : connexions IMAP/SMTP,
//! autoconfig, résolveur SRV simulé, sondeur de ports.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use norelly_domain::actions::{FlagChange, StoreOutcome};
use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::connection::ConnectionError;
use norelly_domain::discovery::{
    AutoconfigRequest, DiscoveredServers, ServerCandidate, SrvRecord, SrvService,
};
use norelly_domain::endpoint::{Host, Protocol, ServerEndpoint};
use norelly_domain::fetch_policy::BodyFetch;
use norelly_domain::mailbox::{Flags, ListedMailbox, MailboxName, MessageMeta, ModSeq, Uid};
use norelly_domain::ports::{
    AutoconfigClient, ConnectRequest, DiscoveryError, FetchedBody, IdleOutcome, MailSender,
    OutgoingMessage, PortProber, RemoteMailboxConnector, RemoteMailboxSession, SendError,
    SrvResolver,
};
use norelly_domain::sync::{
    FlagUpdate, SelectRequest, SelectedMailbox, ServerQuery, SyncExtensions,
};

use crate::fake_imap::{FakeImap, FakeImapSession};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Faux serveurs IMAP et SMTP : résultat fixé par point de connexion
/// (succès par défaut), requêtes mémorisées.
#[derive(Debug, Clone, Default)]
pub struct FakeMailServers {
    results: Arc<Mutex<BTreeMap<String, Result<ServerCapabilities, ConnectionError>>>>,
    seen: Arc<Mutex<Vec<(Protocol, ConnectRequest)>>>,
    imap: FakeImap,
}

impl FakeMailServers {
    /// Faux serveur IMAP derrière les sessions ouvertes.
    #[must_use]
    pub fn imap(&self) -> &FakeImap {
        &self.imap
    }

    /// Faux serveurs dont les sessions IMAP s'appuient sur `imap`.
    #[must_use]
    pub fn with_imap(imap: FakeImap) -> Self {
        Self {
            imap,
            ..Self::default()
        }
    }

    /// Fixe le résultat des connexions vers `endpoint`.
    pub fn set(
        &self,
        endpoint: &ServerEndpoint,
        result: Result<ServerCapabilities, ConnectionError>,
    ) {
        lock(&self.results).insert(endpoint.to_string(), result);
    }

    /// Connexions tentées.
    #[must_use]
    pub fn requests(&self) -> Vec<(Protocol, ConnectRequest)> {
        lock(&self.seen).clone()
    }

    fn answer(
        &self,
        protocol: Protocol,
        request: &ConnectRequest,
    ) -> Result<ServerCapabilities, ConnectionError> {
        lock(&self.seen).push((protocol, request.clone()));
        // Défense en profondeur, comme les vrais adaptateurs.
        request.plaintext.authorize(&request.endpoint)?;
        lock(&self.results)
            .get(&request.endpoint.to_string())
            .cloned()
            .unwrap_or_else(|| Ok(ServerCapabilities::default()))
    }
}

/// Session IMAP factice : capacités fixées par [`FakeMailServers::set`]
/// (celles du faux serveur par défaut), opérations sur [`FakeImap`].
#[derive(Debug)]
pub struct FakeSession {
    capabilities: ServerCapabilities,
    inner: FakeImapSession,
    logout_error: Option<ConnectionError>,
}

impl FakeSession {
    /// Session sur `imap` avec des capacités imposées ; `logout_error` :
    /// erreur renvoyée par `logout`.
    pub fn new(
        capabilities: ServerCapabilities,
        imap: &FakeImap,
        logout_error: Option<ConnectionError>,
    ) -> Result<Self, ConnectionError> {
        Ok(Self {
            capabilities,
            inner: imap.session()?,
            logout_error,
        })
    }
}

impl RemoteMailboxSession for FakeSession {
    fn capabilities(&self) -> &ServerCapabilities {
        &self.capabilities
    }
    fn enable_sync_extensions(
        &mut self,
    ) -> impl Future<Output = Result<SyncExtensions, ConnectionError>> + Send {
        self.inner.enable_sync_extensions()
    }
    fn list_mailboxes(
        &mut self,
    ) -> impl Future<Output = Result<Vec<ListedMailbox>, ConnectionError>> + Send {
        self.inner.list_mailboxes()
    }
    fn select(
        &mut self,
        mailbox: &MailboxName,
        request: &SelectRequest,
    ) -> impl Future<Output = Result<SelectedMailbox, ConnectionError>> + Send {
        self.inner.select(mailbox, request)
    }
    fn fetch_flags(
        &mut self,
        query: ServerQuery,
    ) -> impl Future<Output = Result<Vec<FlagUpdate>, ConnectionError>> + Send {
        self.inner.fetch_flags(query)
    }
    fn search_all_uids(
        &mut self,
    ) -> impl Future<Output = Result<BTreeSet<Uid>, ConnectionError>> + Send {
        self.inner.search_all_uids()
    }
    fn fetch_meta(
        &mut self,
        uids: &[Uid],
    ) -> impl Future<Output = Result<Vec<MessageMeta>, ConnectionError>> + Send {
        self.inner.fetch_meta(uids)
    }
    fn fetch_body(
        &mut self,
        uid: Uid,
        fetch: &BodyFetch,
    ) -> impl Future<Output = Result<Option<FetchedBody>, ConnectionError>> + Send {
        self.inner.fetch_body(uid, fetch)
    }
    fn select_writable(
        &mut self,
        mailbox: &MailboxName,
    ) -> impl Future<Output = Result<SelectedMailbox, ConnectionError>> + Send {
        self.inner.select_writable(mailbox)
    }
    fn message_flags(
        &mut self,
        uid: Uid,
    ) -> impl Future<Output = Result<Option<Flags>, ConnectionError>> + Send {
        self.inner.message_flags(uid)
    }
    fn store_flag(
        &mut self,
        uid: Uid,
        change: FlagChange,
        unchanged_since: Option<ModSeq>,
    ) -> impl Future<Output = Result<StoreOutcome, ConnectionError>> + Send {
        self.inner.store_flag(uid, change, unchanged_since)
    }
    fn mark_deleted(
        &mut self,
        uid: Uid,
    ) -> impl Future<Output = Result<(), ConnectionError>> + Send {
        self.inner.mark_deleted(uid)
    }
    fn move_to(
        &mut self,
        uid: Uid,
        to: &MailboxName,
    ) -> impl Future<Output = Result<(), ConnectionError>> + Send {
        self.inner.move_to(uid, to)
    }
    fn copy_to(
        &mut self,
        uid: Uid,
        to: &MailboxName,
    ) -> impl Future<Output = Result<(), ConnectionError>> + Send {
        self.inner.copy_to(uid, to)
    }
    fn expunge_uid(
        &mut self,
        uid: Uid,
    ) -> impl Future<Output = Result<(), ConnectionError>> + Send {
        self.inner.expunge_uid(uid)
    }
    fn expunge_all(&mut self) -> impl Future<Output = Result<u32, ConnectionError>> + Send {
        self.inner.expunge_all()
    }
    fn deleted_uids(
        &mut self,
    ) -> impl Future<Output = Result<BTreeSet<Uid>, ConnectionError>> + Send {
        self.inner.deleted_uids()
    }
    fn idle(
        &mut self,
        max: Duration,
    ) -> impl Future<Output = Result<IdleOutcome, ConnectionError>> + Send {
        self.inner.idle(max)
    }
    async fn logout(self) -> Result<(), ConnectionError> {
        if let Some(e) = self.logout_error {
            return Err(e);
        }
        self.inner.logout().await
    }
}

impl RemoteMailboxConnector for FakeMailServers {
    type Session = FakeSession;
    async fn connect(&self, request: &ConnectRequest) -> Result<FakeSession, ConnectionError> {
        let fixed = lock(&self.results).contains_key(&request.endpoint.to_string());
        let capabilities = self.answer(Protocol::Imap, request)?;
        let inner = self.imap.session()?;
        let capabilities = if fixed {
            capabilities
        } else {
            self.imap.capabilities()
        };
        Ok(FakeSession {
            capabilities,
            inner,
            logout_error: None,
        })
    }
}

impl MailSender for FakeMailServers {
    async fn verify(
        &self,
        request: &ConnectRequest,
    ) -> Result<ServerCapabilities, ConnectionError> {
        self.answer(Protocol::Smtp, request)
    }
    async fn send(&self, request: &ConnectRequest, _: &OutgoingMessage) -> Result<(), SendError> {
        self.answer(Protocol::Smtp, request).map(|_| ())?;
        Ok(())
    }
}

/// Réponses de [`FakeAutoconfig`], par URL.
type AutoconfigResponses = BTreeMap<String, Result<Option<DiscoveredServers>, DiscoveryError>>;
/// Enregistrements de [`FakeSrvResolver`], par nom.
type SrvRecords = BTreeMap<String, Result<Vec<SrvRecord>, DiscoveryError>>;

/// Faux client autoconfig : réponses par URL (aucun fichier par défaut).
#[derive(Debug, Clone, Default)]
pub struct FakeAutoconfig {
    responses: Arc<Mutex<AutoconfigResponses>>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl FakeAutoconfig {
    pub fn set(&self, url: &str, response: Result<Option<DiscoveredServers>, DiscoveryError>) {
        lock(&self.responses).insert(url.to_owned(), response);
    }

    /// URL interrogées, dans l'ordre.
    #[must_use]
    pub fn requested(&self) -> Vec<String> {
        lock(&self.seen).clone()
    }
}

impl AutoconfigClient for FakeAutoconfig {
    async fn fetch(
        &self,
        request: &AutoconfigRequest,
    ) -> Result<Option<DiscoveredServers>, DiscoveryError> {
        lock(&self.seen).push(request.url.clone());
        lock(&self.responses)
            .get(&request.url)
            .cloned()
            .unwrap_or(Ok(None))
    }
}

/// Résolveur SRV simulé : enregistrements par nom (`_imaps._tcp.domaine`).
#[derive(Debug, Clone, Default)]
pub struct FakeSrvResolver {
    records: Arc<Mutex<SrvRecords>>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl FakeSrvResolver {
    pub fn set(
        &self,
        service: SrvService,
        domain: &str,
        records: Result<Vec<SrvRecord>, DiscoveryError>,
    ) {
        lock(&self.records).insert(format!("{}.{domain}", service.label()), records);
    }

    #[must_use]
    pub fn requested(&self) -> Vec<String> {
        lock(&self.seen).clone()
    }
}

impl SrvResolver for FakeSrvResolver {
    async fn lookup(
        &self,
        service: SrvService,
        domain: &Host,
    ) -> Result<Vec<SrvRecord>, DiscoveryError> {
        let name = format!("{}.{}", service.label(), domain.as_str());
        lock(&self.seen).push(name.clone());
        lock(&self.records)
            .get(&name)
            .cloned()
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}

/// Faux sondeur : seuls les points de connexion déclarés répondent.
#[derive(Debug, Clone, Default)]
pub struct FakeProber {
    reachable: Arc<Mutex<Vec<ServerEndpoint>>>,
    probed: Arc<Mutex<Vec<ServerCandidate>>>,
}

impl FakeProber {
    pub fn reachable(&self, endpoint: ServerEndpoint) {
        lock(&self.reachable).push(endpoint);
    }

    /// Candidats sondés.
    #[must_use]
    pub fn probed(&self) -> Vec<ServerCandidate> {
        lock(&self.probed).clone()
    }
}

impl PortProber for FakeProber {
    async fn probe(&self, candidates: Vec<ServerCandidate>) -> Vec<ServerCandidate> {
        lock(&self.probed).extend(candidates.iter().cloned());
        let reachable = lock(&self.reachable).clone();
        candidates
            .into_iter()
            .filter(|c| reachable.contains(&c.endpoint))
            .collect()
    }
}
