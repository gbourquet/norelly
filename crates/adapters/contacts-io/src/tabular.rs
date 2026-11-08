//! Lecture des exports CSV de Google Contacts, d'Outlook et de
//! Thunderbird (en-têtes anglais, et français pour Outlook et
//! Thunderbird), par la crate `csv`. Le format est reconnu à l'en-tête,
//! le séparateur (`,`, `;` ou tabulation) à la première ligne.

use std::collections::HashMap;

use norelly_domain::contact_import::{
    ContactFormat, DecodedContacts, ImportedCard, MAX_IMPORT_CARDS,
};
use norelly_domain::contacts::{ContactInput, EmailInput, PhoneInput};
use norelly_domain::ports::ContactCodecError;

use crate::{MAX_FIELD_BYTES, MAX_VALUES_PER_CARD};

/// Rôle d'une colonne.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    DisplayName,
    GivenName,
    FamilyName,
    Nickname,
    Organization,
    Notes,
    /// Adresse ; préférée pour l'adresse principale de Thunderbird.
    Email {
        preferred: bool,
    },
    /// Téléphone, avec son étiquette (comme les types vCard).
    Phone(&'static str),
    /// Google : étiquette ou valeur de l'adresse ou du téléphone n° `n`.
    GoogleEmailLabel(u32),
    GoogleEmailValue(u32),
    GooglePhoneLabel(u32),
    GooglePhoneValue(u32),
}

/// Colonne numérotée de Google : `<préfixe> <n> - <suffixe>`.
fn google_slot(header: &str, prefix: &str, suffixes: &[&str]) -> Option<u32> {
    let rest = header.strip_prefix(prefix)?.strip_prefix(' ')?;
    let (number, suffix) = rest.split_once(" - ")?;
    suffixes
        .contains(&suffix)
        .then(|| number.parse().ok())
        .flatten()
}

/// Rôle d'un en-tête (en minuscules, sans espaces de bord) ; `None` pour
/// une colonne ignorée.
fn column(header: &str) -> Option<Column> {
    use Column::{
        DisplayName, Email, FamilyName, GivenName, GoogleEmailLabel, GoogleEmailValue,
        GooglePhoneLabel, GooglePhoneValue, Nickname, Notes, Organization, Phone,
    };
    if let Some(n) = google_slot(header, "e-mail", &["value"]) {
        return Some(GoogleEmailValue(n));
    }
    if let Some(n) = google_slot(header, "e-mail", &["label", "type"]) {
        return Some(GoogleEmailLabel(n));
    }
    if let Some(n) = google_slot(header, "phone", &["value"]) {
        return Some(GooglePhoneValue(n));
    }
    if let Some(n) = google_slot(header, "phone", &["label", "type"]) {
        return Some(GooglePhoneLabel(n));
    }
    Some(match header {
        "display name" | "nom à afficher" | "name" => DisplayName,
        "first name" | "given name" | "prénom" => GivenName,
        "last name" | "family name" | "nom de famille" | "nom" => FamilyName,
        "nickname" | "surnom" => Nickname,
        "organization name" | "organization 1 - name" | "organization" | "company" | "société" => {
            Organization
        }
        "notes" => Notes,
        "primary email" | "adresse électronique principale" => Email { preferred: true },
        "secondary email"
        | "adresse électronique secondaire"
        | "e-mail address"
        | "e-mail 2 address"
        | "e-mail 3 address"
        | "adresse de messagerie"
        | "adresse de messagerie 2"
        | "adresse de messagerie 3" => Email { preferred: false },
        "work phone"
        | "business phone"
        | "business phone 2"
        | "company main phone"
        | "tél. professionnel"
        | "téléphone (bureau)"
        | "téléphone 2 (bureau)"
        | "téléphone bureau" => Phone("work"),
        "home phone"
        | "home phone 2"
        | "tél. personnel"
        | "téléphone (domicile)"
        | "téléphone 2 (domicile)"
        | "téléphone domicile" => Phone("home"),
        "mobile number" | "mobile phone" | "portable" | "téléphone mobile" | "tél. mobile" => {
            Phone("cell")
        }
        "fax number" | "business fax" | "home fax" | "other fax" | "fax" | "télécopie" => {
            Phone("fax")
        }
        "pager number" | "pager" | "récepteur de radiomessagerie" => Phone("pager"),
        "other phone" | "autre téléphone" => Phone("other"),
        "primary phone" | "téléphone principal" => Phone(""),
        _ => return None,
    })
}

/// Format d'après les en-têtes reconnus ; `None` si aucun ne porte
/// d'adresse ou de nom d'un format connu.
fn format(headers: &[String], columns: &[Option<Column>]) -> Option<ContactFormat> {
    let has = |names: &[&str]| headers.iter().any(|h| names.contains(&h.as_str()));
    if columns
        .iter()
        .any(|c| matches!(c, Some(Column::GoogleEmailValue(_))))
    {
        Some(ContactFormat::GoogleCsv)
    } else if has(&["primary email", "adresse électronique principale"]) {
        Some(ContactFormat::ThunderbirdCsv)
    } else if has(&["e-mail address", "adresse de messagerie"]) {
        Some(ContactFormat::OutlookCsv)
    } else {
        None
    }
}

/// Séparateur le plus fréquent de la première ligne (virgule à égalité).
fn delimiter(text: &str) -> u8 {
    let first = text.lines().next().unwrap_or_default();
    let count = |c| first.matches(c).count();
    [(b',', count(',')), (b';', count(';')), (b'\t', count('\t'))]
        .into_iter()
        .fold(
            (b',', 0),
            |best, (d, n)| if n > best.1 { (d, n) } else { best },
        )
        .0
}

/// Valeurs d'une cellule Google (plusieurs séparées par « ::: »).
fn google_values(cell: &str) -> impl Iterator<Item = &str> {
    cell.split(":::").map(str::trim).filter(|v| !v.is_empty())
}

/// Étiquette Google : « * » en tête marque l'adresse principale.
fn google_label(cell: &str) -> (String, bool) {
    let cell = cell.trim();
    match cell.strip_prefix('*') {
        Some(rest) => (rest.trim().to_owned(), true),
        None => (cell.to_owned(), false),
    }
}

fn card(columns: &[Option<Column>], record: &csv::StringRecord) -> ImportedCard {
    if record.iter().any(|f| f.len() > MAX_FIELD_BYTES) {
        return ImportedCard::Unreadable;
    }
    let mut input = ContactInput::default();
    let mut labels: HashMap<(bool, u32), (String, bool)> = HashMap::new();
    for (column, cell) in columns.iter().zip(record.iter()) {
        match column {
            Some(Column::GoogleEmailLabel(n)) => {
                labels.insert((true, *n), google_label(cell));
            }
            Some(Column::GooglePhoneLabel(n)) => {
                labels.insert((false, *n), google_label(cell));
            }
            _ => {}
        }
    }
    let set = |field: &mut String, cell: &str| {
        if field.is_empty() {
            cell.trim().clone_into(field);
        }
    };
    for (column, cell) in columns.iter().zip(record.iter()) {
        let Some(column) = column else {
            continue;
        };
        match *column {
            Column::DisplayName => set(&mut input.display_name, cell),
            Column::GivenName => set(&mut input.given_name, cell),
            Column::FamilyName => set(&mut input.family_name, cell),
            Column::Nickname => set(&mut input.nickname, cell),
            Column::Organization => set(&mut input.organization, cell),
            Column::Notes => set(&mut input.notes, cell),
            Column::Email { preferred } => {
                let address = cell.trim();
                if !address.is_empty() {
                    input.emails.push(EmailInput {
                        address: address.to_owned(),
                        label: String::new(),
                        preferred,
                    });
                }
            }
            Column::Phone(label) => {
                let number = cell.trim();
                if !number.is_empty() {
                    input.phones.push(PhoneInput {
                        number: number.to_owned(),
                        label: label.to_owned(),
                    });
                }
            }
            Column::GoogleEmailValue(n) => {
                let (label, preferred) = labels.get(&(true, n)).cloned().unwrap_or_default();
                for (i, address) in google_values(cell).enumerate() {
                    input.emails.push(EmailInput {
                        address: address.to_owned(),
                        label: label.clone(),
                        preferred: preferred && i == 0,
                    });
                }
            }
            Column::GooglePhoneValue(n) => {
                let (label, _) = labels.get(&(false, n)).cloned().unwrap_or_default();
                for number in google_values(cell) {
                    input.phones.push(PhoneInput {
                        number: number.to_owned(),
                        label: label.clone(),
                    });
                }
            }
            Column::GoogleEmailLabel(_) | Column::GooglePhoneLabel(_) => {}
        }
        if input.emails.len() > MAX_VALUES_PER_CARD || input.phones.len() > MAX_VALUES_PER_CARD {
            return ImportedCard::Unreadable;
        }
    }
    ImportedCard::Contact(input)
}

/// Fiches d'un export CSV ; `UnknownFormat` si l'en-tête n'est pas celui
/// d'un format reconnu. Les lignes vides sont omises ; une ligne mal
/// formée est une fiche illisible.
pub(crate) fn decode(text: &str) -> Result<DecodedContacts, ContactCodecError> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter(text))
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut records = reader.records();
    let headers: Vec<String> = match records.next() {
        Some(Ok(record)) => record.iter().map(|h| h.trim().to_lowercase()).collect(),
        _ => return Err(ContactCodecError::UnknownFormat),
    };
    let columns: Vec<Option<Column>> = headers.iter().map(|h| column(h)).collect();
    let format = format(&headers, &columns).ok_or(ContactCodecError::UnknownFormat)?;
    let mut cards = Vec::new();
    for record in records {
        let card = match record {
            Ok(record) if record.iter().all(|f| f.trim().is_empty()) => continue,
            Ok(record) => card(&columns, &record),
            Err(_) => ImportedCard::Unreadable,
        };
        cards.push(card);
        if cards.len() > MAX_IMPORT_CARDS {
            return Err(ContactCodecError::TooManyCards);
        }
    }
    Ok(DecodedContacts { format, cards })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(text: &str) -> DecodedContacts {
        decode(text).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn unknown_headers_are_refused() {
        assert_eq!(decode(""), Err(ContactCodecError::UnknownFormat));
        assert_eq!(
            decode("a,b,c\n1,2,3\n"),
            Err(ContactCodecError::UnknownFormat)
        );
        // Un nom seul ne suffit pas : il faut une colonne d'adresse.
        assert_eq!(
            decode("First Name,Last Name\nA,B\n"),
            Err(ContactCodecError::UnknownFormat)
        );
    }

    #[test]
    fn tab_separated_and_short_rows_are_read() {
        let d = decoded("First Name\tE-mail Address\tNotes\nAnne\tanne@example.com\n");
        assert_eq!(d.format, ContactFormat::OutlookCsv);
        let [ImportedCard::Contact(c)] = &d.cards[..] else {
            panic!("{:?}", d.cards)
        };
        assert_eq!(c.given_name, "Anne");
        assert_eq!(c.emails[0].address, "anne@example.com");
    }

    #[test]
    fn oversized_fields_or_too_many_values_make_the_row_unreadable() {
        let long = "x".repeat(MAX_FIELD_BYTES + 1);
        let d = decoded(&format!("E-mail Address,Notes\na@example.com,{long}\n"));
        assert_eq!(d.cards, [ImportedCard::Unreadable]);
        let many = vec!["a@example.com"; MAX_VALUES_PER_CARD + 1].join(" ::: ");
        let d = decoded(&format!("E-mail 1 - Value\n\"{many}\"\n"));
        assert_eq!(d.cards, [ImportedCard::Unreadable]);
    }

    #[test]
    fn too_many_rows_refuse_the_file() {
        let rows = "a@example.com\n".repeat(MAX_IMPORT_CARDS);
        assert_eq!(
            decode(&format!("Primary Email\n{rows}")).map(|d| d.cards.len()),
            Ok(MAX_IMPORT_CARDS)
        );
        assert_eq!(
            decode(&format!("Primary Email\n{rows}b@example.com\n")),
            Err(ContactCodecError::TooManyCards)
        );
    }

    #[test]
    fn google_labels_without_values_and_old_type_columns() {
        let d = decoded(
            "Name,E-mail 1 - Type,E-mail 1 - Value,Phone 1 - Type,Phone 1 - Value\nAnne,* Home,anne@example.com,Mobile,06\n",
        );
        let [ImportedCard::Contact(c)] = &d.cards[..] else {
            panic!("{:?}", d.cards)
        };
        assert_eq!(c.display_name, "Anne");
        assert_eq!(
            c.emails,
            [EmailInput {
                address: "anne@example.com".into(),
                label: "Home".into(),
                preferred: true
            }]
        );
        assert_eq!(c.phones[0].label, "Mobile");
    }
}
