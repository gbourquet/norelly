//! Tests de l'adaptateur OAuth : faux navigateur qui suit l'URL comme
//! l'utilisateur, point d'accès des jetons simulé par wiremock.

use std::sync::{Arc, Mutex};

use norelly_test_support::contracts::oauth_client_contract;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

/// Faux navigateur : retient l'URL et renvoie au client un retour, comme
/// après une connexion réussie (`code`) ou un refus.
#[derive(Clone)]
struct Browser {
    seen: Arc<Mutex<Vec<String>>>,
    /// Requêtes à envoyer à la redirection ; `{state}` est remplacé.
    callbacks: Vec<&'static str>,
}

impl Browser {
    fn new(callbacks: Vec<&'static str>) -> Self {
        Self {
            seen: Arc::default(),
            callbacks,
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
        let callbacks: Vec<String> = self
            .callbacks
            .iter()
            .map(|c| c.replace("{state}", &state))
            .collect();
        tokio::spawn(async move {
            for target in callbacks {
                if let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)).await {
                    let _ = s
                        .write_all(
                            format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes(),
                        )
                        .await;
                    let mut sink = Vec::new();
                    let _ = s.read_to_end(&mut sink).await;
                }
            }
        });
        Ok(())
    }
}

async fn token_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=le-code"))
        .and(body_string_contains("code_verifier="))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"access_token":"acc","expires_in":3599,"refresh_token":"r1","token_type":"Bearer"}"#,
        ))
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
    let Ok(c) = HttpOAuthClient::plain_http_for_tests(
        browser,
        format!("{}/token", server.uri()),
        Duration::from_secs(5),
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
    let server = token_server().await;
    // Un retour forgé (mauvais state) puis un favicon sont ignorés.
    let browser = Browser::new(vec![
        "/?code=forge&state=autre",
        "/favicon.ico",
        "/?code=le-code&state={state}",
    ]);
    let grant = client(&server, browser.clone())
        .authorize(OAuthProvider::Microsoft, &hint())
        .await;
    let Ok(grant) = grant else {
        panic!("connexion refusée : {grant:?}");
    };
    assert_eq!(grant.access_token.expose(), "acc");
    assert_eq!(grant.expires_in, 3599);
    assert_eq!(grant.refresh_token.as_ref().map(Secret::expose), Some("r1"));
    let url = browser.url();
    assert!(url.starts_with(OAuthProvider::Microsoft.config().authorize_url));
    assert_eq!(param(&url, "code_challenge_method"), Some("S256"));
    assert!(param(&url, "code_challenge").is_some_and(|c| c.len() == 43));
    assert!(param(&url, "state").is_some_and(|s| s.len() >= 22));
}

#[tokio::test]
async fn denied_authorization_is_cancelled() {
    let server = token_server().await;
    let browser = Browser::new(vec!["/?error=access_denied&state={state}"]);
    let result = client(&server, browser)
        .authorize(OAuthProvider::Microsoft, &hint())
        .await;
    assert_eq!(
        result.map(|_| ()),
        Err(TokenError::Cancelled("access_denied".to_owned()))
    );
}

#[tokio::test]
async fn refresh_classifies_provider_errors_without_echoing_the_body() {
    let server = token_server().await;
    let c = client(&server, Browser::new(vec![]));
    let refreshed = c
        .refresh(OAuthProvider::Microsoft, &Secret::new("r1".to_owned()))
        .await;
    assert_eq!(
        refreshed.map(|g| g.access_token.expose().to_owned()),
        Ok("acc2".to_owned())
    );
    let revoked = c
        .refresh(OAuthProvider::Microsoft, &Secret::new("revoked".to_owned()))
        .await;
    assert_eq!(revoked.map(|_| ()), Err(TokenError::ReauthRequired));
}

#[tokio::test]
async fn passes_oauth_client_contract() {
    let server = token_server().await;
    oauth_client_contract(&client(&server, Browser::new(vec![]))).await;
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
