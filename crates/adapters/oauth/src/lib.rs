//! Adaptateur OAuth 2.0 pour application de bureau (RFC 8252) : ouverture
//! du navigateur du système, retour sur une redirection locale
//! (`127.0.0.1`, port éphémère), code d'autorisation protégé par PKCE
//! (RFC 7636), échange et rafraîchissement par HTTPS avec la vérification
//! de certificats de la plateforme (ADR-002).
//!
//! Les règles (URL, formulaires, contrôle du retour, classement des
//! erreurs) sont celles du domaine ([`norelly_domain::oauth`]). Aucun jeton
//! n'apparaît dans les journaux ni dans les erreurs : le corps des réponses
//! n'est jamais recopié.

use std::time::Duration;

use norelly_domain::address::EmailAddress;
use norelly_domain::credentials::Secret;
use norelly_domain::oauth::{
    AuthorizationRequest, CallbackError, OAuthConfig, OAuthIdentity, OAuthProvider, TokenError,
    authorization_url, classify_token_error, code_exchange_form, parse_callback, refresh_form,
};
use norelly_domain::ports::{OAuthClient, OAuthGrant};
use norelly_tls::TlsClientFactory;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Attente maximale du retour du navigateur.
pub const AUTHORIZE_TIMEOUT: Duration = Duration::from_secs(300);
/// Délai d'une requête au point d'accès des jetons.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Taille maximale lue d'une requête du navigateur.
const MAX_CALLBACK_BYTES: usize = 8 * 1024;
/// Taille maximale d'une réponse du point d'accès des jetons.
const MAX_TOKEN_RESPONSE_BYTES: usize = 64 * 1024;

/// Page renvoyée au navigateur (texte neutre, bilingue, sans donnée).
const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Norelly</title>\
<p>Norelly : vous pouvez fermer cette page.</p><p>Norelly: you can close this page.</p>";

/// Ouverture d'une URL dans le navigateur du système.
pub trait BrowserOpener: Send + Sync {
    fn open(&self, url: &str) -> Result<(), String>;
}

/// Erreur de construction du client.
#[derive(Debug)]
pub struct OAuthSetupError(pub String);

impl std::fmt::Display for OAuthSetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "client OAuth impossible à construire : {}", self.0)
    }
}

/// Client OAuth.
#[derive(Debug)]
pub struct HttpOAuthClient<B> {
    http: reqwest::Client,
    browser: B,
    /// Point d'accès des jetons imposé : **tests uniquement**.
    token_url: Option<String>,
    authorize_timeout: Duration,
}

impl<B: BrowserOpener> HttpOAuthClient<B> {
    /// HTTPS seulement, sans redirection, vérification de la plateforme.
    pub fn new(tls: &TlsClientFactory, browser: B) -> Result<Self, OAuthSetupError> {
        let config = tls
            .standard_client_config()
            .map_err(|e| OAuthSetupError(e.to_string()))?;
        let http = reqwest::Client::builder()
            .use_preconfigured_tls(config)
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .user_agent("Norelly")
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| OAuthSetupError(e.to_string()))?;
        Ok(Self {
            http,
            browser,
            token_url: None,
            authorize_timeout: AUTHORIZE_TIMEOUT,
        })
    }

    /// Client en HTTP clair vers `token_url` : **tests uniquement**
    /// (`wiremock` ne sert pas de HTTPS).
    #[doc(hidden)]
    pub fn plain_http_for_tests(
        browser: B,
        token_url: String,
        authorize_timeout: Duration,
    ) -> Result<Self, OAuthSetupError> {
        let config = TlsClientFactory::platform()
            .standard_client_config()
            .map_err(|e| OAuthSetupError(e.to_string()))?;
        let http = reqwest::Client::builder()
            .use_preconfigured_tls(config)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| OAuthSetupError(e.to_string()))?;
        Ok(Self {
            http,
            browser,
            token_url: Some(token_url),
            authorize_timeout,
        })
    }

    fn token_url<'a>(&'a self, config: &'a OAuthConfig) -> &'a str {
        self.token_url.as_deref().unwrap_or(config.token_url)
    }

    async fn authorize_impl(
        &self,
        provider: OAuthProvider,
        login_hint: &EmailAddress,
    ) -> Result<OAuthGrant, TokenError> {
        let config = provider.config();
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| TokenError::Network(e.to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|e| TokenError::Network(e.to_string()))?
            .port();
        // `localhost` : seule forme enregistrée chez Microsoft pour une
        // application de bureau (port libre) ; écoute sur 127.0.0.1.
        let redirect_uri = format!("http://localhost:{port}/");
        let verifier = Secret::new(random_token(32)?);
        let state = random_token(16)?;
        let challenge = base64url(&Sha256::digest(verifier.expose().as_bytes()));
        let url = authorization_url(
            config,
            &AuthorizationRequest {
                redirect_uri: &redirect_uri,
                state: &state,
                code_challenge: &challenge,
                login_hint,
            },
        );
        self.browser.open(&url).map_err(TokenError::Network)?;
        let code = tokio::time::timeout(self.authorize_timeout, wait_for_code(&listener, &state))
            .await
            .map_err(|_| TokenError::Cancelled("délai dépassé".to_owned()))??;
        drop(listener);
        let form = code_exchange_form(config, &code, &redirect_uri, &verifier);
        self.post_token(config, form).await
    }

    async fn refresh_impl(
        &self,
        provider: OAuthProvider,
        refresh_token: &Secret,
    ) -> Result<OAuthGrant, TokenError> {
        let config = provider.config();
        self.post_token(config, refresh_form(config, refresh_token))
            .await
    }

    async fn post_token(
        &self,
        config: &OAuthConfig,
        form: Secret,
    ) -> Result<OAuthGrant, TokenError> {
        let response = self
            .http
            .post(self.token_url(config))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(form.expose().to_owned())
            .send()
            .await
            .map_err(|e| TokenError::Network(e.without_url().to_string()))?;
        let status = response.status().as_u16();
        let body = read_bounded(response).await?;
        parse_token_response(status, &body)
    }
}

impl<B: BrowserOpener> OAuthClient for HttpOAuthClient<B> {
    async fn authorize(
        &self,
        provider: OAuthProvider,
        login_hint: &EmailAddress,
    ) -> Result<OAuthGrant, TokenError> {
        self.authorize_impl(provider, login_hint).await
    }

    async fn refresh(
        &self,
        provider: OAuthProvider,
        refresh_token: &Secret,
    ) -> Result<OAuthGrant, TokenError> {
        self.refresh_impl(provider, refresh_token).await
    }
}

/// Attend la requête du navigateur portant le bon `state`. Un retour
/// forgé ou illisible (autre page locale, favicon) reçoit une erreur et
/// l'attente continue : il ne peut pas annuler la connexion.
async fn wait_for_code(listener: &TcpListener, state: &str) -> Result<Secret, TokenError> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| TokenError::Network(e.to_string()))?;
        let Some(target) = read_request_target(&mut stream).await else {
            respond(&mut stream, "400 Bad Request", "").await;
            continue;
        };
        match parse_callback(&target, state) {
            Ok(code) => {
                respond(&mut stream, "200 OK", DONE_PAGE).await;
                return Ok(code);
            }
            Err(CallbackError::Denied(reason)) => {
                respond(&mut stream, "200 OK", DONE_PAGE).await;
                return Err(TokenError::Cancelled(reason));
            }
            Err(CallbackError::MissingCode) => {
                respond(&mut stream, "400 Bad Request", "").await;
                return Err(TokenError::Protocol("retour sans code".to_owned()));
            }
            Err(CallbackError::StateMismatch | CallbackError::Malformed) => {
                respond(&mut stream, "400 Bad Request", "").await;
            }
        }
    }
}

/// Cible d'une requête `GET <cible> HTTP/1.x`, lue dans la limite de
/// [`MAX_CALLBACK_BYTES`].
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(chunk.get(..n)?);
        if buf.windows(2).any(|w| w == b"\r\n") {
            break;
        }
        if buf.len() > MAX_CALLBACK_BYTES {
            return None;
        }
    }
    let line = buf.split(|b| *b == b'\r').next()?;
    let line = std::str::from_utf8(line).ok()?;
    let mut parts = line.split(' ');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("GET"), Some(target), Some(version)) if version.starts_with("HTTP/1.") => {
            Some(target.to_owned())
        }
        _ => None,
    }
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn read_bounded(mut response: reqwest::Response) -> Result<Vec<u8>, TokenError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| TokenError::Network(e.without_url().to_string()))?
    {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_TOKEN_RESPONSE_BYTES {
            return Err(TokenError::Protocol("réponse trop longue".to_owned()));
        }
    }
    Ok(body)
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    scope: Option<String>,
    error: Option<String>,
}

/// Revendications utiles d'un `id_token` (OpenID Connect).
#[derive(serde::Deserialize)]
struct IdTokenClaims {
    preferred_username: Option<String>,
    email: Option<String>,
    iss: Option<String>,
    /// Chaîne ou liste de chaînes (OpenID Connect).
    aud: Option<serde_json::Value>,
    /// Booléen, ou chaîne `"true"`/`"false"` (déjà vu chez Google).
    email_verified: Option<serde_json::Value>,
}

/// Identité portée par un `id_token` (JWT `en-tête.contenu.signature`).
/// La signature n'est pas vérifiée : le jeton vient directement du point
/// d'accès des jetons, en HTTPS vérifié, en échange du code et du
/// vérificateur PKCE. `None` si le jeton est illisible.
fn identity_of(id_token: &str) -> Option<OAuthIdentity> {
    let mut parts = id_token.split('.');
    let (Some(_), Some(payload), Some(_), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let claims: IdTokenClaims = serde_json::from_slice(&base64url_decode(payload)?).ok()?;
    Some(OAuthIdentity {
        preferred_username: claims.preferred_username,
        email: claims.email,
        issuer: claims.iss,
        audience: match claims.aud {
            Some(serde_json::Value::String(aud)) => vec![aud],
            Some(serde_json::Value::Array(auds)) => auds
                .into_iter()
                .filter_map(|a| a.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        },
        email_verified: match claims.email_verified {
            Some(serde_json::Value::Bool(v)) => Some(v),
            Some(serde_json::Value::String(v)) if v == "true" => Some(true),
            Some(serde_json::Value::String(v)) if v == "false" => Some(false),
            _ => None,
        },
    })
}

/// Réponse du point d'accès des jetons (RFC 6749, sections 5.1 et 5.2).
fn parse_token_response(status: u16, body: &[u8]) -> Result<OAuthGrant, TokenError> {
    let parsed: Option<TokenResponse> = serde_json::from_slice(body).ok();
    if !(200..300).contains(&status) {
        let code = parsed.and_then(|p| p.error).filter(|c| is_error_code(c));
        return Err(classify_token_error(status, code.as_deref()));
    }
    let parsed = parsed.ok_or_else(|| TokenError::Protocol("réponse illisible".to_owned()))?;
    let access_token = parsed
        .access_token
        .filter(|t| !t.is_empty())
        .ok_or_else(|| TokenError::Protocol("réponse sans jeton d'accès".to_owned()))?;
    Ok(OAuthGrant {
        access_token: Secret::new(access_token),
        expires_in: parsed.expires_in.unwrap_or(3_600),
        refresh_token: parsed
            .refresh_token
            .filter(|t| !t.is_empty())
            .map(Secret::new),
        identity: parsed.id_token.as_deref().and_then(identity_of),
        granted_scopes: parsed.scope,
    })
}

/// Code d'erreur RFC 6749 : court, ASCII imprimable sans guillemet ni
/// barre oblique inverse (il finit dans une erreur affichée).
fn is_error_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|b| (0x20..=0x7e).contains(&b) && b != b'"' && b != b'\\')
}

/// `bytes` octets d'aléa cryptographique, encodés en base64url.
fn random_token(bytes: usize) -> Result<String, TokenError> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| TokenError::Network(e.to_string()))?;
    Ok(base64url(&buf))
}

/// Inverse de [`base64url`] : `None` si un caractère n'est pas de
/// l'alphabet base64url (remplissage compris) ou si la longueur est
/// impossible.
fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

/// base64url sans remplissage (RFC 4648, section 5).
fn base64url(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..=chunk.len() {
            let index = (n >> (18 - 6 * i)) & 0x3f;
            out.push(char::from(ALPHABET[index as usize]));
        }
    }
    out
}

#[cfg(test)]
mod tests;
