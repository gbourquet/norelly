//! UTF-7 modifié des noms de boîtes IMAP (RFC 3501, 5.1.3).
//!
//! Les caractères ASCII imprimables passent tels quels, sauf `&` codé
//! `&-` ; le reste est codé en UTF-16 big-endian, en base64 modifiée
//! (`,` au lieu de `/`, sans `=`), entre `&` et `-`.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

fn value(b: u8) -> Option<u32> {
    ALPHABET
        .iter()
        .position(|&a| a == b)
        .and_then(|p| u32::try_from(p).ok())
}

/// Encode un nom pour l'envoyer au serveur.
#[must_use]
pub(crate) fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending: Vec<u16> = Vec::new();
    let flush = |pending: &mut Vec<u16>, out: &mut String| {
        if pending.is_empty() {
            return;
        }
        out.push('&');
        let bytes: Vec<u8> = pending.iter().flat_map(|u| u.to_be_bytes()).collect();
        for chunk in bytes.chunks(3) {
            let b = [
                chunk.first().copied().unwrap_or(0),
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            let digits = chunk.len() + 1;
            for i in 0..digits {
                let idx = (n >> (18 - 6 * i)) & 0x3f;
                out.push(char::from(ALPHABET[idx as usize]));
            }
        }
        out.push('-');
        pending.clear();
    };
    for c in name.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut pending, &mut out);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

/// Décode un nom reçu ; `None` si l'encodage est invalide (le nom est
/// alors pris tel quel par l'appelant, s'il est de l'UTF-8 valide).
#[must_use]
pub(crate) fn decode(raw: &[u8]) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let b = *raw.get(i)?;
        if b != b'&' {
            if !(0x20..=0x7e).contains(&b) {
                return None;
            }
            out.push(char::from(b));
            i += 1;
            continue;
        }
        let end = raw.get(i + 1..)?.iter().position(|&c| c == b'-')? + i + 1;
        let encoded = raw.get(i + 1..end)?;
        if encoded.is_empty() {
            out.push('&');
        } else {
            let mut bits: u32 = 0;
            let mut nbits = 0;
            let mut units = Vec::new();
            let mut bytes = Vec::new();
            for &c in encoded {
                bits = (bits << 6) | value(c)?;
                nbits += 6;
                if nbits >= 8 {
                    nbits -= 8;
                    bytes.push(u8::try_from((bits >> nbits) & 0xff).ok()?);
                }
            }
            // Bits de remplissage : forcément nuls et moins de 6.
            if nbits >= 6 || bits & ((1 << nbits) - 1) != 0 || bytes.len() % 2 != 0 {
                return None;
            }
            for pair in bytes.chunks(2) {
                units.push(u16::from_be_bytes([*pair.first()?, *pair.get(1)?]));
            }
            let text = String::from_utf16(&units).ok()?;
            // Un segment codé ne doit pas contenir d'ASCII imprimable.
            if text.chars().any(|c| (' '..='~').contains(&c)) {
                return None;
            }
            out.push_str(&text);
        }
        i = end + 1;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_examples() {
        assert_eq!(
            encode("~peter/mail/台北/日本語"),
            "~peter/mail/&U,BTFw-/&ZeVnLIqe-"
        );
        assert_eq!(
            decode(b"~peter/mail/&U,BTFw-/&ZeVnLIqe-").as_deref(),
            Some("~peter/mail/台北/日本語")
        );
        assert_eq!(encode("A&B"), "A&-B");
        assert_eq!(decode(b"A&-B").as_deref(), Some("A&B"));
    }

    #[test]
    fn french_names_roundtrip() {
        for name in [
            "Éléments envoyés",
            "Corbeille",
            "Brouillons",
            "Été 2026 😀",
            "&&",
            "",
        ] {
            assert_eq!(
                decode(encode(name).as_bytes()).as_deref(),
                Some(name),
                "{name}"
            );
        }
        assert_eq!(encode("Envoyés"), "Envoy&AOk-s");
    }

    #[test]
    fn invalid_input_is_rejected() {
        for bad in [
            &b"&U,BTFw"[..], // pas de fin
            b"&A-",          // bits incomplets
            b"&AGE-",        // ASCII codé (« a »)
            b"\xc3\xa9",     // 8 bits bruts
            b"&2D0-",        // demi-paire de substitution
            b"a\x01b",
        ] {
            assert_eq!(decode(bad), None, "{bad:?}");
        }
    }
}
