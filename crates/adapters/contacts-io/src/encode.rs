//! Écriture des fichiers de contacts (export) : vCard 4.0 et 3.0, CSV aux
//! colonnes de Google, LDIF de Thunderbird. Les fiches arrivent préparées
//! par le domaine (`export_card`) : ce module ne fait que la mise en
//! forme, dans une sortie bornée ([`Output`]).

use std::io;

use norelly_domain::contact_export::MAX_EXPORT_FILE_BYTES;
use norelly_domain::ports::ContactCodecError;

pub(crate) mod csv;
pub(crate) mod ldif;
pub(crate) mod vcard;

/// Octets écrits, au plus `limit` : au-delà, l'écriture échoue sans rien
/// ajouter (aucun fichier démesuré n'est produit en mémoire).
#[derive(Debug)]
pub(crate) struct Output {
    bytes: Vec<u8>,
    limit: usize,
}

impl Output {
    pub(crate) fn new() -> Self {
        Self::with_limit(MAX_EXPORT_FILE_BYTES)
    }

    pub(crate) fn with_limit(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }

    pub(crate) fn push(&mut self, text: &str) -> Result<(), ContactCodecError> {
        self.push_bytes(text.as_bytes())
    }

    fn push_bytes(&mut self, bytes: &[u8]) -> Result<(), ContactCodecError> {
        if self.bytes.len().saturating_add(bytes.len()) > self.limit {
            return Err(ContactCodecError::TooLarge);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl io::Write for Output {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.push_bytes(buf)
            .map(|()| buf.len())
            .map_err(io::Error::other)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Valeur en base64 (RFC 4648, avec `=` final).
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let [b0, b1, b2] = [0, 1, 2].map(|i| chunk.get(i).copied().unwrap_or(0));
        let n = u32::from(b0) << 16 | u32::from(b1) << 8 | u32::from(b2);
        for i in 0..4 {
            if i <= chunk.len() {
                let index = (n >> (18 - 6 * i)) & 0x3F;
                out.push(char::from(ALPHABET[index as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::text;

    #[test]
    fn output_refuses_to_grow_past_its_limit() {
        let mut out = Output::with_limit(5);
        assert_eq!(out.push("abc"), Ok(()));
        assert_eq!(out.push("def"), Err(ContactCodecError::TooLarge));
        assert_eq!(out.push("de"), Ok(()));
        assert!(out.write_all(b"x").is_err());
        assert_eq!(out.into_bytes(), b"abcde");
    }

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        for (raw, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(raw.as_bytes()), encoded);
        }
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(text::base64(&base64(&all)), Some(all));
    }
}
