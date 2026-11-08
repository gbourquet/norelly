//! Écriture CSV aux colonnes de Google Contacts (lisibles par Outlook) :
//! UTF-8 avec BOM, séparateur virgule, fins de ligne CRLF, guillemets
//! quand il le faut (crate `csv`). Une adresse ou un téléphone par paire
//! de colonnes « E-mail n - Label / Value », l'adresse préférée marquée
//! « * » ; chaque cellule neutralisée contre les formules
//! (`neutralize_csv_cell`).

use norelly_domain::contact_export::{ExportCard, neutralize_csv_cell};
use norelly_domain::ports::ContactCodecError;

use super::Output;

/// Fiches `cards` dans `out`.
pub(crate) fn encode(cards: &[ExportCard], out: &mut Output) -> Result<(), ContactCodecError> {
    out.push("\u{FEFF}")?;
    let emails = cards
        .iter()
        .map(|c| c.emails.len())
        .max()
        .unwrap_or(0)
        .max(1);
    let phones = cards.iter().map(|c| c.phones.len()).max().unwrap_or(0);
    let mut header: Vec<String> = [
        "Name",
        "First Name",
        "Last Name",
        "Nickname",
        "Organization Name",
        "Notes",
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

    let mut writer = ::csv::WriterBuilder::new()
        .terminator(::csv::Terminator::CRLF)
        .from_writer(out);
    let too_large = |_| ContactCodecError::TooLarge;
    writer.write_record(&header).map_err(too_large)?;
    for card in cards {
        let text = |v: &Option<String>| v.clone().unwrap_or_default();
        let mut row = vec![
            text(&card.full_name),
            text(&card.given_name),
            text(&card.family_name),
            text(&card.nickname),
            text(&card.organization),
            text(&card.notes),
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
            "Name,First Name,Last Name,Nickname,Organization Name,Notes,\
             E-mail 1 - Label,E-mail 1 - Value,E-mail 2 - Label,E-mail 2 - Value,\
             Phone 1 - Label,Phone 1 - Value\r\n\
             Anne,,,,,,*,anne@example.com,,,,\r\n\
             ,Bob,Durand,Bobby,ACME,\"Ligne 1\nLigne 2, \"\"citée\"\"\",\
             * Work,bob@example.com,Home,bob@example.org,Mobile,'+33 6 12\r\n"
        );
    }

    #[test]
    fn every_cell_is_neutralized() {
        let card = ExportCard {
            full_name: Some("=1+1".into()),
            organization: Some("@SUM(A1)".into()),
            notes: Some("-x".into()),
            emails: vec![email("a@example.com", Some("=lab"), false)],
            ..ExportCard::default()
        };
        let text = written(&[card]);
        let row = text.lines().nth(1).unwrap_or_default();
        assert_eq!(row, "'=1+1,,,,'@SUM(A1),'-x,'=lab,a@example.com");
    }

    #[test]
    fn no_card_still_writes_a_recognizable_header() {
        assert_eq!(
            written(&[]),
            "Name,First Name,Last Name,Nickname,Organization Name,Notes,\
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
