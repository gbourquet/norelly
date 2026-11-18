//! Faux serveur CardDAV (ADR-014) : carnet en mémoire ([`CardDavModel`]),
//! servi en HTTPS par [`FakeCardDavServer`] (certificat d'une CA de test),
//! ou lu directement par le faux client ([`crate::carddav_contracts`]).
//!
//! Il suit le chemin de découverte d'un serveur réel :
//! `/.well-known/carddav` redirige vers `/dav/`, qui désigne le principal
//! `/dav/principals/<user>/`, dont le dossier des carnets est
//! `/dav/books/<user>/`, qui contient le carnet `contacts/`. Fiches par
//! `PROPFIND` (ETag), `REPORT addressbook-multiget`, `GET`, `PUT`
//! (`If-Match`, `If-None-Match: *`) et `DELETE` (`If-Match`). Identifiants
//! en HTTP Basic, refusés par 401.
//!
//! Des réponses peuvent être imposées ([`FakeCardDavServer::override_response`])
//! pour exercer un serveur hostile.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::certs::{CertError, TestCa};

/// Taille maximale d'une requête lue (en-têtes et corps).
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;

/// Fiche du faux carnet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCard {
    pub etag: String,
    pub data: String,
}

/// Carnet distant en mémoire, partagé par le faux serveur HTTPS et le faux
/// client.
#[derive(Debug, Default)]
pub struct CardDavModel {
    /// Fiches, par chemin absolu.
    pub cards: BTreeMap<String, StoredCard>,
    next: u64,
}

/// Échec d'une écriture conditionnelle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelError {
    /// Version attendue différente, ou fiche déjà présente.
    Precondition,
    NotFound,
}

impl CardDavModel {
    fn next_etag(&mut self) -> String {
        self.next += 1;
        format!("\"v{}\"", self.next)
    }

    /// Ajoute ou remplace une fiche sans condition ; renvoie son ETag.
    pub fn put(&mut self, path: &str, data: &str) -> String {
        let etag = self.next_etag();
        self.cards.insert(
            path.to_owned(),
            StoredCard {
                etag: etag.clone(),
                data: data.to_owned(),
            },
        );
        etag
    }

    /// Écriture conditionnelle : `if_match` (version attendue) ou, sans
    /// elle, fiche absente exigée.
    pub fn put_if(
        &mut self,
        path: &str,
        if_match: Option<&str>,
        data: &str,
    ) -> Result<String, ModelError> {
        let current = self.cards.get(path).map(|c| c.etag.as_str());
        match (if_match, current) {
            (None, None) => {}
            (Some(expected), Some(current)) if expected == current => {}
            _ => return Err(ModelError::Precondition),
        }
        Ok(self.put(path, data))
    }

    /// Suppression conditionnelle.
    pub fn delete_if(&mut self, path: &str, if_match: &str) -> Result<(), ModelError> {
        match self.cards.get(path) {
            None => Err(ModelError::NotFound),
            Some(card) if card.etag != if_match => Err(ModelError::Precondition),
            Some(_) => {
                self.cards.remove(path);
                Ok(())
            }
        }
    }
}

/// Encodage base64 (identifiants HTTP Basic attendus).
#[must_use]
pub fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(TABLE[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Réponse imposée.
#[derive(Debug, Clone)]
pub struct RawResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Requête reçue (journal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedRequest {
    pub method: String,
    pub path: String,
    /// Identifiants présentés (en-tête `Authorization` présent).
    pub authorized: bool,
}

#[derive(Debug)]
struct Shared {
    model: Arc<Mutex<CardDavModel>>,
    user: String,
    authorization: String,
    etag_on_put: Mutex<bool>,
    overrides: Mutex<HashMap<(String, String), RawResponse>>,
    log: Mutex<Vec<LoggedRequest>>,
}

/// Faux serveur CardDAV en HTTPS sur `127.0.0.1`, nommé `localhost`.
#[derive(Debug, Clone)]
pub struct FakeCardDavServer {
    shared: Arc<Shared>,
    port: u16,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeCardDavServer {
    /// Démarre le serveur ; son certificat est émis par `ca` pour
    /// `localhost`.
    pub async fn start(ca: &TestCa, user: &str, password: &str) -> Result<Self, CertError> {
        Self::start_with(
            ca,
            user,
            password,
            Arc::new(Mutex::new(CardDavModel::default())),
        )
        .await
    }

    /// Démarre le serveur sur un carnet donné.
    pub async fn start_with(
        ca: &TestCa,
        user: &str,
        password: &str,
        model: Arc<Mutex<CardDavModel>>,
    ) -> Result<Self, CertError> {
        let config = ca.issue(&["localhost"])?.server_config()?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| CertError::Rustls(rustls::Error::General(e.to_string())))?;
        let port = listener
            .local_addr()
            .map_err(|e| CertError::Rustls(rustls::Error::General(e.to_string())))?
            .port();
        let shared = Arc::new(Shared {
            model,
            user: user.to_owned(),
            authorization: format!("Basic {}", base64(format!("{user}:{password}").as_bytes())),
            etag_on_put: Mutex::new(true),
            overrides: Mutex::new(HashMap::new()),
            log: Mutex::new(Vec::new()),
        });
        let acceptor = TlsAcceptor::from(config);
        let server = Arc::clone(&shared);
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let shared = Arc::clone(&server);
                tokio::spawn(async move {
                    if let Ok(stream) = acceptor.accept(tcp).await {
                        let _ = serve(stream, &shared).await;
                    }
                });
            }
        });
        Ok(Self { shared, port })
    }

    /// Adresse du serveur (racine).
    #[must_use]
    pub fn url(&self) -> String {
        format!("https://localhost:{}/", self.port)
    }

    /// Chemin du carnet.
    #[must_use]
    pub fn book_path(&self) -> String {
        format!("/dav/books/{}/contacts/", self.shared.user)
    }

    /// Carnet partagé.
    #[must_use]
    pub fn model(&self) -> Arc<Mutex<CardDavModel>> {
        Arc::clone(&self.shared.model)
    }

    /// ETag rendue (ou non) après un `PUT`.
    pub fn set_etag_on_put(&self, on: bool) {
        *lock(&self.shared.etag_on_put) = on;
    }

    /// Réponse imposée pour `method` sur `path`.
    pub fn override_response(&self, method: &str, path: &str, response: RawResponse) {
        lock(&self.shared.overrides).insert((method.to_owned(), path.to_owned()), response);
    }

    /// Requêtes reçues.
    #[must_use]
    pub fn requests(&self) -> Vec<LoggedRequest> {
        lock(&self.shared.log).clone()
    }
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

async fn read_request<S: AsyncRead + Unpin>(stream: &mut S) -> Option<Request> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 || buf.len() > MAX_REQUEST_BYTES {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let path = first.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    if length > MAX_REQUEST_BYTES {
        return None;
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn multistatus(responses: &str) -> RawResponse {
    xml(
        207,
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<d:multistatus xmlns:d=\"DAV:\" xmlns:card=\"urn:ietf:params:xml:ns:carddav\">{responses}</d:multistatus>"
        ),
    )
}

fn xml(status: u16, body: String) -> RawResponse {
    RawResponse {
        status,
        headers: vec![(
            "Content-Type".into(),
            "application/xml; charset=utf-8".into(),
        )],
        body: body.into_bytes(),
    }
}

fn status(code: u16) -> RawResponse {
    RawResponse {
        status: code,
        headers: Vec::new(),
        body: Vec::new(),
    }
}

fn prop_response(href: &str, props: &str) -> String {
    format!(
        "<d:response><d:href>{}</d:href><d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
        escape(href)
    )
}

/// Chemins `href` cités dans un corps `REPORT addressbook-multiget`.
fn requested_hrefs(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("href>") {
        rest = &rest[start + 5..];
        if let Some(end) = rest.find('<') {
            let href = rest[..end].trim();
            if !href.is_empty() {
                out.push(href.replace("&amp;", "&"));
            }
            rest = &rest[end..];
        }
    }
    out
}

fn handle(request: &Request, shared: &Shared) -> RawResponse {
    if let Some(r) = lock(&shared.overrides).get(&(request.method.clone(), request.path.clone())) {
        return r.clone();
    }
    if request.header("authorization") != Some(shared.authorization.as_str()) {
        return RawResponse {
            status: 401,
            headers: vec![(
                "WWW-Authenticate".into(),
                "Basic realm=\"norelly-test\"".into(),
            )],
            body: Vec::new(),
        };
    }
    let user = &shared.user;
    let principal = format!("/dav/principals/{user}/");
    let home = format!("/dav/books/{user}/");
    let book = format!("{home}contacts/");
    let depth = request.header("depth").unwrap_or("0");
    let path = request.path.as_str();
    match request.method.as_str() {
        "PROPFIND" if path == "/.well-known/carddav" => RawResponse {
            status: 301,
            headers: vec![("Location".into(), "/dav/".into())],
            body: Vec::new(),
        },
        "PROPFIND" if path == "/dav/" || path == "/" => multistatus(&prop_response(
            path,
            &format!(
                "<d:current-user-principal><d:href>{principal}</d:href></d:current-user-principal><d:resourcetype><d:collection/></d:resourcetype>"
            ),
        )),
        "PROPFIND" if path == principal => multistatus(&prop_response(
            path,
            &format!(
                "<card:addressbook-home-set><d:href>{home}</d:href></card:addressbook-home-set><d:resourcetype><d:principal/></d:resourcetype>"
            ),
        )),
        "PROPFIND" if path == home => {
            let mut body = prop_response(&home, "<d:resourcetype><d:collection/></d:resourcetype>");
            if depth == "1" {
                body.push_str(&prop_response(
                    &book,
                    "<d:resourcetype><d:collection/><card:addressbook/></d:resourcetype><d:displayname>Contacts</d:displayname>",
                ));
            }
            multistatus(&body)
        }
        "PROPFIND" if path == book => {
            let mut body = prop_response(
                &book,
                "<d:resourcetype><d:collection/><card:addressbook/></d:resourcetype><d:displayname>Contacts</d:displayname>",
            );
            if depth == "1" {
                for (href, card) in &lock(&shared.model).cards {
                    body.push_str(&prop_response(
                        href,
                        &format!(
                            "<d:getetag>{}</d:getetag><d:resourcetype/>",
                            escape(&card.etag)
                        ),
                    ));
                }
            }
            multistatus(&body)
        }
        "REPORT" if path == book => {
            let model = lock(&shared.model);
            let mut body = String::new();
            for href in requested_hrefs(&String::from_utf8_lossy(&request.body)) {
                match model.cards.get(&href) {
                    Some(card) => body.push_str(&prop_response(
                        &href,
                        &format!(
                            "<d:getetag>{}</d:getetag><card:address-data>{}</card:address-data>",
                            escape(&card.etag),
                            escape(&card.data)
                        ),
                    )),
                    None => body.push_str(&format!(
                        "<d:response><d:href>{}</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>",
                        escape(&href)
                    )),
                }
            }
            multistatus(&body)
        }
        "GET" if path.starts_with(&book) => match lock(&shared.model).cards.get(path) {
            Some(card) => RawResponse {
                status: 200,
                headers: vec![
                    ("Content-Type".into(), "text/vcard".into()),
                    ("ETag".into(), card.etag.clone()),
                ],
                body: card.data.clone().into_bytes(),
            },
            None => status(404),
        },
        "PUT" if path.starts_with(&book) && path.len() > book.len() => {
            let data = String::from_utf8_lossy(&request.body).into_owned();
            let if_match = request.header("if-match");
            let none_match = request.header("if-none-match") == Some("*");
            if if_match.is_none() && !none_match {
                // Écriture inconditionnelle : jamais faite par le client.
                return status(428);
            }
            match lock(&shared.model).put_if(path, if_match, &data) {
                Ok(etag) => {
                    let mut headers = Vec::new();
                    if *lock(&shared.etag_on_put) {
                        headers.push(("ETag".into(), etag));
                    }
                    RawResponse {
                        status: if if_match.is_some() { 204 } else { 201 },
                        headers,
                        body: Vec::new(),
                    }
                }
                Err(_) => status(412),
            }
        }
        "DELETE" if path.starts_with(&book) => {
            let Some(if_match) = request.header("if-match") else {
                return status(428);
            };
            match lock(&shared.model).delete_if(path, if_match) {
                Ok(()) => status(204),
                Err(ModelError::NotFound) => status(404),
                Err(ModelError::Precondition) => status(412),
            }
        }
        _ => status(404),
    }
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    shared: &Shared,
) -> std::io::Result<()> {
    while let Some(request) = read_request(&mut stream).await {
        lock(&shared.log).push(LoggedRequest {
            method: request.method.clone(),
            path: request.path.clone(),
            authorized: request.header("authorization").is_some(),
        });
        let response = handle(&request, shared);
        let mut head = format!("HTTP/1.1 {} X\r\n", response.status);
        for (name, value) in &response.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str(&format!("Content-Length: {}\r\n\r\n", response.body.len()));
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(&response.body).await?;
        stream.flush().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_examples() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn conditional_writes_follow_etags() {
        let mut model = CardDavModel::default();
        let first = model.put_if("/a", None, "A");
        assert!(first.is_ok());
        assert_eq!(model.put_if("/a", None, "A"), Err(ModelError::Precondition));
        assert_eq!(
            model.put_if("/a", Some("\"autre\""), "A"),
            Err(ModelError::Precondition)
        );
        let etag = first.unwrap_or_default();
        assert!(model.put_if("/a", Some(&etag), "B").is_ok());
        assert_eq!(model.delete_if("/a", &etag), Err(ModelError::Precondition));
        assert_eq!(model.delete_if("/b", &etag), Err(ModelError::NotFound));
    }

    #[test]
    fn multiget_hrefs_are_found() {
        let body = "<card:addressbook-multiget xmlns:d=\"DAV:\"><d:prop/><d:href>/a.vcf</d:href><d:href>/b&amp;c.vcf</d:href></card:addressbook-multiget>";
        assert_eq!(requested_hrefs(body), ["/a.vcf", "/b&c.vcf"]);
    }
}
