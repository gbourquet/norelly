//! Adaptateur OAuth 2.0 pour application de bureau (RFC 8252) : ouverture
//! du navigateur du système, retour sur une redirection locale
//! (`localhost`, port éphémère, écouté sur `127.0.0.1` et `::1`), code
//! d'autorisation protégé par PKCE
//! (RFC 7636), échange et rafraîchissement par HTTPS avec la vérification
//! de certificats de la plateforme (ADR-002).
//!
//! Les règles (URL, formulaires, contrôle du retour, classement des
//! erreurs) sont celles du domaine ([`norelly_domain::oauth`]). Aucun jeton
//! n'apparaît dans les journaux ni dans les erreurs : le corps des réponses
//! n'est jamais recopié.

use std::io::ErrorKind;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use norelly_domain::address::EmailAddress;
use norelly_domain::credentials::{Secret, SecretBytes};
use norelly_domain::oauth::{
    AuthorizationRequest, CallbackError, OAuthConfig, OAuthIdentity, TokenError, authorization_url,
    classify_token_error, code_exchange_form, parse_callback, refresh_form,
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
/// Délai de lecture d'une requête du navigateur, entière, puis d'écriture
/// de la réponse : une connexion locale lente est coupée.
const CALLBACK_IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Essais d'un nouveau port quand celui de `127.0.0.1` est pris sur `::1`.
const BIND_ATTEMPTS: usize = 5;
/// Connexions locales lues en même temps ; les suivantes attendent dans la
/// file du système qu'une place se libère (au plus deux délais d'E/S).
const MAX_CONNECTIONS: usize = 32;
/// Pause après une erreur passagère d'`accept` (trop de fichiers ouverts…).
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(100);
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
    #[cfg(test)]
    pub(crate) fn plain_http_for_tests(
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
        config: &OAuthConfig,
        login_hint: &EmailAddress,
    ) -> Result<OAuthGrant, TokenError> {
        let redirect = bind_redirect()?;
        // `localhost` : seule forme enregistrée chez Microsoft pour une
        // application de bureau (port libre). Le navigateur peut la résoudre
        // en `127.0.0.1` ou en `::1` : les deux sont écoutées (ADR-013).
        let redirect_uri = format!("http://localhost:{}/", redirect.port);
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
        let code = tokio::time::timeout(self.authorize_timeout, wait_for_code(&redirect, &state))
            .await
            .map_err(|_| TokenError::Cancelled("délai dépassé".to_owned()))??;
        drop(redirect);
        let form = code_exchange_form(config, &code, &redirect_uri, &verifier);
        self.post_token(config, form).await
    }

    async fn refresh_impl(
        &self,
        config: &OAuthConfig,
        refresh_token: &Secret,
    ) -> Result<OAuthGrant, TokenError> {
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
        parse_token_response(status, body.expose())
    }
}

impl<B: BrowserOpener> OAuthClient for HttpOAuthClient<B> {
    async fn authorize(
        &self,
        config: &OAuthConfig,
        login_hint: &EmailAddress,
    ) -> Result<OAuthGrant, TokenError> {
        self.authorize_impl(config, login_hint).await
    }

    async fn refresh(
        &self,
        config: &OAuthConfig,
        refresh_token: &Secret,
    ) -> Result<OAuthGrant, TokenError> {
        self.refresh_impl(config, refresh_token).await
    }
}

/// Écoute de la redirection : `127.0.0.1` et, si la machine a IPv6, `::1`
/// au même port.
#[derive(Debug)]
struct Redirect {
    port: u16,
    v4: TcpListener,
    v6: Option<TcpListener>,
}

fn bind_redirect() -> Result<Redirect, TokenError> {
    bind_redirect_with(|port| std::net::TcpListener::bind((Ipv6Addr::LOCALHOST, port)))
}

/// Écoute `127.0.0.1` sur un port libre, puis `::1` au même port avec
/// `bind_v6`, pour qu'aucun autre programme ne reçoive le retour adressé à
/// `localhost` :
/// - port déjà pris sur `::1` : nouveau port, au plus [`BIND_ATTEMPTS`] fois ;
/// - adresse `::1` indisponible (pas d'IPv6) : `127.0.0.1` seul ;
/// - autre erreur : échec.
fn bind_redirect_with(
    mut bind_v6: impl FnMut(u16) -> std::io::Result<std::net::TcpListener>,
) -> Result<Redirect, TokenError> {
    let network = |e: std::io::Error| TokenError::Network(e.to_string());
    for _ in 0..BIND_ATTEMPTS {
        let v4 = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(network)?;
        let port = v4.local_addr().map_err(network)?.port();
        let v6 = match bind_v6(port) {
            Ok(v6) => Some(v6),
            Err(e) if port_refused(&e) => continue,
            Err(e) if ipv6_absent(&e) => None,
            Err(e) => return Err(network(e)),
        };
        return Ok(Redirect {
            port,
            v4: tokio_listener(v4)?,
            v6: v6.map(tokio_listener).transpose()?,
        });
    }
    Err(TokenError::Network(
        "aucun port libre à la fois sur 127.0.0.1 et ::1".to_owned(),
    ))
}

/// Port inutilisable sur `::1` : déjà pris, ou réservé (Windows : plages
/// exclues, refus d'accès). Un autre port est essayé.
fn port_refused(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::AddrInUse | ErrorKind::PermissionDenied)
}

/// Machine sans IPv6 : adresse `::1` indisponible, ou famille d'adresses
/// absente du système (`EAFNOSUPPORT`, `WSAEAFNOSUPPORT`).
fn ipv6_absent(e: &std::io::Error) -> bool {
    #[cfg(target_os = "linux")]
    const EAFNOSUPPORT: i32 = 97;
    #[cfg(windows)]
    const EAFNOSUPPORT: i32 = 10047;
    #[cfg(not(any(target_os = "linux", windows)))]
    const EAFNOSUPPORT: i32 = 47;
    e.kind() == ErrorKind::AddrNotAvailable || e.raw_os_error() == Some(EAFNOSUPPORT)
}

fn tokio_listener(listener: std::net::TcpListener) -> Result<TcpListener, TokenError> {
    listener
        .set_nonblocking(true)
        .and_then(|()| TcpListener::from_std(listener))
        .map_err(|e| TokenError::Network(e.to_string()))
}

/// Connexion acceptée sur `listener`, ou jamais s'il n'y en a pas.
async fn accept_on(
    listener: Option<&TcpListener>,
) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
    match listener {
        Some(listener) => listener.accept().await,
        None => std::future::pending().await,
    }
}

/// Attend la requête du navigateur portant le bon `state`. Un retour
/// forgé ou illisible (autre page locale, favicon) reçoit une erreur et
/// l'attente continue : il ne peut pas annuler la connexion. Chaque
/// connexion est lue par sa propre tâche, dans un délai borné : un client
/// lent ne retarde pas les autres. Au plus [`MAX_CONNECTIONS`] à la fois :
/// un afflux de connexions ne fait pas manquer de descripteurs ; une erreur
/// d'`accept` est passagère et n'arrête pas l'attente. Abandonner ce futur
/// ferme l'écoute et interrompt ces tâches.
async fn wait_for_code(redirect: &Redirect, state: &str) -> Result<Secret, TokenError> {
    let state: Arc<str> = Arc::from(state);
    let mut connections = tokio::task::JoinSet::new();
    loop {
        let room = connections.len() < MAX_CONNECTIONS;
        let accepted = tokio::select! {
            accepted = redirect.v4.accept(), if room => accepted,
            accepted = accept_on(redirect.v6.as_ref()), if room => accepted,
            Some(done) = connections.join_next() => {
                if let Ok(Some(result)) = done {
                    return result;
                }
                continue;
            }
        };
        match accepted {
            Ok((stream, _)) => {
                connections.spawn(handle_callback(stream, Arc::clone(&state)));
            }
            Err(e) => {
                tracing::debug!(error = %e, "connexion locale non acceptée");
                tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
            }
        }
    }
}

/// Lit une requête du navigateur et y répond. `Some` : l'attente s'achève
/// (code reçu, refus, retour sans code) ; `None` : requête ignorée.
async fn handle_callback(
    mut stream: TcpStream,
    state: Arc<str>,
) -> Option<Result<Secret, TokenError>> {
    let target = tokio::time::timeout(CALLBACK_IO_TIMEOUT, read_request_target(&mut stream))
        .await
        .ok()
        .flatten();
    let Some(target) = target else {
        respond(&mut stream, "400 Bad Request", "").await;
        return None;
    };
    match parse_callback(target.expose(), &state) {
        Ok(code) => {
            respond(&mut stream, "200 OK", DONE_PAGE).await;
            Some(Ok(code))
        }
        Err(CallbackError::Denied(reason)) => {
            respond(&mut stream, "200 OK", DONE_PAGE).await;
            Some(Err(TokenError::Cancelled(reason)))
        }
        Err(CallbackError::MissingCode) => {
            respond(&mut stream, "400 Bad Request", "").await;
            Some(Err(TokenError::Protocol("retour sans code".to_owned())))
        }
        Err(CallbackError::StateMismatch | CallbackError::Malformed) => {
            respond(&mut stream, "400 Bad Request", "").await;
            None
        }
    }
}

/// Cible d'une requête `GET <cible> HTTP/1.x`, lue dans la limite de
/// [`MAX_CALLBACK_BYTES`].
/// La requête (qui porte le code d'autorisation) est effacée après lecture.
async fn read_request_target(stream: &mut TcpStream) -> Option<Secret> {
    let mut buf = Vec::with_capacity(MAX_CALLBACK_BYTES + 1024);
    let mut chunk = [0u8; 1024];
    let complete = fill_request_line(stream, &mut buf, &mut chunk).await;
    chunk.fill(0);
    std::hint::black_box(&chunk);
    let buf = SecretBytes::new(buf);
    if !complete {
        return None;
    }
    let line = buf.expose().split(|b| *b == b'\r').next()?;
    let line = std::str::from_utf8(line).ok()?;
    let mut parts = line.split(' ');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("GET"), Some(target), Some(version)) if version.starts_with("HTTP/1.") => {
            Some(Secret::new(target.to_owned()))
        }
        _ => None,
    }
}

/// Lit jusqu'à la fin de la première ligne ; faux si la connexion se ferme
/// avant ou si [`MAX_CALLBACK_BYTES`] est dépassé. Le tampon est réservé
/// d'emblée : aucune réallocation n'en laisse de copie.
async fn fill_request_line(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
    chunk: &mut [u8; 1024],
) -> bool {
    loop {
        let Ok(n) = stream.read(chunk).await else {
            return false;
        };
        let Some(read) = chunk.get(..n).filter(|r| !r.is_empty()) else {
            return false;
        };
        buf.extend_from_slice(read);
        if buf.windows(2).any(|w| w == b"\r\n") {
            return true;
        }
        if buf.len() > MAX_CALLBACK_BYTES {
            return false;
        }
    }
}

/// Répond puis ferme, dans le délai de [`CALLBACK_IO_TIMEOUT`] : un client
/// qui ne lit pas sa réponse ne retient pas la tâche.
async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = tokio::time::timeout(CALLBACK_IO_TIMEOUT, async {
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
    })
    .await;
}

/// Corps de la réponse du point d'accès des jetons, lu dans la limite de
/// [`MAX_TOKEN_RESPONSE_BYTES`]. Ce tampon assemblé est effacé à sa
/// destruction, même après une erreur de lecture, et réservé d'emblée
/// (aucune réallocation n'en laisse de copie). Les morceaux reçus du client
/// HTTP, eux, ne sont pas effacés.
async fn read_bounded(response: reqwest::Response) -> Result<SecretBytes, TokenError> {
    let mut body = Vec::with_capacity(MAX_TOKEN_RESPONSE_BYTES);
    let read = fill_bounded(response, &mut body).await;
    let body = SecretBytes::new(body);
    read.map(|()| body)
}

async fn fill_bounded(
    mut response: reqwest::Response,
    body: &mut Vec<u8>,
) -> Result<(), TokenError> {
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| TokenError::Network(e.without_url().to_string()))?
    {
        if chunk.len() > MAX_TOKEN_RESPONSE_BYTES.saturating_sub(body.len()) {
            return Err(TokenError::Protocol("réponse trop longue".to_owned()));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    #[serde(default, deserialize_with = "secret")]
    access_token: Option<Secret>,
    expires_in: Option<u64>,
    #[serde(default, deserialize_with = "secret")]
    refresh_token: Option<Secret>,
    #[serde(default, deserialize_with = "secret")]
    id_token: Option<Secret>,
    scope: Option<String>,
    error: Option<String>,
}

/// Jeton lu directement en [`Secret`] : effacé à sa destruction, même si
/// la réponse est rejetée ensuite.
fn secret<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Secret>, D::Error> {
    Ok(<Option<String> as serde::Deserialize>::deserialize(d)?.map(Secret::new))
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
        access_token,
        expires_in: parsed.expires_in.unwrap_or(3_600),
        refresh_token: parsed.refresh_token.filter(|t| !t.is_empty()),
        identity: parsed
            .id_token
            .as_ref()
            .and_then(|t| identity_of(t.expose())),
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
