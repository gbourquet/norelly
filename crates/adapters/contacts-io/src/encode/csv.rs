//! Écriture CSV aux colonnes de Google Contacts (lisibles par Outlook) :
//! UTF-8 avec BOM, séparateur virgule, fins de ligne CRLF, guillemets
//! quand il le faut (crate `csv`). Une adresse ou un téléphone par paire
//! de colonnes « E-mail n - Label / Value », l'adresse préférée marquée
//! « * » ; chaque cellule neutralisée contre les formules
//! (`neutralize_csv_cell`). La colonne « Labels » (`google_labels`) range
//! chaque fiche dans « Mes contacts » à l'import Google, au lieu d'un
//! libellé propre à l'import, et un favori aussi dans ses favoris.

use norelly_domain::contact_export::{ExportCard, google_labels, neutralize_csv_cell};
use norelly_domain::ports::ContactCodecError;

use super::Output;

/// Colonnes d'une adresse postale Google, dans l'ordre écrit.
const PLACE_PARTS: [&str; 6] = [
    "Label",
    "Street",
    "City",
    "Region",
    "Postal Code",
    "Country",
];

/// Colonnes du fichier, fixées par toutes ses fiches : autant de paires
/// d'adresses, de téléphones, d'adresses postales et de sites que la fiche
/// qui en a le plus (une adresse au moins) ; titre et anniversaire
/// seulement si une fiche en a.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Layout {
    emails: usize,
    phones: usize,
    titles: bool,
    birthdays: bool,
    places: usize,
    sites: usize,
}

impl Layout {
    /// Colonnes élargies aux fiches `cards`.
    pub(crate) fn survey(&mut self, cards: &[ExportCard]) {
        for card in cards {
            self.emails = self.emails.max(card.emails.len());
            self.phones = self.phones.max(card.phones.len());
            self.titles |= card.title.is_some();
            self.birthdays |= card.birthday.is_some();
            self.places = self.places.max(card.postal_addresses.len());
            self.sites = self.sites.max(card.websites.len());
        }
    }

    /// La fiche `card` s'écrit-elle entière dans ces colonnes ?
    pub(crate) fn fits(&self, card: &ExportCard) -> bool {
        card.emails.len() <= self.emails()
            && card.phones.len() <= self.phones
            && (self.titles || card.title.is_none())
            && (self.birthdays || card.birthday.is_none())
            && card.postal_addresses.len() <= self.places
            && card.websites.len() <= self.sites
    }

    fn emails(&self) -> usize {
        self.emails.max(1)
    }
}

/// Fiches `cards` dans `out`, en-tête compris.
#[cfg(test)]
pub(crate) fn encode(cards: &[ExportCard], out: &mut Output) -> Result<(), ContactCodecError> {
    let mut layout = Layout::default();
    layout.survey(cards);
    encode_header(&layout, out)?;
    encode_rows(cards, &layout, out)
}

/// Rédacteur CSV de Google (virgule, CRLF) vers `out`.
fn writer(out: &mut Output) -> ::csv::Writer<&mut Output> {
    ::csv::WriterBuilder::new()
        .terminator(::csv::Terminator::CRLF)
        .from_writer(out)
}

/// Début du fichier : BOM, puis ligne d'en-tête des colonnes `layout`.
pub(crate) fn encode_header(layout: &Layout, out: &mut Output) -> Result<(), ContactCodecError> {
    out.push("\u{FEFF}")?;
    let Layout {
        phones,
        titles,
        birthdays,
        places,
        sites,
        ..
    } = *layout;
    let emails = layout.emails();
    let mut header: Vec<String> = [
        "Name",
        "First Name",
        "Last Name",
        "Nickname",
        "Organization Name",
        "Notes",
        "Labels",
    ]
    .map(str::to_owned)
    .to_vec();
    for n in 1..=emails {
        header.push(format!("E-mail {n} - Label"));
        header.push(format!("E-mail {n} - Value"));
    }
    for n in 1..=phones {
        header.push(format!("Phone {n} - Label"));
        header.push(format!("Phone {n} - Value"));
    }
    // Colonnes des détails, seulement si une fiche en a.
    if titles {
        header.push("Organization Title".to_owned());
    }
    if birthdays {
        header.push("Birthday".to_owned());
    }
    for n in 1..=places {
        for part in PLACE_PARTS {
            header.push(format!("Address {n} - {part}"));
        }
    }
    for n in 1..=sites {
        header.push(format!("Website {n} - Label"));
        header.push(format!("Website {n} - Value"));
    }
    let mut writer = writer(out);
    writer
        .write_record(&header)
        .map_err(|_| ContactCodecError::TooLarge)?;
    writer.flush().map_err(|_| ContactCodecError::TooLarge)
}

/// Lignes des fiches `cards` dans `out`, aux colonnes `layout` (repérées
/// sur toutes les fiches du fichier) ; une fiche qui n'y tient pas est
/// refusée, jamais tronquée.
pub(crate) fn encode_rows(
    cards: &[ExportCard],
    layout: &Layout,
    out: &mut Output,
) -> Result<(), ContactCodecError> {
    if !cards.iter().all(|card| layout.fits(card)) {
        return Err(ContactCodecError::NotSurveyed);
    }
    let Layout {
        phones,
        titles,
        birthdays,
        places,
        sites,
        ..
    } = *layout;
    let emails = layout.emails();
    let mut writer = writer(out);
    let too_large = |_| ContactCodecError::TooLarge;
    for card in cards {
        let text = |v: &Option<String>| v.clone().unwrap_or_default();
        let mut row = vec![
            text(&card.full_name),
            text(&card.given_name),
            text(&card.family_name),
            text(&card.nickname),
            text(&card.organization),
            text(&card.notes),
            google_labels(card.favorite).to_owned(),
        ];
        for n in 0..emails {
            match card.emails.get(n) {
                Some(email) => {
                    let label = text(&email.label);
                    row.push(if email.preferred {
                        format!("* {label}").trim_end().to_owned()
                    } else {
                        label
                    });
                    row.push(email.address.clone());
                }
                None => row.extend([String::new(), String::new()]),
            }
        }
        for n in 0..phones {
            match card.phones.get(n) {
                Some(phone) => {
                    row.push(text(&phone.label));
                    row.push(phone.number.clone());
                }
                None => row.extend([String::new(), String::new()]),
            }
        }
        if titles {
            row.push(text(&card.title));
        }
        if birthdays {
            row.push(card.birthday.map(|b| b.iso()).unwrap_or_default());
        }
        for n in 0..places {
            match card.postal_addresses.get(n) {
                Some(place) => row.extend(
                    [
                        &place.label,
                        &place.street,
                        &place.locality,
                        &place.region,
                        &place.postal_code,
                        &place.country,
                    ]
                    .map(text),
                ),
                None => row.extend(PLACE_PARTS.map(|_| String::new())),
            }
        }
        for n in 0..sites {
            match card.websites.get(n) {
                Some(site) => {
                    row.push(text(&site.label));
                    row.push(site.url.clone());
                }
                None => row.extend([String::new(), String::new()]),
            }
        }
        writer
            .write_record(
                row.iter()
                    .map(|cell| neutralize_csv_cell(cell).into_owned()),
            )
            .map_err(too_large)?;
    }
    writer.flush().map_err(|_| ContactCodecError::TooLarge)
}

#[cfg(test)]
mod tests {
    use norelly_domain::contact_export::{ExportEmail, ExportPhone};

    use super::*;

    fn written(cards: &[ExportCard]) -> String {
        let mut out = Output::new();
        encode(cards, &mut out).unwrap_or_else(|e| panic!("{e}"));
        let bytes = out.into_bytes();
        let text = bytes
            .strip_prefix(b"\xEF\xBB\xBF")
            .unwrap_or_else(|| panic!("BOM absent"));
        String::from_utf8(text.to_vec()).unwrap_or_else(|e| panic!("{e}"))
    }

    fn email(address: &str, label: Option<&str>, preferred: bool) -> ExportEmail {
        ExportEmail {
            address: address.into(),
            label: label.map(str::to_owned),
            preferred,
        }
    }

    #[test]
    fn header_has_as_many_slots_as_the_largest_card() {
        let one = ExportCard {
            full_name: Some("Anne".into()),
            emails: vec![email("anne@example.com", None, true)],
            ..ExportCard::default()
        };
        let two = ExportCard {
            given_name: Some("Bob".into()),
            family_name: Some("Durand".into()),
            nickname: Some("Bobby".into()),
            organization: Some("ACME".into()),
            notes: Some("Ligne 1\nLigne 2, \"citée\"".into()),
            emails: vec![
                email("bob@example.com", Some("Work"), true),
                email("bob@example.org", Some("Home"), false),
            ],
            phones: vec![ExportPhone {
                number: "+33 6 12".into(),
                label: Some("Mobile".into()),
            }],
            ..ExportCard::default()
        };
        assert_eq!(
            written(&[one, two]),
            "Name,First Name,Last Name,Nickname,Organization Name,Notes,Labels,\
             E-mail 1 - Label,E-mail 1 - Value,E-mail 2 - Label,E-mail 2 - Value,\
             Phone 1 - Label,Phone 1 - Value\r\n\
             Anne,,,,,,* myContacts,*,anne@example.com,,,,\r\n\
             ,Bob,Durand,Bobby,ACME,\"Ligne 1\nLigne 2, \"\"citée\"\"\",* myContacts,\
             * Work,bob@example.com,Home,bob@example.org,Mobile,'+33 6 12\r\n"
        );
    }

    #[test]
    fn details_get_google_columns_only_when_a_card_has_them() {
        use norelly_domain::contact_details::Birthday;
        use norelly_domain::contact_export::{ExportPostalAddress, ExportWebsite};
        let card = ExportCard {
            title: Some("Cheffe".into()),
            birthday: Birthday::new(None, 4, 12),
            emails: vec![email("a@example.com", None, true)],
            postal_addresses: vec![ExportPostalAddress {
                street: Some("1 rue A\nBât. B".into()),
                locality: Some("Lyon".into()),
                region: Some("Rhône".into()),
                postal_code: Some("69001".into()),
                country: Some("-France".into()),
                label: Some("Home".into()),
            }],
            websites: vec![ExportWebsite {
                url: "https://example.com".into(),
                label: Some("Blog".into()),
            }],
            ..ExportCard::default()
        };
        assert_eq!(
            written(&[card]),
            "Name,First Name,Last Name,Nickname,Organization Name,Notes,Labels,\
             E-mail 1 - Label,E-mail 1 - Value,Organization Title,Birthday,\
             Address 1 - Label,Address 1 - Street,Address 1 - City,Address 1 - Region,\
             Address 1 - Postal Code,Address 1 - Country,\
             Website 1 - Label,Website 1 - Value\r\n\
             ,,,,,,* myContacts,*,a@example.com,Cheffe,'--04-12,\
             Home,\"1 rue A\nBât. B\",Lyon,Rhône,69001,'-France,\
             Blog,https://example.com\r\n"
        );
    }

    #[test]
    fn every_cell_is_neutralized() {
        let card = ExportCard {
            full_name: Some("=1+1".into()),
            organization: Some("@SUM(A1)".into()),
            nickname: Some("\u{FE62}1".into()),
            notes: Some("-x".into()),
            emails: vec![email("a@example.com", Some("\u{2212}lab"), false)],
            ..ExportCard::default()
        };
        let text = written(&[card]);
        let row = text.lines().nth(1).unwrap_or_default();
        assert_eq!(
            row,
            "'=1+1,,,'\u{FE62}1,'@SUM(A1),'-x,* myContacts,'\u{2212}lab,a@example.com"
        );
    }

    #[test]
    fn the_favorite_joins_my_contacts_in_the_labels_column() {
        let favorite = ExportCard {
            full_name: Some("Zoé".into()),
            emails: vec![email("zoe@example.com", None, true)],
            favorite: true,
            ..ExportCard::default()
        };
        let text = written(&[favorite]);
        let row = text.lines().nth(1).unwrap_or_default();
        assert_eq!(row, "Zoé,,,,,,* myContacts ::: * starred,*,zoe@example.com");
    }

    #[test]
    fn no_card_still_writes_a_recognizable_header() {
        assert_eq!(
            written(&[]),
            "Name,First Name,Last Name,Nickname,Organization Name,Notes,Labels,\
             E-mail 1 - Label,E-mail 1 - Value\r\n"
        );
    }

    #[test]
    fn output_limit_stops_the_writing() {
        let card = ExportCard {
            notes: Some("x".repeat(500)),
            emails: vec![email("a@example.com", None, true)],
            ..ExportCard::default()
        };
        let mut out = Output::with_limit(300);
        assert_eq!(encode(&[card], &mut out), Err(ContactCodecError::TooLarge));
    }
}
