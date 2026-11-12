//! Phase en clair (bannière, EHLO, STARTTLS) et flux confié à `lettre`.

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use lettre::transport::smtp::client::AsyncTokioStream;
use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::connection::{ConnectionError, TimeoutStage, server_text};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

/// Longueur maximale d'une ligne de réponse SMTP (RFC 5321 : 512).
const MAX_LINE: usize = 4096;
/// Nombre maximal de lignes d'une réponse multiligne.
const MAX_LINES: usize = 128;

/// Réponse SMTP : code et lignes de texte.
#[derive(Debug)]
pub(crate) struct Reply {
    pub code: u16,
    pub lines: Vec<String>,
}

fn protocol(what: &str) -> ConnectionError {
    ConnectionError::Protocol(what.to_owned())
}

/// Dialogue en clair avant TLS. Toutes les réponses sont hostiles : elles
/// ne servent qu'à décider de STARTTLS.
pub(crate) struct PlainSession {
    tcp: TcpStream,
    buffer: Vec<u8>,
    timeout: Duration,
}

impl PlainSession {
    pub(crate) fn new(tcp: TcpStream, timeout: Duration) -> Self {
        Self {
            tcp,
            buffer: Vec::new(),
            timeout,
        }
    }

    async fn read_line(&mut self) -> Result<String, ConnectionError> {
        loop {
            if let Some(pos) = self.buffer.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=pos).collect();
                let line = line
                    .strip_suffix(b"\r\n")
                    .ok_or_else(|| protocol("fin de ligne invalide"))?;
                return Ok(String::from_utf8_lossy(line).into_owned());
            }
            if self.buffer.len() > MAX_LINE {
                return Err(protocol("ligne trop longue"));
            }
            let mut chunk = [0u8; 1024];
            let n = tokio::time::timeout(self.timeout, self.tcp.read(&mut chunk))
                .await
                .map_err(|_| ConnectionError::Timeout(TimeoutStage::Response))?
                .map_err(|e| crate::io_failure(&e))?;
            if n == 0 {
                return Err(ConnectionError::Closed);
            }
            self.buffer.extend_from_slice(&chunk[..n]);
        }
    }

    /// Lit une réponse complète (multiligne `ddd-…` puis `ddd …`).
    async fn read_reply(&mut self) -> Result<Reply, ConnectionError> {
        let mut lines = Vec::new();
        let mut code = None;
        loop {
            let line = self.read_line().await?;
            let bytes = line.as_bytes();
            let valid = bytes.len() >= 3
                && bytes[..3].iter().all(u8::is_ascii_digit)
                && matches!(bytes.get(3), None | Some(b' ' | b'-'));
            if !valid {
                return Err(ConnectionError::Protocol(format!(
                    "réponse SMTP invalide : {}",
                    server_text(bytes)
                )));
            }
            let this_code: u16 = line[..3].parse().map_err(|_| protocol("code invalide"))?;
            if *code.get_or_insert(this_code) != this_code {
                return Err(protocol("codes incohérents dans une réponse multiligne"));
            }
            lines.push(line.get(4..).unwrap_or_default().to_owned());
            if lines.len() > MAX_LINES {
                return Err(protocol("réponse trop longue"));
            }
            if bytes.get(3) != Some(&b'-') {
                return Ok(Reply {
                    code: this_code,
                    lines,
                });
            }
        }
    }

    async fn command(&mut self, command: &[u8]) -> Result<Reply, ConnectionError> {
        tokio::time::timeout(self.timeout, self.tcp.write_all(command))
            .await
            .map_err(|_| ConnectionError::Timeout(TimeoutStage::Write))?
            .map_err(|e| crate::io_failure(&e))?;
        self.read_reply().await
    }

    /// Bannière `220`.
    pub(crate) async fn greeting(&mut self) -> Result<(), ConnectionError> {
        let reply = self.read_reply().await?;
        if reply.code != 220 {
            return Err(ConnectionError::Protocol(format!(
                "bannière SMTP refusée ({}) : {}",
                reply.code,
                server_text(reply.lines.join(" ").as_bytes())
            )));
        }
        Ok(())
    }

    /// `EHLO` ; capacités annoncées en clair.
    pub(crate) async fn ehlo(&mut self) -> Result<ServerCapabilities, ConnectionError> {
        let reply = self.command(b"EHLO [127.0.0.1]\r\n").await?;
        if reply.code != 250 {
            return Err(ConnectionError::Protocol(format!(
                "EHLO refusé ({})",
                reply.code
            )));
        }
        Ok(ServerCapabilities::from_smtp_ehlo(
            reply.lines.iter().skip(1),
        ))
    }

    /// `STARTTLS` ; renvoie le flux TCP prêt pour la poignée de main.
    pub(crate) async fn starttls(mut self) -> Result<TcpStream, ConnectionError> {
        let reply = self.command(b"STARTTLS\r\n").await?;
        if reply.code != 220 {
            return Err(ConnectionError::StartTlsRejected(server_text(
                reply.lines.join(" ").as_bytes(),
            )));
        }
        self.into_tcp()
    }

    /// Rend le flux TCP ; refuse s'il reste des octets reçus non lus
    /// (données injectées avant la poignée de main TLS).
    pub(crate) fn into_tcp(self) -> Result<TcpStream, ConnectionError> {
        if !self.buffer.is_empty() {
            return Err(ConnectionError::PlaintextInjection);
        }
        Ok(self.tcp)
    }
}

/// Flux sous-jacent du flux confié à `lettre`.
#[derive(Debug)]
enum Inner {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

/// Flux confié à `lettre` : préfixe local éventuel (bannière synthétique
/// après STARTTLS), puis le flux réel.
pub(crate) struct LettreStream {
    prefix: Vec<u8>,
    position: usize,
    inner: Inner,
    peer: SocketAddr,
}

impl fmt::Debug for LettreStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LettreStream")
            .field("inner", &self.inner)
            .field("peer", &self.peer)
            .finish_non_exhaustive()
    }
}

impl LettreStream {
    pub(crate) fn tls(stream: TlsStream<TcpStream>, peer: SocketAddr, prefix: Vec<u8>) -> Self {
        Self {
            prefix,
            position: 0,
            inner: Inner::Tls(Box::new(stream)),
            peer,
        }
    }

    pub(crate) fn plain(stream: TcpStream, peer: SocketAddr, prefix: Vec<u8>) -> Self {
        Self {
            prefix,
            position: 0,
            inner: Inner::Plain(stream),
            peer,
        }
    }
}

impl AsyncRead for LettreStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Some(rest) = this.prefix.get(this.position..)
            && !rest.is_empty()
        {
            let n = rest.len().min(buf.remaining());
            buf.put_slice(&rest[..n]);
            this.position += n;
            return Poll::Ready(Ok(()));
        }
        match &mut this.inner {
            Inner::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Inner::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for LettreStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().inner {
            Inner::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Inner::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().inner {
            Inner::Plain(s) => Pin::new(s).poll_flush(cx),
            Inner::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().inner {
            Inner::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Inner::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

impl AsyncTokioStream for LettreStream {
    fn peer_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.peer)
    }
}
