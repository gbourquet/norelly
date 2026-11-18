//! Détails d'une fiche contact calqués sur vCard (ADR-012) : anniversaire
//! (`BDAY`, année facultative) et site web (`URL`). Les adresses postales
//! (`ADR`) et le titre (`TITLE`) sont validés avec la fiche
//! ([`crate::contacts::ContactDraft::new`]).

use std::fmt;

use crate::reading::{ExternalLink, LinkScheme, MAX_LINK_LEN};

/// Nombre maximal d'adresses postales d'un contact.
pub const MAX_POSTAL_ADDRESSES: usize = 10;
/// Longueur maximale d'un élément d'adresse postale (ville, région, code
/// postal, pays), en caractères.
pub const MAX_POSTAL_FIELD_CHARS: usize = 200;
/// Longueur maximale de la rue (plusieurs lignes), en caractères.
pub const MAX_POSTAL_STREET_CHARS: usize = 500;
/// Nombre maximal de sites web d'un contact.
pub const MAX_CONTACT_WEBSITES: usize = 10;
/// Longueur maximale d'un site web : celle d'un lien ouvert.
pub const MAX_WEBSITE_CHARS: usize = MAX_LINK_LEN;

/// Anniversaire : jour et mois, année facultative (de 1 à 9999). Le
/// 29 février est admis sans année, ou une année bissextile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Birthday {
    year: Option<u16>,
    month: u8,
    day: u8,
}

impl Birthday {
    /// Date valide, sinon `None`.
    #[must_use]
    pub fn new(year: Option<u16>, month: u8, day: u8) -> Option<Self> {
        let year_ok = year.is_none_or(|y| (1..=9999).contains(&y));
        let leap = year.is_none_or(|y| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0);
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return None,
        };
        (year_ok && (1..=days).contains(&day)).then_some(Self { year, month, day })
    }

    /// Date écrite `AAAA-MM-JJ`, `AAAAMMJJ`, `--MM-JJ` ou `--MMJJ` (formes
    /// de vCard et d'ISO 8601), suivie au besoin d'une heure (`T…`, ignorée) ;
    /// `None` pour toute autre forme ou une date impossible.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        let date = raw.split_once(['T', 't']).map_or(raw, |(date, _)| date);
        if !date.is_ascii() {
            return None;
        }
        let at = |i: usize| date.as_bytes().get(i).copied();
        let no_year = date.starts_with("--");
        let (year, month, day) = match date.len() {
            10 if at(4) == Some(b'-') && at(7) == Some(b'-') => {
                (Some(date.get(0..4)?), date.get(5..7)?, date.get(8..10)?)
            }
            8 if !no_year => (Some(date.get(0..4)?), date.get(4..6)?, date.get(6..8)?),
            7 if no_year && at(4) == Some(b'-') => (None, date.get(2..4)?, date.get(5..7)?),
            6 if no_year => (None, date.get(2..4)?, date.get(4..6)?),
            _ => return None,
        };
        let number = |digits: &str| -> Option<u16> {
            digits
                .bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| digits.parse().ok())
                .flatten()
        };
        let year = year.map(number).map_or(Some(None), |y| y.map(Some))?;
        let month = u8::try_from(number(month)?).ok()?;
        let day = u8::try_from(number(day)?).ok()?;
        Self::new(year, month, day)
    }

    #[must_use]
    pub fn year(&self) -> Option<u16> {
        self.year
    }

    #[must_use]
    pub fn month(&self) -> u8 {
        self.month
    }

    #[must_use]
    pub fn day(&self) -> u8 {
        self.day
    }

    /// Forme étendue : `1985-04-12`, ou `--04-12` sans année (forme
    /// enregistrée, échangée avec l'interface, écrite en vCard 3.0 et CSV).
    #[must_use]
    pub fn iso(&self) -> String {
        match self.year {
            Some(year) => format!("{year:04}-{:02}-{:02}", self.month, self.day),
            None => format!("--{:02}-{:02}", self.month, self.day),
        }
    }

    /// Forme de base : `19850412`, ou `--0412` sans année (vCard 4.0).
    #[must_use]
    pub fn basic(&self) -> String {
        match self.year {
            Some(year) => format!("{year:04}{:02}{:02}", self.month, self.day),
            None => format!("--{:02}{:02}", self.month, self.day),
        }
    }
}

impl fmt::Display for Birthday {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.iso())
    }
}

/// Adresse d'un site web saisi ou importé, telle qu'elle sera ouverte :
/// `http` ou `https` seulement, selon les règles d'un lien externe
/// ([`ExternalLink`] : sans espace, caractère de contrôle ni identifiants,
/// au plus [`MAX_WEBSITE_CHARS`] octets). Sans schéma
/// (`www.example.com`), `https://` est ajouté ; un hôte non ASCII (sosies
/// d'une autre écriture) est refusé, sa forme punycode admise ; tout autre schéma
/// (`javascript:`, `file:`, `mailto:`) est refusé. `None` si refusé.
#[must_use]
pub fn website_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let link = if has_scheme(raw) {
        ExternalLink::parse(raw)
    } else {
        ExternalLink::parse(&format!("https://{raw}"))
    }
    .ok()?;
    // Hôte en ASCII seulement : une lettre d'une autre écriture peut
    // imiter un domaine connu (« раураl » cyrillique) sur la fiche. Un nom
    // de domaine international s'écrit sous sa forme punycode (`xn--…`).
    let after = link.as_str().split_once("//").map_or("", |(_, rest)| rest);
    let host = after.split(['/', '?', '#']).next().unwrap_or_default();
    (host.is_ascii() && matches!(link.scheme(), LinkScheme::Http | LinkScheme::Https))
        .then(|| link.as_str().to_owned())
}

/// Le texte commence-t-il par un schéma (`lettre (lettre|chiffre|+|-|.)* :`)
/// qui n'est pas un nom d'hôte suivi d'un port (`example.com:8080`) ?
fn has_scheme(raw: &str) -> bool {
    let Some((scheme, rest)) = raw.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && !rest.starts_with(|c: char| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: Option<u16>, month: u8, day: u8) -> Birthday {
        Birthday::new(year, month, day).unwrap_or_else(|| panic!("{year:?}-{month}-{day}"))
    }

    #[test]
    fn birthday_is_a_real_date_with_an_optional_year() {
        assert_eq!(
            Birthday::new(Some(1985), 4, 12).map(|b| b.iso()),
            Some("1985-04-12".into())
        );
        assert_eq!(
            Birthday::new(None, 4, 12).map(|b| b.iso()),
            Some("--04-12".into())
        );
        for (year, month, day) in [
            (Some(1985), 0, 1),
            (Some(1985), 13, 1),
            (Some(1985), 1, 0),
            (Some(1985), 1, 32),
            (Some(1985), 4, 31),
            (Some(1985), 2, 29),
            (Some(1900), 2, 29),
            (Some(0), 1, 1),
            (Some(10_000), 1, 1),
            (None, 2, 30),
        ] {
            assert_eq!(
                Birthday::new(year, month, day),
                None,
                "{year:?}-{month}-{day}"
            );
        }
        // 29 février : sans année, ou une année bissextile.
        assert!(Birthday::new(None, 2, 29).is_some());
        assert!(Birthday::new(Some(2000), 2, 29).is_some());
        assert!(Birthday::new(Some(2024), 2, 29).is_some());
        let b = date(Some(7), 1, 2);
        assert_eq!((b.year(), b.month(), b.day()), (Some(7), 1, 2));
        assert_eq!(b.iso(), "0007-01-02");
        assert_eq!(b.basic(), "00070102");
        assert_eq!(date(None, 12, 31).basic(), "--1231");
        assert_eq!(date(Some(1985), 4, 12).to_string(), "1985-04-12");
    }

    #[test]
    fn birthday_reads_vcard_and_iso_forms() {
        for (raw, expected) in [
            ("1985-04-12", Some(date(Some(1985), 4, 12))),
            (" 19850412 ", Some(date(Some(1985), 4, 12))),
            ("--04-12", Some(date(None, 4, 12))),
            ("--0412", Some(date(None, 4, 12))),
            ("1985-04-12T00:00:00Z", Some(date(Some(1985), 4, 12))),
            ("19850412T101500", Some(date(Some(1985), 4, 12))),
            ("--0229", Some(date(None, 2, 29))),
        ] {
            assert_eq!(Birthday::parse(raw), expected, "{raw:?}");
        }
        for raw in [
            "",
            "1985",
            "1985-04",
            "--04",
            "85-04-12",
            "1985/04/12",
            "12/04/1985",
            "1985-4-12",
            "198504121",
            "1985-02-30",
            "--1301",
            "١٩٨٥٠٤١٢",
            "1985-04-12x",
            "+1985-04-12",
        ] {
            assert_eq!(Birthday::parse(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn websites_open_over_http_or_https_only() {
        for (raw, expected) in [
            ("https://example.com", "https://example.com"),
            ("  http://example.com/a?b#c  ", "http://example.com/a?b#c"),
            ("HTTPS://Example.com/Page", "https://Example.com/Page"),
            ("www.example.com", "https://www.example.com"),
            ("example.com:8080/x", "https://example.com:8080/x"),
            ("example.com/a:b", "https://example.com/a:b"),
            ("https://xn--caf-dma.fr/é", "https://xn--caf-dma.fr/é"),
        ] {
            assert_eq!(website_url(raw).as_deref(), Some(expected), "{raw:?}");
        }
        for raw in [
            "",
            "   ",
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "file:///etc/passwd",
            "mailto:a@example.com",
            "data:text/html,x",
            "ftp://example.com",
            "https://user:pass@example.com",
            "https://exa mple.com",
            "https://example.com/\u{202E}gpj.exe",
            "https://",
            "https://example.com\\x",
            "www.exa\u{0}mple.com",
            // Hôte non ASCII (sosies cyrilliques de « paypal ») : refusé,
            // la forme punycode (`xn--…`) est admise.
            "https://раураl.com/login",
            "www.café.fr",
        ] {
            assert_eq!(website_url(raw), None, "{raw:?}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_WEBSITE_CHARS));
        assert_eq!(website_url(&long), None);
        let fits = format!("https://example.com/{}", "a".repeat(MAX_WEBSITE_CHARS - 20));
        assert_eq!(website_url(&fits).map(|u| u.len()), Some(MAX_WEBSITE_CHARS));
    }
}
