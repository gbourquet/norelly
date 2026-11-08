//! Suites de contrat des ports de l'import du carnet ([`TextNormalizer`],
//! [`ContactCodec`]), jouées contre les faux ([`FakeNfc`],
//! [`FakeContactCodec`]) et contre l'adaptateur `contacts-io`.

use norelly_domain::contact_import::{DecodedContacts, MAX_IMPORT_CARDS, MAX_IMPORT_FILE_BYTES};
use norelly_domain::ports::{ContactCodec, ContactCodecError, TextNormalizer};

/// Compositions connues du faux : forme décomposée, forme NFC.
const COMPOSITIONS: [(&str, &str); 6] = [
    ("e\u{301}", "é"),
    ("E\u{301}", "É"),
    ("e\u{300}", "è"),
    ("c\u{327}", "ç"),
    // Signe angström (décomposition singleton) : A rond en NFC.
    ("\u{212B}", "Å"),
    // Signe kelvin : K latin en NFC.
    ("\u{212A}", "K"),
];

/// Faux normaliseur : ne compose que [`COMPOSITIONS`].
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeNfc;

impl TextNormalizer for FakeNfc {
    fn nfc(&self, text: &str) -> String {
        COMPOSITIONS
            .iter()
            .fold(text.to_owned(), |acc, (from, to)| acc.replace(from, to))
    }
}

/// Contrat de la normalisation NFC.
pub fn text_normalizer_contract(normalizer: &impl TextNormalizer) {
    assert_eq!(normalizer.nfc(""), "");
    assert_eq!(normalizer.nfc("Alice Martin"), "Alice Martin");
    for (decomposed, composed) in COMPOSITIONS {
        assert_eq!(normalizer.nfc(decomposed), composed, "{decomposed:?}");
        assert_eq!(normalizer.nfc(composed), composed, "NFC non idempotente");
    }
    assert_eq!(
        normalizer.nfc("E\u{301}lodie Ce\u{301}zanne"),
        "Élodie Cézanne"
    );
    // Une forme de compatibilité (arobase pleine largeur) reste telle
    // quelle : NFC n'est pas NFKC.
    assert_eq!(normalizer.nfc("\u{FF20}"), "\u{FF20}");
}

/// Faux traducteur : rend `decoded` pour les octets `known`, refuse tout
/// autre contenu comme inconnu, et tout fichier trop gros.
#[derive(Debug, Clone)]
pub struct FakeContactCodec {
    pub known: Vec<u8>,
    pub decoded: DecodedContacts,
}

impl ContactCodec for FakeContactCodec {
    fn decode(&self, bytes: &[u8]) -> Result<DecodedContacts, ContactCodecError> {
        if bytes.len() > MAX_IMPORT_FILE_BYTES {
            return Err(ContactCodecError::TooLarge);
        }
        if bytes == self.known.as_slice() {
            Ok(self.decoded.clone())
        } else {
            Err(ContactCodecError::UnknownFormat)
        }
    }
}

/// Contrat d'un traducteur de fichiers de contacts : `sample` se lit
/// comme `expected` ; un contenu sans format reconnu est refusé, un
/// fichier trop gros aussi, quel que soit son contenu.
pub fn contact_codec_contract(
    codec: &impl ContactCodec,
    sample: &[u8],
    expected: &DecodedContacts,
) {
    assert_eq!(codec.decode(sample).as_ref(), Ok(expected));
    assert!(expected.cards.len() <= MAX_IMPORT_CARDS);
    assert_eq!(
        codec.decode(b"\x00\x01\x02 rien de connu"),
        Err(ContactCodecError::UnknownFormat)
    );
    assert_eq!(codec.decode(b""), Err(ContactCodecError::UnknownFormat));
    let mut big = sample.to_vec();
    big.resize(MAX_IMPORT_FILE_BYTES + 1, b'\n');
    assert_eq!(codec.decode(&big), Err(ContactCodecError::TooLarge));
}

/// Fichier d'exemple du contrat : une carte vCard 3.0.
pub const SAMPLE_VCARD: &[u8] =
    b"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Alice Martin\r\nN:Martin;Alice;;;\r\nEMAIL;TYPE=INTERNET:alice@example.com\r\nEND:VCARD\r\n";

#[cfg(test)]
mod tests {
    use norelly_domain::contact_import::{ContactFormat, ImportedCard};
    use norelly_domain::contacts::{ContactInput, EmailInput};

    use super::*;

    #[test]
    fn fake_nfc_passes_contract() {
        text_normalizer_contract(&FakeNfc);
    }

    #[test]
    fn fake_codec_passes_contract() {
        let decoded = DecodedContacts {
            format: ContactFormat::VCard,
            cards: vec![ImportedCard::Contact(ContactInput {
                display_name: "Alice Martin".into(),
                emails: vec![EmailInput {
                    address: "alice@example.com".into(),
                    ..EmailInput::default()
                }],
                ..ContactInput::default()
            })],
        };
        let codec = FakeContactCodec {
            known: SAMPLE_VCARD.to_vec(),
            decoded: decoded.clone(),
        };
        contact_codec_contract(&codec, SAMPLE_VCARD, &decoded);
    }
}
