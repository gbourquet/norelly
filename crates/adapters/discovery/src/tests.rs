//! Tests de l'adaptateur : autoconfig contre `wiremock`, sondage contre le
//! faux serveur scripté.

use std::path::PathBuf;
use std::time::Duration;

use norelly_domain::discovery::{
    AutoconfigRequest, DiscoverySource, ServerCandidate, UsernameTemplate,
};
use norelly_domain::endpoint::{Host, Protocol, Security, ServerEndpoint};
use norelly_domain::ports::{AutoconfigClient, DiscoveryError, PortProber};
use norelly_test_support::fake_server::{Script, ScriptedServer};
use norelly_tls::TlsClientFactory;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::{HttpAutoconfigClient, TcpPortProber};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/ispdb")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn host(raw: &str) -> Host {
    Host::parse(raw).unwrap_or_else(|_| panic!("hôte : {raw}"))
}

fn request(url: String) -> AutoconfigRequest {
    AutoconfigRequest {
        source: DiscoverySource::ProviderAutoconfig,
        url,
        domain: host("example.com"),
    }
}

fn client() -> HttpAutoconfigClient {
    HttpAutoconfigClient::plain_http_for_tests().unwrap_or_else(|e| panic!("{e}"))
}

#[tokio::test]
async fn fetches_and_parses_autoconfig() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/mail/config-v1.1.xml"))
        .and(query_param("emailaddress", "alice@example.com"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(fixture("provider-full.xml"))
                .insert_header("content-type", "text/xml"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let url = format!(
        "{}/mail/config-v1.1.xml?emailaddress=alice%40example.com",
        server.uri()
    );
    let result = client().fetch(&request(url)).await;
    let Ok(Some(servers)) = result else {
        panic!("résultat inattendu : {result:?}");
    };
    assert_eq!(servers.imap.len(), 2);
    assert_eq!(servers.smtp.len(), 2);
}

#[tokio::test]
async fn request_has_no_cookie_and_no_referer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let _ = client()
        .fetch(&request(format!("{}/x", server.uri())))
        .await;
    let received = server.received_requests().await.unwrap_or_default();
    let Some(req) = received.first() else {
        panic!("aucune requête reçue");
    };
    assert!(req.headers.get("cookie").is_none());
    assert!(req.headers.get("referer").is_none());
    assert_eq!(
        req.headers.get("user-agent").and_then(|v| v.to_str().ok()),
        Some("Norelly")
    );
}

#[tokio::test]
async fn not_found_is_none_and_errors_are_errors() {
    let server = MockServer::start().await;
    Mock::given(path("/absent"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(path("/gone"))
        .respond_with(ResponseTemplate::new(410))
        .mount(&server)
        .await;
    Mock::given(path("/boom"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    Mock::given(path("/garbage"))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(fixture("hostile/billion-laughs.xml")),
        )
        .mount(&server)
        .await;
    let c = client();
    let uri = server.uri();
    assert_eq!(c.fetch(&request(format!("{uri}/absent"))).await, Ok(None));
    assert_eq!(c.fetch(&request(format!("{uri}/gone"))).await, Ok(None));
    assert!(matches!(
        c.fetch(&request(format!("{uri}/boom"))).await,
        Err(DiscoveryError::Invalid(_))
    ));
    assert!(matches!(
        c.fetch(&request(format!("{uri}/garbage"))).await,
        Err(DiscoveryError::Invalid(_))
    ));
}

#[tokio::test]
async fn oversized_response_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'a'; 2 * 1024 * 1024]))
        .mount(&server)
        .await;
    let result = client()
        .fetch(&request(format!("{}/big", server.uri())))
        .await;
    assert!(
        matches!(result, Err(DiscoveryError::Invalid(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn redirect_loops_are_bounded() {
    let server = MockServer::start().await;
    Mock::given(path("/loop"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/loop"))
        .mount(&server)
        .await;
    let result = client()
        .fetch(&request(format!("{}/loop", server.uri())))
        .await;
    assert!(result.is_err(), "{result:?}");
    assert!(server.received_requests().await.unwrap_or_default().len() <= 4);
}

#[tokio::test]
async fn slow_server_times_out() {
    let server = MockServer::start().await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;
    let started = std::time::Instant::now();
    let result = client()
        .fetch(&request(format!("{}/slow", server.uri())))
        .await;
    assert_eq!(result, Err(DiscoveryError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(20));
}

/// Le client réel refuse le HTTP en clair, avant toute connexion.
#[tokio::test]
async fn production_client_refuses_plain_http() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(fixture("provider-full.xml")))
        .mount(&server)
        .await;
    let Ok(c) = HttpAutoconfigClient::new(&TlsClientFactory::platform()) else {
        panic!("client HTTPS");
    };
    let result = c.fetch(&request(format!("{}/x", server.uri()))).await;
    assert!(
        matches!(result, Err(DiscoveryError::Invalid(_))),
        "{result:?}"
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}

/// Une URL `http://` est refusée sans connexion, même vers un port fermé
/// (la politique de redirection est testée dans `http::tests`).
#[tokio::test]
async fn production_client_refuses_http_url_without_connecting() {
    let Ok(c) = HttpAutoconfigClient::new(&TlsClientFactory::platform()) else {
        panic!("client HTTPS");
    };
    let result = c.fetch(&request("http://127.0.0.1:9/x".to_owned())).await;
    assert!(
        matches!(result, Err(DiscoveryError::Invalid(_))),
        "{result:?}"
    );
}

fn candidate(protocol: Protocol, port: u16, security: Security) -> ServerCandidate {
    ServerCandidate {
        protocol,
        endpoint: ServerEndpoint::new(host("127.0.0.1"), port, security)
            .unwrap_or_else(|_| panic!("point")),
        username: UsernameTemplate::email_address(),
    }
}

/// Port local libre et fermé.
async fn closed_port() -> u16 {
    let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.1:0").await else {
        panic!("bind");
    };
    listener.local_addr().map(|a| a.port()).unwrap_or(9)
}

#[tokio::test]
async fn probe_keeps_responding_servers_in_order() {
    let Ok(imap) = ScriptedServer::start(
        Script::new().send("* OK [CAPABILITY IMAP4rev1 STARTTLS] prêt\r\n"),
        None,
    )
    .await
    else {
        panic!("faux serveur");
    };
    let Ok(smtp) = ScriptedServer::start(Script::new().send("220 smtp prêt\r\n"), None).await
    else {
        panic!("faux serveur");
    };
    let Ok(wrong) = ScriptedServer::start(Script::new().send("HTTP/1.1 400 Bad\r\n"), None).await
    else {
        panic!("faux serveur");
    };
    let closed = closed_port().await;
    let prober = TcpPortProber::with_timeouts(Duration::from_secs(2), Duration::from_secs(2));
    let found = prober
        .probe(vec![
            candidate(Protocol::Imap, closed, Security::Tls),
            candidate(Protocol::Imap, wrong.port(), Security::StartTls),
            candidate(Protocol::Imap, imap.port(), Security::StartTls),
            candidate(Protocol::Smtp, smtp.port(), Security::StartTls),
        ])
        .await;
    assert_eq!(
        found.iter().map(|c| c.endpoint.port()).collect::<Vec<_>>(),
        [imap.port(), smtp.port()]
    );
}

#[tokio::test]
async fn probe_is_parallel_and_bounded_in_time() {
    // Serveurs muets : la salutation n'arrive jamais.
    let mut silent = Vec::new();
    let mut candidates = Vec::new();
    for _ in 0..6 {
        let Ok(s) = ScriptedServer::start(Script::new().wait(Duration::from_secs(10)), None).await
        else {
            panic!("faux serveur");
        };
        candidates.push(candidate(Protocol::Imap, s.port(), Security::StartTls));
        silent.push(s);
    }
    let prober =
        TcpPortProber::with_timeouts(Duration::from_millis(500), Duration::from_millis(500));
    let started = std::time::Instant::now();
    let found = prober.probe(candidates).await;
    assert!(found.is_empty());
    // 6 sondes de 0,5 s en parallèle : bien moins que 6 × 0,5 s.
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn probe_tls_port_accepting_connections() {
    let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.1:0").await else {
        panic!("bind");
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    tokio::spawn(async move {
        let _conn = listener.accept().await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    let found = TcpPortProber::default()
        .probe(vec![candidate(Protocol::Imap, port, Security::Tls)])
        .await;
    assert_eq!(found.len(), 1);
}
