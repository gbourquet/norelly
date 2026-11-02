//! Règles de lecture : liens ouverts dans le navigateur, images distantes
//! autorisées par expéditeur, noms de fichiers des pièces jointes.
//!
//! Tout ce qui vient d'un message (liens, noms de fichiers, expéditeur) est
//! hostile : ces règles décident en Rust de ce qui peut sortir du rendu
//! isolé (ADR-004).

use std::fmt;

use crate::address::EmailAddress;
use crate::connection::is_format_char;
use crate::mailbox::Correspondent;

// ---------------------------------------------------------------------------
// Liens externes
// ---------------------------------------------------------------------------

/// Longueur maximale d'un lien ouvert, en octets.
pub const MAX_LINK_LEN: usize = 2048;

/// Refus d'un lien.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("lien vide ou trop long")]
    Length,
    #[error("lien avec un caractère interdit")]
    InvalidCharacter,
    #[error("schéma de lien non autorisé (seuls http, https et mailto le sont)")]
    Scheme,
    #[error("lien sans hôte")]
    MissingHost,
    #[error("lien avec des identifiants (user@hôte), refusé")]
    Credentials,
}

/// Schéma d'un lien autorisé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkScheme {
    Http,
    Https,
    Mailto,
}

/// Lien que l'on accepte d'ouvrir dans le navigateur ou le client de
/// messagerie du système : `http`, `https` ou `mailto`, sans caractère de
/// contrôle ni espace, sans identifiants dans l'autorité.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalLink {
    scheme: LinkScheme,
    url: String,
}

impl ExternalLink {
    pub fn parse(raw: &str) -> Result<Self, LinkError> {
        let raw = raw.trim();
        if raw.is_empty() || raw.len() > MAX_LINK_LEN {
            return Err(LinkError::Length);
        }
        if raw
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || is_format_char(c) || c == '\\')
        {
            return Err(LinkError::InvalidCharacter);
        }
        let (scheme, rest) = raw.split_once(':').ok_or(LinkError::Scheme)?;
        let scheme = match scheme.to_ascii_lowercase().as_str() {
            "http" => LinkScheme::Http,
            "https" => LinkScheme::Https,
            "mailto" => LinkScheme::Mailto,
            _ => return Err(LinkError::Scheme),
        };
        match scheme {
            LinkScheme::Http | LinkScheme::Https => {
                let after = rest.strip_prefix("//").ok_or(LinkError::MissingHost)?;
                let authority = after.split(['/', '?', '#']).next().unwrap_or_default();
                if authority.contains('@') {
                    return Err(LinkError::Credentials);
                }
                let host = authority
                    .rsplit_once(':')
                    .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
                    .map_or(authority, |(h, _)| h);
                let valid_host = !host.is_empty()
                    && host
                        .chars()
                        .all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':'));
                if !valid_host {
                    return Err(LinkError::MissingHost);
                }
            }
            LinkScheme::Mailto => {
                if rest.is_empty() {
                    return Err(LinkError::MissingHost);
                }
            }
        }
        let prefix = match scheme {
            LinkScheme::Http => "http",
            LinkScheme::Https => "https",
            LinkScheme::Mailto => "mailto",
        };
        Ok(Self {
            scheme,
            url: format!("{prefix}:{rest}"),
        })
    }

    #[must_use]
    pub fn scheme(&self) -> LinkScheme {
        self.scheme
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.url
    }
}

impl fmt::Display for ExternalLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.url)
    }
}

// ---------------------------------------------------------------------------
// Images distantes
// ---------------------------------------------------------------------------

/// Décision d'affichage du contenu distant d'un message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteContent {
    /// Images distantes bloquées (bannière proposée).
    Blocked,
    /// Images distantes servies par le proxy.
    Allowed,
}

/// Images distantes d'un message : autorisées seulement si l'expéditeur
/// (premier `From`, adresse valide) figure parmi les expéditeurs
/// autorisés par l'utilisateur. Un `From` est falsifiable : l'autorisation
/// ne protège que la vie privée (pistage), jamais l'intégrité, puisque les
/// images passent de toute façon par le proxy sans cookies.
#[must_use]
pub fn remote_content(sender_allowed: bool) -> RemoteContent {
    if sender_allowed {
        RemoteContent::Allowed
    } else {
        RemoteContent::Blocked
    }
}

/// Adresse de l'expéditeur, normalisée pour l'autorisation ; `None` si le
/// message n'a pas d'expéditeur ou si son adresse est invalide (jamais
/// autorisé).
#[must_use]
pub fn sender_address(from: Option<&Correspondent>) -> Option<EmailAddress> {
    from.and_then(|c| EmailAddress::parse(&c.address).ok())
}

/// Vrai pour une adresse IP joignable sur Internet. Le proxy des images
/// distantes refuse toutes les autres (boucle locale, réseaux privés,
/// lien local, multidiffusion, plages réservées) : un mail ne peut pas
/// faire sonder le réseau local de l'utilisateur.
#[must_use]
pub fn is_public_address(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_unspecified()
                || v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (b == 18 || b == 19))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_address(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_unspecified()
                || v6.is_loopback()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8)
                || (s[0] == 0x0064 && s[1] == 0xff9b)
                || (s[0] == 0x2002)
                || s[..6] == [0, 0, 0, 0, 0, 0])
        }
    }
}

// ---------------------------------------------------------------------------
// Pièces jointes
// ---------------------------------------------------------------------------

/// Longueur maximale d'un nom de fichier écrit sur disque, en octets.
pub const MAX_FILE_NAME_BYTES: usize = 200;
/// Nom utilisé quand le nom annoncé est vide ou inutilisable.
pub const DEFAULT_FILE_NAME: &str = "piece-jointe";

/// Noms réservés de Windows (avec ou sans extension).
const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com0", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "lpt0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    "conin$", "conout$",
];

/// Nom de fichier sûr pour l'écriture dans un dossier choisi par
/// l'utilisateur, sur Windows comme sous Linux :
/// - séparateurs (`/`, `\`), caractères interdits par Windows (`:*?"<>|`),
///   caractères de contrôle et de mise en forme (dont les inversions de
///   sens d'écriture, `exe.pdf` déguisé) remplacés par `_` ;
/// - points et espaces de début et de fin retirés (`..`, fichiers cachés,
///   noms que Windows tronque) ;
/// - noms réservés de Windows (`CON`, `nul.txt`, `COM1`…) préfixés ;
/// - longueur bornée à [`MAX_FILE_NAME_BYTES`] en gardant l'extension.
///
/// Le résultat ne contient jamais de séparateur : joint à un dossier, il
/// désigne toujours un fichier de ce dossier.
#[must_use]
pub fn safe_file_name(raw: &str) -> String {
    let replaced: String = raw
        .chars()
        .map(|c| {
            if c.is_control()
                || is_format_char(c)
                || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = replaced.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let mut name = if trimmed.is_empty() {
        DEFAULT_FILE_NAME.to_owned()
    } else {
        trimmed.to_owned()
    };
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    if WINDOWS_RESERVED.contains(&stem.to_lowercase().as_str()) {
        name.insert(0, '_');
    }
    truncate_keeping_extension(&name, MAX_FILE_NAME_BYTES)
}

fn truncate_keeping_extension(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_owned();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        // Extension plausible : courte, sans espace.
        Some((s, e)) if !s.is_empty() && e.len() <= 16 && !e.contains(' ') => (s, Some(e)),
        _ => (name, None),
    };
    let budget = max.saturating_sub(ext.map_or(0, |e| e.len() + 1));
    let mut cut = budget.min(stem.len());
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    let stem = stem
        .get(..cut)
        .unwrap_or_default()
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    let stem = if stem.is_empty() {
        DEFAULT_FILE_NAME
    } else {
        stem
    };
    match ext {
        Some(e) => format!("{stem}.{e}"),
        None => stem.to_owned(),
    }
}

/// Nom de fichier distinct pour éviter d'écraser un fichier existant :
/// `rapport.pdf` → `rapport (2).pdf`.
#[must_use]
pub fn numbered_file_name(name: &str, n: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({n}).{ext}"),
        _ => format!("{name} ({n})"),
    }
}

/// Extensions que le système exécuterait (ou interpréterait) à
/// l'ouverture : une telle pièce jointe ne s'ouvre pas directement, elle
/// peut seulement être enregistrée.
const EXECUTABLE_EXTENSIONS: &[&str] = &[
    "exe",
    "com",
    "bat",
    "cmd",
    "scr",
    "pif",
    "msi",
    "msp",
    "mst",
    "cpl",
    "dll",
    "sys",
    "hta",
    "js",
    "jse",
    "vbs",
    "vbe",
    "wsf",
    "wsh",
    "ws",
    "ps1",
    "psm1",
    "psd1",
    "ps1xml",
    "psc1",
    "lnk",
    "url",
    "inf",
    "reg",
    "scf",
    "jar",
    "appref-ms",
    "application",
    "gadget",
    "msc",
    "chm",
    "iso",
    "img",
    "vhd",
    "vhdx",
    "library-ms",
    "settingcontent-ms",
    "sh",
    "bash",
    "zsh",
    "csh",
    "ksh",
    "run",
    "bin",
    "desktop",
    "appimage",
    "deb",
    "rpm",
    "flatpakref",
    "py",
    "pyw",
    "pl",
    "rb",
    "php",
    "elf",
    "out",
    "command",
    "app",
    "pkg",
    "dmg",
    "xll",
    "xlam",
    "docm",
    "dotm",
    "xlsm",
    "xltm",
    "pptm",
    "potm",
    "ppam",
    "ppsm",
    "sldm",
];

/// Vrai si la pièce jointe ne doit pas être ouverte directement (type
/// exécutable ou macro), d'après l'extension de son nom assaini.
#[must_use]
pub fn is_unsafe_to_open(file_name: &str) -> bool {
    let safe = safe_file_name(file_name);
    match safe.rsplit_once('.') {
        Some((_, ext)) => EXECUTABLE_EXTENSIONS.contains(&ext.to_lowercase().as_str()),
        // Sans extension : le système pourrait le traiter comme exécutable.
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_https_and_mailto_links_are_accepted() {
        for ok in [
            "https://example.com",
            "HTTPS://example.com/a?b=c#d",
            "http://example.com:8080/x",
            "http://[2001:db8::1]/",
            "https://exemple.fr/é",
            "https://xn--exmple-cua.fr",
            "mailto:alice@example.com?subject=salut",
        ] {
            assert!(ExternalLink::parse(ok).is_ok(), "{ok}");
        }
        assert_eq!(
            ExternalLink::parse("HTTPS://example.com").map(|l| l.to_string()),
            Ok("https://example.com".into())
        );
        for (bad, err) in [
            ("javascript:alert(1)", LinkError::Scheme),
            ("JaVaScRiPt:alert(1)", LinkError::Scheme),
            ("data:text/html,<script>", LinkError::Scheme),
            ("file:///etc/passwd", LinkError::Scheme),
            ("vbscript:x", LinkError::Scheme),
            ("smb://serveur/partage", LinkError::Scheme),
            ("ms-msdt:/id", LinkError::Scheme),
            ("example.com", LinkError::Scheme),
            ("", LinkError::Length),
            ("java\tscript:alert(1)", LinkError::InvalidCharacter),
            ("https://exa mple.com", LinkError::InvalidCharacter),
            (
                "https://example.com/\u{202e}fdp.exe",
                LinkError::InvalidCharacter,
            ),
            ("https:\\\\example.com", LinkError::InvalidCharacter),
            ("https:example.com", LinkError::MissingHost),
            ("https://", LinkError::MissingHost),
            ("https://user:pw@example.com", LinkError::Credentials),
            ("https://banque.fr@attaquant.net/", LinkError::Credentials),
            ("mailto:", LinkError::MissingHost),
        ] {
            assert_eq!(ExternalLink::parse(bad), Err(err), "{bad:?}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_LINK_LEN));
        assert_eq!(ExternalLink::parse(&long), Err(LinkError::Length));
    }

    #[test]
    fn remote_content_requires_an_allowed_valid_sender() {
        assert_eq!(remote_content(false), RemoteContent::Blocked);
        assert_eq!(remote_content(true), RemoteContent::Allowed);
        assert!(sender_address(None).is_none());
        let bad = Correspondent::new("x", "pas une adresse");
        assert!(sender_address(Some(&bad)).is_none());
        let good = Correspondent::new("Alice", "Alice@Example.com");
        assert!(sender_address(Some(&good)).is_some());
    }

    #[test]
    fn only_public_addresses_are_reachable() {
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "192.0.0.8",
            "198.18.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "240.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "fc00::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "2001:db8::1",
            "64:ff9b::a00:1",
            "2002:a00:1::",
            "::127.0.0.1",
        ] {
            let ip: std::net::IpAddr = private.parse().unwrap_or_else(|_| panic!("{private}"));
            assert!(!is_public_address(ip), "{private}");
        }
        for public in [
            "93.184.216.34",
            "8.8.8.8",
            "2606:4700::1111",
            "::ffff:8.8.8.8",
        ] {
            let ip: std::net::IpAddr = public.parse().unwrap_or_else(|_| panic!("{public}"));
            assert!(is_public_address(ip), "{public}");
        }
    }

    #[test]
    fn file_names_are_safe_on_every_platform() {
        for (raw, safe) in [
            ("rapport.pdf", "rapport.pdf"),
            ("../../etc/passwd", "_.._etc_passwd"),
            ("..\\..\\Windows\\win.ini", "_.._Windows_win.ini"),
            ("/etc/shadow", "_etc_shadow"),
            ("C:\\boot.ini", "C__boot.ini"),
            ("..", DEFAULT_FILE_NAME),
            ("...", DEFAULT_FILE_NAME),
            ("", DEFAULT_FILE_NAME),
            ("   ", DEFAULT_FILE_NAME),
            (".bashrc", "bashrc"),
            ("facture.pdf.", "facture.pdf"),
            ("facture.pdf   ", "facture.pdf"),
            ("CON", "_CON"),
            ("con.txt", "_con.txt"),
            ("Nul .txt", "_Nul .txt"),
            ("COM1.log", "_COM1.log"),
            ("lpt9", "_lpt9"),
            ("console.txt", "console.txt"),
            ("a\u{0}b\nc\rd.txt", "a_b_c_d.txt"),
            ("fac\u{202e}fdp.exe", "fac_fdp.exe"),
            ("a:b*c?d\"e<f>g|h.txt", "a_b_c_d_e_f_g_h.txt"),
            ("été 2026.jpg", "été 2026.jpg"),
        ] {
            assert_eq!(safe_file_name(raw), safe, "{raw:?}");
        }
    }

    #[test]
    fn long_names_keep_their_extension() {
        let name = format!("{}.pdf", "a".repeat(500));
        let safe = safe_file_name(&name);
        assert!(safe.len() <= MAX_FILE_NAME_BYTES);
        assert!(safe.ends_with(".pdf"));
        let accents = format!("{}.txt", "é".repeat(300));
        let safe = safe_file_name(&accents);
        assert!(safe.len() <= MAX_FILE_NAME_BYTES && safe.ends_with(".txt"));
        let no_ext = "b".repeat(300);
        assert_eq!(safe_file_name(&no_ext).len(), MAX_FILE_NAME_BYTES);
    }

    #[test]
    fn numbered_names_avoid_overwriting() {
        assert_eq!(numbered_file_name("rapport.pdf", 2), "rapport (2).pdf");
        assert_eq!(numbered_file_name("rapport", 3), "rapport (3)");
    }

    #[test]
    fn executables_are_never_opened_directly() {
        for name in [
            "setup.exe",
            "FACTURE.PDF.EXE",
            "script.ps1",
            "x.sh",
            "app.desktop",
            "macro.docm",
            "lien.lnk",
            "sans-extension",
            "fac\u{202e}fdp.exe",
        ] {
            assert!(is_unsafe_to_open(name), "{name}");
        }
        for name in ["rapport.pdf", "photo.JPG", "notes.txt", "tableau.xlsx"] {
            assert!(!is_unsafe_to_open(name), "{name}");
        }
    }
}
