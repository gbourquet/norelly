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

use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
use norelly_domain::contact_details::Birthday;
use norelly_domain::contact_export::{ExportCard, ExportFormat, ExportList, MAX_EXPORT_CONTACTS};
use norelly_domain::contact_import::{ContactFormat, DecodedContacts, MAX_IMPORT_FILE_BYTES};
use norelly_domain::contact_lists::MAX_CONTACT_LIST_MEMBERS;
use norelly_domain::contact_photo::CardPhotos;
use norelly_domain::ports::{ContactCodec, ContactCodecError, TextNormalizer};

mod encode;
mod groups;
mod ldif;
mod photo;
mod tabular;
mod text;
mod vcard;

/// Membres lus au plus dans une liste (doublons compris) : au-delà, la
/// liste est illisible ; le domaine refuse aussi une liste de plus de
/// [`MAX_CONTACT_LIST_MEMBERS`] membres distincts.
pub const MAX_LIST_MEMBERS_READ: usize = 2 * MAX_CONTACT_LIST_MEMBERS;

pub use photo::ImagePhotos;

/// Longueur maximale d'un champ lu, en octets : au-delà, la fiche est
/// illisible (les notes, champ le plus long, tiennent en 40 Kio).
pub const MAX_FIELD_BYTES: usize = 64 * 1024;
/// Nombre maximal d'adresses, et de téléphones, lus dans une fiche.
pub const MAX_VALUES_PER_CARD: usize = 100;

/// Anniversaire lu : dans la forme du domaine (`Birthday::iso`) s'il en
/// a une forme reconnue, tel quel sinon (le domaine l'écarte alors).
pub(crate) fn birthday_text(raw: &str) -> String {
    let raw = raw.trim();
    Birthday::parse(raw).map_or_else(|| raw.to_owned(), |b| b.iso())
}

/// Anniversaire tiré d'une année (facultative), d'un mois et d'un jour
/// séparés (Thunderbird : CSV et LDIF), dans la forme du domaine ; vide
/// sans mois ni jour, tel quel s'ils ne sont pas des nombres (le domaine
/// l'écarte alors).
pub(crate) fn birthday_of_parts(year: &str, month: &str, day: &str) -> String {
    let (year, month, day) = (year.trim(), month.trim(), day.trim());
    if month.is_empty() && day.is_empty() {
        return String::new();
    }
    let number = |text: &str| text.parse::<u32>().ok();
    match (number(year), number(month), number(day)) {
        (Some(y), Some(m), Some(d)) => birthday_text(&format!("{y:04}-{m:02}-{d:02}")),
        (None, Some(m), Some(d)) if year.is_empty() => birthday_text(&format!("--{m:02}-{d:02}")),
        _ => format!("{year}-{month}-{day}"),
    }
}

/// Lignes d'une rue (rue, complément, boîte postale…) réunies, vides
/// omises.
pub(crate) fn street_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    lines
        .into_iter()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

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
            let (cards, photos) = vcard::decode_with_photos(&text)?;
            Ok(DecodedContacts {
                format: ContactFormat::VCard,
                cards,
                photos,
            })
        } else if ldif::looks_like(&text) {
            Ok(DecodedContacts {
                format: ContactFormat::Ldif,
                cards: ldif::decode(&text)?,
                photos: CardPhotos::new(),
            })
        } else {
            tabular::decode(&text)
        }
    }

    fn encode(
        &self,
        cards: &[ExportCard],
        lists: &[ExportList],
        format: ExportFormat,
    ) -> Result<Vec<u8>, ContactCodecError> {
        if cards.len().saturating_add(lists.len()) > MAX_EXPORT_CONTACTS {
            return Err(ContactCodecError::TooManyCards);
        }
        let mut out = encode::Output::new();
        match format {
            ExportFormat::VCard4 => {
                encode::vcard::encode(cards, lists, encode::vcard::Version::V4, &mut out)
            }
            ExportFormat::VCard3 => {
                encode::vcard::encode(cards, lists, encode::vcard::Version::V3, &mut out)
            }
            // Le CSV de Google ne porte pas de listes.
            ExportFormat::GoogleCsv => encode::csv::encode(cards, &mut out),
            ExportFormat::Ldif => encode::ldif::encode(cards, lists, &mut out),
        }?;
        Ok(out.into_bytes())
    }
}

/// Normalisation NFC et NFD par `icu_normalizer` (données Unicode
/// compilées).
#[derive(Debug, Default, Clone, Copy)]
pub struct IcuNfc;

impl TextNormalizer for IcuNfc {
    fn nfc(&self, text: &str) -> String {
        ComposingNormalizerBorrowed::new_nfc()
            .normalize(text)
            .into_owned()
    }

    fn nfd(&self, text: &str) -> String {
        DecomposingNormalizerBorrowed::new_nfd()
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
            Just(&b"KIND:group\nUID:urn:uuid:a\nMEMBER:urn:uuid:A\nMEMBER:mailto:%4"[..]),
            Just(&b"objectclass: groupOfNames\nmember: cn=x\\2,mail=\\"[..]),
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
