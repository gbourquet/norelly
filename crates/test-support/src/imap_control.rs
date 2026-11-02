//! Modification d'une boîte « côté serveur » pendant un test : créer,
//! supprimer, ajouter un message, changer un drapeau, supprimer un
//! message. Implémenté pour le faux serveur en mémoire ([`FakeImap`]) et
//! pour un vrai serveur ([`ImapControl`], client IMAP minimal sur TLS,
//! indépendant du code testé).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use norelly_domain::mailbox::{Flags, Uid};
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::certs::crypto_provider;
use crate::fake_imap::FakeImap;

/// Opérations « côté serveur ».
pub trait MailboxControl: Clone + Send + Sync + 'static {
    fn create(&self, name: &str) -> impl Future<Output = Result<(), String>> + Send;
    fn delete(&self, name: &str) -> impl Future<Output = Result<(), String>> + Send;
    fn append(
        &self,
        name: &str,
        raw: &[u8],
        seen: bool,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn set_seen(
        &self,
        name: &str,
        uid: u32,
        seen: bool,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn expunge(&self, name: &str, uid: u32) -> impl Future<Output = Result<(), String>> + Send;
}

fn flags(seen: bool) -> Flags {
    Flags::from_tokens(if seen { vec!["\\Seen"] } else { vec![] })
}

impl MailboxControl for FakeImap {
    async fn create(&self, name: &str) -> Result<(), String> {
        self.create_mailbox(name, &[]);
        Ok(())
    }

    async fn delete(&self, name: &str) -> Result<(), String> {
        self.delete_mailbox(name);
        Ok(())
    }

    async fn append(&self, name: &str, raw: &[u8], seen: bool) -> Result<(), String> {
        FakeImap::append(self, name, raw, flags(seen), 1_700_000_000)
            .map(|_| ())
            .ok_or_else(|| format!("boîte {name} inconnue"))
    }

    async fn set_seen(&self, name: &str, uid: u32, seen: bool) -> Result<(), String> {
        let uid = Uid::new(uid).ok_or("UID nul")?;
        self.set_flags(name, uid, flags(seen));
        Ok(())
    }

    async fn expunge(&self, name: &str, uid: u32) -> Result<(), String> {
        let uid = Uid::new(uid).ok_or("UID nul")?;
        FakeImap::expunge(self, name, uid);
        Ok(())
    }
}

/// UTF-7 modifié (RFC 3501, 5.1.3), encodage seulement : réimplémenté ici
/// pour ne pas dépendre du code testé.
#[must_use]
pub fn utf7_encode(name: &str) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";
    let mut out = String::new();
    let mut units: Vec<u16> = Vec::new();
    let flush = |units: &mut Vec<u16>, out: &mut String| {
        if units.is_empty() {
            return;
        }
        let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_be_bytes()).collect();
        out.push('&');
        let mut acc: u32 = 0;
        let mut bits = 0;
        for b in bytes {
            acc = (acc << 8) | u32::from(b);
            bits += 8;
            while bits >= 6 {
                bits -= 6;
                out.push(char::from(B64[((acc >> bits) & 0x3f) as usize]));
            }
        }
        if bits > 0 {
            out.push(char::from(B64[((acc << (6 - bits)) & 0x3f) as usize]));
        }
        out.push('-');
        units.clear();
    };
    for c in name.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut units, &mut out);
            out.push_str(if c == '&' { "&-" } else { "" });
            if c != '&' {
                out.push(c);
            }
        } else {
            let mut buf = [0u16; 2];
            units.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut units, &mut out);
    out
}

fn quoted(name: &str) -> String {
    let encoded = utf7_encode(name);
    format!("\"{}\"", encoded.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Client de contrôle d'un vrai serveur IMAP (TLS implicite, LOGIN).
#[derive(Debug, Clone)]
pub struct ImapControl {
    host: String,
    port: u16,
    user: String,
    password: String,
    roots: Arc<Vec<CertificateDer<'static>>>,
}

struct Conn {
    reader: BufReader<TlsStream<TcpStream>>,
    tag: u32,
}

const TIMEOUT: Duration = Duration::from_secs(20);

impl Conn {
    async fn line(&mut self) -> Result<String, String> {
        let mut line = String::new();
        let n = tokio::time::timeout(TIMEOUT, self.reader.read_line(&mut line))
            .await
            .map_err(|_| "délai dépassé".to_owned())?
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("connexion fermée".into());
        }
        Ok(line)
    }

    /// Envoie une commande et lit jusqu'à la réponse étiquetée ; renvoie
    /// les lignes non étiquetées. `literal` : envoyé après la
    /// continuation `+`.
    async fn command(&mut self, text: &str, literal: Option<&[u8]>) -> Result<Vec<String>, String> {
        self.tag += 1;
        let tag = format!("C{}", self.tag);
        let stream = self.reader.get_mut();
        stream
            .write_all(format!("{tag} {text}\r\n").as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut untagged = Vec::new();
        if let Some(data) = literal {
            let line = self.line().await?;
            if !line.starts_with('+') {
                return Err(format!("continuation attendue : {line}"));
            }
            let stream = self.reader.get_mut();
            stream.write_all(data).await.map_err(|e| e.to_string())?;
            stream.write_all(b"\r\n").await.map_err(|e| e.to_string())?;
        }
        loop {
            let line = self.line().await?;
            // Littéraux dans les réponses : lus et ignorés.
            if let Some(size) = line
                .trim_end()
                .strip_suffix('}')
                .and_then(|l| l.rsplit_once('{'))
                .and_then(|(_, n)| n.parse::<usize>().ok())
            {
                let mut skip = vec![0u8; size];
                self.reader
                    .read_exact(&mut skip)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            if let Some(rest) = line.strip_prefix(&format!("{tag} ")) {
                if rest.starts_with("OK") {
                    return Ok(untagged);
                }
                return Err(format!("{text} : {}", rest.trim_end()));
            }
            untagged.push(line);
        }
    }
}

impl ImapControl {
    #[must_use]
    pub fn new(
        host: &str,
        port: u16,
        user: &str,
        password: &str,
        roots: Vec<CertificateDer<'static>>,
    ) -> Self {
        Self {
            host: host.to_owned(),
            port,
            user: user.to_owned(),
            password: password.to_owned(),
            roots: Arc::new(roots),
        }
    }

    async fn connect(&self) -> Result<Conn, String> {
        let mut store = rustls::RootCertStore::empty();
        for root in self.roots.iter() {
            store.add(root.clone()).map_err(|e| e.to_string())?;
        }
        let config = rustls::ClientConfig::builder_with_provider(crypto_provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(store)
            .with_no_client_auth();
        let tcp = TcpStream::connect((self.host.as_str(), self.port))
            .await
            .map_err(|e| e.to_string())?;
        let name = ServerName::try_from(self.host.clone()).map_err(|e| e.to_string())?;
        let tls = TlsConnector::from(Arc::new(config))
            .connect(name, tcp)
            .await
            .map_err(|e| e.to_string())?;
        let mut conn = Conn {
            reader: BufReader::new(tls),
            tag: 0,
        };
        conn.line().await?; // salutation
        conn.command(
            &format!("LOGIN \"{}\" \"{}\"", self.user, self.password),
            None,
        )
        .await?;
        Ok(conn)
    }

    /// Exécute des commandes sur une connexion neuve.
    async fn run(&self, commands: &[(&str, Option<&[u8]>)]) -> Result<Vec<String>, String> {
        let mut conn = self.connect().await?;
        let mut out = Vec::new();
        for (text, literal) in commands {
            out.extend(conn.command(text, *literal).await?);
        }
        let _ = conn.command("LOGOUT", None).await;
        Ok(out)
    }
}

impl MailboxControl for ImapControl {
    async fn create(&self, name: &str) -> Result<(), String> {
        self.run(&[(&format!("CREATE {}", quoted(name)), None)])
            .await
            .map(|_| ())
    }

    async fn delete(&self, name: &str) -> Result<(), String> {
        self.run(&[(&format!("DELETE {}", quoted(name)), None)])
            .await
            .map(|_| ())
    }

    async fn append(&self, name: &str, raw: &[u8], seen: bool) -> Result<(), String> {
        let flags = if seen { "(\\Seen) " } else { "" };
        let command = format!("APPEND {} {flags}{{{}}}", quoted(name), raw.len());
        self.run(&[(&command, Some(raw))]).await.map(|_| ())
    }

    async fn set_seen(&self, name: &str, uid: u32, seen: bool) -> Result<(), String> {
        let sign = if seen { '+' } else { '-' };
        self.run(&[
            (&format!("SELECT {}", quoted(name)), None),
            (
                &format!("UID STORE {uid} {sign}FLAGS.SILENT (\\Seen)"),
                None,
            ),
        ])
        .await
        .map(|_| ())
    }

    async fn expunge(&self, name: &str, uid: u32) -> Result<(), String> {
        self.run(&[
            (&format!("SELECT {}", quoted(name)), None),
            (&format!("UID STORE {uid} +FLAGS.SILENT (\\Deleted)"), None),
            (&format!("UID EXPUNGE {uid}"), None),
        ])
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf7_matches_rfc_examples() {
        assert_eq!(
            utf7_encode("~peter/mail/台北/日本語"),
            "~peter/mail/&U,BTFw-/&ZeVnLIqe-"
        );
        assert_eq!(utf7_encode("A&B"), "A&-B");
        assert_eq!(utf7_encode("Envoyés"), "Envoy&AOk-s");
    }
}
