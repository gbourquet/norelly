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

/// `id_token` non signé portant `claims` (JSON).
fn id_token(claims: &str) -> String {
    format!(
        "{}.{}.c2ln",
        base64url(br#"{"alg":"RS256","typ":"JWT"}"#),
        base64url(claims.as_bytes())
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
        id_token(
            r#"{"preferred_username":"alice@outlook.com","email":"alice@outlook.com","sub":"x"}"#
        )
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
    // Un retour forgé (mauvais state) puis un favicon sont ignorés.
    let browser = Browser::new(vec![
        "/?code=forge&state=autre",
        "/favicon.ico",
        "/?code=le-code&state={state}",
    ]);
    let server = token_server(&browser).await;
    let grant = client(&server, browser.clone())
        .authorize(OAuthProvider::Microsoft, &hint())
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
            preferred_username: Some("alice@outlook.com".to_owned()),
            email: Some("alice@outlook.com".to_owned()),
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
        .authorize(OAuthProvider::Microsoft, &hint())
        .await;
    assert_eq!(
        result.map(|_| ()),
        Err(TokenError::Cancelled("access_denied".to_owned()))
    );
}

#[tokio::test]
async fn refresh_classifies_provider_errors_without_echoing_the_body() {
    let browser = Browser::new(vec![]);
    let server = token_server(&browser).await;
    let c = client(&server, browser);
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
    let browser = Browser::new(vec![]);
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
        }))
    );
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
