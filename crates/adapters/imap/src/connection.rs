//! Pilote d'E/S de la machine d'états `imap-next` (sans E/S) : lecture
//! ligne par ligne, délais, tailles bornées.
//!
//! On ne fournit à `imap-next` qu'une ligne à la fois : les octets reçus
//! au-delà restent dans notre tampon, ce qui permet de détecter des données
//! envoyées par le serveur après la réponse à `STARTTLS` (injection en clair
//! avant la poignée de main TLS, classe CVE-2011-0411).

use std::borrow::Cow;
use std::fmt;
use std::time::Duration;

use imap_next::client::{Client, Event, Options};
use imap_next::imap_types::auth::{AuthMechanism as ImapMechanism, AuthenticateData};
use imap_next::imap_types::command::{Command, CommandBody};
use imap_next::imap_types::core::Tag;
use imap_next::imap_types::response::{
    Code, CommandContinuationRequest, Data, Greeting, GreetingKind, Status, StatusBody, StatusKind,
};
use imap_next::imap_types::secret::Secret as ImapSecret;
use imap_next::{Interrupt, Io, State};
use norelly_domain::auth::{AuthError, AuthMechanism, MAX_SASL_STEPS};
use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::connection::{ConnectionError, TimeoutStage, server_text};
use norelly_domain::credentials::Credentials;
use norelly_domain::ports::Authenticator;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use zeroize::Zeroize;

use crate::nesting::{MAX_NESTING, Nesting};

/// Taille maximale d'une réponse (littéraux compris).
pub(crate) const MAX_RESPONSE_SIZE: u32 = 64 * 1024 * 1024;
/// Taille maximale d'une réponse avant TLS (salutation, CAPABILITY,
/// STARTTLS) : rien de légitime n'y dépasse quelques Kio.
pub(crate) const PRE_TLS_MAX_RESPONSE_SIZE: u32 = 64 * 1024;
/// Octets reçus au plus pour une seule commande (toutes réponses
/// comprises) : borne la mémoire retenue jusqu'à la réponse étiquetée.
pub(crate) const MAX_COMMAND_BYTES: usize = 256 * 1024 * 1024;
/// Durée maximale d'une commande ordinaire (en plus du délai par lecture :
/// un serveur qui envoie un octet de temps en temps ne la prolonge pas).
pub(crate) const COMMAND_DEADLINE: Duration = Duration::from_secs(120);
/// Durée maximale d'une commande de synchronisation (liste complète,
/// message volumineux).
pub(crate) const BULK_DEADLINE: Duration = Duration::from_secs(30 * 60);
/// Nombre maximal de réponses non étiquetées conservées pour une commande
/// ordinaire.
const MAX_UNTAGGED_PER_COMMAND: usize = 10_000;
/// Nombre maximal de réponses de données pour une commande de
/// synchronisation (`FETCH 1:*`, `SEARCH`, `LIST`) : une boîte de
/// plusieurs centaines de milliers de messages reste possible.
pub(crate) const MAX_SYNC_DATA: usize = 2_000_000;

/// Flux sous-jacent.
#[derive(Debug)]
pub(crate) enum Transport {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
    /// Après extraction du flux TCP (STARTTLS) ou fermeture.
    Detached,
}

impl Transport {
    async fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.read(buf).await,
            Self::Tls(s) => s.read(buf).await,
            Self::Detached => Ok(0),
        }
    }

    async fn write_all_flush(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        async fn go<S: AsyncRead + AsyncWrite + Unpin>(
            s: &mut S,
            bytes: &[u8],
        ) -> std::io::Result<()> {
            s.write_all(bytes).await?;
            s.flush().await
        }
        match self {
            Self::Plain(s) => go(s, bytes).await,
            Self::Tls(s) => go(s.as_mut(), bytes).await,
            Self::Detached => Err(std::io::ErrorKind::NotConnected.into()),
        }
    }
}

/// Réponse complète à une commande.
pub(crate) struct Reply {
    pub data: Vec<Data<'static>>,
    /// Réponses d'état non étiquetées (`* OK [UIDVALIDITY …]`…).
    pub untagged: Vec<StatusBody<'static>>,
    pub status: StatusBody<'static>,
    /// Réponses écartées pendant la commande (indécodables, trop
    /// imbriquées).
    pub skipped: usize,
}

/// Connexion IMAP pilotée.
pub(crate) struct Connection {
    transport: Transport,
    client: Client,
    /// Octets reçus, pas encore fournis à `client`.
    inbuf: Vec<u8>,
    /// Préfixe de `inbuf` déjà examiné sans y trouver de `\n`.
    scanned: usize,
    /// Taille maximale d'une réponse sur ce canal.
    max_response_size: u32,
    timeout: Duration,
    next_tag: u32,
    /// IDLE en cours : vrai entre l'acceptation et la réponse étiquetée.
    /// Reste vrai si l'attente a été abandonnée (futur annulé) : la
    /// session est alors inutilisable et n'est plus que fermée.
    in_idle: bool,
    /// Commande en cours : reste vrai si son futur a été abandonné ; la
    /// session est alors désynchronisée et n'est plus que fermée.
    busy: bool,
    /// Réponse étiquetée reçue avant l'envoi complet de la commande (`NO`
    /// à la place de la continuation d'un littéral) : `imap-next` 0.3.4
    /// garde alors le littéral en attente et bloquerait les commandes
    /// suivantes. La session n'est plus que fermée.
    literal_pending: bool,
    /// Session authentifiée : une réponse indécodable est alors écartée
    /// (et comptée) au lieu d'interrompre la commande.
    authenticated: bool,
    /// Réponses écartées (indécodables ou trop imbriquées) pendant la
    /// commande en cours.
    skipped: usize,
    /// Octets reçus pendant la commande en cours.
    command_bytes: usize,
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("transport", &self.transport)
            .field("buffered", &self.inbuf.len())
            .finish_non_exhaustive()
    }
}

fn protocol(what: &str) -> ConnectionError {
    ConnectionError::Protocol(what.to_owned())
}

fn io_error(e: &std::io::Error) -> ConnectionError {
    match e.kind() {
        std::io::ErrorKind::UnexpectedEof
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::NotConnected => ConnectionError::Closed,
        _ => ConnectionError::Network(e.to_string()),
    }
}

/// Capacités IMAP d'une liste `imap-types`.
fn capabilities_of<'a, T: fmt::Display + 'a>(
    caps: impl IntoIterator<Item = &'a T>,
) -> ServerCapabilities {
    ServerCapabilities::from_imap(caps.into_iter().map(ToString::to_string))
}

impl Connection {
    /// `expect_greeting` : faux après STARTTLS (pas de nouvelle salutation).
    /// `max_response_size` : [`PRE_TLS_MAX_RESPONSE_SIZE`] sur un canal en
    /// clair, [`MAX_RESPONSE_SIZE`] sinon.
    pub(crate) fn new(
        transport: Transport,
        expect_greeting: bool,
        timeout: Duration,
        max_response_size: u32,
    ) -> Self {
        let mut options = Options::default();
        options.max_response_size = max_response_size;
        options.discard_greeting = !expect_greeting;
        Self {
            transport,
            client: Client::new(options),
            inbuf: Vec::new(),
            scanned: 0,
            max_response_size,
            timeout,
            next_tag: 0,
            in_idle: false,
            busy: false,
            literal_pending: false,
            authenticated: false,
            skipped: 0,
            command_bytes: 0,
        }
    }

    /// Session authentifiée : les réponses indécodables seront écartées.
    pub(crate) fn mark_authenticated(&mut self) {
        self.authenticated = true;
    }

    /// Exécute une commande dont la réponse peut compter beaucoup de
    /// données (jusqu'à [`MAX_SYNC_DATA`]).
    pub(crate) async fn execute_bulk(
        &mut self,
        body: CommandBody<'static>,
    ) -> Result<Reply, ConnectionError> {
        self.execute_with(body, false, MAX_SYNC_DATA, BULK_DEADLINE)
            .await
    }

    fn ensure_usable(&self) -> Result<(), ConnectionError> {
        if self.in_idle || self.busy || self.literal_pending {
            return Err(protocol("session interrompue pendant une commande"));
        }
        Ok(())
    }

    /// `IDLE` jusqu'à une donnée non sollicitée signalant un changement
    /// (`EXISTS`, `EXPUNGE`, `FETCH`, `VANISHED`) ou jusqu'à `max`, puis
    /// `DONE`. Renvoie `true` si un changement a été signalé.
    pub(crate) async fn idle(&mut self, max: Duration) -> Result<bool, ConnectionError> {
        self.ensure_usable()?;
        self.command_bytes = 0;
        let tag = self.tag()?;
        let command = Command::new(tag.clone(), CommandBody::Idle)
            .map_err(|_| protocol("commande invalide"))?;
        self.client.enqueue_command(command);
        // Acceptation (continuation `+`).
        loop {
            match self.next_event().await? {
                Event::IdleCommandSent { .. } | Event::DataReceived { .. } => {}
                Event::StatusReceived {
                    status: Status::Untagged(_),
                } => {}
                Event::IdleAccepted { .. } => {
                    self.in_idle = true;
                    break;
                }
                Event::IdleRejected { status, .. } => {
                    return Err(ConnectionError::Protocol(format!(
                        "IDLE refusé : {}",
                        status_text(&status)
                    )));
                }
                Event::StatusReceived {
                    status: Status::Bye(_),
                } => return Err(ConnectionError::Closed),
                other => {
                    return Err(ConnectionError::Protocol(format!(
                        "événement inattendu avant IDLE : {}",
                        event_name(&other)
                    )));
                }
            }
        }
        // Attente : le délai de lecture est la durée restante (passé en
        // paramètre, jamais stocké : l'abandon du futur ne le laisse pas en
        // place pour les commandes suivantes).
        let deadline = tokio::time::Instant::now() + max;
        let mut changed = false;
        while !changed {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let event = self.next_event_within(remaining).await;
            match event {
                Err(ConnectionError::Timeout(TimeoutStage::Response)) => break,
                Err(e) => return Err(e),
                Ok(Event::DataReceived { data }) => {
                    changed = matches!(
                        data,
                        Data::Exists(_)
                            | Data::Expunge(_)
                            | Data::Fetch { .. }
                            | Data::Vanished { .. }
                    );
                }
                Ok(Event::StatusReceived {
                    status: Status::Bye(_),
                }) => return Err(ConnectionError::Closed),
                Ok(Event::StatusReceived { .. }) => {}
                Ok(other) => {
                    return Err(ConnectionError::Protocol(format!(
                        "événement inattendu pendant IDLE : {}",
                        event_name(&other)
                    )));
                }
            }
        }
        if self.client.set_idle_done().is_none() {
            return Err(protocol("sortie d'IDLE impossible"));
        }
        loop {
            match self.next_event().await? {
                Event::IdleDoneSent { .. } | Event::DataReceived { .. } => {}
                Event::StatusReceived { status } => match status {
                    Status::Tagged(tagged) if tagged.tag == tag => {
                        self.in_idle = false;
                        ensure_ok(&tagged.body)?;
                        return Ok(changed);
                    }
                    Status::Tagged(_) => return Err(protocol("réponse étiquetée inattendue")),
                    Status::Untagged(_) => {}
                    Status::Bye(_) => return Err(ConnectionError::Closed),
                },
                other => {
                    return Err(ConnectionError::Protocol(format!(
                        "événement inattendu après IDLE : {}",
                        event_name(&other)
                    )));
                }
            }
        }
    }

    /// Relève la taille maximale des réponses (canal en clair consenti,
    /// une fois authentifié : il transporte alors des messages).
    pub(crate) fn raise_response_limit(&mut self) {
        self.max_response_size = MAX_RESPONSE_SIZE;
        let mut options = Options::default();
        options.max_response_size = MAX_RESPONSE_SIZE;
        options.discard_greeting = true;
        // La machine d'états n'a pas d'accesseur pour ses options : elle
        // est recréée entre deux commandes (aucun état en cours).
        self.client = Client::new(options);
    }

    /// Extrait le flux TCP en clair (avant STARTTLS). Refuse s'il reste des
    /// octets reçus non traités : le serveur (ou un attaquant) a envoyé des
    /// données qui seraient sinon interprétées après la poignée de main.
    pub(crate) fn into_plain_tcp(mut self) -> Result<TcpStream, ConnectionError> {
        if !self.inbuf.is_empty() {
            return Err(ConnectionError::PlaintextInjection);
        }
        match std::mem::replace(&mut self.transport, Transport::Detached) {
            Transport::Plain(tcp) => Ok(tcp),
            _ => Err(protocol("STARTTLS sur une connexion déjà chiffrée")),
        }
    }

    /// Avance la machine d'états jusqu'au prochain événement.
    async fn next_event(&mut self) -> Result<Event, ConnectionError> {
        self.next_event_within(self.timeout).await
    }

    /// Comme [`Self::next_event`], avec un délai de lecture donné.
    async fn next_event_within(&mut self, timeout: Duration) -> Result<Event, ConnectionError> {
        loop {
            match self.client.next() {
                Ok(event) => return Ok(event),
                Err(Interrupt::Error(e)) => {
                    use imap_next::client::Error as E;
                    let what = match e {
                        E::MalformedMessage { .. } => "réponse malformée",
                        E::ExpectedCrlfGotLf { .. } => "fin de ligne invalide (LF sans CR)",
                        E::ResponseTooLong { .. } => "réponse trop longue",
                    };
                    // `imap-next` a écarté la réponse fautive ; une fois
                    // authentifié, on continue (un message indécodable ne
                    // doit pas bloquer la synchronisation de tous les
                    // autres).
                    if !self.authenticated {
                        return Err(protocol(what));
                    }
                    tracing::warn!(raison = what, "réponse IMAP écartée");
                    self.skipped += 1;
                }
                Err(Interrupt::Io(Io::Output(mut bytes))) => {
                    let result =
                        tokio::time::timeout(timeout, self.transport.write_all_flush(&bytes)).await;
                    // Les octets peuvent contenir le mot de passe.
                    bytes.zeroize();
                    result
                        .map_err(|_| ConnectionError::Timeout(TimeoutStage::Write))?
                        .map_err(|e| io_error(&e))?;
                }
                Err(Interrupt::Io(Io::NeedMoreInput)) => self.feed_response(timeout).await?,
            }
        }
    }

    /// Lit la prochaine ligne (jusqu'à `\n` compris) dans le tampon, en
    /// lisant le réseau si nécessaire ; au plus `max` octets.
    async fn read_line(
        &mut self,
        timeout: Duration,
        max: usize,
    ) -> Result<Vec<u8>, ConnectionError> {
        loop {
            // Reprend après la partie déjà examinée : coût linéaire même
            // pour une ligne énorme reçue par petits blocs.
            let unseen = self.inbuf.get(self.scanned..).unwrap_or_default();
            if let Some(offset) = unseen.iter().position(|&b| b == b'\n') {
                let pos = self.scanned + offset;
                self.scanned = 0;
                if pos >= max {
                    return Err(protocol("réponse trop longue"));
                }
                return Ok(self.inbuf.drain(..=pos).collect());
            }
            self.scanned = self.inbuf.len();
            if self.inbuf.len() > max {
                return Err(protocol("ligne trop longue"));
            }
            self.read_more(timeout).await?;
        }
    }

    /// Lit exactement `n` octets (contenu d'un littéral).
    async fn read_exact(
        &mut self,
        timeout: Duration,
        n: usize,
    ) -> Result<Vec<u8>, ConnectionError> {
        while self.inbuf.len() < n {
            self.read_more(timeout).await?;
        }
        self.scanned = 0;
        Ok(self.inbuf.drain(..n).collect())
    }

    async fn read_more(&mut self, timeout: Duration) -> Result<(), ConnectionError> {
        let mut chunk = [0u8; 16 * 1024];
        let n = tokio::time::timeout(timeout, self.transport.read_some(&mut chunk))
            .await
            .map_err(|_| ConnectionError::Timeout(TimeoutStage::Response))?
            .map_err(|e| io_error(&e))?;
        if n == 0 {
            return Err(ConnectionError::Closed);
        }
        self.inbuf.extend_from_slice(&chunk[..n]);
        Ok(())
    }

    /// Fournit à `client` la prochaine réponse complète (ligne, littéraux
    /// et suites de ligne), après contrôle de sa taille, du budget de la
    /// commande et de sa profondeur d'imbrication ([`nesting`]). Une
    /// réponse trop imbriquée n'est jamais décodée : elle est écartée une
    /// fois authentifié, refusée avant.
    ///
    /// Les octets reçus au-delà de la réponse restent dans notre tampon,
    /// ce qui permet de détecter des données envoyées après la réponse à
    /// `STARTTLS`.
    ///
    /// [`nesting`]: crate::nesting
    async fn feed_response(&mut self, timeout: Duration) -> Result<(), ConnectionError> {
        let max = self.max_response_size as usize;
        let mut response = Vec::new();
        let mut nesting = Nesting::default();
        let mut with_literal = false;
        loop {
            let line = self.read_line(timeout, max - response.len()).await?;
            let literal = nesting.scan_line(&line);
            response.extend_from_slice(&line);
            let Some(size) = literal else {
                break;
            };
            with_literal = true;
            let size = usize::try_from(size).unwrap_or(usize::MAX);
            if size > max - response.len() {
                return Err(protocol("réponse trop longue"));
            }
            let bytes = self.read_exact(timeout, size).await?;
            response.extend_from_slice(&bytes);
        }
        if !with_literal {
            ascii_status_text(&mut response);
        }
        self.command_bytes = self.command_bytes.saturating_add(response.len());
        if self.command_bytes > MAX_COMMAND_BYTES {
            return Err(protocol("réponse trop volumineuse pour une seule commande"));
        }
        if nesting.max_depth() > MAX_NESTING {
            if !self.authenticated {
                return Err(protocol("réponse trop imbriquée"));
            }
            tracing::warn!(
                profondeur = nesting.max_depth(),
                "réponse IMAP trop imbriquée écartée"
            );
            self.skipped += 1;
            response.zeroize();
            return Ok(());
        }
        self.client.enqueue_input(&response);
        response.zeroize();
        Ok(())
    }

    /// Lit la salutation du serveur.
    pub(crate) async fn read_greeting(&mut self) -> Result<Greeting<'static>, ConnectionError> {
        let Event::GreetingReceived { greeting } = self.next_event().await? else {
            return Err(protocol("salutation attendue"));
        };
        if greeting.kind == GreetingKind::Bye {
            return Err(ConnectionError::Protocol(format!(
                "le serveur refuse la connexion : {}",
                server_text(greeting.text.as_ref().as_bytes())
            )));
        }
        Ok(greeting)
    }

    fn tag(&mut self) -> Result<Tag<'static>, ConnectionError> {
        self.next_tag += 1;
        Tag::try_from(format!("N{}", self.next_tag)).map_err(|_| protocol("étiquette invalide"))
    }

    /// Exécute une commande et attend sa réponse étiquetée. `allow_bye` :
    /// `BYE` attendu (LOGOUT).
    pub(crate) async fn execute(
        &mut self,
        body: CommandBody<'static>,
        allow_bye: bool,
    ) -> Result<Reply, ConnectionError> {
        self.execute_with(body, allow_bye, MAX_UNTAGGED_PER_COMMAND, COMMAND_DEADLINE)
            .await
    }

    /// Exécute une commande : au plus `max_data` réponses de données, et
    /// réponse étiquetée attendue avant `budget`. Une commande abandonnée
    /// (futur annulé) rend la session inutilisable.
    pub(crate) async fn execute_with(
        &mut self,
        body: CommandBody<'static>,
        allow_bye: bool,
        max_data: usize,
        budget: Duration,
    ) -> Result<Reply, ConnectionError> {
        self.ensure_usable()?;
        self.busy = true;
        self.command_bytes = 0;
        self.skipped = 0;
        let result = self.run_command(body, allow_bye, max_data, budget).await;
        self.busy = false;
        result
    }

    async fn run_command(
        &mut self,
        body: CommandBody<'static>,
        allow_bye: bool,
        max_data: usize,
        budget: Duration,
    ) -> Result<Reply, ConnectionError> {
        let deadline = tokio::time::Instant::now() + budget;
        let tag = self.tag()?;
        let command = Command::new(tag.clone(), body).map_err(|_| protocol("commande invalide"))?;
        self.client.enqueue_command(command);
        let mut data = Vec::new();
        let mut untagged = Vec::new();
        let mut sent = false;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(ConnectionError::Timeout(TimeoutStage::Response));
            }
            match self.next_event_within(self.timeout.min(remaining)).await? {
                Event::CommandSent { .. } => sent = true,
                Event::DataReceived { data: d } => {
                    if data.len() >= max_data {
                        return Err(protocol("trop de réponses non étiquetées"));
                    }
                    data.push(d);
                }
                Event::StatusReceived { status } => match status {
                    Status::Tagged(tagged) if tagged.tag == tag => {
                        if !sent {
                            tracing::warn!(
                                "réponse avant l'envoi complet de la commande : session fermée"
                            );
                            self.literal_pending = true;
                        }
                        return Ok(Reply {
                            data,
                            untagged,
                            status: tagged.body,
                            skipped: self.skipped,
                        });
                    }
                    Status::Tagged(_) => return Err(protocol("réponse étiquetée inattendue")),
                    Status::Untagged(body) => {
                        if untagged.len() >= MAX_UNTAGGED_PER_COMMAND {
                            return Err(protocol("trop de réponses non étiquetées"));
                        }
                        untagged.push(body);
                    }
                    Status::Bye(_) if allow_bye => {}
                    Status::Bye(_) => return Err(ConnectionError::Closed),
                },
                // Refus avant l'envoi d'un littéral (APPEND) : un `NO`
                // laisse la session intacte, comme après la commande.
                Event::CommandRejected { status, .. } => {
                    if let Status::Tagged(tagged) = &status {
                        ensure_ok(&tagged.body)?;
                    }
                    return Err(ConnectionError::Protocol(format!(
                        "commande rejetée : {}",
                        status_text(&status)
                    )));
                }
                _ => return Err(protocol("événement inattendu")),
            }
        }
    }

    /// `CAPABILITY`.
    pub(crate) async fn capability(&mut self) -> Result<ServerCapabilities, ConnectionError> {
        let reply = self.execute(CommandBody::Capability, false).await?;
        ensure_ok(&reply.status)?;
        let mut tokens = Vec::new();
        for data in &reply.data {
            if let Data::Capability(caps) = data {
                tokens.extend(caps.as_ref().iter().map(ToString::to_string));
            }
        }
        if tokens.is_empty() {
            return Err(protocol("réponse CAPABILITY vide"));
        }
        Ok(ServerCapabilities::from_imap(tokens))
    }

    /// `STARTTLS` ; renvoie le flux TCP prêt pour la poignée de main.
    pub(crate) async fn starttls(mut self) -> Result<TcpStream, ConnectionError> {
        let reply = self.execute(CommandBody::StartTLS, false).await?;
        if reply.status.kind != StatusKind::Ok {
            return Err(ConnectionError::StartTlsRejected(server_text(
                reply.status.text.as_ref().as_bytes(),
            )));
        }
        self.into_plain_tcp()
    }

    /// Commande `LOGIN`.
    pub(crate) async fn login(&mut self, credentials: &Credentials) -> Result<(), ConnectionError> {
        let body = CommandBody::login(
            credentials.username().as_str().to_owned(),
            credentials.secret().expose().to_owned(),
        )
        .map_err(|_| protocol("identifiants non représentables en IMAP"))?;
        let reply = self.execute(body, false).await?;
        auth_status(&reply.status)
    }

    /// `AUTHENTICATE` (SASL).
    pub(crate) async fn authenticate(
        &mut self,
        authenticator: &dyn Authenticator,
        credentials: &Credentials,
        sasl_ir: bool,
    ) -> Result<(), ConnectionError> {
        let mechanism = match authenticator.mechanism() {
            AuthMechanism::Plain => ImapMechanism::Plain,
            AuthMechanism::Login => ImapMechanism::Login,
            AuthMechanism::XOAuth2 => ImapMechanism::XOAuth2,
            other => return Err(AuthError::Unsupported(other).into()),
        };
        let mut step = 0;
        let initial_response = if sasl_ir && authenticator.supports_initial_response() {
            let response = authenticator.respond(credentials, 0, b"")?;
            step = 1;
            Some(ImapSecret::new(Cow::Owned(response.expose().to_vec())))
        } else {
            None
        };
        let tag = self.tag()?;
        let command = Command::new(
            tag,
            CommandBody::Authenticate {
                mechanism,
                initial_response,
            },
        )
        .map_err(|_| protocol("commande invalide"))?;
        self.client.enqueue_command(command);
        let mut failure: Option<AuthError> = None;
        loop {
            match self.next_event().await? {
                Event::AuthenticateStarted { .. } | Event::DataReceived { .. } => {}
                Event::StatusReceived {
                    status: Status::Untagged(_),
                } => {}
                Event::AuthenticateContinuationRequestReceived {
                    continuation_request,
                    ..
                } => {
                    let challenge: &[u8] = match &continuation_request {
                        CommandContinuationRequest::Base64(bytes) => bytes,
                        CommandContinuationRequest::Basic(_) => b"",
                    };
                    let data = if step >= MAX_SASL_STEPS {
                        failure = Some(AuthError::UnexpectedChallenge { step });
                        AuthenticateData::Cancel
                    } else {
                        match authenticator.respond(credentials, step, challenge) {
                            Ok(response) => AuthenticateData::Continue(ImapSecret::new(
                                Cow::Owned(response.expose().to_vec()),
                            )),
                            Err(e) => {
                                failure = Some(e);
                                AuthenticateData::Cancel
                            }
                        }
                    };
                    step += 1;
                    if self.client.set_authenticate_data(data).is_err() {
                        return Err(protocol("échange d'authentification incohérent"));
                    }
                }
                Event::AuthenticateStatusReceived { status, .. } => {
                    if let Some(e) = failure {
                        return Err(e.into());
                    }
                    return match status {
                        Status::Tagged(tagged) => auth_status(&tagged.body),
                        _ => Err(protocol("réponse d'authentification inattendue")),
                    };
                }
                Event::StatusReceived {
                    status: Status::Bye(_),
                } => return Err(ConnectionError::Closed),
                other => {
                    tracing::debug!(event = ?std::mem::discriminant(&other), "événement inattendu");
                    return Err(ConnectionError::Protocol(format!(
                        "événement inattendu pendant l'authentification : {}",
                        event_name(&other)
                    )));
                }
            }
        }
    }

    /// `LOGOUT`.
    pub(crate) async fn logout(&mut self) -> Result<(), ConnectionError> {
        if self.in_idle || self.busy || self.literal_pending {
            // IDLE abandonné : rien d'autre ne peut être envoyé proprement ;
            // la connexion est simplement fermée.
            self.transport = Transport::Detached;
            return Ok(());
        }
        match self.execute(CommandBody::Logout, true).await {
            Ok(_) | Err(ConnectionError::Closed) => {
                self.transport = Transport::Detached;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

/// Capacités annoncées dans le code de réponse `[CAPABILITY …]`.
pub(crate) fn greeting_capabilities(greeting: &Greeting<'_>) -> Option<ServerCapabilities> {
    match &greeting.code {
        Some(Code::Capability(caps)) => Some(capabilities_of(caps.as_ref())),
        _ => None,
    }
}

fn status_text(status: &Status<'_>) -> String {
    let text = match status {
        Status::Untagged(body) => body.text.as_ref(),
        Status::Tagged(tagged) => tagged.body.text.as_ref(),
        Status::Bye(bye) => bye.text.as_ref(),
    };
    server_text(text.as_bytes())
}

/// `OK` attendu ; `NO` : commande refusée, session intacte
/// ([`ConnectionError::Rejected`]) ; `BAD` : erreur de protocole.
pub(crate) fn ensure_ok(status: &StatusBody<'_>) -> Result<(), ConnectionError> {
    let text = || server_text(status.text.as_ref().as_bytes());
    match status.kind {
        StatusKind::Ok => Ok(()),
        StatusKind::No => Err(ConnectionError::Rejected(text())),
        StatusKind::Bad => Err(ConnectionError::Protocol(format!(
            "commande refusée : {}",
            text()
        ))),
    }
}

fn auth_status(status: &StatusBody<'_>) -> Result<(), ConnectionError> {
    if status.kind == StatusKind::Ok {
        Ok(())
    } else {
        Err(AuthError::Rejected {
            message: server_text(status.text.as_ref().as_bytes()),
        }
        .into())
    }
}

/// Nom d'un événement, sans son contenu (qui peut contenir des secrets).
fn event_name(event: &Event) -> &'static str {
    match event {
        Event::GreetingReceived { .. } => "GreetingReceived",
        Event::CommandSent { .. } => "CommandSent",
        Event::CommandRejected { .. } => "CommandRejected",
        Event::AuthenticateStarted { .. } => "AuthenticateStarted",
        Event::AuthenticateContinuationRequestReceived { .. } => {
            "AuthenticateContinuationRequestReceived"
        }
        Event::AuthenticateStatusReceived { .. } => "AuthenticateStatusReceived",
        Event::IdleCommandSent { .. } => "IdleCommandSent",
        Event::IdleAccepted { .. } => "IdleAccepted",
        Event::IdleRejected { .. } => "IdleRejected",
        Event::IdleDoneSent { .. } => "IdleDoneSent",
        Event::DataReceived { .. } => "DataReceived",
        Event::StatusReceived { .. } => "StatusReceived",
        Event::ContinuationRequestReceived { .. } => "ContinuationRequestReceived",
    }
}

/// Ligne d'état (`OK`, `NO`, `BAD`, `BYE`, `PREAUTH`, étiquetée ou non) :
/// octets non ASCII remplacés par `?`. Certains serveurs (Dovecot) y citent
/// un nom de boîte en UTF-8, comme IMAP4rev2 le permet, ce que la
/// grammaire IMAP4rev1 d'`imap-next` refuse : la réponse étiquetée serait
/// écartée et la commande attendrait jusqu'au délai. Le texte d'état n'est
/// qu'informatif ; les autres réponses (noms de `LIST`, contenus) ne sont
/// pas touchées.
fn ascii_status_text(line: &mut [u8]) {
    let mut words = line.splitn(3, |&b| b == b' ');
    let (Some(_tag), Some(kind)) = (words.next(), words.next()) else {
        return;
    };
    let status = [&b"OK"[..], b"NO", b"BAD", b"BYE", b"PREAUTH"]
        .iter()
        .any(|k| kind.eq_ignore_ascii_case(k));
    if !status {
        return;
    }
    for b in line.iter_mut().filter(|b| !b.is_ascii()) {
        *b = b'?';
    }
}

#[cfg(test)]
mod tests {
    use super::ascii_status_text;

    fn cleaned(line: &str) -> String {
        let mut bytes = line.as_bytes().to_vec();
        ascii_status_text(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn only_status_lines_are_made_ascii() {
        assert_eq!(
            cleaned("N3 NO existe : Été\r\n"),
            "N3 NO existe : ??t??\r\n"
        );
        assert_eq!(
            cleaned("* ok [ALERT] réduit\r\n"),
            "* ok [ALERT] r??duit\r\n"
        );
        assert_eq!(cleaned("* BYE à bientôt\r\n"), "* BYE ?? bient??t\r\n");
        // Données : noms de boîtes et contenus intacts.
        for line in [
            "* LIST () \"/\" Été\r\n",
            "* 3 FETCH (FLAGS (\\Seen))\r\n",
            "* NOTED Été\r\n",
            "Été\r\n",
        ] {
            assert_eq!(cleaned(line), line);
        }
    }
}
