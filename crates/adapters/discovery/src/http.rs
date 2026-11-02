//! Client autoconfig HTTPS (fournisseur et ISPDB de Mozilla).
//!
//! HTTPS uniquement, redirections HTTPS seulement (3 au plus), sans cookies
//! ni `Referer`, User-Agent neutre, délais courts, réponse bornée.

use std::time::Duration;

use norelly_domain::discovery::{AutoconfigRequest, DiscoveredServers};
use norelly_domain::ports::{AutoconfigClient, DiscoveryError};
use norelly_tls::TlsClientFactory;
use reqwest::StatusCode;
use reqwest::redirect::Policy;

use crate::autoconfig_xml::{self, MAX_XML_BYTES};

/// Délai de connexion.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Délai total d'une requête.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Nombre maximal de redirections suivies.
const MAX_REDIRECTS: usize = 3;

/// Erreur de construction du client.
#[derive(Debug, thiserror::Error)]
#[error("client HTTPS impossible à construire : {0}")]
pub struct HttpSetupError(String);

/// Client autoconfig.
#[derive(Debug, Clone)]
pub struct HttpAutoconfigClient {
    client: reqwest::Client,
}

impl HttpAutoconfigClient {
    /// Client HTTPS avec la vérification de certificats de la plateforme
    /// (aucune exception : un certificat invalide fait échouer la source).
    pub fn new(tls: &TlsClientFactory) -> Result<Self, HttpSetupError> {
        let config = tls
            .standard_client_config()
            .map_err(|e| HttpSetupError(e.to_string()))?;
        Self::build(
            reqwest::Client::builder().use_preconfigured_tls(config),
            true,
        )
    }

    /// Client qui accepte le HTTP en clair : **tests uniquement**
    /// (`wiremock` ne sert pas de HTTPS).
    #[cfg(test)]
    pub(crate) fn plain_http_for_tests() -> Result<Self, HttpSetupError> {
        let config = TlsClientFactory::platform()
            .standard_client_config()
            .map_err(|e| HttpSetupError(e.to_string()))?;
        Self::build(
            reqwest::Client::builder().use_preconfigured_tls(config),
            false,
        )
    }

    fn build(builder: reqwest::ClientBuilder, https_only: bool) -> Result<Self, HttpSetupError> {
        let client = builder
            .https_only(https_only)
            .redirect(Policy::custom(move |attempt| {
                match redirect_decision(
                    attempt.previous().len(),
                    attempt.url().scheme(),
                    https_only,
                ) {
                    Ok(()) => attempt.follow(),
                    Err(reason) => attempt.error(reason),
                }
            }))
            .referer(false)
            .user_agent("Norelly")
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| HttpSetupError(e.to_string()))?;
        Ok(Self { client })
    }

    /// Corps de la réponse, `None` si le fichier n'existe pas.
    async fn get(&self, url: &str) -> Result<Option<Vec<u8>>, DiscoveryError> {
        let mut response = self.client.get(url).send().await.map_err(request_error)?;
        let status = response.status();
        if matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE) {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(DiscoveryError::Invalid(format!("HTTP {}", status.as_u16())));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_XML_BYTES as u64)
        {
            return Err(DiscoveryError::Invalid("réponse trop volumineuse".into()));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            if body.len() + chunk.len() > MAX_XML_BYTES {
                return Err(DiscoveryError::Invalid("réponse trop volumineuse".into()));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Some(body))
    }
}

/// Décision sur une redirection : `previous` redirections déjà suivies,
/// `scheme` de la cible. Refuse au-delà de [`MAX_REDIRECTS`] et, en HTTPS
/// seul, toute cible non HTTPS (un autoconfig servi en clair est
/// falsifiable, donc capable de détourner le mot de passe).
fn redirect_decision(previous: usize, scheme: &str, https_only: bool) -> Result<(), &'static str> {
    if previous >= MAX_REDIRECTS {
        Err("trop de redirections")
    } else if https_only && scheme != "https" {
        Err("redirection hors HTTPS refusée")
    } else {
        Ok(())
    }
}

fn request_error(error: reqwest::Error) -> DiscoveryError {
    if error.is_timeout() {
        DiscoveryError::Timeout
    } else if error.is_redirect() || error.is_builder() {
        DiscoveryError::Invalid(error.to_string())
    } else {
        DiscoveryError::Network(error.to_string())
    }
}

impl AutoconfigClient for HttpAutoconfigClient {
    async fn fetch(
        &self,
        request: &AutoconfigRequest,
    ) -> Result<Option<DiscoveredServers>, DiscoveryError> {
        let Some(body) = self.get(&request.url).await? else {
            return Ok(None);
        };
        autoconfig_xml::parse(&body, &request.domain).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects_to_plain_http_are_refused() {
        assert_eq!(redirect_decision(0, "https", true), Ok(()));
        assert!(redirect_decision(0, "http", true).is_err());
        assert!(redirect_decision(1, "HTTP", true).is_err());
        assert!(redirect_decision(0, "ftp", true).is_err());
        // Client de test (wiremock, HTTP) seulement.
        assert_eq!(redirect_decision(0, "http", false), Ok(()));
    }

    #[test]
    fn redirect_chain_is_bounded() {
        assert_eq!(redirect_decision(MAX_REDIRECTS - 1, "https", true), Ok(()));
        assert!(redirect_decision(MAX_REDIRECTS, "https", true).is_err());
        assert!(redirect_decision(MAX_REDIRECTS, "http", false).is_err());
    }
}
