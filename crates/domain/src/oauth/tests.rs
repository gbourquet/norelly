use super::*;

fn address(raw: &str) -> EmailAddress {
    let Ok(a) = EmailAddress::parse(raw) else {
        panic!("adresse valide refusée : {raw}");
    };
    a
}

fn secret(raw: &str) -> Secret {
    Secret::new(raw.to_owned())
}

#[test]
fn provider_name_roundtrip() {
    assert_eq!(
        OAuthProvider::parse("microsoft"),
        Some(OAuthProvider::Microsoft)
    );
    assert_eq!(OAuthProvider::Microsoft.as_str(), "microsoft");
    assert_eq!(OAuthProvider::parse("google"), None);
    assert_eq!(OAuthProvider::parse(""), None);
}

#[test]
fn microsoft_personal_domains_are_recognized() {
    for raw in ["a@outlook.com", "a@HOTMAIL.FR", "a@live.com", "a@msn.com"] {
        assert_eq!(
            OAuthProvider::for_address(&address(raw)),
            Some(OAuthProvider::Microsoft),
            "{raw}"
        );
    }
    for raw in ["a@gmail.com", "a@outlook.com.evil.example", "a@example.com"] {
        assert_eq!(OAuthProvider::for_address(&address(raw)), None, "{raw}");
    }
}

#[test]
fn microsoft_config_uses_consumers_and_outlook_office_scopes() {
    let c = OAuthProvider::Microsoft.config();
    assert!(
        c.authorize_url
            .starts_with("https://login.microsoftonline.com/consumers/")
    );
    assert!(
        c.token_url
            .starts_with("https://login.microsoftonline.com/consumers/")
    );
    assert!(c.scopes.contains(&"offline_access"));
    assert!(c.scopes.iter().all(|s| !s.contains("office365")));
}

#[test]
fn authorization_url_carries_pkce_state_and_encoded_parameters() {
    let hint = address("alice+test@outlook.com");
    let url = authorization_url(
        OAuthProvider::Microsoft.config(),
        &AuthorizationRequest {
            redirect_uri: "http://127.0.0.1:49152/",
            state: "st4te",
            code_challenge: "ch-al_lenge",
            login_hint: &hint,
        },
    );
    let Some((base, query)) = url.split_once('?') else {
        panic!("URL sans requête : {url}");
    };
    assert_eq!(base, OAuthProvider::Microsoft.config().authorize_url);
    let params: Vec<&str> = query.split('&').collect();
    for expected in [
        "response_type=code",
        "client_id=10a28ed0-5f58-4684-bc78-f4a2c6c1a0eb",
        "redirect_uri=http%3A%2F%2F127.0.0.1%3A49152%2F",
        "state=st4te",
        "code_challenge=ch-al_lenge",
        "code_challenge_method=S256",
        "login_hint=alice%2Btest%40outlook.com",
        "scope=https%3A%2F%2Foutlook.office.com%2FIMAP.AccessAsUser.All%20https%3A%2F%2Foutlook.office.com%2FSMTP.Send%20offline_access",
    ] {
        assert!(params.contains(&expected), "{expected} absent de {url}");
    }
}

#[test]
fn token_forms_are_encoded() {
    let c = OAuthProvider::Microsoft.config();
    let form = code_exchange_form(c, &secret("a&b=c"), "http://127.0.0.1:1/", &secret("v"));
    assert_eq!(
        form.expose(),
        "grant_type=authorization_code&client_id=10a28ed0-5f58-4684-bc78-f4a2c6c1a0eb\
         &code=a%26b%3Dc&redirect_uri=http%3A%2F%2F127.0.0.1%3A1%2F&code_verifier=v"
    );
    let form = refresh_form(c, &secret("r t"));
    assert!(
        form.expose()
            .starts_with("grant_type=refresh_token&client_id=")
    );
    assert!(form.expose().contains("&refresh_token=r%20t"));
    assert!(
        form.expose()
            .contains("&scope=https%3A%2F%2Foutlook.office.com")
    );
}

#[test]
fn callback_returns_code_when_state_matches() {
    let code = parse_callback("/?code=M.C%2B1-ab&state=xyz", "xyz");
    assert_eq!(
        code.map(|c| c.expose().to_owned()),
        Ok("M.C+1-ab".to_owned())
    );
    let code = parse_callback("/?state=xyz&code=c", "xyz");
    assert_eq!(code.map(|c| c.expose().to_owned()), Ok("c".to_owned()));
}

#[test]
fn callback_checks_state_first() {
    let err = |t: &str| parse_callback(t, "xyz").map(|c| c.expose().to_owned());
    assert_eq!(
        err("/?code=c&state=other"),
        Err(CallbackError::StateMismatch)
    );
    assert_eq!(err("/?code=c"), Err(CallbackError::StateMismatch));
    assert_eq!(
        err("/?error=access_denied"),
        Err(CallbackError::StateMismatch)
    );
    assert_eq!(
        err("/?code=c&state=xyz&state=xyz"),
        Err(CallbackError::Malformed)
    );
    assert_eq!(
        parse_callback("/?code=c&state=", "").map(|c| c.expose().to_owned()),
        Err(CallbackError::StateMismatch)
    );
    assert_eq!(
        err("/?error=access_denied&state=xyz"),
        Err(CallbackError::Denied("access_denied".to_owned()))
    );
    assert_eq!(err("/?state=xyz"), Err(CallbackError::MissingCode));
    assert_eq!(err("/favicon.ico"), Err(CallbackError::Malformed));
    assert_eq!(err("/?code=%ZZ&state=xyz"), Err(CallbackError::Malformed));
}

#[test]
fn xoauth2_response_format() {
    let r = xoauth2_initial_response("alice@outlook.com", &secret("tok"));
    assert_eq!(
        r.expose(),
        b"user=alice@outlook.com\x01auth=Bearer tok\x01\x01"
    );
}

#[test]
fn access_token_freshness_keeps_a_margin() {
    let t = AccessToken {
        token: secret("t"),
        expires_at: 1_000,
    };
    assert!(t.is_fresh(1_000 - ACCESS_TOKEN_MARGIN_SECS - 1));
    assert!(!t.is_fresh(1_000 - ACCESS_TOKEN_MARGIN_SECS));
    assert!(!t.is_fresh(2_000));
}

#[test]
fn token_errors_are_classified() {
    assert_eq!(
        classify_token_error(400, Some("invalid_grant")),
        TokenError::ReauthRequired
    );
    assert_eq!(
        classify_token_error(400, Some("invalid_client")),
        TokenError::Protocol("invalid_client".to_owned())
    );
    assert_eq!(
        classify_token_error(503, None),
        TokenError::Network("HTTP 503".to_owned())
    );
}

#[test]
fn smtp_oauth_disabled_is_detected() {
    assert!(is_smtp_oauth_disabled(
        "535 5.7.139 Authentication unsuccessful, SmtpClientAuthentication is disabled"
    ));
    assert!(!is_smtp_oauth_disabled(
        "535 5.7.3 Authentication unsuccessful"
    ));
}

#[test]
fn token_goes_only_to_provider_servers_over_tls() {
    use crate::auth::AuthError;
    use crate::endpoint::{Host, Security, ServerEndpoint};
    let endpoint = |host: &str, port: u16, security: Security| {
        let Ok(h) = Host::parse(host) else {
            panic!("hôte valide refusé : {host}");
        };
        let Ok(e) = ServerEndpoint::new(h, port, security) else {
            panic!("point d'accès valide refusé");
        };
        e
    };
    let ms = OAuthProvider::Microsoft;
    let c = ms.config();
    assert_eq!(
        token_destination_allowed(ms, &endpoint(c.imap_host, 993, Security::Tls)),
        Ok(())
    );
    assert_eq!(
        token_destination_allowed(ms, &endpoint(c.smtp_host, 587, Security::StartTls)),
        Ok(())
    );
    assert_eq!(
        token_destination_allowed(ms, &endpoint("OUTLOOK.Office365.com", 993, Security::Tls)),
        Ok(()),
        "nom d'hôte insensible à la casse"
    );
    for (host, security) in [
        ("imap.attaquant.example", Security::Tls),
        ("outlook.office365.com.attaquant.example", Security::Tls),
        (c.imap_host, Security::Plain),
        (c.smtp_host, Security::Plain),
    ] {
        assert_eq!(
            token_destination_allowed(ms, &endpoint(host, 993, security)),
            Err(AuthError::TokenDestinationRefused {
                host: host.to_owned()
            }),
            "{host} {security:?}"
        );
    }
}

#[test]
fn connect_request_refuses_token_for_foreign_host() {
    use crate::auth::AuthError;
    use crate::credentials::{Credentials, Username};
    use crate::endpoint::{Host, Security, ServerEndpoint};
    use crate::ports::ConnectRequest;
    use crate::security::PlaintextPolicy;
    let request = |host: &str, oauth: bool| {
        let (Ok(h), Ok(u)) = (Host::parse(host), Username::parse("alice@outlook.com")) else {
            panic!("valeurs valides refusées");
        };
        let Ok(endpoint) = ServerEndpoint::new(h, 993, Security::Tls) else {
            panic!("point d'accès valide refusé");
        };
        let credentials = if oauth {
            Credentials::oauth2(u, secret("t"), OAuthProvider::Microsoft)
        } else {
            Credentials::new(u, secret("pw"))
        };
        let Ok(credentials) = credentials else {
            panic!("identifiants valides refusés");
        };
        ConnectRequest {
            endpoint,
            credentials,
            certificate_exceptions: Vec::new(),
            plaintext: PlaintextPolicy::deny_all(),
        }
    };
    assert_eq!(
        request("imap.attaquant.example", true).authorize_credentials(),
        Err(AuthError::TokenDestinationRefused {
            host: "imap.attaquant.example".to_owned()
        })
    );
    assert_eq!(
        request("outlook.office365.com", true).authorize_credentials(),
        Ok(())
    );
    assert_eq!(
        request("imap.attaquant.example", false).authorize_credentials(),
        Ok(())
    );
}

#[test]
fn microsoft_mailbox_not_connected_is_detected() {
    assert!(is_mailbox_not_connected(
        "User is authenticated but not connected."
    ));
    assert!(is_mailbox_not_connected(
        "BAD User is Authenticated but not Connected"
    ));
    assert!(!is_mailbox_not_connected("AUTHENTICATE failed."));
}

fn pending(obtained_at: u64, expires_in: u64) -> PendingAuthorization {
    PendingAuthorization {
        provider: OAuthProvider::Microsoft,
        address: address("alice@outlook.com"),
        obtained_at,
        expires_in,
    }
}

#[test]
fn pending_authorization_is_reused_for_the_same_address_shortly_after() {
    let alice = address("alice@outlook.com");
    let p = pending(1_000, 3_600);
    assert!(p.usable_for(OAuthProvider::Microsoft, &alice, 1_000));
    assert!(p.usable_for(
        OAuthProvider::Microsoft,
        &alice,
        1_000 + PENDING_AUTHORIZATION_MAX_AGE_SECS - 1
    ));
}

#[test]
fn pending_authorization_is_not_reused_too_late_or_for_another_address() {
    let alice = address("alice@outlook.com");
    let p = pending(1_000, 3_600);
    assert!(!p.usable_for(
        OAuthProvider::Microsoft,
        &alice,
        1_000 + PENDING_AUTHORIZATION_MAX_AGE_SECS
    ));
    assert!(!p.usable_for(OAuthProvider::Microsoft, &address("bob@outlook.com"), 1_000));
    assert!(!p.usable_for(OAuthProvider::Microsoft, &alice, 999));
    // Jeton d'accès bientôt expiré : marge de rafraîchissement respectée.
    let short = pending(1_000, ACCESS_TOKEN_MARGIN_SECS + 10);
    assert!(short.usable_for(OAuthProvider::Microsoft, &alice, 1_009));
    assert!(!short.usable_for(OAuthProvider::Microsoft, &alice, 1_010));
}
