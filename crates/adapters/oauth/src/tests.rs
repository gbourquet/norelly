//! Tests de l'adaptateur OAuth : faux navigateur qui suit l'URL comme
//! l'utilisateur, point d'accès des jetons simulé par wiremock.

use std::sync::{Arc, Mutex};

use norelly_test_support::contracts::oauth_client_contract;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use norelly_domain::oauth::{MICROSOFT_CONSUMERS_TENANT, OAuthProvider, OAuthSubject};

/// Visite du faux navigateur : port de la redirection et `state` reçus.
type Visit = Arc<dyn Fn(u16, String) + Send + Sync>;

/// Faux navigateur : retient l'URL et renvoie au client un retour, comme
/// après une connexion réussie (`code`) ou un refus.
#[derive(Clone)]
struct Browser {
    seen: Arc<Mutex<Vec<String>>>,
    visit: Visit,
}

impl Browser {
    /// Requêtes envoyées l'une après l'autre à la redirection, sur
    /// `127.0.0.1` ; `{state}` est remplacé.
    fn new(callbacks: Vec<&'static str>) -> Self {
        Self::with(move |port, state| {
            let callbacks: Vec<String> = callbacks
                .iter()
                .map(|c| c.replace("{state}", &state))
                .collect();
            tokio::spawn(async move {
                for target in callbacks {
                    let _ = get(LOCALHOST_V4, port, &target).await;
                }
            });
        })
    }
    /// Visite libre : `visit` reçoit le port et le `state`.
    fn with(visit: impl Fn(u16, String) + Send + Sync + 'static) -> Self {
        Self {
            seen: Arc::default(),
            visit: Arc::new(visit),
        }
    }
    fn url(&self) -> String {
        self.seen
            .lock()
            .ok()
            .and_then(|s| s.first().cloned())
            .unwrap_or_default()
    }
}

const LOCALHOST_V4: std::net::IpAddr = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
const LOCALHOST_V6: std::net::IpAddr = std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST);

/// Requête `GET target` vers `ip:port` ; réponse complète, ou `None` si la
/// connexion est refusée.
async fn get(ip: std::net::IpAddr, port: u16, target: &str) -> Option<String> {
    let mut s = TcpStream::connect((ip, port)).await.ok()?;
    s.write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
        .await
        .ok()?;
    let mut response = Vec::new();
    let _ = s.read_to_end(&mut response).await;
    Some(String::from_utf8_lossy(&response).into_owned())
}

/// Vrai si cette machine a une boucle locale IPv6.
fn ipv6_available() -> bool {
    std::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0)).is_ok()
}

/// Expéditeur à usage unique partagé par les visites du faux navigateur.
type Once<T> = Arc<Mutex<Option<tokio::sync::oneshot::Sender<T>>>>;

fn once<T>() -> (Once<T>, tokio::sync::oneshot::Receiver<T>) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    (Arc::new(Mutex::new(Some(tx))), rx)
}

fn fire<T>(once: &Once<T>, value: T) {
    if let Some(tx) = once.lock().ok().and_then(|mut t| t.take()) {
        let _ = tx.send(value);
    }
}

fn param<'a>(url: &'a str, name: &str) -> Option<&'a str> {
    url.split_once('?')?
        .1
        .split('&')
        .find_map(|p| p.strip_prefix(name)?.strip_prefix('='))
}

impl BrowserOpener for Browser {
    fn open(&self, url: &str) -> Result<(), String> {
        if let Ok(mut s) = self.seen.lock() {
            s.push(url.to_owned());
        }
        let state = param(url, "state").unwrap_or_default().to_owned();
        let redirect = param(url, "redirect_uri").unwrap_or_default();
        let port: u16 = redirect
            .trim_start_matches("http%3A%2F%2Flocalhost%3A")
            .trim_end_matches("%2F")
            .parse()
            .map_err(|_| format!("redirection inattendue : {redirect}"))?;
        (self.visit)(port, state);
        Ok(())
    }
}

/// `id_token` non signé portant `claims` (JSON).
fn id_token(claims: &str) -> String {
    format!(
        "{}.{}.c2ln",
        base64url(br#"{"alg":"RS256","typ":"JWT"}"#),
        base64url(claims.as_bytes())
    )
}

/// Revendications de l'`id_token` d'`alice@outlook.com`, comme les émet
/// Microsoft pour un compte personnel.
fn microsoft_claims() -> String {
    format!(
        r#"{{"iss":"https://login.microsoftonline.com/{MICROSOFT_CONSUMERS_TENANT}/v2.0","aud":"{}","tid":"{MICROSOFT_CONSUMERS_TENANT}","preferred_username":"alice@outlook.com","email":"alice@outlook.com","oid":"00000000-0000-0000-66f3-3332eca7ea81","sub":"x"}}"#,
        OAuthProvider::Microsoft.config().client_id
    )
}

/// Vrai si le `code_verifier` du formulaire est celui dont le haché a été
/// envoyé dans l'URL ouverte par `browser` (PKCE, RFC 7636).
fn pkce_matches(browser: &Browser, request: &wiremock::Request) -> bool {
    let url = browser.url();
    let Some(challenge) = param(&url, "code_challenge") else {
        return false;
    };
    let body = String::from_utf8_lossy(&request.body);
    body.split('&')
        .find_map(|p| p.strip_prefix("code_verifier="))
        .is_some_and(|verifier| base64url(&Sha256::digest(verifier.as_bytes())) == challenge)
}

/// Point d'accès des jetons ; l'échange du code n'aboutit que si le
/// vérificateur PKCE correspond à l'URL ouverte par `browser`.
async fn token_server(browser: &Browser) -> MockServer {
    let server = MockServer::start().await;
    let seen = browser.clone();
    let body = format!(
        r#"{{"access_token":"acc","expires_in":3599,"refresh_token":"r1","token_type":"Bearer","id_token":"{}"}}"#,
        id_token(&microsoft_claims())
    );
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=le-code"))
        .and(move |r: &wiremock::Request| pkce_matches(&seen, r))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("refresh_token=r1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"acc2","expires_in":3599,"refresh_token":"r2"}"#,
            ),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("refresh_token=revoked"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"error":"invalid_grant","error_description":"AADSTS70000 jeton revoked"}"#,
        ))
        .mount(&server)
        .await;
    server
}

fn client(server: &MockServer, browser: Browser) -> HttpOAuthClient<Browser> {
    client_waiting(server, browser, Duration::from_secs(5))
}

/// Client qui attend le retour du navigateur au plus `authorize_timeout`.
fn client_waiting(
    server: &MockServer,
    browser: Browser,
    authorize_timeout: Duration,
) -> HttpOAuthClient<Browser> {
    let Ok(c) = HttpOAuthClient::plain_http_for_tests(
        browser,
        format!("{}/token", server.uri()),
        authorize_timeout,
    ) else {
        panic!("client de test impossible");
    };
    c
}

fn hint() -> EmailAddress {
    let Ok(a) = EmailAddress::parse("alice@outlook.com") else {
        panic!("adresse valide refusée");
    };
    a
}

#[tokio::test]
async fn authorize_opens_browser_and_exchanges_code() {
    // Un retour forgé (mauvais state) puis un favicon sont ignorés.
    let browser = Browser::new(vec![
        "/?code=forge&state=autre",
        "/favicon.ico",
        "/?code=le-code&state={state}",
    ]);
    let server = token_server(&browser).await;
    let grant = client(&server, browser.clone())
        .authorize(OAuthProvider::Microsoft.config(), &hint())
        .await;
    let Ok(grant) = grant else {
        panic!("connexion refusée : {grant:?}");
    };
    assert_eq!(grant.access_token.expose(), "acc");
    assert_eq!(grant.expires_in, 3599);
    assert_eq!(grant.refresh_token.as_ref().map(Secret::expose), Some("r1"));
    assert_eq!(
        grant.identity,
        Some(OAuthIdentity {
            email: Some("alice@outlook.com".to_owned()),
            subject: OAuthSubject::parse("00000000-0000-0000-66f3-3332eca7ea81"),
            ..norelly_test_support::contracts::microsoft_identity("alice@outlook.com")
        })
    );
    let url = browser.url();
    assert!(param(&url, "scope").is_some_and(|s| s.contains("openid")));
    assert!(url.starts_with(OAuthProvider::Microsoft.config().authorize_url));
    assert_eq!(param(&url, "code_challenge_method"), Some("S256"));
    assert!(param(&url, "code_challenge").is_some_and(|c| c.len() == 43));
    assert!(param(&url, "state").is_some_and(|s| s.len() >= 22));
}

#[tokio::test]
async fn denied_authorization_is_cancelled() {
    let browser = Browser::new(vec!["/?error=access_denied&state={state}"]);
    let server = token_server(&browser).await;
    let result = client(&server, browser)
        .authorize(OAuthProvider::Microsoft.config(), &hint())
        .await;
    assert_eq!(
        result.map(|_| ()),
        Err(TokenError::Cancelled("access_denied".to_owned()))
    );
}

#[tokio::test]
async fn authorize_receives_the_callback_on_ipv6_loopback() {
    // `localhost` peut se résoudre en `::1` : le retour y est reçu, aucun
    // autre programme ne peut occuper ce port à la place de Norelly.
    if !ipv6_available() {
        return;
    }
    let browser = Browser::with(|port, state| {
        tokio::spawn(async move {
            let _ = get(LOCALHOST_V6, port, &format!("/?code=le-code&state={state}")).await;
        });
    });
    let server = token_server(&browser).await;
    let grant = client(&server, browser)
        .authorize(OAuthProvider::Microsoft.config(), &hint())
        .await;
    assert_eq!(
        grant.map(|g| g.access_token.expose().to_owned()),
        Ok("acc".to_owned())
    );
}

#[tokio::test]
async fn a_slow_local_client_neither_blocks_nor_holds_the_wait() {
    // Un programme local ouvre une connexion et envoie un octet par
    // seconde : le vrai retour, arrivé pendant ce temps, est reçu, et la
    // connexion lente est coupée au bout du délai de lecture de la requête.
    let (closed_tx, closed_rx) = once();
    let browser = Browser::with(move |port, state| {
        let closed_tx = Arc::clone(&closed_tx);
        tokio::spawn(async move {
            let Ok(slow) = TcpStream::connect((LOCALHOST_V4, port)).await else {
                return;
            };
            let started = tokio::time::Instant::now();
            let (mut reader, mut writer) = slow.into_split();
            let trickle = tokio::spawn(async move {
                for byte in b"GET /?code=lent".iter().cycle().take(60) {
                    if writer.write_all(&[*byte]).await.is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
            let mut sink = Vec::new();
            let _ = reader.read_to_end(&mut sink).await;
            trickle.abort();
            fire(&closed_tx, started.elapsed());
        });
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = get(LOCALHOST_V4, port, &format!("/?code=le-code&state={state}")).await;
        });
    });
    let server = token_server(&browser).await;
    let grant = client_waiting(&server, browser, Duration::from_secs(3))
        .authorize(OAuthProvider::Microsoft.config(), &hint())
        .await;
    assert_eq!(
        grant.map(|g| g.access_token.expose().to_owned()),
        Ok("acc".to_owned())
    );
    let closed = tokio::time::timeout(Duration::from_secs(10), closed_rx).await;
    assert!(
        matches!(closed, Ok(Ok(elapsed)) if elapsed < Duration::from_secs(8)),
        "connexion lente non coupée : {closed:?}"
    );
}

#[tokio::test]
async fn a_flood_of_silent_connections_does_not_stop_the_callback() {
    // Plus de connexions muettes que de places : elles sont coupées à leur
    // délai, la liste d'attente du système garde les suivantes, et le vrai
    // retour finit par être lu.
    let browser = Browser::with(|port, state| {
        tokio::spawn(async move {
            let mut silent = Vec::new();
            for _ in 0..MAX_CONNECTIONS + 8 {
                if let Ok(s) = TcpStream::connect((LOCALHOST_V4, port)).await {
                    silent.push(s);
                }
            }
            let _ = get(LOCALHOST_V4, port, &format!("/?code=le-code&state={state}")).await;
            drop(silent);
        });
    });
    let server = token_server(&browser).await;
    let grant = client_waiting(&server, browser, CALLBACK_IO_TIMEOUT * 3)
        .authorize(OAuthProvider::Microsoft.config(), &hint())
        .await;
    assert!(grant.is_ok(), "{grant:?}");
}

#[tokio::test]
async fn an_oversized_callback_is_refused_and_the_wait_goes_on() {
    let (refused_tx, refused_rx) = once();
    let browser = Browser::with(move |port, state| {
        let refused_tx = Arc::clone(&refused_tx);
        tokio::spawn(async move {
            let long = format!("/?pad={}", "a".repeat(MAX_CALLBACK_BYTES + 1));
            fire(&refused_tx, get(LOCALHOST_V4, port, &long).await);
            let _ = get(LOCALHOST_V4, port, &format!("/?code=le-code&state={state}")).await;
        });
    });
    let server = token_server(&browser).await;
    let grant = client(&server, browser)
        .authorize(OAuthProvider::Microsoft.config(), &hint())
        .await;
    assert!(grant.is_ok(), "{grant:?}");
    let refused = refused_rx.await.ok().flatten().unwrap_or_default();
    assert!(refused.starts_with("HTTP/1.1 400"), "{refused}");
}

#[tokio::test]
async fn authorize_gives_up_after_its_timeout() {
    let browser = Browser::new(vec![]);
    let server = token_server(&browser).await;
    let result = client_waiting(&server, browser, Duration::from_millis(200))
        .authorize(OAuthProvider::Microsoft.config(), &hint())
        .await;
    assert_eq!(
        result.map(|_| ()),
        Err(TokenError::Cancelled("délai dépassé".to_owned()))
    );
}

#[tokio::test]
async fn dropping_authorize_closes_the_redirect() {
    // Fenêtre d'ajout fermée : le futur est abandonné, plus rien n'écoute,
    // un vieil onglet ne livre plus rien.
    let (port_tx, port_rx) = once();
    let browser = Browser::with(move |port, _| fire(&port_tx, port));
    let server = token_server(&browser).await;
    let c = client(&server, browser);
    let hint = hint();
    let port = {
        let authorize = c.authorize(OAuthProvider::Microsoft.config(), &hint);
        tokio::pin!(authorize);
        tokio::select! {
            _ = &mut authorize => panic!("connexion terminée sans retour"),
            port = port_rx => port.unwrap_or_default(),
        }
    };
    assert_ne!(port, 0);
    assert!(TcpStream::connect((LOCALHOST_V4, port)).await.is_err());
    if ipv6_available() {
        assert!(TcpStream::connect((LOCALHOST_V6, port)).await.is_err());
    }
}

#[tokio::test]
async fn an_oversized_token_response_is_refused() {
    let server = MockServer::start().await;
    let padding = "a".repeat(MAX_TOKEN_RESPONSE_BYTES);
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!(r#"{{"access_token":"acc","pad":"{padding}"}}"#)),
        )
        .mount(&server)
        .await;
    let refreshed = client(&server, Browser::new(vec![]))
        .refresh(
            OAuthProvider::Microsoft.config(),
            &Secret::new("r1".to_owned()),
        )
        .await;
    assert_eq!(
        refreshed.map(|_| ()),
        Err(TokenError::Protocol("réponse trop longue".to_owned()))
    );
}

#[tokio::test]
async fn a_trickling_request_is_cut_after_the_read_timeout() {
    // Un octet par seconde : chaque lecture aboutit, mais la requête
    // entière dépasse le délai ; la connexion reçoit un refus.
    let Ok(listener) = TcpListener::bind((LOCALHOST_V4, 0)).await else {
        panic!("écoute impossible");
    };
    let Ok(addr) = listener.local_addr() else {
        panic!("adresse inconnue");
    };
    let client = tokio::spawn(async move {
        let Ok(stream) = TcpStream::connect(addr).await else {
            return String::new();
        };
        let (mut reader, mut writer) = stream.into_split();
        let trickle = tokio::spawn(async move {
            for byte in b"GET /?code=lent".iter().cycle().take(60) {
                if writer.write_all(&[*byte]).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        let mut response = Vec::new();
        let _ = reader.read_to_end(&mut response).await;
        trickle.abort();
        String::from_utf8_lossy(&response).into_owned()
    });
    let Ok((stream, _)) = listener.accept().await else {
        panic!("connexion non reçue");
    };
    let started = tokio::time::Instant::now();
    let handled = handle_callback(stream, Arc::from("state")).await;
    assert!(handled.is_none());
    let elapsed = started.elapsed();
    assert!(
        elapsed >= CALLBACK_IO_TIMEOUT && elapsed < CALLBACK_IO_TIMEOUT * 2,
        "{elapsed:?}"
    );
    let response = client.await.unwrap_or_default();
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
}

/// Liaison IPv6 simulée : erreurs rendues dans l'ordre, puis vraie liaison.
fn v6_binder(
    errors: Vec<std::io::ErrorKind>,
    calls: Arc<Mutex<Vec<u16>>>,
) -> impl FnMut(u16) -> std::io::Result<std::net::TcpListener> {
    let mut errors = errors.into_iter();
    move |port| {
        if let Ok(mut c) = calls.lock() {
            c.push(port);
        }
        match errors.next() {
            Some(kind) => Err(std::io::Error::from(kind)),
            None => std::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, port)),
        }
    }
}

fn port_of(listener: &TcpListener) -> Option<u16> {
    listener.local_addr().ok().map(|a| a.port())
}

#[tokio::test]
async fn redirect_listens_on_both_loopbacks_at_the_same_port() {
    if !ipv6_available() {
        return;
    }
    let Ok(redirect) = bind_redirect() else {
        panic!("écoute impossible");
    };
    assert_eq!(port_of(&redirect.v4), Some(redirect.port));
    assert_eq!(redirect.v6.as_ref().and_then(port_of), Some(redirect.port));
}

#[tokio::test]
async fn a_port_taken_on_ipv6_is_given_up_for_another() {
    if !ipv6_available() {
        return;
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let Ok(redirect) = bind_redirect_with(v6_binder(
        vec![std::io::ErrorKind::AddrInUse],
        Arc::clone(&calls),
    )) else {
        panic!("écoute impossible");
    };
    let calls = calls.lock().map(|c| c.clone()).unwrap_or_default();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls.last(), Some(&redirect.port));
    assert_eq!(redirect.v6.as_ref().and_then(port_of), Some(redirect.port));
}

#[tokio::test]
async fn without_ipv6_the_redirect_listens_on_ipv4_only() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let Ok(redirect) = bind_redirect_with(v6_binder(
        vec![std::io::ErrorKind::AddrNotAvailable],
        Arc::clone(&calls),
    )) else {
        panic!("écoute impossible");
    };
    assert!(redirect.v6.is_none());
    assert_eq!(port_of(&redirect.v4), Some(redirect.port));
    assert_eq!(calls.lock().map(|c| c.len()).unwrap_or_default(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn a_kernel_without_ipv6_listens_on_ipv4_only() {
    // `ipv6.disable=1` : la famille d'adresses elle-même est absente.
    #[cfg(target_os = "linux")]
    const EAFNOSUPPORT: i32 = 97;
    #[cfg(not(target_os = "linux"))]
    const EAFNOSUPPORT: i32 = 47;
    let Ok(redirect) = bind_redirect_with(|_| Err(std::io::Error::from_raw_os_error(EAFNOSUPPORT)))
    else {
        panic!("écoute impossible");
    };
    assert!(redirect.v6.is_none());
}

#[tokio::test]
async fn a_port_refused_on_ipv6_is_given_up_for_another() {
    // Windows : port réservé sur ::1 (WSAEACCES, refus d'accès).
    if !ipv6_available() {
        return;
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let Ok(redirect) = bind_redirect_with(v6_binder(
        vec![std::io::ErrorKind::PermissionDenied],
        Arc::clone(&calls),
    )) else {
        panic!("écoute impossible");
    };
    assert_eq!(calls.lock().map(|c| c.len()).unwrap_or_default(), 2);
    assert!(redirect.v6.is_some());
}

#[tokio::test]
async fn other_ipv6_errors_and_a_port_always_taken_fail() {
    let denied = bind_redirect_with(v6_binder(
        vec![std::io::ErrorKind::InvalidInput],
        Arc::default(),
    ));
    assert!(matches!(denied, Err(TokenError::Network(_))));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let always_taken = bind_redirect_with(v6_binder(
        vec![std::io::ErrorKind::AddrInUse; BIND_ATTEMPTS + 1],
        Arc::clone(&calls),
    ));
    assert!(matches!(always_taken, Err(TokenError::Network(_))));
    assert_eq!(
        calls.lock().map(|c| c.len()).unwrap_or_default(),
        BIND_ATTEMPTS
    );
}

#[tokio::test]
async fn refresh_classifies_provider_errors_without_echoing_the_body() {
    let browser = Browser::new(vec![]);
    let server = token_server(&browser).await;
    let c = client(&server, browser);
    let refreshed = c
        .refresh(
            OAuthProvider::Microsoft.config(),
            &Secret::new("r1".to_owned()),
        )
        .await;
    assert_eq!(
        refreshed.map(|g| g.access_token.expose().to_owned()),
        Ok("acc2".to_owned())
    );
    let revoked = c
        .refresh(
            OAuthProvider::Microsoft.config(),
            &Secret::new("revoked".to_owned()),
        )
        .await;
    assert_eq!(revoked.map(|_| ()), Err(TokenError::ReauthRequired));
}

#[tokio::test]
async fn refresh_uses_the_configuration_it_is_given() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("client_id=id-de-test"))
        .and(body_string_contains("client_secret=secret-de-test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"access_token":"acc3","expires_in":60}"#),
        )
        .mount(&server)
        .await;
    let config = norelly_domain::oauth::OAuthConfig {
        client_id: "id-de-test",
        client_secret: Some("secret-de-test"),
        ..OAuthProvider::Google.config().clone()
    };
    let refreshed = client(&server, Browser::new(vec![]))
        .refresh(&config, &Secret::new("r1".to_owned()))
        .await;
    assert_eq!(
        refreshed.map(|g| g.access_token.expose().to_owned()),
        Ok("acc3".to_owned())
    );
}

#[tokio::test]
async fn passes_oauth_client_contract() {
    // Connexion réussie dans le navigateur : le contrat l'exige.
    let browser = Browser::new(vec!["/?code=le-code&state={state}"]);
    let server = token_server(&browser).await;
    oauth_client_contract(&client(&server, browser)).await;
}

#[test]
fn token_response_parsing() {
    assert!(matches!(
        parse_token_response(200, b"{}"),
        Err(TokenError::Protocol(_))
    ));
    assert_eq!(
        parse_token_response(400, b"{\"error\":\"bad\\\"code\"}").map(|_| ()),
        Err(TokenError::Network("HTTP 400".to_owned())),
        "code d'erreur suspect non repris"
    );
    assert_eq!(
        parse_token_response(500, b"<html>").map(|_| ()),
        Err(TokenError::Network("HTTP 500".to_owned()))
    );
}

#[test]
fn granted_scopes_are_kept() {
    let scopes = |body: &str| parse_token_response(200, body.as_bytes()).map(|g| g.granted_scopes);
    assert_eq!(
        scopes(r#"{"access_token":"a","scope":"openid https://mail.google.com/"}"#),
        Ok(Some("openid https://mail.google.com/".to_owned()))
    );
    assert_eq!(scopes(r#"{"access_token":"a"}"#), Ok(None));
}

#[test]
fn id_token_gives_the_signed_in_identity() {
    let body = |token: &str| {
        format!(r#"{{"access_token":"a","refresh_token":"r","id_token":"{token}"}}"#).into_bytes()
    };
    let identity = |token: &str| parse_token_response(200, &body(token)).map(|g| g.identity);
    assert_eq!(
        identity(&id_token(
            r#"{"preferred_username":"+33600000000","email":"Bob@Live.fr"}"#
        )),
        Ok(Some(OAuthIdentity {
            preferred_username: Some("+33600000000".to_owned()),
            email: Some("Bob@Live.fr".to_owned()),
            ..OAuthIdentity::default()
        }))
    );
    // Microsoft : émetteur, destinataire et locataire du compte.
    let microsoft = identity(&id_token(
        r#"{"iss":"https://login.microsoftonline.com/t/v2.0","aud":"client","tid":"t","preferred_username":"a@outlook.com","oid":"00000000-0000-0000-66f3-3332eca7ea81","sub":"pairwise"}"#,
    ));
    assert!(
        microsoft
            .as_ref()
            .is_ok_and(|i| i.as_ref().is_some_and(|i| i.subject.is_some())),
        "identifiant stable du compte non lu"
    );
    assert_eq!(
        microsoft,
        Ok(Some(OAuthIdentity {
            preferred_username: Some("a@outlook.com".to_owned()),
            issuer: Some("https://login.microsoftonline.com/t/v2.0".to_owned()),
            audience: vec!["client".to_owned()],
            tenant: Some("t".to_owned()),
            subject: OAuthSubject::parse("00000000-0000-0000-66f3-3332eca7ea81"),
            ..OAuthIdentity::default()
        }))
    );
    // Identifiant stable illisible : ignoré, comme s'il manquait.
    for oid in [r#""""#, r#""a b""#, "42", "null"] {
        assert_eq!(
            identity(&id_token(&format!(r#"{{"oid":{oid}}}"#)))
                .ok()
                .flatten()
                .map(|i| i.subject),
            Some(None),
            "{oid}"
        );
    }
    // Google : émetteur, destinataire (chaîne ou liste) et adresse vérifiée
    // (booléen, ou chaîne que Google a déjà renvoyée).
    let google = |claims: &str| identity(&id_token(claims)).ok().flatten();
    let g = google(
        r#"{"iss":"https://accounts.google.com","aud":"id.apps.googleusercontent.com","email":"a@gmail.com","email_verified":true}"#,
    );
    assert_eq!(
        g,
        Some(OAuthIdentity {
            email: Some("a@gmail.com".to_owned()),
            issuer: Some("https://accounts.google.com".to_owned()),
            audience: vec!["id.apps.googleusercontent.com".to_owned()],
            email_verified: Some(true),
            ..OAuthIdentity::default()
        })
    );
    let g = google(r#"{"aud":["x","y"],"email_verified":"true"}"#);
    assert_eq!(
        g.as_ref().map(|i| i.audience.clone()),
        Some(vec!["x".to_owned(), "y".to_owned()])
    );
    assert_eq!(g.and_then(|i| i.email_verified), Some(true));
    for raw in [r#""false""#, "false", r#""oui""#, "1"] {
        assert_eq!(
            google(&format!(r#"{{"email_verified":{raw}}}"#))
                .and_then(|i| i.email_verified.filter(|v| *v)),
            None,
            "{raw}"
        );
    }
    assert_eq!(
        identity(&id_token(r#"{"sub":"x"}"#)),
        Ok(Some(OAuthIdentity::default()))
    );
    // `id_token` illisible : pas d'identité, le cas d'usage refusera.
    for broken in [
        "",
        "abc",
        "a.!!!.c",
        &format!("a.{}.c", base64url(b"pas du json")),
    ] {
        assert_eq!(identity(broken), Ok(None), "{broken}");
    }
    // Réponse de rafraîchissement, sans `id_token`.
    assert_eq!(
        parse_token_response(200, br#"{"access_token":"a"}"#).map(|g| g.identity),
        Ok(None)
    );
}

#[test]
fn base64url_decoding_inverts_encoding() {
    for data in [&b""[..], b"f", b"fo", b"foo", b"foob", &[0xfb, 0xff, 0x00]] {
        assert_eq!(base64url_decode(&base64url(data)).as_deref(), Some(data));
    }
    assert_eq!(base64url_decode("Zg=="), None, "remplissage refusé");
    assert_eq!(base64url_decode("Z+8"), None, "alphabet standard refusé");
    assert_eq!(base64url_decode("Z"), None, "longueur impossible");
}

#[test]
fn base64url_matches_rfc_4648() {
    assert_eq!(base64url(b""), "");
    assert_eq!(base64url(b"f"), "Zg");
    assert_eq!(base64url(b"fo"), "Zm8");
    assert_eq!(base64url(b"foo"), "Zm9v");
    assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    // Exemple de la RFC 7636, annexe B.
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    assert_eq!(
        base64url(&Sha256::digest(verifier.as_bytes())),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}
