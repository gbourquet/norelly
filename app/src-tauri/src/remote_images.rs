//! Proxy des images distantes (`remote-img://`, ADR-004) : la webview ne
//! contacte jamais un serveur distant ; Rust télécharge l'image à sa place,
//! seulement si l'expéditeur a été autorisé (décision prise par le
//! protocole, voir `protocols.rs`).
//!
//! - `http` et `https` seulement, sans cookies ni `Referer`, User-Agent
//!   neutre, 3 redirections au plus ;
//! - adresses publiques seulement (résolution DNS filtrée, adresses IP
//!   littérales vérifiées, y compris après redirection) : un mail ne peut
//!   pas faire sonder le réseau local ;
//! - réponse bornée en taille et en durée, types d'image matriciels
//!   seulement (pas de SVG).

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use norelly_domain::reading::{ExternalLink, LinkScheme, is_public_address};
use norelly_tls::TlsClientFactory;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::Policy;

/// Taille maximale d'une image, en octets.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REDIRECTS: usize = 3;
/// User-Agent neutre, identique pour tous les utilisateurs.
const USER_AGENT: &str = "Mozilla/5.0";

/// Types servis (le type annoncé par le serveur doit en faire partie).
pub const IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/gif",
    "image/jpeg",
    "image/webp",
    "image/bmp",
    "image/avif",
    "image/x-icon",
    "image/vnd.microsoft.icon",
];

/// Échec du proxy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProxyError {
    #[error("adresse d'image refusée")]
    Refused,
    #[error("téléchargement impossible : {0}")]
    Network(String),
    #[error("réponse refusée : {0}")]
    Response(String),
}

/// Résolution DNS qui ne renvoie que des adresses publiques.
#[derive(Debug)]
struct PublicOnly;

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let found: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|a| is_public_address(a.ip()))
                .collect();
            if found.is_empty() {
                return Err("aucune adresse publique".into());
            }
            let addrs: Addrs = Box::new(found.into_iter());
            Ok(addrs)
        })
    }
}

/// Vrai si l'hôte de l'adresse est un nom (résolu par [`PublicOnly`]) ou
/// une adresse IP publique.
fn host_allowed(url: &reqwest::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => !d.is_empty() && !d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => is_public_address(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => is_public_address(IpAddr::V6(ip)),
        None => false,
    }
}

/// Type d'image annoncé, normalisé, s'il est accepté.
#[must_use]
pub fn accepted_type(content_type: Option<&str>) -> Option<&'static str> {
    let raw = content_type?.split(';').next()?.trim().to_ascii_lowercase();
    IMAGE_TYPES.iter().copied().find(|t| *t == raw)
}

/// Client du proxy.
#[derive(Debug, Clone)]
pub struct RemoteImageProxy {
    client: reqwest::Client,
}

impl RemoteImageProxy {
    pub fn new(tls: &TlsClientFactory) -> Result<Self, String> {
        let config = tls.standard_client_config().map_err(|e| e.to_string())?;
        let client = reqwest::Client::builder()
            .use_preconfigured_tls(config)
            .dns_resolver(PublicOnly)
            .redirect(Policy::custom(|attempt| {
                let ok = attempt.previous().len() < MAX_REDIRECTS
                    && matches!(attempt.url().scheme(), "http" | "https")
                    && host_allowed(attempt.url());
                if ok { attempt.follow() } else { attempt.stop() }
            }))
            .referer(false)
            .user_agent(USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client })
    }

    /// Télécharge une image : type accepté et contenu.
    pub async fn fetch(&self, raw: &str) -> Result<(&'static str, Vec<u8>), ProxyError> {
        let link = ExternalLink::parse(raw).map_err(|_| ProxyError::Refused)?;
        if link.scheme() == LinkScheme::Mailto {
            return Err(ProxyError::Refused);
        }
        let url = reqwest::Url::parse(link.as_str()).map_err(|_| ProxyError::Refused)?;
        if !host_allowed(&url) {
            return Err(ProxyError::Refused);
        }
        let mut response = self
            .client
            .get(url)
            .header(reqwest::header::ACCEPT, "image/*")
            .send()
            .await
            .map_err(|e| ProxyError::Network(e.without_url().to_string()))?;
        if !response.status().is_success() {
            return Err(ProxyError::Response(format!(
                "statut {}",
                response.status()
            )));
        }
        let content_type = accepted_type(
            response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
        )
        .ok_or_else(|| ProxyError::Response("type non accepté".into()))?;
        if response
            .content_length()
            .is_some_and(|l| l > MAX_IMAGE_BYTES as u64)
        {
            return Err(ProxyError::Response("image trop volumineuse".into()));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| ProxyError::Network(e.without_url().to_string()))?
        {
            if body.len() + chunk.len() > MAX_IMAGE_BYTES {
                return Err(ProxyError::Response("image trop volumineuse".into()));
            }
            body.extend_from_slice(&chunk);
        }
        Ok((content_type, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_raster_image_types_are_accepted() {
        assert_eq!(
            accepted_type(Some("image/PNG; charset=x")),
            Some("image/png")
        );
        assert_eq!(accepted_type(Some("image/svg+xml")), None);
        assert_eq!(accepted_type(Some("text/html")), None);
        assert_eq!(accepted_type(None), None);
    }

    #[test]
    fn private_hosts_are_refused_before_any_request() {
        for url in [
            "http://127.0.0.1/a.png",
            "http://[::1]/a.png",
            "http://192.168.1.1/a.png",
            "http://169.254.169.254/latest/meta-data",
            "http://localhost/a.png",
            "http://[::ffff:10.0.0.1]/a.png",
        ] {
            let parsed = reqwest::Url::parse(url).unwrap_or_else(|e| panic!("{url}: {e}"));
            assert!(!host_allowed(&parsed), "{url}");
        }
        let public = reqwest::Url::parse("https://images.exemple.fr/a.png")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(host_allowed(&public));
    }

    #[tokio::test]
    async fn refused_urls_never_reach_the_network() {
        let proxy =
            RemoteImageProxy::new(&TlsClientFactory::platform()).unwrap_or_else(|e| panic!("{e}"));
        for url in [
            "http://127.0.0.1:9/a.png",
            "javascript:alert(1)",
            "mailto:a@b.c",
            "file:///etc/passwd",
            "http://localhost:9/a.png",
        ] {
            assert_eq!(proxy.fetch(url).await, Err(ProxyError::Refused), "{url}");
        }
        // Nom qui ne résout que vers la boucle locale : refusé par le DNS
        // filtré (aucune adresse publique).
        let local = proxy.fetch("http://localhost.:9/a.png").await;
        assert!(
            matches!(local, Err(ProxyError::Network(_) | ProxyError::Refused)),
            "{local:?}"
        );
    }
}
