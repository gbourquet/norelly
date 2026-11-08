//! Suites de contrat des ports de l'import et de l'export du carnet
//! ([`TextNormalizer`], [`ContactCodec`]), jouées contre les faux
//! ([`FakeNfc`], [`FakeContactCodec`]) et contre l'adaptateur
//! `contacts-io`.

use std::sync::{Mutex, PoisonError};

use norelly_domain::contact_export::{
    ExportCard, ExportFormat, MAX_EXPORT_CONTACTS, MAX_EXPORT_FILE_BYTES, export_card,
};
use norelly_domain::contact_import::{
    ContactFormat, DecodedContacts, ImportedCard, MAX_IMPORT_CARDS, MAX_IMPORT_FILE_BYTES,
};
use norelly_domain::contacts::{ContactDraft, ContactInput, EmailInput, PhoneInput};
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

/// Faux traducteur : rend `decoded` pour les octets `known`, et les
/// fiches d'un fichier qu'il a lui-même écrit ; refuse tout autre contenu
/// comme inconnu, et tout fichier trop gros. Un fichier écrit est un
/// numéro d'ordre : ses fiches sont gardées en mémoire.
#[derive(Debug)]
pub struct FakeContactCodec {
    pub known: Vec<u8>,
    pub decoded: DecodedContacts,
    exported: Mutex<Vec<(Vec<u8>, DecodedContacts)>>,
}

impl FakeContactCodec {
    #[must_use]
    pub fn new(known: Vec<u8>, decoded: DecodedContacts) -> Self {
        Self {
            known,
            decoded,
            exported: Mutex::default(),
        }
    }
}

impl ContactCodec for FakeContactCodec {
    fn decode(&self, bytes: &[u8]) -> Result<DecodedContacts, ContactCodecError> {
        if bytes.len() > MAX_IMPORT_FILE_BYTES {
            return Err(ContactCodecError::TooLarge);
        }
        if bytes == self.known.as_slice() {
            return Ok(self.decoded.clone());
        }
        let exported = self.exported.lock().unwrap_or_else(PoisonError::into_inner);
        exported
            .iter()
            .find(|(written, _)| written == bytes)
            .map(|(_, decoded)| decoded.clone())
            .ok_or(ContactCodecError::UnknownFormat)
    }

    fn encode(
        &self,
        cards: &[ExportCard],
        format: ExportFormat,
    ) -> Result<Vec<u8>, ContactCodecError> {
        if cards.len() > MAX_EXPORT_CONTACTS {
            return Err(ContactCodecError::TooManyCards);
        }
        let size: usize = cards.iter().map(card_size).sum();
        if size > MAX_EXPORT_FILE_BYTES {
            return Err(ContactCodecError::TooLarge);
        }
        let mut exported = self.exported.lock().unwrap_or_else(PoisonError::into_inner);
        let bytes = format!("export factice n° {}", exported.len()).into_bytes();
        exported.push((
            bytes.clone(),
            DecodedContacts {
                format: import_format(format),
                cards: cards.iter().map(imported).collect(),
            },
        ));
        Ok(bytes)
    }
}

/// Taille d'une fiche pour le faux : la somme de ses textes.
fn card_size(card: &ExportCard) -> usize {
    let texts = [
        &card.full_name,
        &card.given_name,
        &card.family_name,
        &card.nickname,
        &card.organization,
        &card.notes,
    ];
    texts.into_iter().flatten().map(String::len).sum::<usize>()
        + card
            .emails
            .iter()
            .map(|e| e.address.len() + e.label.as_ref().map_or(0, String::len))
            .sum::<usize>()
        + card
            .phones
            .iter()
            .map(|p| p.number.len() + p.label.as_ref().map_or(0, String::len))
            .sum::<usize>()
}

/// Format reconnu à la lecture d'un export dans `format`.
#[must_use]
pub fn import_format(format: ExportFormat) -> ContactFormat {
    match format {
        ExportFormat::VCard4 | ExportFormat::VCard3 => ContactFormat::VCard,
        ExportFormat::GoogleCsv => ContactFormat::GoogleCsv,
        ExportFormat::Ldif => ContactFormat::Ldif,
    }
}

/// Fiche qu'un traducteur fidèle relit dans l'export de `card`.
#[must_use]
pub fn imported(card: &ExportCard) -> ImportedCard {
    let owned = |v: &Option<String>| v.clone().unwrap_or_default();
    ImportedCard::Contact(ContactInput {
        display_name: owned(&card.full_name),
        given_name: owned(&card.given_name),
        family_name: owned(&card.family_name),
        nickname: owned(&card.nickname),
        organization: owned(&card.organization),
        notes: owned(&card.notes),
        emails: card
            .emails
            .iter()
            .map(|e| EmailInput {
                address: e.address.clone(),
                label: owned(&e.label),
                preferred: e.preferred,
            })
            .collect(),
        phones: card
            .phones
            .iter()
            .map(|p| PhoneInput {
                number: p.number.clone(),
                label: owned(&p.label),
            })
            .collect(),
        favorite: false,
    })
}

/// Formats d'export, tous.
pub const EXPORT_FORMATS: [ExportFormat; 4] = [
    ExportFormat::VCard4,
    ExportFormat::VCard3,
    ExportFormat::GoogleCsv,
    ExportFormat::Ldif,
];

fn draft(input: ContactInput) -> ContactDraft {
    ContactDraft::new(input).unwrap_or_else(|e| panic!("fiche de test refusée : {e}"))
}

/// Fiches d'exemple de l'export : accents, notes sur plusieurs lignes,
/// plusieurs adresses dont la préférée (pas la première), séparateurs et
/// échappements de chaque format, valeurs qui ressemblent à des formules,
/// lignes longues à replier.
#[must_use]
pub fn sample_export_drafts() -> Vec<ContactDraft> {
    let email = |address: &str, label: &str, preferred| EmailInput {
        address: address.into(),
        label: label.into(),
        preferred,
    };
    let phone = |number: &str, label: &str| PhoneInput {
        number: number.into(),
        label: label.into(),
    };
    vec![
        draft(ContactInput {
            display_name: "Hélène Lefèvre".into(),
            given_name: "Hélène".into(),
            family_name: "Lefèvre".into(),
            nickname: "Lène".into(),
            organization: "Mairie de Saint-Étienne".into(),
            notes: "Première ligne\nseconde ligne, avec ; et \\ et \"guillemets\"\n\tindentée"
                .into(),
            emails: vec![
                email("helene@example.org", "Maison", false),
                email("helene.lefevre@example.fr", "Travail", true),
                email("lene@example.net", "", false),
            ],
            phones: vec![
                phone("+33 6 11 22 33 44", "Portable"),
                phone("04 77 00 00 00", ""),
            ],
            favorite: true,
        }),
        draft(ContactInput {
            given_name: "Seán".into(),
            family_name: "O'Brien; dit, \"Jo\"".into(),
            organization: "=HYPERLINK(\"http://example.com\")".into(),
            notes: "-moins\n@arobase\n+plus".into(),
            emails: vec![email("sean@example.ie", "", false)],
            phones: vec![phone("-12", "")],
            ..ContactInput::default()
        }),
        draft(ContactInput {
            notes: "Une note longue qui dépasse largement soixante-quinze octets, \
                    pour forcer le repli des lignes : ééééééééééééééééééééééééé \
                    çàùœ€ 😀😀😀😀😀😀😀😀😀😀 fin."
                .into(),
            emails: vec![email("anonyme@example.com", "", false)],
            ..ContactInput::default()
        }),
        draft(ContactInput {
            display_name: " ".into(),
            nickname: "Lolo".into(),
            emails: vec![
                email("a@example.com", "", false),
                email("b@example.com", "", false),
                email("c@example.com", "", true),
            ],
            ..ContactInput::default()
        }),
    ]
}

/// Contrat de l'écriture : ce que `codec` écrit dans chaque format, il le
/// relit à l'identique ([`imported`]) ; trop de fiches ou un fichier trop
/// gros sont refusés.
pub fn contact_encoding_contract(codec: &impl ContactCodec, drafts: &[ContactDraft]) {
    for format in EXPORT_FORMATS {
        let cards: Vec<ExportCard> = drafts.iter().map(|d| export_card(d, format)).collect();
        let bytes = codec
            .encode(&cards, format)
            .unwrap_or_else(|e| panic!("{format:?} : {e}"));
        assert!(bytes.len() <= MAX_EXPORT_FILE_BYTES);
        assert_eq!(
            codec.decode(&bytes),
            Ok(DecodedContacts {
                format: import_format(format),
                cards: cards.iter().map(imported).collect(),
            }),
            "{format:?}"
        );
    }
    let tiny = export_card(&drafts[0], ExportFormat::VCard4);
    let many = vec![tiny; MAX_EXPORT_CONTACTS + 1];
    for format in EXPORT_FORMATS {
        assert_eq!(
            codec.encode(&many, format),
            Err(ContactCodecError::TooManyCards),
            "{format:?}"
        );
    }
    // Des notes de longueur maximale, assez de fiches pour dépasser la
    // taille d'un fichier.
    let notes = "é".repeat(norelly_domain::contacts::MAX_CONTACT_NOTES_CHARS);
    let heavy = ExportCard {
        notes: Some(notes),
        ..export_card(&drafts[0], ExportFormat::VCard4)
    };
    let heavy = vec![heavy; MAX_EXPORT_FILE_BYTES / 20_000 + 1];
    for format in EXPORT_FORMATS {
        assert_eq!(
            codec.encode(&heavy, format),
            Err(ContactCodecError::TooLarge),
            "{format:?}"
        );
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
        let codec = FakeContactCodec::new(SAMPLE_VCARD.to_vec(), decoded.clone());
        contact_codec_contract(&codec, SAMPLE_VCARD, &decoded);
    }

    #[test]
    fn fake_codec_passes_encoding_contract() {
        let codec = FakeContactCodec::new(
            Vec::new(),
            DecodedContacts {
                format: ContactFormat::VCard,
                cards: Vec::new(),
            },
        );
        contact_encoding_contract(&codec, &sample_export_drafts());
    }
}
