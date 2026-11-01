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

/// Taille maximale d'une réponse (littéraux compris).
pub(crate) const MAX_RESPONSE_SIZE: u32 = 64 * 1024 * 1024;

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
    pub status: StatusBody<'static>,
}

/// Connexion IMAP pilotée.
pub(crate) struct Connection {
    transport: Transport,
    client: Client,
    /// Octets reçus, pas encore fournis à `client`.
    inbuf: Vec<u8>,
    timeout: Duration,
    next_tag: u32,
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
    pub(crate) fn new(transport: Transport, expect_greeting: bool, timeout: Duration) -> Self {
        let mut options = Options::default();
        options.max_response_size = MAX_RESPONSE_SIZE;
        options.discard_greeting = !expect_greeting;
        Self {
            transport,
            client: Client::new(options),
            inbuf: Vec::new(),
            timeout,
            next_tag: 0,
        }
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
        loop {
            match self.client.next() {
                Ok(event) => return Ok(event),
                Err(Interrupt::Error(e)) => {
                    use imap_next::client::Error as E;
                    return Err(protocol(match e {
                        E::MalformedMessage { .. } => "réponse malformée",
                        E::ExpectedCrlfGotLf { .. } => "fin de ligne invalide (LF sans CR)",
                        E::ResponseTooLong { .. } => "réponse trop longue",
                    }));
                }
                Err(Interrupt::Io(Io::Output(mut bytes))) => {
                    let result =
                        tokio::time::timeout(self.timeout, self.transport.write_all_flush(&bytes))
                            .await;
                    // Les octets peuvent contenir le mot de passe.
                    bytes.zeroize();
                    result
                        .map_err(|_| ConnectionError::Timeout(TimeoutStage::Write))?
                        .map_err(|e| io_error(&e))?;
                }
                Err(Interrupt::Io(Io::NeedMoreInput)) => self.feed_line().await?,
            }
        }
    }

    /// Fournit à `client` la prochaine ligne complète (jusqu'à `\n`
    /// compris), en lisant le réseau si nécessaire.
    async fn feed_line(&mut self) -> Result<(), ConnectionError> {
        loop {
            if let Some(pos) = self.inbuf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.inbuf.drain(..=pos).collect();
                self.client.enqueue_input(&line);
                return Ok(());
            }
            if self.inbuf.len() > MAX_RESPONSE_SIZE as usize {
                return Err(protocol("ligne trop longue"));
            }
            let mut chunk = [0u8; 16 * 1024];
            let n = tokio::time::timeout(self.timeout, self.transport.read_some(&mut chunk))
                .await
                .map_err(|_| ConnectionError::Timeout(TimeoutStage::Response))?
                .map_err(|e| io_error(&e))?;
            if n == 0 {
                return Err(ConnectionError::Closed);
            }
            self.inbuf.extend_from_slice(&chunk[..n]);
        }
    }

    /// Lit la salutation du serveur.
    pub(crate) async fn read_greeting(&mut self) -> Result<Greeting<'static>, ConnectionError> {
        loop {
            match self.next_event().await? {
                Event::GreetingReceived { greeting } => {
                    if greeting.kind == GreetingKind::Bye {
                        return Err(ConnectionError::Protocol(format!(
                            "le serveur refuse la connexion : {}",
                            server_text(greeting.text.as_ref().as_bytes())
                        )));
                    }
                    return Ok(greeting);
                }
                _ => return Err(protocol("salutation attendue")),
            }
        }
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
        let tag = self.tag()?;
        let command = Command::new(tag.clone(), body).map_err(|_| protocol("commande invalide"))?;
        self.client.enqueue_command(command);
        let mut data = Vec::new();
        loop {
            match self.next_event().await? {
                Event::CommandSent { .. } => {}
                Event::DataReceived { data: d } => data.push(d),
                Event::StatusReceived { status } => match status {
                    Status::Tagged(tagged) if tagged.tag == tag => {
                        return Ok(Reply {
                            data,
                            status: tagged.body,
                        });
                    }
                    Status::Tagged(_) => return Err(protocol("réponse étiquetée inattendue")),
                    Status::Untagged(_) => {}
                    Status::Bye(_) if allow_bye => {}
                    Status::Bye(_) => return Err(ConnectionError::Closed),
                },
                Event::CommandRejected { status, .. } => {
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

fn ensure_ok(status: &StatusBody<'_>) -> Result<(), ConnectionError> {
    if status.kind == StatusKind::Ok {
        Ok(())
    } else {
        Err(ConnectionError::Protocol(format!(
            "commande refusée : {}",
            server_text(status.text.as_ref().as_bytes())
        )))
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
