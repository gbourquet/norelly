//! Faux adaptateurs pour les tests des cas d'usage : connexions IMAP/SMTP,
//! autoconfig, résolveur SRV simulé, sondeur de ports.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::connection::ConnectionError;
use norelly_domain::discovery::{
    AutoconfigRequest, DiscoveredServers, ServerCandidate, SrvRecord, SrvService,
};
use norelly_domain::endpoint::{Host, Protocol, ServerEndpoint};
use norelly_domain::ports::{
    AutoconfigClient, ConnectRequest, DiscoveryError, MailSender, OutgoingMessage, PortProber,
    RemoteMailboxConnector, RemoteMailboxSession, SendError, SrvResolver,
};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Faux serveurs IMAP et SMTP : résultat fixé par point de connexion
/// (succès par défaut), requêtes mémorisées.
#[derive(Debug, Clone, Default)]
pub struct FakeMailServers {
    results: Arc<Mutex<BTreeMap<String, Result<ServerCapabilities, ConnectionError>>>>,
    seen: Arc<Mutex<Vec<(Protocol, ConnectRequest)>>>,
}

impl FakeMailServers {
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

/// Session IMAP factice.
#[derive(Debug)]
pub struct FakeSession(ServerCapabilities);

impl RemoteMailboxSession for FakeSession {
    fn capabilities(&self) -> &ServerCapabilities {
        &self.0
    }
    async fn logout(self) -> Result<(), ConnectionError> {
        Ok(())
    }
}

impl RemoteMailboxConnector for FakeMailServers {
    type Session = FakeSession;
    async fn connect(&self, request: &ConnectRequest) -> Result<FakeSession, ConnectionError> {
        self.answer(Protocol::Imap, request).map(FakeSession)
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
