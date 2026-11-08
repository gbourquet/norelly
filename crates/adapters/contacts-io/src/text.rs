//! Encodages des fichiers de contacts : texte du fichier (BOM UTF-8 ou
//! UTF-16, UTF-8, repli Windows-1252), jeux de caractères déclarés,
//! quoted-printable et base64. Écrits pour le projet (formats simples,
//! sans dépendance).

/// Caractères Windows-1252 des octets 0x80 à 0x9F (les autres octets
/// valent leur point de code, comme en ISO-8859-1). Les cinq octets non
/// attribués deviennent U+FFFD.
const CP1252_HIGH: [char; 32] = [
    '€', '\u{FFFD}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{FFFD}', 'Ž',
    '\u{FFFD}', '\u{FFFD}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{FFFD}',
    'ž', 'Ÿ',
];

/// Octets lus en Windows-1252 (toujours possible).
pub(crate) fn windows_1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => CP1252_HIGH[usize::from(b - 0x80)],
            _ => char::from(b),
        })
        .collect()
}

/// Texte UTF-16 (sans BOM) ; `None` s'il est mal formé.
fn utf16(bytes: &[u8], little_endian: bool) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units = bytes.as_chunks::<2>().0.iter().map(|&pair| {
        if little_endian {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    });
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .ok()
}

/// Texte d'un fichier : UTF-16 si son BOM l'annonce, UTF-8 (BOM retiré)
/// s'il est valide, Windows-1252 sinon (exports d'Outlook et d'Excel).
/// `None` pour un UTF-16 mal formé.
pub(crate) fn file_text(bytes: &[u8]) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    Some(match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => windows_1252(bytes),
    })
}

/// Octets d'une valeur dans le jeu de caractères déclaré (vCard 2.1
/// `CHARSET`) : UTF-8, ou Windows-1252 pour les jeux latins (ISO-8859-1
/// est lu comme Windows-1252, son sur-ensemble, comme le font les
/// navigateurs). Sans jeu déclaré ou pour un jeu inconnu : UTF-8 s'il
/// est valide, Windows-1252 sinon. `None` pour un UTF-8 déclaré invalide.
pub(crate) fn charset_text(bytes: &[u8], charset: Option<&str>) -> Option<String> {
    let charset = charset.map(str::to_ascii_lowercase);
    match charset.as_deref() {
        Some("utf-8" | "utf8") => std::str::from_utf8(bytes).ok().map(str::to_owned),
        Some("windows-1252" | "cp1252" | "iso-8859-1" | "iso-8859-15" | "latin1") => {
            Some(windows_1252(bytes))
        }
        _ => Some(match std::str::from_utf8(bytes) {
            Ok(text) => text.to_owned(),
            Err(_) => windows_1252(bytes),
        }),
    }
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Valeur quoted-printable (RFC 2045) : `=XX` décodé, `=` en fin de ligne
/// (saut de ligne souple) retiré. `None` pour une séquence invalide.
pub(crate) fn quoted_printable(value: &str) -> Option<Vec<u8>> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'=' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        match bytes.get(i + 1..i + 3) {
            Some([b'\r', b'\n']) => i += 3,
            Some([b'\n', _]) => i += 2,
            Some(&[high, low]) => {
                out.push(hex(high)? << 4 | hex(low)?);
                i += 3;
            }
            // `=` final : saut de ligne souple.
            _ if bytes.get(i + 1).is_none_or(|&b| b == b'\n') => i += 2,
            _ => return None,
        }
    }
    Some(out)
}

fn base64_value(b: u8) -> Option<u32> {
    match b {
        b'A'..=b'Z' => Some(u32::from(b - b'A')),
        b'a'..=b'z' => Some(u32::from(b - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(b - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Valeur base64 (RFC 4648, espaces ignorés, `=` final facultatif).
/// `None` pour un caractère hors alphabet ou une longueur impossible.
pub(crate) fn base64(value: &str) -> Option<Vec<u8>> {
    let symbols: Vec<u8> = value.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let data = symbols
        .iter()
        .rposition(|&b| b != b'=')
        .map_or(&symbols[..0], |last| &symbols[..=last]);
    if symbols.len() - data.len() > 2 || data.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(data.len() / 4 * 3 + 2);
    for chunk in data.chunks(4) {
        let mut acc = 0_u32;
        for &b in chunk {
            acc = acc << 6 | base64_value(b)?;
        }
        let bits = 6 * chunk.len();
        acc <<= 24 - bits;
        let [_, b0, b1, b2] = acc.to_be_bytes();
        out.extend_from_slice(&[b0, b1, b2][..bits / 8]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_text_reads_boms_utf8_and_falls_back_to_windows_1252() {
        assert_eq!(file_text(b"\xEF\xBB\xBFabc").as_deref(), Some("abc"));
        assert_eq!(file_text("é".as_bytes()).as_deref(), Some("é"));
        assert_eq!(file_text(b"caf\xE9 \x80").as_deref(), Some("café €"));
        assert_eq!(file_text(b"\xFF\xFEa\x00\xE9\x00").as_deref(), Some("aé"));
        assert_eq!(file_text(b"\xFE\xFF\x00a").as_deref(), Some("a"));
        assert_eq!(file_text(b"\xFF\xFEa"), None);
        // Demi-paire de substitution isolée.
        assert_eq!(file_text(b"\xFF\xFE\x00\xD8"), None);
    }

    #[test]
    fn windows_1252_maps_every_byte() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(windows_1252(&all).chars().count(), 256);
        assert_eq!(windows_1252(b"\x9C\x81"), "œ\u{FFFD}");
    }

    #[test]
    fn charset_text_follows_the_declared_charset() {
        assert_eq!(
            charset_text(b"\xE9", Some("ISO-8859-1")).as_deref(),
            Some("é")
        );
        assert_eq!(
            charset_text("é".as_bytes(), Some("UTF-8")).as_deref(),
            Some("é")
        );
        assert_eq!(charset_text(b"\xE9", Some("utf-8")), None);
        assert_eq!(charset_text(b"\xE9", None).as_deref(), Some("é"));
        assert_eq!(
            charset_text("é".as_bytes(), Some("x-inconnu")).as_deref(),
            Some("é")
        );
    }

    #[test]
    fn quoted_printable_decodes_escapes_and_soft_breaks() {
        assert_eq!(
            quoted_printable("Premi=E8re=0D=0A"),
            Some(b"Premi\xE8re\r\n".to_vec())
        );
        assert_eq!(quoted_printable("a=\r\nb=\nc="), Some(b"abc".to_vec()));
        assert_eq!(quoted_printable("=c3=a9"), Some("é".as_bytes().to_vec()));
        assert_eq!(quoted_printable("=ZZ"), None);
        assert_eq!(quoted_printable("=E"), None);
    }

    #[test]
    fn base64_decodes_with_or_without_padding() {
        assert_eq!(base64("SMOpbMOobmU=").as_deref(), Some("Hélène".as_bytes()));
        assert_eq!(base64("SGk").as_deref(), Some(&b"Hi"[..]));
        assert_eq!(base64("SG k=\n").as_deref(), Some(&b"Hi"[..]));
        assert_eq!(base64("").as_deref(), Some(&b""[..]));
        assert_eq!(base64("S"), None);
        assert_eq!(base64("SGk==="), None);
        assert_eq!(base64("SG*k"), None);
    }
}
