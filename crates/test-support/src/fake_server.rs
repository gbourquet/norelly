//! Faux serveurs scriptés (IMAP et SMTP) : une suite d'étapes « attendre
//! une ligne / envoyer des octets / passer en TLS », jouée sur une seule
//! connexion. Tout ce que le client envoie (après déchiffrement TLS) est
//! enregistré dans une transcription, pour vérifier par exemple qu'aucun
//! identifiant n'est parti.
//!
//! Dans le texte envoyé, `{tag}` est remplacé par l'étiquette (premier mot)
//! de la dernière commande reçue par [`Expect::Contains`] ou [`Expect::Any`].

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

/// Délai d'attente d'une ligne du client.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Taille maximale d'une ligne reçue.
const MAX_LINE: usize = 1 << 20;

/// Ce que le serveur attend du client.
#[derive(Debug, Clone)]
pub enum Expect {
    /// Une ligne contenant ce texte (insensible à la casse) ; met à jour
    /// `{tag}`.
    Contains(String),
    /// N'importe quelle ligne ; met à jour `{tag}`.
    Any,
    /// N'importe quelle ligne de données (réponse SASL, corps DATA…), sans
    /// toucher à `{tag}`.
    AnyData,
    /// Une ligne de données contenant ce texte, sans toucher à `{tag}`.
    DataContains(String),
    /// Des lignes de données jusqu'à une ligne égale à ce texte (sans la
    /// fin de ligne), par exemple `.` pour la fin de DATA en SMTP.
    DataUntil(String),
    /// Fermeture de la connexion par le client.
    Eof,
}

/// Étape du script.
#[derive(Debug, Clone)]
pub enum Step {
    Expect(Expect),
    /// Texte envoyé tel quel (après substitution de `{tag}`).
    Send(String),
    /// Octets bruts envoyés tels quels.
    SendRaw(Vec<u8>),
    /// Négociation TLS côté serveur (après STARTTLS).
    StartTls,
    /// Pause.
    Wait(Duration),
    /// Fermeture de la connexion par le serveur.
    Close,
}

/// Script d'un faux serveur, construit par chaînage.
#[derive(Debug, Clone, Default)]
pub struct Script {
    steps: Vec<Step>,
}

impl Script {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn send(mut self, text: impl Into<String>) -> Self {
        self.steps.push(Step::Send(text.into()));
        self
    }

    #[must_use]
    pub fn send_raw(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.steps.push(Step::SendRaw(bytes.into()));
        self
    }

    #[must_use]
    pub fn expect(mut self, needle: impl Into<String>) -> Self {
        self.steps
            .push(Step::Expect(Expect::Contains(needle.into())));
        self
    }

    #[must_use]
    pub fn expect_any(mut self) -> Self {
        self.steps.push(Step::Expect(Expect::Any));
        self
    }

    #[must_use]
    pub fn expect_data(mut self) -> Self {
        self.steps.push(Step::Expect(Expect::AnyData));
        self
    }

    #[must_use]
    pub fn expect_data_with(mut self, needle: impl Into<String>) -> Self {
        self.steps
            .push(Step::Expect(Expect::DataContains(needle.into())));
        self
    }

    #[must_use]
    pub fn expect_data_until(mut self, terminator: impl Into<String>) -> Self {
        self.steps
            .push(Step::Expect(Expect::DataUntil(terminator.into())));
        self
    }

    #[must_use]
    pub fn expect_eof(mut self) -> Self {
        self.steps.push(Step::Expect(Expect::Eof));
        self
    }

    #[must_use]
    pub fn starttls(mut self) -> Self {
        self.steps.push(Step::StartTls);
        self
    }

    #[must_use]
    pub fn wait(mut self, duration: Duration) -> Self {
        self.steps.push(Step::Wait(duration));
        self
    }

    #[must_use]
    pub fn close(mut self) -> Self {
        self.steps.push(Step::Close);
        self
    }
}

/// Échec du script côté serveur.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("E/S : {0}")]
    Io(#[from] std::io::Error),
    #[error("delai depasse en attendant : {0:?}")]
    Timeout(Expect),
    #[error("le client a ferme la connexion en attendant : {0:?}")]
    ClientClosed(Expect),
    #[error("ligne inattendue : {got:?} (attendu : {expected:?})")]
    Unexpected { expected: Expect, got: String },
    #[error("ligne trop longue")]
    LineTooLong,
    #[error("STARTTLS demande sur une connexion deja chiffree")]
    AlreadyTls,
    #[error("le client a envoye des donnees avant la negociation TLS")]
    EarlyData,
    #[error("tache du serveur interrompue")]
    Join,
}

/// Octets reçus du client.
#[derive(Debug, Clone, Default)]
pub struct Transcript {
    received: Vec<u8>,
}

impl Transcript {
    /// Tout ce que le client a envoyé.
    #[must_use]
    pub fn received(&self) -> &[u8] {
        &self.received
    }

    /// Vrai si `needle` apparaît dans ce que le client a envoyé.
    #[must_use]
    pub fn contains(&self, needle: &[u8]) -> bool {
        !needle.is_empty() && self.received.windows(needle.len()).any(|w| w == needle)
    }

    /// Ce que le client a envoyé, en texte (pertes UTF-8 remplacées).
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.received).into_owned()
    }
}

/// Résultat d'une exécution : la transcription est toujours disponible,
/// même si le script a échoué.
#[derive(Debug)]
pub struct Outcome {
    pub transcript: Transcript,
    pub result: Result<(), ScriptError>,
}

/// Faux serveur scripté, à l'écoute sur `127.0.0.1` (port libre), pour une
/// seule connexion.
#[derive(Debug)]
pub struct ScriptedServer {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<u8>>>,
    task: JoinHandle<Result<(), ScriptError>>,
}

impl ScriptedServer {
    /// Serveur en clair au départ (STARTTLS possible avec `tls`).
    pub async fn start(
        script: Script,
        tls: Option<Arc<rustls::ServerConfig>>,
    ) -> std::io::Result<Self> {
        Self::spawn(script, tls, false).await
    }

    /// Serveur en TLS implicite.
    pub async fn start_tls(
        script: Script,
        tls: Arc<rustls::ServerConfig>,
    ) -> std::io::Result<Self> {
        Self::spawn(script, Some(tls), true).await
    }

    async fn spawn(
        script: Script,
        tls: Option<Arc<rustls::ServerConfig>>,
        implicit_tls: bool,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let received = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&received);
        let task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await?;
            let mut conn = Connection {
                stream: ServerStream::Plain(tcp),
                buffer: Vec::new(),
                tag: String::new(),
                received: recorder,
                tls,
            };
            if implicit_tls {
                conn.upgrade().await?;
            }
            conn.run(script).await
        });
        Ok(Self {
            addr,
            received,
            task,
        })
    }

    /// Port d'écoute.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Attend la fin du script.
    pub async fn finish(self) -> Outcome {
        let result = match self.task.await {
            Ok(result) => result,
            Err(_) => Err(ScriptError::Join),
        };
        let received = self
            .received
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Outcome {
            transcript: Transcript { received },
            result,
        }
    }
}

#[derive(Debug)]
enum ServerStream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
    Upgrading,
}

impl ServerStream {
    async fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.read(buf).await,
            Self::Tls(s) => s.read(buf).await,
            Self::Upgrading => Ok(0),
        }
    }

    async fn write_all_flush(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        async fn go<S: AsyncWrite + AsyncRead + Unpin>(
            s: &mut S,
            bytes: &[u8],
        ) -> std::io::Result<()> {
            s.write_all(bytes).await?;
            s.flush().await
        }
        match self {
            Self::Plain(s) => go(s, bytes).await,
            Self::Tls(s) => go(s.as_mut(), bytes).await,
            Self::Upgrading => Ok(()),
        }
    }
}

struct Connection {
    stream: ServerStream,
    buffer: Vec<u8>,
    tag: String,
    received: Arc<Mutex<Vec<u8>>>,
    tls: Option<Arc<rustls::ServerConfig>>,
}

impl Connection {
    async fn run(&mut self, script: Script) -> Result<(), ScriptError> {
        for step in script.steps {
            match step {
                Step::Expect(expect) => self.expect(expect).await?,
                Step::Send(text) => {
                    let text = text.replace("{tag}", &self.tag);
                    self.stream.write_all_flush(text.as_bytes()).await?;
                }
                Step::SendRaw(bytes) => self.stream.write_all_flush(&bytes).await?,
                Step::StartTls => {
                    if !self.buffer.is_empty() {
                        return Err(ScriptError::EarlyData);
                    }
                    self.upgrade().await?;
                }
                Step::Wait(d) => tokio::time::sleep(d).await,
                Step::Close => {
                    self.stream = ServerStream::Upgrading;
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    async fn upgrade(&mut self) -> Result<(), ScriptError> {
        let config = self.tls.clone().ok_or_else(|| {
            ScriptError::Io(std::io::Error::other("aucune configuration TLS fournie"))
        })?;
        let ServerStream::Plain(tcp) = std::mem::replace(&mut self.stream, ServerStream::Upgrading)
        else {
            return Err(ScriptError::AlreadyTls);
        };
        let tls = TlsAcceptor::from(config).accept(tcp).await?;
        self.stream = ServerStream::Tls(Box::new(tls));
        Ok(())
    }

    async fn expect(&mut self, expect: Expect) -> Result<(), ScriptError> {
        if let Expect::DataUntil(terminator) = &expect {
            loop {
                let line = match tokio::time::timeout(READ_TIMEOUT, self.read_line()).await {
                    Err(_) => return Err(ScriptError::Timeout(expect)),
                    Ok(result) => result?,
                };
                let Some(line) = line else {
                    return Err(ScriptError::ClientClosed(expect));
                };
                if String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']) == terminator {
                    return Ok(());
                }
            }
        }
        let line = match tokio::time::timeout(READ_TIMEOUT, self.read_line()).await {
            Err(_) => return Err(ScriptError::Timeout(expect)),
            Ok(result) => result?,
        };
        let Some(line) = line else {
            return match expect {
                Expect::Eof => Ok(()),
                other => Err(ScriptError::ClientClosed(other)),
            };
        };
        let text = String::from_utf8_lossy(&line).into_owned();
        let matched = match &expect {
            Expect::Contains(needle) | Expect::DataContains(needle) => {
                text.to_lowercase().contains(&needle.to_lowercase())
            }
            Expect::Any | Expect::AnyData => true,
            Expect::Eof | Expect::DataUntil(_) => false,
        };
        if !matched {
            return Err(ScriptError::Unexpected {
                expected: expect,
                got: text,
            });
        }
        if matches!(expect, Expect::Contains(_) | Expect::Any) {
            self.tag = text
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
        }
        Ok(())
    }

    /// Lit une ligne terminée par `\n` ; `None` si le client a fermé.
    async fn read_line(&mut self) -> Result<Option<Vec<u8>>, ScriptError> {
        loop {
            if let Some(pos) = self.buffer.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=pos).collect();
                return Ok(Some(line));
            }
            if self.buffer.len() > MAX_LINE {
                return Err(ScriptError::LineTooLong);
            }
            let mut chunk = [0u8; 4096];
            let n = match self.stream.read_some(&mut chunk).await {
                Ok(n) => n,
                // Coupure brutale (RST, fin TLS non propre) : fermeture.
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::UnexpectedEof
                    ) =>
                {
                    0
                }
                Err(e) => return Err(e.into()),
            };
            if n == 0 {
                return Ok(None);
            }
            self.received
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(&chunk[..n]);
            self.buffer.extend_from_slice(&chunk[..n]);
        }
    }
}

/// Aides pour écrire des scripts IMAP.
pub mod imap {
    /// Salutation `* OK [CAPABILITY …]`.
    #[must_use]
    pub fn greeting(capabilities: &str) -> String {
        format!("* OK [CAPABILITY {capabilities}] Faux serveur IMAP pret\r\n")
    }

    /// Réponse à `CAPABILITY`.
    #[must_use]
    pub fn capability(capabilities: &str) -> String {
        format!("* CAPABILITY {capabilities}\r\n{{tag}} OK CAPABILITY termine\r\n")
    }

    /// Réponse étiquetée OK.
    #[must_use]
    pub fn ok(text: &str) -> String {
        format!("{{tag}} OK {text}\r\n")
    }
}

/// Aides pour écrire des scripts SMTP.
pub mod smtp {
    /// Bannière 220.
    #[must_use]
    pub fn greeting() -> String {
        "220 faux.serveur ESMTP pret\r\n".to_owned()
    }

    /// Réponse multiligne à `EHLO` avec les extensions données.
    #[must_use]
    pub fn ehlo(extensions: &[&str]) -> String {
        let mut out = String::new();
        let mut lines = vec!["faux.serveur bonjour"];
        lines.extend_from_slice(extensions);
        for (i, line) in lines.iter().enumerate() {
            let sep = if i + 1 == lines.len() { ' ' } else { '-' };
            out.push_str(&format!("250{sep}{line}\r\n"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};

    #[tokio::test]
    async fn plays_script_and_records_transcript() -> Result<(), Box<dyn std::error::Error>> {
        let server = ScriptedServer::start(
            Script::new()
                .send(imap::greeting("IMAP4rev1"))
                .expect("NOOP")
                .send(imap::ok("fait"))
                .expect_eof(),
            None,
        )
        .await?;
        let tcp = TcpStream::connect(("127.0.0.1", server.port())).await?;
        let mut reader = BufReader::new(tcp);
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        assert!(line.starts_with("* OK [CAPABILITY IMAP4rev1]"));
        reader.get_mut().write_all(b"A7 NOOP\r\n").await?;
        line.clear();
        reader.read_line(&mut line).await?;
        assert_eq!(line, "A7 OK fait\r\n");
        drop(reader);
        let outcome = server.finish().await;
        outcome.result?;
        assert!(outcome.transcript.contains(b"A7 NOOP"));
        Ok(())
    }

    #[tokio::test]
    async fn reports_unexpected_line() -> Result<(), Box<dyn std::error::Error>> {
        let server = ScriptedServer::start(Script::new().expect("LOGIN"), None).await?;
        let mut tcp = TcpStream::connect(("127.0.0.1", server.port())).await?;
        tcp.write_all(b"A1 CAPABILITY\r\n").await?;
        let outcome = server.finish().await;
        assert!(matches!(
            outcome.result,
            Err(ScriptError::Unexpected { .. })
        ));
        Ok(())
    }

    #[test]
    fn smtp_ehlo_formatting() {
        assert_eq!(
            smtp::ehlo(&["STARTTLS", "AUTH PLAIN"]),
            "250-faux.serveur bonjour\r\n250-STARTTLS\r\n250 AUTH PLAIN\r\n"
        );
    }
}
