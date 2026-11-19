//! Synchronisation CardDAV du carnet (ADR-014).
//!
//! Le carnet local (ADR-012) est tenu à jour avec **un** carnet distant,
//! dans les deux sens. Ce module porte les règles, sans E/S :
//!
//! - [`DavUrl`] : adresse HTTPS d'un serveur ou d'un carnet, sans
//!   identifiants ni requête ; une ressource citée par le serveur n'est
//!   suivie que sur la même origine ([`DavUrl::join`]) : le mot de passe ne
//!   part jamais ailleurs ;
//! - [`CardDavAccount`] : serveur, identifiant, carnet choisi ;
//! - [`newest_side`] : la dernière modification gagne (horloge reçue en
//!   paramètre) ;
//! - [`merge_card`] : une fiche modifiée ici remplace, dans la vCard du
//!   serveur, les seules propriétés que le carnet connaît ; le reste
//!   (photo, second prénom, anniversaire en texte…) est gardé ;
//! - [`plan_sync`] : ce que la synchronisation écrit de chaque côté.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::contact_details::{Birthday, website_url};
use crate::contact_import::{DecodedContacts, DuplicatePolicy, ImportedCard, plan_import};
use crate::contacts::{AddressKey, Contact, ContactDraft, ContactId};
use crate::credentials::{CredentialsError, MAX_PASSWORD_LEN, Secret, Username};
use crate::endpoint::Host;
use crate::ports::TextNormalizer;

#[cfg(test)]
mod tests;

/// Longueur maximale d'une adresse de serveur, de carnet ou de ressource.
pub const MAX_DAV_URL_CHARS: usize = 2048;
/// Nombre maximal de fiches d'un carnet distant (celui d'un import).
pub const MAX_REMOTE_CARDS: usize = crate::contact_import::MAX_IMPORT_CARDS;
/// Nombre maximal de carnets proposés par un serveur.
pub const MAX_ADDRESS_BOOKS: usize = 100;
/// Taille maximale d'une réponse de découverte, en octets.
pub const MAX_DISCOVERY_RESPONSE_BYTES: usize = 1024 * 1024;
/// Taille maximale de la liste des fiches d'un carnet, en octets.
pub const MAX_LISTING_BYTES: usize = 16 * 1024 * 1024;
/// Taille maximale d'une fiche distante, en octets (photo comprise).
pub const MAX_CARD_BYTES: usize = 1024 * 1024;
/// Fiches demandées au plus par requête.
pub const MAX_MULTIGET_HREFS: usize = 20;
/// Taille maximale d'une réponse de fiches, en octets.
pub const MAX_MULTIGET_BYTES: usize = MAX_MULTIGET_HREFS * MAX_CARD_BYTES + 1024 * 1024;
/// Fiches téléchargées au plus par synchronisation, en octets : au-delà,
/// les suivantes sont remises à la synchronisation suivante (un carnet
/// plein de photos se reprend en plusieurs fois, sans tout charger).
pub const MAX_SYNC_FETCH_BYTES: usize = 64 * 1024 * 1024;
/// Profondeur maximale d'une réponse XML.
pub const MAX_XML_DEPTH: usize = 32;
/// Redirections suivies au plus pendant la découverte.
pub const MAX_DAV_REDIRECTS: usize = 3;
/// Longueur maximale du nom d'un carnet affiché, en caractères.
pub const MAX_BOOK_NAME_CHARS: usize = 200;
/// Longueur maximale d'une ETag, en caractères.
pub const MAX_ETAG_CHARS: usize = 256;
/// Longueur maximale de l'UID d'une fiche créée ici.
pub const MAX_CARD_UID_CHARS: usize = 64;

// ---------------------------------------------------------------------------
// Adresses
// ---------------------------------------------------------------------------

/// Adresse refusée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DavUrlError {
    #[error("adresse vide")]
    Empty,
    #[error("adresse trop longue")]
    TooLong,
    /// Seul HTTPS est permis : le mot de passe part à chaque requête.
    #[error("adresse non HTTPS")]
    NotHttps,
    /// Identifiants dans l'adresse.
    #[error("identifiants dans l'adresse")]
    UserInfo,
    #[error("requête ou fragment dans l'adresse")]
    QueryOrFragment,
    #[error("nom d'hôte invalide")]
    InvalidHost,
    #[error("port invalide")]
    InvalidPort,
    #[error("chemin invalide")]
    InvalidPath,
}

/// Adresse HTTPS d'un serveur, d'un carnet ou d'une fiche : hôte validé
/// ([`Host`]), port, chemin absolu en ASCII (caractères hors ASCII
/// encodés en `%XX`), sans `.` ni `..`, sans requête ni fragment.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DavUrl {
    host: Host,
    port: Option<u16>,
    path: String,
}

/// Chemin absolu d'une ressource (fiche) sur le serveur du carnet.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Href(String);

/// Caractère non réservé (RFC 3986) : son encodage `%XX` est retiré.
fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

fn hex_value(b: u8) -> Option<u8> {
    char::from(b)
        .to_digit(16)
        .and_then(|d| u8::try_from(d).ok())
}

/// Chemin absolu validé, sous sa forme canonique : `%XX` des caractères non
/// réservés décodés, autres en majuscules, pour qu'une ressource ait une
/// seule écriture quelle que soit celle du serveur. `encode` : espace et
/// caractères hors ASCII encodés (adresse citée par un serveur) ; sinon
/// refusés (adresse saisie). Sans `.` ni `..`, même encodés.
fn canonical_path(path: &str, encode: bool) -> Result<String, DavUrlError> {
    if path.chars().count() > MAX_DAV_URL_CHARS {
        return Err(DavUrlError::TooLong);
    }
    if !path.starts_with('/') || path.starts_with("//") {
        return Err(DavUrlError::InvalidPath);
    }
    let bytes = path.as_bytes();
    let mut out = String::with_capacity(path.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' {
            let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex_value),
                bytes.get(i + 2).copied().and_then(hex_value),
            ) else {
                return Err(DavUrlError::InvalidPath);
            };
            let decoded = hi * 16 + lo;
            if unreserved(decoded) {
                out.push(char::from(decoded));
            } else {
                out.push_str(&format!("%{decoded:02X}"));
            }
            i += 3;
            continue;
        }
        let printable = b.is_ascii_graphic() && !matches!(b, b'?' | b'#' | b'\\');
        if printable {
            out.push(char::from(b));
        } else if encode && (b == b' ' || !b.is_ascii()) {
            out.push_str(&format!("%{b:02X}"));
        } else {
            return Err(DavUrlError::InvalidPath);
        }
        i += 1;
    }
    if out.chars().count() > MAX_DAV_URL_CHARS {
        return Err(DavUrlError::TooLong);
    }
    if out
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(DavUrlError::InvalidPath);
    }
    Ok(out)
}

/// Hôte et port d'une autorité (`hôte`, `hôte:port`, `[ipv6]:port`).
fn split_authority(authority: &str) -> Result<(Host, Option<u16>), DavUrlError> {
    if authority.contains('@') {
        return Err(DavUrlError::UserInfo);
    }
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or(DavUrlError::InvalidHost)?;
        if after.is_empty() {
            (host, None)
        } else {
            (
                host,
                Some(after.strip_prefix(':').ok_or(DavUrlError::InvalidPort)?),
            )
        }
    } else {
        match authority.split_once(':') {
            Some((_, port)) if port.contains(':') => return Err(DavUrlError::InvalidHost),
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return Err(DavUrlError::InvalidHost);
    }
    let host = Host::parse(host).map_err(|_| DavUrlError::InvalidHost)?;
    let port = match port {
        None => None,
        Some(raw) => {
            if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
                return Err(DavUrlError::InvalidPort);
            }
            match raw.parse::<u16>() {
                Ok(0) | Err(_) => return Err(DavUrlError::InvalidPort),
                Ok(443) => None,
                Ok(port) => Some(port),
            }
        }
    };
    Ok((host, port))
}

impl Href {
    /// Chemin absolu valide (voir [`DavUrl`]).
    pub fn parse(raw: &str) -> Result<Self, DavUrlError> {
        canonical_path(raw, false).map(Self)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl DavUrl {
    /// Adresse saisie ou reçue : sans schéma, HTTPS est supposé.
    pub fn parse(raw: &str) -> Result<Self, DavUrlError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(DavUrlError::Empty);
        }
        if raw.chars().count() > MAX_DAV_URL_CHARS {
            return Err(DavUrlError::TooLong);
        }
        let rest = match raw.find("://") {
            Some(i) if raw[..i].eq_ignore_ascii_case("https") => &raw[i + 3..],
            Some(_) => return Err(DavUrlError::NotHttps),
            None => raw,
        };
        if rest.contains(['?', '#']) {
            return Err(DavUrlError::QueryOrFragment);
        }
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (host, port) = split_authority(authority)?;
        Ok(Self {
            host,
            port,
            path: canonical_path(path, false)?,
        })
    }

    #[must_use]
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// Port (443 par défaut).
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port.unwrap_or(443)
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Même hôte et même port.
    #[must_use]
    pub fn same_origin(&self, other: &Self) -> bool {
        self.host == other.host && self.port() == other.port()
    }

    /// Ressource citée par le serveur (`href` : adresse complète, chemin
    /// absolu ou relatif à cette adresse), sur la même origine seulement.
    #[must_use]
    pub fn join(&self, href: &str) -> Option<Self> {
        let href = href.trim();
        if href.is_empty() || href.starts_with("//") {
            return None;
        }
        if let Some((scheme, rest)) = href.split_once("://") {
            if !scheme.eq_ignore_ascii_case("https") || rest.contains(['?', '#']) {
                return None;
            }
            let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
            let (host, port) = split_authority(authority).ok()?;
            let url = Self {
                host,
                port,
                path: canonical_path(if path.is_empty() { "/" } else { path }, true).ok()?,
            };
            return self.same_origin(&url).then_some(url);
        }
        if href.contains(['?', '#']) {
            return None;
        }
        let path = if href.starts_with('/') {
            href.to_owned()
        } else {
            let dir = &self.path[..=self.path.rfind('/')?];
            format!("{dir}{href}")
        };
        Some(Self {
            host: self.host.clone(),
            port: self.port,
            path: canonical_path(&path, true).ok()?,
        })
    }

    /// `href` est une fiche de ce carnet : sous son chemin, sans être le
    /// carnet lui-même.
    #[must_use]
    pub fn contains(&self, href: &Href) -> bool {
        let book = self.collection();
        href.0.len() > book.path.len() && href.0.starts_with(&book.path)
    }

    /// La même adresse en collection (chemin terminé par `/`).
    #[must_use]
    pub fn collection(&self) -> Self {
        let mut url = self.clone();
        if !url.path.ends_with('/') {
            url.path.push('/');
        }
        url
    }

    /// Même origine, autre chemin.
    #[must_use]
    pub fn at(&self, href: &Href) -> Self {
        Self {
            host: self.host.clone(),
            port: self.port,
            path: href.0.clone(),
        }
    }

    /// Chemin de l'adresse, en ressource.
    #[must_use]
    pub fn href(&self) -> Href {
        Href(self.path.clone())
    }
}

impl fmt::Display for DavUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // L'hôte IPv6 s'affiche entre crochets.
        write!(f, "https://{}", self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        f.write_str(&self.path)
    }
}

/// Adresses à essayer pour découvrir les carnets : depuis la racine d'un
/// serveur, d'abord l'adresse bien connue (RFC 6764), puis la racine ;
/// sinon l'adresse saisie seule.
#[must_use]
pub fn discovery_candidates(server: &DavUrl) -> Vec<DavUrl> {
    if server.path == "/" {
        let mut well_known = server.clone();
        "/.well-known/carddav".clone_into(&mut well_known.path);
        vec![well_known, server.clone()]
    } else {
        vec![server.clone()]
    }
}

/// Cible d'une redirection reçue de `from` (`location`) après `followed`
/// redirections : sur la même origine, en HTTPS, dans la limite de
/// [`MAX_DAV_REDIRECTS`] ; sinon `None`.
#[must_use]
pub fn redirect_target(from: &DavUrl, location: &str, followed: usize) -> Option<DavUrl> {
    if followed >= MAX_DAV_REDIRECTS {
        return None;
    }
    from.join(location)
}

/// Étiquette de version d'une ressource (`ETag`), en ASCII visible,
/// bornée.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ETag(String);

impl ETag {
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        // ASCII visible seulement (RFC 9110) : une ETag passe telle quelle
        // dans un en-tête `If-Match`.
        let valid = !raw.is_empty()
            && raw.len() <= MAX_ETAG_CHARS
            && raw.bytes().all(|b| b.is_ascii_graphic());
        valid.then(|| Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// UID d'une fiche créée ici (lettres, chiffres et `-`), qui nomme aussi
/// sa ressource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardUid(String);

impl CardUid {
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let valid = (1..=MAX_CARD_UID_CHARS).contains(&raw.len())
            && raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        valid.then(|| Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Ressource d'une fiche créée ici : `<uid>.vcf` dans le carnet.
#[must_use]
pub fn new_card_href(book: &DavUrl, uid: &CardUid) -> Href {
    Href(format!("{}{}.vcf", book.collection().path, uid.0))
}

// ---------------------------------------------------------------------------
// Compte
// ---------------------------------------------------------------------------

/// Compte CardDAV refusé.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DavAccountError {
    #[error(transparent)]
    Url(#[from] DavUrlError),
    #[error(transparent)]
    Username(CredentialsError),
    /// Un identifiant HTTP Basic ne peut pas contenir `:` (RFC 7617).
    #[error("identifiant contenant « : »")]
    UsernameColon,
    #[error(transparent)]
    Password(CredentialsError),
    /// Le carnet n'est pas sur le serveur du compte.
    #[error("carnet hors du serveur")]
    BookOutsideServer,
}

/// Identifiant du compte CardDAV, utilisable en HTTP Basic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DavUsername(Username);

impl DavUsername {
    pub fn parse(raw: &str) -> Result<Self, DavAccountError> {
        let username = Username::parse(raw).map_err(DavAccountError::Username)?;
        if raw.contains(':') {
            return Err(DavAccountError::UsernameColon);
        }
        Ok(Self(username))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Mot de passe d'application : non vide, sans caractère nul, borné.
pub fn validate_password(password: &Secret) -> Result<(), DavAccountError> {
    let raw = password.expose();
    let error = if raw.is_empty() {
        CredentialsError::EmptySecret
    } else if raw.contains('\0') {
        CredentialsError::SecretNulCharacter
    } else if raw.len() > MAX_PASSWORD_LEN {
        CredentialsError::SecretTooLong
    } else {
        return Ok(());
    };
    Err(DavAccountError::Password(error))
}

/// Identifiants présentés au serveur (HTTP Basic, sur TLS seulement). Le
/// mot de passe est masqué dans `Debug` ([`Secret`]).
#[derive(Debug, Clone)]
pub struct DavCredentials {
    pub username: DavUsername,
    pub password: Secret,
}

/// Carnet distant : adresse (collection) et nom affiché (nettoyé, borné).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressBook {
    url: DavUrl,
    name: String,
}

impl AddressBook {
    #[must_use]
    pub fn new(url: DavUrl, name: &str) -> Self {
        let cleaned: String = name
            .chars()
            .map(|c| {
                if c.is_control() || crate::connection::is_format_char(c) {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        let name: String = cleaned
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(MAX_BOOK_NAME_CHARS)
            .collect();
        Self {
            url: url.collection(),
            name,
        }
    }

    #[must_use]
    pub fn url(&self) -> &DavUrl {
        &self.url
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Compte CardDAV : un seul, relié au carnet unique.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardDavAccount {
    server: DavUrl,
    username: DavUsername,
    book: AddressBook,
}

impl CardDavAccount {
    /// Le carnet doit être sur le serveur (même origine).
    pub fn new(
        server: DavUrl,
        username: DavUsername,
        book: AddressBook,
    ) -> Result<Self, DavAccountError> {
        if !book.url.same_origin(&server) {
            return Err(DavAccountError::BookOutsideServer);
        }
        Ok(Self {
            server,
            username,
            book,
        })
    }

    #[must_use]
    pub fn server(&self) -> &DavUrl {
        &self.server
    }

    #[must_use]
    pub fn username(&self) -> &DavUsername {
        &self.username
    }

    #[must_use]
    pub fn book(&self) -> &AddressBook {
        &self.book
    }
}

// ---------------------------------------------------------------------------
// Dernière modification gagnante
// ---------------------------------------------------------------------------

/// Côté retenu d'un conflit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Local,
    Remote,
}

/// Côté modifié le plus récemment. `remote` inconnu (fiche sans `REV`)
/// vaut `now` : la modification vient d'être constatée. Une date future
/// (horloge déréglée) est ramenée à `now`. À égalité, le serveur gagne.
#[must_use]
pub fn newest_side(local: i64, remote: Option<i64>, now: i64) -> Side {
    let remote = remote.unwrap_or(now).min(now);
    if local.min(now) > remote {
        Side::Local
    } else {
        Side::Remote
    }
}

/// Jours depuis le 1er janvier 1970 d'une date du calendrier grégorien.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Date du calendrier grégorien d'un nombre de jours depuis 1970.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

/// Nombre fait de `count` chiffres ASCII exactement.
fn digits(raw: &str, count: usize) -> Option<i64> {
    if raw.len() == count && raw.bytes().all(|b| b.is_ascii_digit()) {
        raw.parse().ok()
    } else {
        None
    }
}

/// Décalage `+0200`, `-05:00` ou `+02`, en secondes.
fn parse_offset(raw: &str) -> Option<i64> {
    let (sign, rest) = match raw.split_at_checked(1)? {
        ("+", rest) => (1, rest),
        ("-", rest) => (-1, rest),
        _ => return None,
    };
    let rest = rest.replace(':', "");
    let (hours, minutes) = match rest.len() {
        2 => (digits(&rest, 2)?, 0),
        4 => (digits(&rest[..2], 2)?, digits(&rest[2..], 2)?),
        _ => return None,
    };
    (hours < 24 && minutes < 60).then_some(sign * (hours * 3600 + minutes * 60))
}

/// Date `REV` d'une vCard (`20231118T101010Z`, `2023-11-18T10:10:10Z`,
/// décalage `+0200`, date seule), en secondes depuis 1970 (UTC).
#[must_use]
pub fn parse_rev(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    let (date, time) = match raw.split_once(['T', 't']) {
        Some((date, time)) => (date, Some(time)),
        None => (raw, None),
    };
    let date = date.replace('-', "");
    if date.len() != 8 || !date.is_ascii() {
        return None;
    }
    let year = digits(&date[..4], 4)?;
    let month = digits(&date[4..6], 2)?;
    let day = digits(&date[6..], 2)?;
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    let mut at = days_from_civil(year, month, day) * 86_400;
    if let Some(time) = time {
        let (clock, offset) = if let Some(clock) = time.strip_suffix(['Z', 'z']) {
            (clock, 0)
        } else if let Some(i) = time.find(['+', '-']) {
            (&time[..i], parse_offset(&time[i..])?)
        } else {
            (time, 0)
        };
        let clock = clock.split('.').next().unwrap_or_default().replace(':', "");
        if !clock.is_ascii() {
            return None;
        }
        let (hours, minutes, seconds) = match clock.len() {
            4 => (digits(&clock[..2], 2)?, digits(&clock[2..], 2)?, 0),
            6 => (
                digits(&clock[..2], 2)?,
                digits(&clock[2..4], 2)?,
                digits(&clock[4..], 2)?,
            ),
            _ => return None,
        };
        if hours > 23 || minutes > 59 || seconds > 60 {
            return None;
        }
        at += hours * 3600 + minutes * 60 + seconds - offset;
    }
    Some(at)
}

/// Date `REV` écrite (UTC, forme de base).
#[must_use]
pub fn format_rev(at: i64) -> String {
    let (year, month, day) = civil_from_days(at.div_euclid(86_400));
    let seconds = at.rem_euclid(86_400);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

// ---------------------------------------------------------------------------
// vCard
// ---------------------------------------------------------------------------

/// Version d'une fiche distante (2.1 est réécrite en 3.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardVersion {
    V3,
    V4,
}

/// Propriétés que le carnet connaît : celles qu'il écrit.
const MANAGED_PROPERTIES: [&str; 9] = [
    "FN", "N", "NICKNAME", "ORG", "TITLE", "EMAIL", "TEL", "ADR", "NOTE",
];

/// Propriété distante que le carnet tient, donc remplacée par la sienne
/// à l'écriture : celles de [`MANAGED_PROPERTIES`], et un anniversaire ou
/// un site que le carnet sait lire ([`Birthday::parse`], [`website_url`]).
/// Un anniversaire en texte (« vers 1950 ») ou un lien qui n'est pas un
/// site, que le carnet n'a jamais repris, est gardé tel quel.
fn managed(name: &str, logical: &str) -> bool {
    let value = || property_value(logical).unwrap_or_default().trim();
    match name {
        // Une date lisible (l'année fictive d'Apple en est une).
        "BDAY" => Birthday::parse(value()).is_some(),
        "URL" => website_url(&value().replace("\\,", ",").replace("\\;", ";")).is_some(),
        _ => MANAGED_PROPERTIES.contains(&name),
    }
}
/// Propriétés réécrites à chaque écriture.
const FRAME_PROPERTIES: [&str; 4] = ["BEGIN", "END", "VERSION", "REV"];

/// Ligne logique d'une vCard : texte déplié et lignes physiques d'origine.
struct Line<'a> {
    logical: String,
    physical: Vec<&'a str>,
}

/// Lignes logiques (dépliées, RFC 6350 § 3.2), vides omises.
fn lines(raw: &str) -> Vec<Line<'_>> {
    let mut out: Vec<Line<'_>> = Vec::new();
    for physical in raw.split('\n') {
        let physical = physical.strip_suffix('\r').unwrap_or(physical);
        if let Some(rest) = physical.strip_prefix([' ', '\t'])
            && let Some(last) = out.last_mut()
        {
            last.logical.push_str(rest);
            last.physical.push(physical);
            continue;
        }
        if physical.trim().is_empty() {
            continue;
        }
        out.push(Line {
            logical: physical.to_owned(),
            physical: vec![physical],
        });
    }
    out
}

/// Groupe éventuel et nom (en majuscules) d'une propriété.
fn property_name(logical: &str) -> (Option<String>, String) {
    let end = logical.find([';', ':']).unwrap_or(logical.len());
    let full = &logical[..end];
    match full.rsplit_once('.') {
        Some((group, name)) => (Some(group.to_ascii_uppercase()), name.to_ascii_uppercase()),
        None => (None, full.to_ascii_uppercase()),
    }
}

/// Valeur d'une propriété : après le premier `:` hors guillemets.
fn property_value(logical: &str) -> Option<&str> {
    let mut quoted = false;
    for (i, c) in logical.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => return Some(&logical[i + 1..]),
            _ => {}
        }
    }
    None
}

/// Lignes de la première fiche, entre `BEGIN:VCARD` et `END:VCARD`.
fn card_body<'l, 'a>(lines: &'l [Line<'a>]) -> &'l [Line<'a>] {
    let is = |line: &Line<'_>, name: &str| {
        property_name(&line.logical).1 == name
            && property_value(&line.logical).is_some_and(|v| v.trim().eq_ignore_ascii_case("VCARD"))
    };
    let Some(begin) = lines.iter().position(|l| is(l, "BEGIN")) else {
        return &[];
    };
    let rest = &lines[begin + 1..];
    let end = rest.iter().position(|l| is(l, "END")).unwrap_or(rest.len());
    &rest[..end]
}

fn push_physical(out: &mut String, line: &Line<'_>) {
    for physical in &line.physical {
        out.push_str(physical);
        out.push_str("\r\n");
    }
}

fn version_line(version: CardVersion) -> &'static str {
    match version {
        CardVersion::V4 => "VERSION:4.0\r\n",
        CardVersion::V3 => "VERSION:3.0\r\n",
    }
}

/// Lignes de notre fiche à écrire : ses propriétés, sans cadre ni
/// identité (UID et PRODID viennent de la fiche distante ou de la
/// création).
fn push_ours(out: &mut String, ours: &str, remote: Option<&[Line<'_>]>) {
    for line in card_body(&lines(ours)) {
        let (_, name) = property_name(&line.logical);
        if FRAME_PROPERTIES.contains(&name.as_str()) || matches!(name.as_str(), "UID" | "PRODID") {
            continue;
        }
        // Nom et organisation : les composantes que le carnet ne connaît
        // pas (second prénom, civilité, suffixe ; services) sont reprises
        // de la fiche du serveur.
        let known = match name.as_str() {
            "N" => Some(2),
            "ORG" => Some(1),
            _ => None,
        };
        let remote_value = known.zip(remote).and_then(|(_, body)| {
            body.iter()
                .find(|l| property_name(&l.logical).1 == name)
                .and_then(|l| property_value(&l.logical))
        });
        match (known, remote_value, property_value(&line.logical)) {
            (Some(kept), Some(theirs), Some(value)) => {
                let head = &line.logical[..line.logical.len() - value.len()];
                let mut parts = components(value);
                let theirs = components(theirs);
                if theirs.len() > kept {
                    parts.truncate(kept);
                    parts.extend(theirs.into_iter().skip(kept));
                }
                let merged = format!("{head}{}", parts.join(";"));
                out.push_str(&fold_line(&merged));
            }
            _ => push_physical(out, line),
        }
    }
}

/// Composantes d'une valeur structurée, séparées par `;` hors
/// échappement (`\\;`), échappements gardés.
fn components(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (i, c) in value.char_indices() {
        match c {
            '\\' if !escaped => escaped = true,
            ';' if !escaped => {
                out.push(&value[start..i]);
                start = i + 1;
            }
            _ => escaped = false,
        }
    }
    out.push(&value[start..]);
    out
}

/// Ligne logique repliée à 75 octets (RFC 6350 § 3.2), CRLF final.
fn fold_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 8);
    let mut room = 75;
    for c in line.chars() {
        if c.len_utf8() > room {
            out.push_str("\r\n ");
            room = 74;
        }
        out.push(c);
        room -= c.len_utf8();
    }
    out.push_str("\r\n");
    out
}

/// Valeur de la première propriété `name` (sans groupe ni casse).
#[must_use]
pub fn card_property(raw: &str, name: &str) -> Option<String> {
    let wanted = name.to_ascii_uppercase();
    lines(raw)
        .iter()
        .find(|line| property_name(&line.logical).1 == wanted)
        .and_then(|line| property_value(&line.logical).map(str::to_owned))
}

#[must_use]
pub fn card_version(raw: &str) -> CardVersion {
    match card_property(raw, "VERSION").as_deref().map(str::trim) {
        Some("4.0") => CardVersion::V4,
        _ => CardVersion::V3,
    }
}

/// Date de modification d'une fiche distante (`REV`).
#[must_use]
pub fn card_rev(raw: &str) -> Option<i64> {
    card_property(raw, "REV").and_then(|rev| parse_rev(&rev))
}

/// La fiche distante `remote` dont les propriétés connues du carnet sont
/// remplacées par celles de `ours`, datée de `rev`. Un groupe
/// (`item1.EMAIL` et son `item1.X-ABLabel`) dont une propriété connue est
/// remplacée part en entier ; le reste est gardé tel quel.
#[must_use]
pub fn merge_card(remote: &str, ours: &str, rev: i64) -> String {
    let remote_lines = lines(remote);
    let body = card_body(&remote_lines);
    let replaced_groups: HashSet<String> = body
        .iter()
        .filter_map(|line| {
            let (group, name) = property_name(&line.logical);
            group.filter(|_| managed(&name, &line.logical))
        })
        .collect();
    let mut out = String::from("BEGIN:VCARD\r\n");
    out.push_str(version_line(card_version(remote)));
    for line in body {
        let (group, name) = property_name(&line.logical);
        let replaced = managed(&name, &line.logical)
            || FRAME_PROPERTIES.contains(&name.as_str())
            || group.is_some_and(|g| replaced_groups.contains(&g));
        if !replaced {
            push_physical(&mut out, line);
        }
    }
    out.push_str(&format!("REV:{}\r\n", format_rev(rev)));
    push_ours(&mut out, ours, Some(body));
    out.push_str("END:VCARD\r\n");
    out
}

/// Fiche à créer sur le serveur : `ours`, avec son UID et sa date.
#[must_use]
pub fn new_card(ours: &str, uid: &CardUid, rev: i64) -> String {
    let mut out = String::from("BEGIN:VCARD\r\n");
    out.push_str(version_line(card_version(ours)));
    out.push_str(&format!("UID:{}\r\nREV:{}\r\n", uid.0, format_rev(rev)));
    push_ours(&mut out, ours, None);
    out.push_str("END:VCARD\r\n");
    out
}

/// Fiche du carnet tirée d'une ressource distante : une seule fiche,
/// préparée et validée comme une fiche importée
/// ([`plan_import`] : NFC, nom imitant une adresse retiré, caractères
/// invisibles refusés) ; sinon `None`.
#[must_use]
pub fn remote_draft(
    decoded: &DecodedContacts,
    normalizer: &impl TextNormalizer,
) -> Option<ContactDraft> {
    let [card @ ImportedCard::Contact(_)] = decoded.cards.as_slice() else {
        return None;
    };
    plan_import(
        &[],
        std::slice::from_ref(card),
        DuplicatePolicy::Skip,
        normalizer,
    )
    .create
    .into_iter()
    .next()
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

/// État d'une ressource connue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// Reliée à un contact du carnet.
    Linked(ContactId),
    /// Son contact a été supprimé ici, à cette date.
    Deleted { at: i64 },
    /// Fiche distante que le carnet ne peut pas reprendre (sans adresse,
    /// adresse déjà prise…) : revue quand elle change.
    Ignored,
}

/// Ressource distante connue depuis la dernière synchronisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncLink {
    pub href: Href,
    pub etag: ETag,
    pub state: LinkState,
    /// Fiche locale telle qu'écrite à la dernière synchronisation
    /// (vCard 4.0 du carnet) : un écart dit qu'elle a changé ici depuis.
    pub synced: String,
    /// Fiche distante telle que lue ou écrite à la dernière
    /// synchronisation (vide à la relecture de toutes les ressources : lue
    /// à la demande, `CardDavStore::link_remote`).
    pub remote: String,
}

/// Ressource du carnet distant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    pub href: Href,
    pub etag: ETag,
}

/// Fiche distante téléchargée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCard {
    pub href: Href,
    pub etag: ETag,
    pub data: String,
}

/// Fiche distante téléchargée et lue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedCard {
    pub entry: RemoteEntry,
    pub raw: String,
    /// `None` : fiche que le carnet ne peut pas reprendre.
    pub draft: Option<ContactDraft>,
    /// Adresses lues dans la fiche, même si elle ne peut pas être reprise.
    pub keys: Vec<AddressKey>,
}

/// Contact du carnet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalCard {
    pub contact: Contact,
    /// Sa fiche vCard 4.0 actuelle (comparée à [`SyncLink::synced`]).
    pub snapshot: String,
    /// Date de sa dernière modification ici.
    pub modified_at: i64,
}

/// Écriture décidée par [`plan_sync`].
// Valeurs passagères d'un plan, presque toutes porteuses d'une fiche :
// mettre la fiche en boîte ajouterait une allocation sans rien économiser.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncAction {
    /// Nouvelle fiche distante : créée ici.
    PullCreate { card: FetchedCard },
    /// Fiche distante plus récente : `contact` prend `draft`.
    PullUpdate {
        card: FetchedCard,
        contact: Contact,
        draft: ContactDraft,
    },
    /// Même fiche des deux côtés : reliées sans rien écrire.
    Link { card: FetchedCard, contact: Contact },
    /// Fiche distante changée que le carnet ne peut pas reprendre : le
    /// contact reste tel quel, la version distante est retenue.
    Refresh { card: FetchedCard, contact: Contact },
    /// Fiche distante que le carnet ne peut pas reprendre.
    Ignore { card: FetchedCard },
    /// Fiche supprimée sur le serveur : supprimée ici.
    DeleteLocal { href: Href, contact: Contact },
    /// Nouveau contact : créé sur le serveur.
    PushCreate { contact: Contact },
    /// Contact plus récent : écrit sur le serveur, fusionné dans `base`
    /// (`None` : la fiche distante enregistrée à la dernière
    /// synchronisation), si la ressource est toujours à la version `etag`.
    PushUpdate {
        href: Href,
        etag: ETag,
        base: Option<String>,
        contact: Contact,
    },
    /// Contact supprimé ici : supprimé sur le serveur, s'il est toujours à
    /// la version `etag`.
    DeleteRemote { href: Href, etag: ETag },
    /// Ressource disparue, ou dont le contact est recréé : oubliée.
    Forget { href: Href },
    /// Rien n'est écrit, la ressource est revue la prochaine fois : fiche
    /// reliée changée que le serveur n'a pas renvoyée (trop grosse,
    /// disparue entre-temps), fiche remise à plus tard faute de place dans
    /// la synchronisation, version distante illisible plus récente que le
    /// contact, ou suppression retenue ([`guard_deletions`]).
    Defer { href: Href },
}

impl SyncAction {
    /// Nom court (tests, journaux).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::PullCreate { .. } => "pull-create",
            Self::PullUpdate { .. } => "pull-update",
            Self::Link { .. } => "link",
            Self::Refresh { .. } => "refresh",
            Self::Ignore { .. } => "ignore",
            Self::DeleteLocal { .. } => "delete-local",
            Self::PushCreate { .. } => "push-create",
            Self::PushUpdate { .. } => "push-update",
            Self::DeleteRemote { .. } => "delete-remote",
            Self::Forget { .. } => "forget",
            Self::Defer { .. } => "defer",
        }
    }
}

/// Ressources à télécharger : nouvelles, ou dont l'ETag a changé.
#[must_use]
pub fn hrefs_to_fetch(links: &[SyncLink], remote: &[RemoteEntry]) -> Vec<Href> {
    let known: HashMap<&Href, &ETag> = links.iter().map(|l| (&l.href, &l.etag)).collect();
    let mut seen = HashSet::new();
    remote
        .iter()
        .filter(|e| seen.insert(&e.href) && known.get(&e.href) != Some(&&e.etag))
        .map(|e| e.href.clone())
        .collect()
}

/// Plan de synchronisation entre les ressources connues `links`, le
/// carnet `local`, le carnet distant `remote` et ses fiches téléchargées
/// `fetched` ([`hrefs_to_fetch`]) ; les fiches `postponed`, à télécharger
/// mais remises à plus tard (budget de la synchronisation), attendent sans
/// rien écrire. À l'heure `now`.
///
/// - Modifiée d'un seul côté : l'autre suit (création, modification,
///   suppression).
/// - Modifiée des deux côtés : la modification la plus récente gagne
///   ([`newest_side`] : date du contact ici, `REV` sur le serveur). Une
///   suppression ici est datée ; une suppression sur le serveur ne l'est
///   pas : un contact modifié ici depuis est recréé sur le serveur.
/// - Nouvelle fiche distante qui porte l'adresse d'un contact non relié :
///   reliée à lui (rien n'est écrit si elles sont égales, sinon la plus
///   récente gagne) ; adresse déjà portée par un contact relié ou une
///   autre fiche reprise, ou fiche que le carnet ne peut pas reprendre :
///   ignorée.
/// - Le favori, absent des vCard, reste celui du carnet.
/// - Une ressource changée que le serveur n'a pas renvoyée (trop grosse,
///   disparue entre-temps) : reliée, elle est remise à plus tard ;
///   nouvelle, elle est laissée de côté jusqu'à son prochain changement.
/// - Une fiche déplacée sur le serveur (ancienne ressource disparue,
///   nouvelle aux mêmes adresses) reste reliée à son contact.
/// - Une fiche distante illisible garde ses adresses ([`FetchedCard::keys`]) :
///   un contact non relié qui les porte n'est pas envoyé en double.
#[must_use]
pub fn plan_sync(
    links: &[SyncLink],
    local: &[LocalCard],
    remote: &[RemoteEntry],
    fetched: &[FetchedCard],
    postponed: &[Href],
    now: i64,
) -> Vec<SyncAction> {
    let mut planner = Planner::new(links, local, fetched, now);
    planner.postponed = postponed.iter().collect();
    let present: HashSet<&Href> = remote.iter().map(|e| &e.href).collect();
    for link in links {
        if let LinkState::Linked(id) = link.state
            && !present.contains(&link.href)
            && let Some(card) = planner.local.get(&id).copied()
        {
            for email in card.contact.draft().emails() {
                planner.gone_by_key.insert(email.key(), link);
            }
        }
    }
    planner.run(remote)
}

/// Suppressions dans le carnet au plus, en nombre, avant d'en vérifier la
/// proportion.
pub const MAX_UNCHECKED_LOCAL_DELETIONS: usize = 10;

/// Garde-fou du plan : une écriture qui vise une ressource hors du carnet
/// `book` est retenue ; les suppressions dans le carnet aussi, toutes,
/// quand la liste du serveur n'est pas sûre (`listing_complete` faux :
/// réponses illisibles omises, ou liste vide) ou qu'elles emporteraient
/// plus de la
/// moitié (au-delà de [`MAX_UNCHECKED_LOCAL_DELETIONS`]) : une liste vide
/// ou tronquée par le serveur ne doit pas vider le carnet. Elles restent
/// alors en attente ([`SyncAction::Defer`]), sans rien écrire.
#[must_use]
pub fn guard_deletions(
    actions: Vec<SyncAction>,
    links: &[SyncLink],
    book: &DavUrl,
    listing_complete: bool,
) -> Vec<SyncAction> {
    let linked = links
        .iter()
        .filter(|l| matches!(l.state, LinkState::Linked(_)))
        .count();
    let deletions = actions
        .iter()
        .filter(|a| matches!(a, SyncAction::DeleteLocal { .. }))
        .count();
    let massive = deletions > 0
        && (!listing_complete
            || (deletions > MAX_UNCHECKED_LOCAL_DELETIONS && deletions * 2 > linked));
    actions
        .into_iter()
        .map(|action| {
            let outside = action_href(&action).is_some_and(|h| !book.contains(h));
            let held = massive && matches!(action, SyncAction::DeleteLocal { .. });
            if outside || held {
                SyncAction::Defer {
                    href: action_href(&action).cloned().unwrap_or_else(|| book.href()),
                }
            } else {
                action
            }
        })
        .collect()
}

/// Ressource visée par une écriture.
fn action_href(action: &SyncAction) -> Option<&Href> {
    match action {
        SyncAction::PullCreate { card }
        | SyncAction::PullUpdate { card, .. }
        | SyncAction::Link { card, .. }
        | SyncAction::Refresh { card, .. }
        | SyncAction::Ignore { card } => Some(&card.entry.href),
        SyncAction::DeleteLocal { href, .. }
        | SyncAction::PushUpdate { href, .. }
        | SyncAction::DeleteRemote { href, .. }
        | SyncAction::Forget { href }
        | SyncAction::Defer { href } => Some(href),
        SyncAction::PushCreate { .. } => None,
    }
}

/// La fiche distante `draft` avec le favori du contact local.
fn with_favorite(draft: &ContactDraft, favorite: bool) -> ContactDraft {
    if draft.favorite() == favorite {
        return draft.clone();
    }
    let mut input = draft.to_input();
    input.favorite = favorite;
    ContactDraft::new(input).unwrap_or_else(|_| draft.clone())
}

/// État du calcul de [`plan_sync`].
struct Planner<'a> {
    links: HashMap<&'a Href, &'a SyncLink>,
    local: HashMap<ContactId, &'a LocalCard>,
    local_order: &'a [LocalCard],
    fetched: HashMap<&'a Href, &'a FetchedCard>,
    now: i64,
    /// Contacts reliés à une ressource, ou rapprochés pendant le calcul :
    /// pas de création sur le serveur.
    taken: HashSet<ContactId>,
    /// Contacts non reliés, par adresse.
    unlinked: HashMap<&'a AddressKey, ContactId>,
    /// Adresses déjà portées par un contact relié ou une fiche reprise.
    claimed: HashSet<AddressKey>,
    /// Ressources remises à plus tard (budget).
    postponed: HashSet<&'a Href>,
    /// Ressources reliées disparues du serveur, par adresse de leur
    /// contact : une nouvelle ressource aux mêmes adresses les remplace.
    gone_by_key: HashMap<&'a AddressKey, &'a SyncLink>,
    /// Ressources disparues remplacées par une nouvelle.
    moved: HashSet<&'a Href>,
    actions: Vec<SyncAction>,
}

impl<'a> Planner<'a> {
    fn new(
        links: &'a [SyncLink],
        local: &'a [LocalCard],
        fetched: &'a [FetchedCard],
        now: i64,
    ) -> Self {
        let local_map: HashMap<ContactId, &LocalCard> =
            local.iter().map(|l| (l.contact.id(), l)).collect();
        let taken: HashSet<ContactId> = links
            .iter()
            .filter_map(|l| match l.state {
                LinkState::Linked(id) if local_map.contains_key(&id) => Some(id),
                _ => None,
            })
            .collect();
        let mut unlinked = HashMap::new();
        let mut claimed = HashSet::new();
        for card in local {
            for email in card.contact.draft().emails() {
                if taken.contains(&card.contact.id()) {
                    claimed.insert(email.key().clone());
                } else {
                    unlinked.entry(email.key()).or_insert(card.contact.id());
                }
            }
        }
        Self {
            links: links.iter().map(|l| (&l.href, l)).collect(),
            local: local_map,
            local_order: local,
            fetched: fetched.iter().map(|c| (&c.entry.href, c)).collect(),
            now,
            taken,
            unlinked,
            claimed,
            postponed: HashSet::new(),
            gone_by_key: HashMap::new(),
            moved: HashSet::new(),
            actions: Vec::new(),
        }
    }

    fn run(mut self, remote: &[RemoteEntry]) -> Vec<SyncAction> {
        let mut seen = HashSet::new();
        for entry in remote {
            if seen.insert(&entry.href) {
                self.entry(entry);
            }
        }
        let mut gone: Vec<&SyncLink> = self
            .links
            .values()
            .copied()
            .filter(|l| !seen.contains(&l.href))
            .collect();
        gone.sort_by(|a, b| a.href.cmp(&b.href));
        for link in gone {
            self.gone(link);
        }
        for card in self.local_order {
            if !self.taken.contains(&card.contact.id()) {
                self.actions.push(SyncAction::PushCreate {
                    contact: card.contact.clone(),
                });
            }
        }
        self.actions
    }

    /// Ressource présente sur le serveur.
    fn entry(&mut self, entry: &RemoteEntry) {
        let link = self.links.get(&entry.href).copied();
        let changed = link.is_none_or(|l| l.etag != entry.etag);
        if changed && self.postponed.contains(&entry.href) {
            self.actions.push(SyncAction::Defer {
                href: entry.href.clone(),
            });
            return;
        }
        let card = if changed {
            let Some(card) = self.fetched.get(&entry.href).copied() else {
                self.not_returned(link, entry);
                return;
            };
            Some(card)
        } else {
            None
        };
        match (link, card) {
            (Some(link), None) => self.unchanged(link),
            (Some(link), Some(card)) => match link.state {
                LinkState::Linked(id) => match self.local.get(&id).copied() {
                    Some(local) => self.both(link, local, card),
                    None => self.deleted_changed(card, self.now),
                },
                LinkState::Deleted { at } => self.deleted_changed(card, at),
                LinkState::Ignored => self.new_remote(card),
            },
            (None, Some(card)) => self.new_remote(card),
            (None, None) => {}
        }
    }

    /// Ressource changée que le serveur n'a pas renvoyée (trop grosse,
    /// disparue entre-temps) : reliée, elle est remise à plus tard ;
    /// nouvelle, elle est laissée de côté jusqu'à son prochain changement.
    fn not_returned(&mut self, link: Option<&SyncLink>, entry: &RemoteEntry) {
        let action = match link.map(|l| l.state) {
            Some(LinkState::Linked(_) | LinkState::Deleted { .. }) => SyncAction::Defer {
                href: entry.href.clone(),
            },
            Some(LinkState::Ignored) | None => SyncAction::Ignore {
                card: FetchedCard {
                    entry: entry.clone(),
                    raw: String::new(),
                    draft: None,
                    keys: Vec::new(),
                },
            },
        };
        self.actions.push(action);
    }

    /// Ressource inchangée sur le serveur.
    fn unchanged(&mut self, link: &SyncLink) {
        let delete = SyncAction::DeleteRemote {
            href: link.href.clone(),
            etag: link.etag.clone(),
        };
        match link.state {
            LinkState::Linked(id) => match self.local.get(&id).copied() {
                Some(local) if local.snapshot != link.synced => {
                    self.actions.push(SyncAction::PushUpdate {
                        href: link.href.clone(),
                        etag: link.etag.clone(),
                        base: None,
                        contact: local.contact.clone(),
                    });
                }
                Some(_) => {}
                None => self.actions.push(delete),
            },
            LinkState::Deleted { .. } => self.actions.push(delete),
            LinkState::Ignored => {}
        }
    }

    /// Fiche changée sur le serveur, reliée à un contact du carnet.
    fn both(&mut self, link: &SyncLink, local: &LocalCard, card: &FetchedCard) {
        let local_changed = local.snapshot != link.synced;
        let remote_newer =
            newest_side(local.modified_at, card_rev(&card.raw), self.now) == Side::Remote;
        let action = match &card.draft {
            // Version distante illisible pour le carnet, plus récente : le
            // contact attend, rien n'est écrasé sur le serveur.
            None if local_changed && remote_newer => SyncAction::Defer {
                href: card.entry.href.clone(),
            },
            None if local_changed => push_over(card, local),
            None => SyncAction::Refresh {
                card: card.clone(),
                contact: local.contact.clone(),
            },
            Some(draft) => {
                let remote_wins = !local_changed || remote_newer;
                if remote_wins {
                    pull(card, local, draft)
                } else {
                    push_over(card, local)
                }
            }
        };
        self.actions.push(action);
    }

    /// Fiche changée sur le serveur, dont le contact a été supprimé ici à
    /// la date `at`.
    fn deleted_changed(&mut self, card: &FetchedCard, at: i64) {
        if newest_side(at, card_rev(&card.raw), self.now) == Side::Local {
            self.actions.push(SyncAction::DeleteRemote {
                href: card.entry.href.clone(),
                etag: card.entry.etag.clone(),
            });
        } else {
            self.new_remote(card);
        }
    }

    /// Fiche distante qu'aucun contact ne reflète encore.
    fn new_remote(&mut self, card: &FetchedCard) {
        let Some(draft) = &card.draft else {
            // Ses adresses restent les siennes : un contact non relié qui
            // les porte n'est pas envoyé en double sur le serveur.
            for key in &card.keys {
                if let Some(id) = self.unlinked.get(key).copied() {
                    self.taken.insert(id);
                }
                self.claimed.insert(key.clone());
            }
            self.actions.push(SyncAction::Ignore { card: card.clone() });
            return;
        };
        let keys: Vec<&AddressKey> = draft.emails().iter().map(|e| e.key()).collect();
        // Fiche déplacée sur le serveur : elle reprend le lien de
        // l'ancienne ressource, disparue, de son contact.
        let moved = keys
            .iter()
            .find_map(|k| self.gone_by_key.get(*k).copied())
            .filter(|old| !self.moved.contains(&old.href));
        if let Some(old) = moved
            && let LinkState::Linked(id) = old.state
            && let Some(local) = self.local.get(&id).copied()
        {
            self.moved.insert(&old.href);
            self.claimed.extend(keys.into_iter().cloned());
            self.both(old, local, card);
            return;
        }
        if keys.iter().any(|k| self.claimed.contains(*k)) {
            self.actions.push(SyncAction::Ignore { card: card.clone() });
            return;
        }
        let matched = keys
            .iter()
            .find_map(|k| self.unlinked.get(*k).copied())
            .and_then(|id| self.local.get(&id).copied());
        self.claimed.extend(keys.into_iter().cloned());
        let Some(local) = matched else {
            self.actions
                .push(SyncAction::PullCreate { card: card.clone() });
            return;
        };
        self.taken.insert(local.contact.id());
        for email in local.contact.draft().emails() {
            self.claimed.insert(email.key().clone());
        }
        let same = &with_favorite(draft, local.contact.draft().favorite()) == local.contact.draft();
        let action = if same
            || newest_side(local.modified_at, card_rev(&card.raw), self.now) == Side::Remote
        {
            pull(card, local, draft)
        } else {
            push_over(card, local)
        };
        self.actions.push(action);
    }

    /// Ressource connue disparue du serveur.
    fn gone(&mut self, link: &SyncLink) {
        let href = link.href.clone();
        if self.moved.contains(&link.href) {
            self.actions.push(SyncAction::Forget { href });
            return;
        }
        match link.state {
            LinkState::Linked(id) => match self.local.get(&id).copied() {
                Some(local) if local.snapshot == link.synced => {
                    self.actions.push(SyncAction::DeleteLocal {
                        href,
                        contact: local.contact.clone(),
                    });
                }
                Some(_) => {
                    // Modifié ici depuis : recréé sur le serveur.
                    self.taken.remove(&id);
                    self.actions.push(SyncAction::Forget { href });
                }
                None => self.actions.push(SyncAction::Forget { href }),
            },
            LinkState::Deleted { .. } | LinkState::Ignored => {
                self.actions.push(SyncAction::Forget { href });
            }
        }
    }
}

/// Reprise ici de la fiche distante `draft` pour `local` (rien à écrire
/// si elles sont égales, favori mis à part).
fn pull(card: &FetchedCard, local: &LocalCard, draft: &ContactDraft) -> SyncAction {
    let pulled = with_favorite(draft, local.contact.draft().favorite());
    if &pulled == local.contact.draft() {
        SyncAction::Link {
            card: card.clone(),
            contact: local.contact.clone(),
        }
    } else {
        SyncAction::PullUpdate {
            card: card.clone(),
            contact: local.contact.clone(),
            draft: pulled,
        }
    }
}

/// Écriture du contact `local` sur la version distante `card`.
fn push_over(card: &FetchedCard, local: &LocalCard) -> SyncAction {
    SyncAction::PushUpdate {
        href: card.entry.href.clone(),
        etag: card.entry.etag.clone(),
        base: Some(card.raw.clone()),
        contact: local.contact.clone(),
    }
}

// ---------------------------------------------------------------------------
// Bilan
// ---------------------------------------------------------------------------

/// Bilan d'une synchronisation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Fiches créées, modifiées ou supprimées ici.
    pub pulled: usize,
    /// Fiches créées, modifiées ou supprimées sur le serveur.
    pub pushed: usize,
    /// Fiches distantes que le carnet ne peut pas reprendre, ou écritures
    /// remises à la prochaine synchronisation.
    pub skipped: usize,
}

impl SyncReport {
    /// Compte une écriture réussie.
    pub fn count(&mut self, action: &SyncAction) {
        match action {
            SyncAction::PullCreate { .. }
            | SyncAction::PullUpdate { .. }
            | SyncAction::DeleteLocal { .. } => self.pulled += 1,
            SyncAction::PushCreate { .. }
            | SyncAction::PushUpdate { .. }
            | SyncAction::DeleteRemote { .. } => self.pushed += 1,
            SyncAction::Ignore { .. } | SyncAction::Defer { .. } => self.skipped += 1,
            SyncAction::Link { .. } | SyncAction::Refresh { .. } | SyncAction::Forget { .. } => {}
        }
    }

    /// Écritures faites, des deux côtés.
    #[must_use]
    pub fn changes(&self) -> usize {
        self.pulled + self.pushed
    }
}

/// Échec d'une synchronisation, en code stable (enregistré).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncFailure {
    /// Identifiant ou mot de passe refusé.
    Unauthorized,
    Network,
    Certificate,
    /// Carnet introuvable.
    NotFound,
    TooLarge,
    Server,
    Protocol,
    Storage,
    /// Mot de passe introuvable dans le trousseau.
    Keyring,
}

impl SyncFailure {
    const ALL: [Self; 9] = [
        Self::Unauthorized,
        Self::Network,
        Self::Certificate,
        Self::NotFound,
        Self::TooLarge,
        Self::Server,
        Self::Protocol,
        Self::Storage,
        Self::Keyring,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Network => "network",
            Self::Certificate => "certificate",
            Self::NotFound => "not-found",
            Self::TooLarge => "too-large",
            Self::Server => "server",
            Self::Protocol => "protocol",
            Self::Storage => "storage",
            Self::Keyring => "keyring",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.as_str() == raw)
    }
}
