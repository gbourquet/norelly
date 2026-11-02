//! Sondage des ports usuels : connexions TCP en parallèle, délai court.
//!
//! En STARTTLS, la salutation en clair est lue et vérifiée (`* OK` pour
//! IMAP, `220` pour SMTP) ; en TLS implicite, l'acceptation de la
//! connexion suffit (le certificat sera vérifié lors du test de
//! connexion). Aucune donnée n'est envoyée.

use std::time::Duration;

use norelly_domain::discovery::ServerCandidate;
use norelly_domain::endpoint::{Protocol, Security};
use norelly_domain::ports::PortProber;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::task::JoinSet;

/// Nombre maximal de candidats sondés.
const MAX_CANDIDATES: usize = 32;
/// Taille maximale de la salutation lue.
const MAX_GREETING: usize = 512;

/// Sondeur TCP.
#[derive(Debug, Clone)]
pub struct TcpPortProber {
    connect_timeout: Duration,
    greeting_timeout: Duration,
}

impl Default for TcpPortProber {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(3),
            greeting_timeout: Duration::from_secs(3),
        }
    }
}

impl TcpPortProber {
    #[must_use]
    pub fn with_timeouts(connect: Duration, greeting: Duration) -> Self {
        Self {
            connect_timeout: connect,
            greeting_timeout: greeting,
        }
    }
}

/// Vrai si la salutation correspond au protocole.
fn greeting_matches(protocol: Protocol, greeting: &[u8]) -> bool {
    match protocol {
        Protocol::Imap => greeting.starts_with(b"* OK") || greeting.starts_with(b"* PREAUTH"),
        Protocol::Smtp => greeting.starts_with(b"220"),
    }
}

async fn probe_one(
    candidate: &ServerCandidate,
    connect_timeout: Duration,
    greeting_timeout: Duration,
) -> bool {
    let endpoint = &candidate.endpoint;
    let addr = (endpoint.host().as_str().to_owned(), endpoint.port());
    let Ok(Ok(mut stream)) = tokio::time::timeout(connect_timeout, TcpStream::connect(addr)).await
    else {
        return false;
    };
    if endpoint.security() == Security::Tls {
        return true;
    }
    let mut greeting = Vec::with_capacity(64);
    let read = async {
        let mut buf = [0u8; 128];
        while greeting.len() < MAX_GREETING && !greeting.contains(&b'\n') {
            let n = stream.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            greeting.extend_from_slice(buf.get(..n).unwrap_or_default());
        }
        Ok::<_, std::io::Error>(())
    };
    matches!(
        tokio::time::timeout(greeting_timeout, read).await,
        Ok(Ok(()))
    ) && greeting_matches(candidate.protocol, &greeting)
}

impl PortProber for TcpPortProber {
    async fn probe(&self, candidates: Vec<ServerCandidate>) -> Vec<ServerCandidate> {
        let mut set = JoinSet::new();
        for (i, candidate) in candidates.into_iter().take(MAX_CANDIDATES).enumerate() {
            let (connect, greeting) = (self.connect_timeout, self.greeting_timeout);
            set.spawn(async move {
                let ok = probe_one(&candidate, connect, greeting).await;
                (i, ok.then_some(candidate))
            });
        }
        let mut found: Vec<(usize, ServerCandidate)> = Vec::new();
        while let Some(joined) = set.join_next().await {
            if let Ok((i, Some(candidate))) = joined {
                found.push((i, candidate));
            }
        }
        found.sort_by_key(|(i, _)| *i);
        found.into_iter().map(|(_, c)| c).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greetings() {
        assert!(greeting_matches(Protocol::Imap, b"* OK Dovecot ready.\r\n"));
        assert!(greeting_matches(Protocol::Imap, b"* PREAUTH\r\n"));
        assert!(!greeting_matches(Protocol::Imap, b"220 smtp ready\r\n"));
        assert!(greeting_matches(Protocol::Smtp, b"220 smtp ready\r\n"));
        assert!(!greeting_matches(Protocol::Smtp, b"* OK\r\n"));
        assert!(!greeting_matches(Protocol::Smtp, b"HTTP/1.1 400\r\n"));
    }
}
