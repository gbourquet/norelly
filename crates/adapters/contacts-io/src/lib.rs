//! Adaptateur des fichiers de contacts (ADR-012) : implémente
//! [`ContactCodec`] (lecture vCard 2.1/3.0/4.0, CSV Google, Outlook et
//! Thunderbird, LDIF) et [`TextNormalizer`] (NFC, tables Unicode
//! d'`icu_normalizer`).
//!
//! Traduit sans décider : les fiches lues sont rendues brutes au domaine
//! (`plan_import`), qui les valide, les normalise et repère les doublons.
//! Entrées bornées : taille du fichier
//! ([`MAX_IMPORT_FILE_BYTES`]), nombre de fiches
//! ([`norelly_domain::contact_import::MAX_IMPORT_CARDS`]), longueur d'un
//! champ lu ([`MAX_FIELD_BYTES`]) et nombre d'adresses ou de téléphones
//! d'une fiche ([`MAX_VALUES_PER_CARD`]) ; au-delà, le fichier ou la
//! fiche est refusé, jamais tronqué.

use icu_normalizer::ComposingNormalizerBorrowed;
use norelly_domain::contact_export::{ExportCard, ExportFormat, MAX_EXPORT_CONTACTS};
use norelly_domain::contact_import::{ContactFormat, DecodedContacts, MAX_IMPORT_FILE_BYTES};
use norelly_domain::ports::{ContactCodec, ContactCodecError, TextNormalizer};

mod encode;
mod ldif;
mod tabular;
mod text;
mod vcard;

/// Longueur maximale d'un champ lu, en octets : au-delà, la fiche est
/// illisible (les notes, champ le plus long, tiennent en 40 Kio).
pub const MAX_FIELD_BYTES: usize = 64 * 1024;
/// Nombre maximal d'adresses, et de téléphones, lus dans une fiche.
pub const MAX_VALUES_PER_CARD: usize = 100;

/// Lecteur des fichiers de contacts.
#[derive(Debug, Default, Clone, Copy)]
pub struct ContactsIo;

impl ContactCodec for ContactsIo {
    fn decode(&self, bytes: &[u8]) -> Result<DecodedContacts, ContactCodecError> {
        if bytes.len() > MAX_IMPORT_FILE_BYTES {
            return Err(ContactCodecError::TooLarge);
        }
        let text = text::file_text(bytes).ok_or(ContactCodecError::UnknownFormat)?;
        if text.trim().is_empty() {
            return Err(ContactCodecError::UnknownFormat);
        }
        if vcard::looks_like(&text) {
            Ok(DecodedContacts {
                format: ContactFormat::VCard,
                cards: vcard::decode(&text)?,
            })
        } else if ldif::looks_like(&text) {
            Ok(DecodedContacts {
                format: ContactFormat::Ldif,
                cards: ldif::decode(&text)?,
            })
        } else {
            tabular::decode(&text)
        }
    }

    fn encode(
        &self,
        cards: &[ExportCard],
        format: ExportFormat,
    ) -> Result<Vec<u8>, ContactCodecError> {
        if cards.len() > MAX_EXPORT_CONTACTS {
            return Err(ContactCodecError::TooManyCards);
        }
        let mut out = encode::Output::new();
        match format {
            ExportFormat::VCard4 => {
                encode::vcard::encode(cards, encode::vcard::Version::V4, &mut out)
            }
            ExportFormat::VCard3 => {
                encode::vcard::encode(cards, encode::vcard::Version::V3, &mut out)
            }
            ExportFormat::GoogleCsv => encode::csv::encode(cards, &mut out),
            ExportFormat::Ldif => encode::ldif::encode(cards, &mut out),
        }?;
        Ok(out.into_bytes())
    }
}

/// Normalisation NFC par `icu_normalizer` (données Unicode compilées).
#[derive(Debug, Default, Clone, Copy)]
pub struct IcuNfc;

impl TextNormalizer for IcuNfc {
    fn nfc(&self, text: &str) -> String {
        ComposingNormalizerBorrowed::new_nfc()
            .normalize(text)
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use norelly_domain::contact_import::MAX_IMPORT_CARDS;
    use proptest::prelude::*;

    use super::*;

    /// Morceaux fréquents des trois formats, pour que les octets tirés au
    /// hasard atteignent les analyseurs plutôt que le refus du format.
    fn fragments() -> impl Strategy<Value = Vec<u8>> {
        let piece = prop_oneof![
            Just(&b"BEGIN:VCARD\r\n"[..]),
            Just(&b"END:VCARD\n"[..]),
            Just(&b"EMAIL;TYPE=\"a,b\";PREF=1:"[..]),
            Just(&b"N;ENCODING=QUOTED-PRINTABLE;CHARSET=UTF-8:=C3=A9;="[..]),
            Just(&b"dn: cn=x\n"[..]),
            Just(&b"mail:: SMOp\n"[..]),
            Just(&b"\n \t;:,\\\"="[..]),
            Just(&b"Primary Email,E-mail 1 - Value,Notes\n"[..]),
            Just(&b"\xEF\xBB\xBF\xFF\xFE\xE9"[..]),
        ]
        .prop_map(<[u8]>::to_vec);
        prop::collection::vec(
            prop_oneof![piece, prop::collection::vec(any::<u8>(), 0..8)],
            0..40,
        )
        .prop_map(|parts| parts.concat())
    }

    proptest! {
        #[test]
        fn decoding_never_panics_and_stays_bounded(bytes in fragments()) {
            if let Ok(decoded) = ContactsIo.decode(&bytes) {
                prop_assert!(decoded.cards.len() <= MAX_IMPORT_CARDS);
            }
        }

        #[test]
        fn nfc_is_idempotent(text in ".{0,40}") {
            let once = IcuNfc.nfc(&text);
            prop_assert_eq!(IcuNfc.nfc(&once), once);
        }
    }
}
