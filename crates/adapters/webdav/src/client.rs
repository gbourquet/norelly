//! Client CardDAV en HTTPS.
//!
//! Chaque requête porte les identifiants en HTTP Basic et part vers une
//! adresse que le domaine a validée sur l'origine du serveur ; une
//! redirection n'est suivie que pendant la découverte, et seulement sur
//! cette origine ([`redirect_target`]).

use std::time::Duration;

use norelly_domain::carddav::{
    AddressBook, CardUid, DavCredentials, DavUrl, ETag, Href, MAX_ADDRESS_BOOKS, MAX_CARD_BYTES,
    MAX_DISCOVERY_RESPONSE_BYTES, MAX_LISTING_BYTES, MAX_MULTIGET_BYTES, MAX_MULTIGET_HREFS,
    MAX_REMOTE_CARDS, RemoteCard, RemoteEntry, discovery_candidates, redirect_target,
};
use norelly_domain::ports::{CardDavClient, CardDavError, RemoteListing};
use norelly_tls::TlsClientFactory;
use reqwest::{Method, StatusCode};

use crate::xml::{self, DavResponse};

/// Délai de connexion.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Délai total d'une requête (une liste de 25 000 fiches compris).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// Réponses lues au plus dans une découverte (principal, dossier).
const MAX_DISCOVERY_RESPONSES: usize = MAX_ADDRESS_BOOKS + 1;

/// Erreur de construction du client.
#[derive(Debug)]
pub struct WebDavSetupError(pub String);

impl std::fmt::Display for WebDavSetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "client CardDAV impossible à construire : {}", self.0)
    }
}

/// Client CardDAV.
#[derive(Debug, Clone)]
pub struct WebDavClient {
    http: reqwest::Client,
}

/// Réponse lue, corps borné.
struct Answer {
    status: StatusCode,
    location: Option<String>,
    etag: Option<String>,
    /// Le serveur demande des identifiants HTTP Basic (401 avec
    /// `WWW-Authenticate: Basic`).
    basic_challenge: bool,
    body: Vec<u8>,
}

/// Envoi des identifiants : toujours (carnet relié, origine confirmée),
/// ou seulement une fois que le serveur les a demandés (découverte : un
/// nom de domaine saisi peut désigner un site web qui n'est pas le serveur
/// de contacts ; il ne reçoit jamais le mot de passe).
enum Auth<'a> {
    Always,
    OnChallenge(&'a mut bool),
}

fn method(name: &str) -> Result<Method, CardDavError> {
    Method::from_bytes(name.as_bytes()).map_err(|e| CardDavError::Protocol(e.to_string()))
}

/// Erreur TLS dans la chaîne des causes de `error`, erreurs d'E/S
/// emboîtées comprises (leur cause n'est visible que par `get_ref`).
fn tls_cause<'e>(
    error: &'e (dyn std::error::Error + 'static),
) -> Option<&'e norelly_tls::rustls::Error> {
    if let Some(tls) = error.downcast_ref::<norelly_tls::rustls::Error>() {
        return Some(tls);
    }
    if let Some(io) = error.downcast_ref::<std::io::Error>()
        && let Some(inner) = io.get_ref()
    {
        let inner: &(dyn std::error::Error + 'static) = inner;
        return tls_cause(inner);
    }
    error.source().and_then(tls_cause)
}

/// Problème de certificat à l'origine d'une erreur.
fn certificate_problem(error: &reqwest::Error) -> Option<String> {
    let tls = tls_cause(error)?;
    matches!(tls, norelly_tls::rustls::Error::InvalidCertificate(_))
        .then(|| format!("{:?}", norelly_tls::certificate_issue(tls)))
}

/// Erreur de transport, sans l'adresse (ni identifiant ni chemin dans les
/// journaux).
fn transport_error(error: reqwest::Error) -> CardDavError {
    if let Some(problem) = certificate_problem(&error) {
        return CardDavError::Certificate(problem);
    }
    if error.is_timeout() {
        CardDavError::Timeout
    } else {
        CardDavError::Network(error.without_url().to_string())
    }
}

/// Valeur d'`If-Match` pour `etag`. `If-Match` compare fortement : une
/// ETag faible (`W/"…"`, donnée par certains serveurs ou mandataires) ne
/// correspondrait jamais, sa partie opaque est envoyée.
fn if_match(etag: &ETag) -> &str {
    let raw = etag.as_str();
    raw.strip_prefix("W/").unwrap_or(raw)
}

/// Erreur d'une réponse hors succès.
fn status_error(status: StatusCode) -> CardDavError {
    match status.as_u16() {
        401 | 403 => CardDavError::Unauthorized,
        404 | 410 => CardDavError::NotFound,
        412 => CardDavError::PreconditionFailed,
        413 | 507 => CardDavError::TooLarge,
        code => CardDavError::Server(code),
    }
}

impl WebDavClient {
    /// HTTPS seulement, sans redirection automatique, sans cookies ni
    /// `Referer`, vérification de `tls` sans exception.
    pub fn new(tls: &TlsClientFactory) -> Result<Self, WebDavSetupError> {
        let config = tls
            .standard_client_config()
            .map_err(|e| WebDavSetupError(e.to_string()))?;
        let http = reqwest::Client::builder()
            .use_preconfigured_tls(config)
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .user_agent("Norelly")
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| WebDavSetupError(e.to_string()))?;
        Ok(Self { http })
    }

    /// Envoie une requête vers `url` (validée par le domaine), avec ou sans
    /// identifiants selon `auth`, et lit au plus `max_bytes` octets de
    /// réponse (`TooLarge` au-delà).
    async fn send(
        &self,
        verb: &str,
        url: &DavUrl,
        credentials: &DavCredentials,
        headers: &[(&str, &str)],
        body: Option<String>,
        max_bytes: usize,
    ) -> Result<Answer, CardDavError> {
        self.send_with(
            verb,
            url,
            credentials,
            Auth::Always,
            headers,
            body,
            max_bytes,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_with(
        &self,
        verb: &str,
        url: &DavUrl,
        credentials: &DavCredentials,
        auth: Auth<'_>,
        headers: &[(&str, &str)],
        body: Option<String>,
        max_bytes: usize,
    ) -> Result<Answer, CardDavError> {
        match auth {
            Auth::Always => {
                self.send_once(verb, url, Some(credentials), headers, body, max_bytes)
                    .await
            }
            Auth::OnChallenge(confirmed) => {
                if *confirmed {
                    return self
                        .send_once(verb, url, Some(credentials), headers, body, max_bytes)
                        .await;
                }
                let answer = self
                    .send_once(verb, url, None, headers, body.clone(), max_bytes)
                    .await?;
                if !answer.basic_challenge {
                    return Ok(answer);
                }
                *confirmed = true;
                self.send_once(verb, url, Some(credentials), headers, body, max_bytes)
                    .await
            }
        }
    }

    async fn send_once(
        &self,
        verb: &str,
        url: &DavUrl,
        credentials: Option<&DavCredentials>,
        headers: &[(&str, &str)],
        body: Option<String>,
        max_bytes: usize,
    ) -> Result<Answer, CardDavError> {
        let mut request = self.http.request(method(verb)?, url.to_string());
        if let Some(credentials) = credentials {
            request = request.basic_auth(
                credentials.username.as_str(),
                Some(credentials.password.expose()),
            );
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        let mut response = request.send().await.map_err(transport_error)?;
        let status = response.status();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let location = header("location");
        let etag = header("etag");
        let basic_challenge = status == StatusCode::UNAUTHORIZED
            && response
                .headers()
                .get_all("www-authenticate")
                .iter()
                .filter_map(|v| v.to_str().ok())
                .any(|v| {
                    v.trim_start()
                        .get(..5)
                        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("basic"))
                });
        if response
            .content_length()
            .is_some_and(|n| n > max_bytes as u64)
        {
            return Err(CardDavError::TooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if body.len() + chunk.len() > max_bytes {
                return Err(CardDavError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Answer {
            status,
            location,
            etag,
            basic_challenge,
            body,
        })
    }

    /// `PROPFIND` lu en `207 Multi-Status`.
    #[allow(clippy::too_many_arguments)]
    async fn propfind(
        &self,
        url: &DavUrl,
        credentials: &DavCredentials,
        auth: Auth<'_>,
        depth: &str,
        body: &str,
        max_bytes: usize,
        max_responses: usize,
    ) -> Result<Vec<DavResponse>, CardDavError> {
        let answer = self
            .send_with(
                "PROPFIND",
                url,
                credentials,
                auth,
                &[
                    ("Depth", depth),
                    ("Content-Type", "application/xml; charset=utf-8"),
                ],
                Some(body.to_owned()),
                max_bytes,
            )
            .await?;
        if answer.status.as_u16() != 207 {
            return Err(status_error(answer.status));
        }
        xml::parse_multistatus(&answer.body, max_responses)
    }

    /// `PROPFIND` de découverte, en suivant les redirections sur la même
    /// origine ; renvoie l'adresse atteinte et sa réponse.
    async fn discover_at(
        &self,
        start: &DavUrl,
        credentials: &DavCredentials,
        confirmed: &mut bool,
    ) -> Result<(DavUrl, DavResponse), CardDavError> {
        let mut url = start.clone();
        let mut followed = 0;
        loop {
            let answer = self
                .send_with(
                    "PROPFIND",
                    &url,
                    credentials,
                    Auth::OnChallenge(&mut *confirmed),
                    &[
                        ("Depth", "0"),
                        ("Content-Type", "application/xml; charset=utf-8"),
                    ],
                    Some(xml::DISCOVERY_PROPFIND.to_owned()),
                    MAX_DISCOVERY_RESPONSE_BYTES,
                )
                .await?;
            match answer.status.as_u16() {
                207 => {
                    let responses = xml::parse_multistatus(&answer.body, MAX_DISCOVERY_RESPONSES)?;
                    let found = responses
                        .into_iter()
                        .next()
                        .ok_or_else(|| CardDavError::Protocol("réponse vide".into()))?;
                    return Ok((url, found));
                }
                301 | 302 | 303 | 307 | 308 => {
                    let location = answer.location.unwrap_or_default();
                    url = redirect_target(&url, &location, followed).ok_or_else(|| {
                        CardDavError::Protocol("redirection hors du serveur".into())
                    })?;
                    followed += 1;
                }
                _ => return Err(status_error(answer.status)),
            }
        }
    }

    /// Carnets trouvés depuis `start` : carnet lui-même, ou dossier des
    /// carnets du principal.
    async fn books_from(
        &self,
        start: &DavUrl,
        credentials: &DavCredentials,
        confirmed: &mut bool,
    ) -> Result<Vec<AddressBook>, CardDavError> {
        let (url, found) = self.discover_at(start, credentials, confirmed).await?;
        if found.addressbook {
            return Ok(vec![AddressBook::new(
                url,
                found.displayname.as_deref().unwrap_or_default(),
            )]);
        }
        let home = if let Some(home) = &found.addressbook_home_set {
            url.join(home)
        } else if let Some(principal) = &found.current_user_principal {
            let principal = url
                .join(principal)
                .ok_or_else(|| CardDavError::Protocol("principal hors du serveur".into()))?;
            let (principal, props) = self.discover_at(&principal, credentials, confirmed).await?;
            props
                .addressbook_home_set
                .as_deref()
                .and_then(|home| principal.join(home))
        } else {
            Some(url.clone())
        }
        .ok_or(CardDavError::NoAddressBook)?
        .collection();
        let responses = self
            .propfind(
                &home,
                credentials,
                Auth::OnChallenge(confirmed),
                "1",
                xml::DISCOVERY_PROPFIND,
                MAX_DISCOVERY_RESPONSE_BYTES,
                MAX_DISCOVERY_RESPONSES,
            )
            .await?;
        Ok(responses
            .iter()
            .filter(|r| r.addressbook)
            .filter_map(|r| {
                let book = home.join(&r.href)?;
                Some(AddressBook::new(
                    book,
                    r.displayname.as_deref().unwrap_or_default(),
                ))
            })
            .take(MAX_ADDRESS_BOOKS)
            .collect())
    }

    /// Écriture `PUT` conditionnelle.
    async fn put(
        &self,
        book: &DavUrl,
        credentials: &DavCredentials,
        href: &Href,
        condition: (&str, &str),
        card: &str,
    ) -> Result<Option<ETag>, CardDavError> {
        if !book.contains(href) {
            return Err(CardDavError::Protocol("fiche hors du carnet".into()));
        }
        let answer = self
            .send(
                "PUT",
                &book.at(href),
                credentials,
                &[("Content-Type", "text/vcard; charset=utf-8"), condition],
                Some(card.to_owned()),
                MAX_DISCOVERY_RESPONSE_BYTES,
            )
            .await?;
        if !answer.status.is_success() {
            return Err(status_error(answer.status));
        }
        Ok(answer.etag.as_deref().and_then(ETag::parse))
    }
}

impl CardDavClient for WebDavClient {
    async fn address_books(
        &self,
        server: &DavUrl,
        credentials: &DavCredentials,
    ) -> Result<Vec<AddressBook>, CardDavError> {
        let mut last = CardDavError::NoAddressBook;
        // Le mot de passe ne part qu'une fois demandé par le serveur.
        let mut confirmed = false;
        for candidate in discovery_candidates(server) {
            match self
                .books_from(&candidate, credentials, &mut confirmed)
                .await
            {
                Ok(books) if !books.is_empty() => return Ok(books),
                Ok(_) => last = CardDavError::NoAddressBook,
                // Serveur injoignable, certificat ou identifiants refusés :
                // les autres adresses du même serveur n'y changeront rien.
                Err(
                    e @ (CardDavError::Unauthorized
                    | CardDavError::Certificate(_)
                    | CardDavError::Network(_)
                    | CardDavError::Timeout),
                ) => return Err(e),
                Err(e) => last = e,
            }
        }
        Err(match last {
            CardDavError::NotFound => CardDavError::NoAddressBook,
            other => other,
        })
    }

    async fn entries(
        &self,
        book: &DavUrl,
        credentials: &DavCredentials,
    ) -> Result<RemoteListing, CardDavError> {
        let book = book.collection();
        let responses = self
            .propfind(
                &book,
                credentials,
                Auth::Always,
                "1",
                xml::ENTRIES_PROPFIND,
                MAX_LISTING_BYTES,
                MAX_REMOTE_CARDS + 1,
            )
            .await?;
        let mut entries = Vec::new();
        let mut omitted = 0;
        for response in responses {
            if response.collection {
                continue;
            }
            // Ressource hors du carnet, sans ETag ou illisible : omise, et
            // comptée (la liste n'est plus sûre pour des suppressions).
            let entry = book
                .join(&response.href)
                .map(|url| url.href())
                .filter(|href| book.contains(href))
                .zip(response.etag.as_deref().and_then(ETag::parse));
            let Some((href, etag)) = entry else {
                omitted += 1;
                continue;
            };
            if entries.len() >= MAX_REMOTE_CARDS {
                return Err(CardDavError::TooLarge);
            }
            entries.push(RemoteEntry { href, etag });
        }
        Ok(RemoteListing { entries, omitted })
    }

    async fn cards(
        &self,
        book: &DavUrl,
        credentials: &DavCredentials,
        hrefs: &[Href],
    ) -> Result<Vec<RemoteCard>, CardDavError> {
        if hrefs.is_empty() {
            return Ok(Vec::new());
        }
        if hrefs.len() > MAX_MULTIGET_HREFS {
            return Err(CardDavError::Protocol("trop de fiches demandées".into()));
        }
        let book = book.collection();
        let answer = self
            .send(
                "REPORT",
                &book,
                credentials,
                &[
                    ("Depth", "1"),
                    ("Content-Type", "application/xml; charset=utf-8"),
                ],
                Some(xml::multiget(hrefs)),
                MAX_MULTIGET_BYTES,
            )
            .await?;
        if answer.status.as_u16() != 207 {
            return Err(status_error(answer.status));
        }
        let responses = xml::parse_multistatus(&answer.body, hrefs.len())?;
        Ok(responses
            .into_iter()
            .filter_map(|r| {
                let href = book.join(&r.href)?.href();
                // Seules les fiches demandées, du carnet, sont reprises.
                if !hrefs.contains(&href) || !book.contains(&href) {
                    return None;
                }
                let data = r.address_data?;
                if data.len() > MAX_CARD_BYTES {
                    return None;
                }
                Some(RemoteCard {
                    href,
                    etag: ETag::parse(r.etag.as_deref()?)?,
                    data,
                })
            })
            .collect())
    }

    async fn create(
        &self,
        book: &DavUrl,
        credentials: &DavCredentials,
        href: &Href,
        card: &str,
    ) -> Result<Option<ETag>, CardDavError> {
        self.put(book, credentials, href, ("If-None-Match", "*"), card)
            .await
    }

    async fn update(
        &self,
        book: &DavUrl,
        credentials: &DavCredentials,
        href: &Href,
        etag: &ETag,
        card: &str,
    ) -> Result<Option<ETag>, CardDavError> {
        self.put(book, credentials, href, ("If-Match", if_match(etag)), card)
            .await
    }

    async fn delete(
        &self,
        book: &DavUrl,
        credentials: &DavCredentials,
        href: &Href,
        etag: &ETag,
    ) -> Result<(), CardDavError> {
        if !book.contains(href) {
            return Err(CardDavError::Protocol("fiche hors du carnet".into()));
        }
        let answer = self
            .send(
                "DELETE",
                &book.at(href),
                credentials,
                &[("If-Match", if_match(etag))],
                None,
                MAX_DISCOVERY_RESPONSE_BYTES,
            )
            .await?;
        match answer.status.as_u16() {
            200..=299 | 404 | 410 => Ok(()),
            _ => Err(status_error(answer.status)),
        }
    }

    fn new_uid(&self) -> CardUid {
        let mut bytes = [0u8; 16];
        if getrandom::fill(&mut bytes).is_err() {
            // Aléa indisponible : horloge et adresse d'une variable, pour
            // un UID unique en pratique (il ne protège rien).
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            bytes = nanos.to_le_bytes();
        }
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let uid = format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        );
        // Toujours valide : 36 caractères hexadécimaux et tirets.
        CardUid::parse(&uid).unwrap_or_else(|| unreachable!("UID hexadécimal"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_become_errors() {
        assert_eq!(
            status_error(StatusCode::UNAUTHORIZED),
            CardDavError::Unauthorized
        );
        assert_eq!(
            status_error(StatusCode::FORBIDDEN),
            CardDavError::Unauthorized
        );
        assert_eq!(status_error(StatusCode::GONE), CardDavError::NotFound);
        assert_eq!(
            status_error(StatusCode::PRECONDITION_FAILED),
            CardDavError::PreconditionFailed
        );
        assert_eq!(
            status_error(StatusCode::INSUFFICIENT_STORAGE),
            CardDavError::TooLarge
        );
        assert_eq!(
            status_error(StatusCode::BAD_GATEWAY),
            CardDavError::Server(502)
        );
    }

    #[test]
    fn conditions_use_the_opaque_tag() {
        let tag = |raw: &str| ETag::parse(raw).unwrap_or_else(|| panic!("{raw}"));
        assert_eq!(if_match(&tag("\"abc\"")), "\"abc\"");
        // `If-Match` compare fortement : une ETag faible ne correspondrait
        // jamais.
        assert_eq!(if_match(&tag("W/\"abc\"")), "\"abc\"");
    }

    #[test]
    fn uids_are_random_and_valid() {
        let client =
            WebDavClient::new(&TlsClientFactory::platform()).unwrap_or_else(|e| panic!("{e}"));
        let a = client.new_uid();
        assert_eq!(a.as_str().len(), 36);
        assert_ne!(a, client.new_uid());
    }
}
