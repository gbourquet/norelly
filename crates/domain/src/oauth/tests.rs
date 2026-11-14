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
    assert_eq!(OAuthProvider::parse("google"), Some(OAuthProvider::Google));
    assert_eq!(OAuthProvider::Google.as_str(), "google");
    assert_eq!(OAuthProvider::parse("Google"), None);
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
    for raw in ["a@outlook.com.evil.example", "a@example.com"] {
        assert_eq!(OAuthProvider::for_address(&address(raw)), None, "{raw}");
    }
}

#[test]
fn gmail_addresses_sign_in_with_google() {
    for raw in ["a@gmail.com", "a@GMAIL.com"] {
        assert_eq!(
            OAuthProvider::for_address(&address(raw)),
            Some(OAuthProvider::Google),
            "{raw}"
        );
    }
    // Sosies et domaines Google Workspace : rien ne prouve qu'ils sont
    // chez Google, le mot de passe reste la voie.
    for raw in [
        "a@gmail.com.evil.example",
        "a@mail.gmail.com",
        "a@google.com",
        "a@gmai1.com",
    ] {
        assert_eq!(OAuthProvider::for_address(&address(raw)), None, "{raw}");
    }
}

#[test]
fn a_provider_serves_only_its_own_addresses() {
    assert!(OAuthProvider::Google.serves(&address("alice@GMAIL.com")));
    assert!(OAuthProvider::Microsoft.serves(&address("alice@outlook.com")));
    for (provider, raw) in [
        (OAuthProvider::Google, "alice@societe.fr"),
        (OAuthProvider::Google, "alice@outlook.com"),
        (OAuthProvider::Microsoft, "alice@gmail.com"),
        (OAuthProvider::Microsoft, "alice@example.com"),
    ] {
        assert!(!provider.serves(&address(raw)), "{provider} {raw}");
    }
}

#[test]
fn google_config_uses_google_endpoints_and_full_mail_scope() {
    let c = OAuthProvider::Google.config();
    assert_eq!(
        c.authorize_url,
        "https://accounts.google.com/o/oauth2/v2/auth"
    );
    assert_eq!(c.token_url, "https://oauth2.googleapis.com/token");
    assert_eq!(c.scopes, &["https://mail.google.com/"]);
    assert_eq!(c.identity_scopes, &["openid", "email"]);
    assert_eq!(c.imap_host, "imap.gmail.com");
    assert_eq!(c.smtp_host, "smtp.gmail.com");
    // Client « application de bureau » : Google exige un secret client
    // (non confidentiel), fourni à la compilation comme l'identifiant.
    assert!(c.client_secret.is_some());
    assert_eq!(OAuthProvider::Microsoft.config().client_secret, None);
    let configured = google_with("id.apps.googleusercontent.com", "GOCSPX-s3cr3t");
    assert!(
        !format!("{configured:?}").contains("s3cr3t"),
        "secret client dans Debug"
    );
}

/// Configuration Google complète, avec des valeurs de test.
fn google_with(client_id: &'static str, client_secret: &'static str) -> OAuthConfig {
    OAuthConfig {
        client_id,
        client_secret: Some(client_secret),
        ..GOOGLE.clone()
    }
}

#[test]
fn a_provider_is_usable_only_with_its_client_identifiers() {
    assert!(OAuthProvider::Microsoft.config().is_usable());
    assert!(google_with("id.apps.googleusercontent.com", "GOCSPX-s").is_usable());
    // Compilation sans les identifiants Google : fournisseur inutilisable.
    assert!(!google_with("", "").is_usable());
    assert!(!google_with("id.apps.googleusercontent.com", "").is_usable());
    assert!(!google_with("", "GOCSPX-s").is_usable());
}

#[test]
fn sign_in_is_offered_only_by_a_usable_provider() {
    let gmail = address("alice@gmail.com");
    assert_eq!(
        offered_provider(&gmail, |_| true),
        Some(OAuthProvider::Google)
    );
    // Google inutilisable : l'adresse Gmail garde le mot de passe.
    assert_eq!(
        offered_provider(&gmail, |p| p != OAuthProvider::Google),
        None
    );
    assert_eq!(
        offered_provider(&address("alice@outlook.com"), |p| p
            != OAuthProvider::Google),
        Some(OAuthProvider::Microsoft)
    );
    assert_eq!(
        offered_provider(&address("alice@example.com"), |_| true),
        None
    );
    // Sans le paramètre de test : décision de la configuration compilée.
    assert_eq!(
        OAuthProvider::offered_for(&gmail),
        OAuthProvider::Google
            .config()
            .is_usable()
            .then_some(OAuthProvider::Google)
    );
}

#[test]
fn google_authorization_asks_for_offline_access_and_fresh_consent() {
    let hint = address("alice@gmail.com");
    let url = authorization_url(
        OAuthProvider::Google.config(),
        &AuthorizationRequest {
            redirect_uri: "http://localhost:49152/",
            state: "s",
            code_challenge: "c",
            login_hint: &hint,
        },
    );
    let Some((base, query)) = url.split_once('?') else {
        panic!("URL sans requête : {url}");
    };
    assert_eq!(base, "https://accounts.google.com/o/oauth2/v2/auth");
    let params: Vec<&str> = query.split('&').collect();
    for expected in [
        "scope=https%3A%2F%2Fmail.google.com%2F%20openid%20email",
        // Sans `access_type=offline`, Google ne délivre pas de jeton de
        // rafraîchissement ; sans `consent`, il ne le redonne pas à un
        // compte qui a déjà autorisé Norelly.
        "access_type=offline",
        "prompt=select_account%20consent",
        "login_hint=alice%40gmail.com",
    ] {
        assert!(params.contains(&expected), "{expected} absent de {url}");
    }
    assert!(!query.contains("client_secret"), "{url}");
    assert_eq!(
        params.iter().filter(|p| p.starts_with("prompt=")).count(),
        1
    );
}

#[test]
fn microsoft_authorization_keeps_its_parameters() {
    let hint = address("alice@outlook.com");
    let url = authorization_url(
        OAuthProvider::Microsoft.config(),
        &AuthorizationRequest {
            redirect_uri: "http://localhost:1/",
            state: "s",
            code_challenge: "c",
            login_hint: &hint,
        },
    );
    assert!(url.contains("&prompt=select_account"), "{url}");
    assert!(!url.contains("access_type"), "{url}");
}

#[test]
fn google_token_forms_carry_the_client_secret() {
    let config = google_with("id.apps.googleusercontent.com", "GOCSPX-s3cr3t");
    let c = &config;
    let client_secret = "GOCSPX-s3cr3t";
    let form = code_exchange_form(c, &secret("4/0c"), "http://localhost:1/", &secret("v"));
    assert!(
        form.expose()
            .contains(&format!("&client_secret={client_secret}&")),
        "secret client absent de l'échange"
    );
    assert!(form.expose().contains("&code=4%2F0c&"));
    let form = refresh_form(c, &secret("1//r"));
    assert!(
        form.expose()
            .contains(&format!("&client_secret={client_secret}")),
        "secret client absent du rafraîchissement"
    );
    assert!(form.expose().contains("&refresh_token=1%2F%2Fr"));
    // Google garde les scopes accordés : le rafraîchissement n'en envoie pas.
    assert!(!form.expose().contains("scope="));
}

#[test]
fn google_sign_in_without_mail_access_is_refused() {
    let google = OAuthProvider::Google;
    assert_eq!(
        require_mail_access(
            google,
            Some("openid https://www.googleapis.com/auth/userinfo.email")
        ),
        Err(TokenError::MailAccessRefused)
    );
    assert_eq!(
        require_mail_access(google, Some("")),
        Err(TokenError::MailAccessRefused)
    );
    // Préfixe ou sosie du scope : refusé.
    assert_eq!(
        require_mail_access(google, Some("https://mail.google.com/x openid")),
        Err(TokenError::MailAccessRefused)
    );
    assert_eq!(
        require_mail_access(google, Some("openid https://mail.google.com/ email")),
        Ok(())
    );
    // Scopes non renvoyés : ceux demandés ont été accordés (RFC 6749, 5.1).
    assert_eq!(require_mail_access(google, None), Ok(()));
    // Microsoft renvoie ses scopes sous une autre forme : non concerné.
    assert_eq!(
        require_mail_access(OAuthProvider::Microsoft, Some("openid")),
        Ok(())
    );
}

#[test]
fn abandoned_google_sign_in_suggests_an_app_password() {
    let google = OAuthProvider::Google;
    for reason in ["access_denied", "délai dépassé"] {
        assert_eq!(
            add_account_sign_in_error(google, TokenError::Cancelled(reason.to_owned())),
            TokenError::GoogleSignInRefused,
            "{reason}"
        );
    }
    for other in [
        TokenError::WrongAccount,
        TokenError::OfflineAccessRefused,
        TokenError::ReauthRequired,
        TokenError::Network("HTTP 503".to_owned()),
        TokenError::Protocol("invalid_client".to_owned()),
    ] {
        assert_eq!(add_account_sign_in_error(google, other.clone()), other);
    }
    let cancelled = TokenError::Cancelled("access_denied".to_owned());
    assert_eq!(
        add_account_sign_in_error(OAuthProvider::Microsoft, cancelled.clone()),
        cancelled
    );
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
        "scope=https%3A%2F%2Foutlook.office.com%2FIMAP.AccessAsUser.All%20https%3A%2F%2Foutlook.office.com%2FSMTP.Send%20offline_access%20openid%20email%20profile",
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
    // Le rafraîchissement garde les scopes accordés aux comptes existants.
    assert!(!form.expose().contains("openid"));
}

fn identity(preferred_username: Option<&str>, email: Option<&str>) -> OAuthIdentity {
    OAuthIdentity {
        preferred_username: preferred_username.map(str::to_owned),
        email: email.map(str::to_owned),
    }
}

#[test]
fn identity_of_the_address_is_accepted() {
    let alice = address("alice@outlook.com");
    for id in [
        identity(Some("alice@outlook.com"), None),
        identity(Some("Alice@Outlook.COM"), None),
        // Connexion par numéro de téléphone : l'adresse vient de `email`.
        identity(Some("+33600000000"), Some("alice@outlook.com")),
        identity(None, Some(" alice@outlook.com ")),
    ] {
        assert_eq!(verify_identity(&alice, Some(&id)), Ok(()), "{id:?}");
    }
}

#[test]
fn identity_of_another_account_is_refused() {
    let alice = address("alice@outlook.com");
    for id in [
        identity(Some("mallory@outlook.com"), None),
        identity(Some("mallory@outlook.com"), Some("mallory@hotmail.com")),
        identity(Some("alice@outlook.com.evil.example"), None),
        identity(Some(""), Some("")),
        identity(None, None),
    ] {
        assert_eq!(
            verify_identity(&alice, Some(&id)),
            Err(TokenError::WrongAccount),
            "{id:?}"
        );
    }
}

#[test]
fn missing_identity_is_refused() {
    assert!(matches!(
        verify_identity(&address("alice@outlook.com"), None),
        Err(TokenError::Protocol(_))
    ));
}

#[test]
fn only_a_network_failure_makes_the_provider_unavailable() {
    use crate::auth::AuthError;
    assert!(matches!(
        auth_error_of(&TokenError::Network("HTTP 503".to_owned())),
        AuthError::ProviderUnavailable { .. }
    ));
    assert_eq!(
        auth_error_of(&TokenError::ReauthRequired),
        AuthError::ReauthRequired
    );
    // Application refusée par le fournisseur (`invalid_client`…) : définitif.
    assert!(matches!(
        auth_error_of(&TokenError::Protocol("invalid_client".to_owned())),
        AuthError::Rejected { .. }
    ));
}

#[test]
fn unconfigured_provider_is_a_definitive_refusal_without_reconnection() {
    use crate::auth::AuthError;
    assert_eq!(
        auth_error_of(&TokenError::ProviderNotConfigured),
        AuthError::ProviderNotConfigured
    );
}

#[test]
fn sign_in_without_refresh_token_is_refused() {
    assert_eq!(
        require_offline_access(None),
        Err(TokenError::OfflineAccessRefused)
    );
    assert_eq!(require_offline_access(Some(&secret("r1"))), Ok(()));
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

#[test]
fn google_token_goes_only_to_gmail_servers() {
    use crate::endpoint::{Host, Security, ServerEndpoint};
    let endpoint = |host: &str| {
        let Ok(h) = Host::parse(host) else {
            panic!("hôte valide refusé : {host}");
        };
        let Ok(e) = ServerEndpoint::new(h, 993, Security::Tls) else {
            panic!("point d'accès valide refusé");
        };
        e
    };
    let google = OAuthProvider::Google;
    assert_eq!(
        token_destination_allowed(google, &endpoint("imap.gmail.com")),
        Ok(())
    );
    assert_eq!(
        token_destination_allowed(google, &endpoint("smtp.gmail.com")),
        Ok(())
    );
    for host in ["outlook.office365.com", "imap.gmail.com.attaquant.example"] {
        assert!(
            token_destination_allowed(google, &endpoint(host)).is_err(),
            "{host}"
        );
    }
    assert!(
        token_destination_allowed(OAuthProvider::Microsoft, &endpoint("imap.gmail.com")).is_err()
    );
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
