//! Lecture des exports CSV de Google Contacts, d'Outlook et de
//! Thunderbird (en-têtes anglais, et français pour Outlook et
//! Thunderbird), par la crate `csv`. Le format est reconnu à l'en-tête,
//! le séparateur (`,`, `;` ou tabulation) à la première ligne. Des
//! étiquettes d'un CSV de Google (colonne « Labels », « Group Membership » dans ses
//! anciens exports), seule celle des favoris est lue.

use std::collections::HashMap;

use norelly_domain::contact_export::{is_favorite_google_label, restore_csv_cell};
use norelly_domain::contact_import::{
    ContactFormat, DecodedContacts, ImportedCard, MAX_IMPORT_CARDS,
};
use norelly_domain::contacts::{
    ContactInput, EmailInput, PhoneInput, PostalAddressInput, WebsiteInput,
};
use norelly_domain::ports::ContactCodecError;

use crate::{MAX_FIELD_BYTES, MAX_VALUES_PER_CARD, birthday_text, street_lines};

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
    /// Google : étiquettes de la fiche, dont celle des favoris.
    GoogleLabels,
    /// Titre ou fonction.
    Title,
    /// Anniversaire ; une date à barres obliques se lit dans cet ordre.
    Birthday(DateOrder),
    /// Élément de l'anniversaire de Thunderbird (année, mois, jour).
    BirthPart(usize),
    /// Élément d'une adresse postale.
    Place(PlaceSlot, PlacePart),
    /// Site web, avec son étiquette.
    Website(&'static str),
    /// Google : étiquette ou valeur du site n° `n`.
    GoogleWebsiteLabel(u32),
    GoogleWebsiteValue(u32),
}

/// Ordre d'une date `a/b/aaaa` : mois d'abord (Outlook en anglais) ou jour
/// d'abord (Outlook en français).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateOrder {
    MonthFirst,
    DayFirst,
}

/// Adresse postale d'une ligne : numérotée (Google) ou d'une étiquette
/// fixe (Outlook, Thunderbird).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaceSlot {
    Google(u32),
    Fixed(&'static str),
}

/// Élément d'une adresse postale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlacePart {
    /// Ligne `n` de la rue (rue, complément, boîte postale…).
    Street(u8),
    Locality,
    Region,
    PostalCode,
    Country,
    /// Google : adresse complète, gardée comme rue si rien d'autre.
    Formatted,
    /// Google : étiquette.
    Label,
}

/// Colonne d'une adresse postale de Google (`Address n - …`).
fn google_place(header: &str) -> Option<Column> {
    let rest = header.strip_prefix("address ")?;
    let (number, suffix) = rest.split_once(" - ")?;
    let n: u32 = number.parse().ok()?;
    let part = match suffix {
        "label" | "type" => PlacePart::Label,
        "street" => PlacePart::Street(0),
        "extended address" => PlacePart::Street(1),
        "po box" => PlacePart::Street(2),
        "city" => PlacePart::Locality,
        "region" => PlacePart::Region,
        "postal code" => PlacePart::PostalCode,
        "country" => PlacePart::Country,
        "formatted" => PlacePart::Formatted,
        _ => return None,
    };
    Some(Column::Place(PlaceSlot::Google(n), part))
}

/// Colonne d'une adresse postale d'Outlook ou de Thunderbird, en anglais
/// et (Outlook) en français.
fn fixed_place(header: &str) -> Option<Column> {
    use PlacePart::{Country, Locality, PostalCode, Region, Street};
    let (label, part) = match header {
        "business street" | "work address" | "rue (bureau)" => ("work", Street(0)),
        "business street 2" | "work address 2" | "rue (bureau) 2" => ("work", Street(1)),
        "business street 3" | "rue (bureau) 3" => ("work", Street(2)),
        "business city" | "work city" | "ville (bureau)" => ("work", Locality),
        "business state" | "work state" | "dép/région (bureau)" => ("work", Region),
        "business postal code" | "work zipcode" | "code postal (bureau)" => ("work", PostalCode),
        "business country/region" | "work country" | "pays/région (bureau)" => ("work", Country),
        "home street" | "home address" | "rue (domicile)" => ("home", Street(0)),
        "home street 2" | "home address 2" | "rue (domicile) 2" => ("home", Street(1)),
        "home street 3" | "rue (domicile) 3" => ("home", Street(2)),
        "home city" | "ville (domicile)" => ("home", Locality),
        "home state" | "dép/région (domicile)" => ("home", Region),
        "home postal code" | "home zipcode" | "code postal (domicile)" => ("home", PostalCode),
        "home country/region" | "home country" | "pays/région (domicile)" => ("home", Country),
        "other street" | "rue (autre)" => ("other", Street(0)),
        "other street 2" | "rue (autre) 2" => ("other", Street(1)),
        "other street 3" | "rue (autre) 3" => ("other", Street(2)),
        "other city" | "ville (autre)" => ("other", Locality),
        "other state" | "dép/région (autre)" => ("other", Region),
        "other postal code" | "code postal (autre)" => ("other", PostalCode),
        "other country/region" | "pays/région (autre)" => ("other", Country),
        _ => return None,
    };
    Some(Column::Place(PlaceSlot::Fixed(label), part))
}

/// Adresse postale en cours de lecture.
#[derive(Debug, Default)]
struct Place {
    streets: Vec<(u8, String)>,
    locality: String,
    region: String,
    postal_code: String,
    country: String,
    formatted: String,
    label: String,
}

impl Place {
    fn set(&mut self, part: PlacePart, cell: &str) {
        let field = match part {
            PlacePart::Street(n) => {
                self.streets.push((n, cell.to_owned()));
                return;
            }
            PlacePart::Locality => &mut self.locality,
            PlacePart::Region => &mut self.region,
            PlacePart::PostalCode => &mut self.postal_code,
            PlacePart::Country => &mut self.country,
            PlacePart::Formatted => &mut self.formatted,
            PlacePart::Label => &mut self.label,
        };
        if field.is_empty() {
            cell.clone_into(field);
        }
    }

    /// Adresse lue ; `None` si la ligne n'en a aucun élément.
    fn finish(mut self) -> Option<PostalAddressInput> {
        self.streets.sort_by_key(|(n, _)| *n);
        let mut street = street_lines(self.streets.iter().map(|(_, s)| s.as_str()));
        let parts = [
            &self.locality,
            &self.region,
            &self.postal_code,
            &self.country,
        ];
        // Adresse complète de Google seule : gardée comme rue.
        if street.is_empty() && parts.iter().all(|p| p.is_empty()) {
            street = street_lines(self.formatted.lines());
        }
        if street.is_empty() && parts.iter().all(|p| p.is_empty()) {
            return None;
        }
        let (label, _) = google_label(&self.label);
        Some(PostalAddressInput {
            street,
            locality: self.locality,
            region: self.region,
            postal_code: self.postal_code,
            country: self.country,
            label,
        })
    }
}

/// Anniversaire tiré de l'année (facultative), du mois et du jour de
/// Thunderbird, dans la forme du domaine ; vide sans mois ni jour, tel
/// quel s'ils ne sont pas des nombres.
fn birth_parts([year, month, day]: [&str; 3]) -> String {
    if month.is_empty() && day.is_empty() {
        return String::new();
    }
    let number = |text: &str| text.trim().parse::<u32>().ok();
    match (number(month), number(day), year.trim()) {
        (Some(month), Some(day), "") => birthday_text(&format!("--{month:02}-{day:02}")),
        (Some(month), Some(day), year) => match number(year) {
            Some(year) => birthday_text(&format!("{year:04}-{month:02}-{day:02}")),
            None => format!("{year}-{month}-{day}"),
        },
        _ => format!("{year}-{month}-{day}"),
    }
}

/// Anniversaire d'une cellule : date `a/b/aaaa` remise dans la forme du
/// domaine selon `order`, toute autre forme lue par [`birthday_text`].
fn birthday_cell(cell: &str, order: DateOrder) -> String {
    let parts: Vec<&str> = cell.trim().split('/').collect();
    let numbers: Option<Vec<u32>> = parts.iter().map(|p| p.trim().parse().ok()).collect();
    match (parts.len(), numbers.as_deref()) {
        (3, Some(&[a, b, year])) if parts[2].trim().len() == 4 => {
            let (month, day) = match order {
                DateOrder::MonthFirst => (a, b),
                DateOrder::DayFirst => (b, a),
            };
            birthday_text(&format!("{year:04}-{month:02}-{day:02}"))
        }
        _ => birthday_text(cell),
    }
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
        GoogleLabels, GooglePhoneLabel, GooglePhoneValue, Nickname, Notes, Organization, Phone,
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
    if let Some(n) = google_slot(header, "website", &["value"]) {
        return Some(Column::GoogleWebsiteValue(n));
    }
    if let Some(n) = google_slot(header, "website", &["label", "type"]) {
        return Some(Column::GoogleWebsiteLabel(n));
    }
    if let Some(place) = google_place(header).or_else(|| fixed_place(header)) {
        return Some(place);
    }
    Some(match header {
        // « Title » seul est la civilité d'Outlook (M., Mme) : ignorée.
        "organization title" | "organization 1 - title" | "job title" | "fonction" => Column::Title,
        "birthday" => Column::Birthday(DateOrder::MonthFirst),
        "anniversaire" => Column::Birthday(DateOrder::DayFirst),
        "birth year" => Column::BirthPart(0),
        "birth month" => Column::BirthPart(1),
        "birth day" => Column::BirthPart(2),
        "web page" | "page web" => Column::Website(""),
        "web page 1" => Column::Website("work"),
        "web page 2" | "personal web page" | "page web personnelle" => Column::Website("home"),
        "display name" | "nom à afficher" | "name" => DisplayName,
        "first name" | "given name" | "prénom" => GivenName,
        "last name" | "family name" | "nom de famille" | "nom" => FamilyName,
        "nickname" | "surnom" => Nickname,
        "organization name" | "organization 1 - name" | "organization" | "company" | "société" => {
            Organization
        }
        "notes" => Notes,
        "labels" | "group membership" => GoogleLabels,
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
    // Cellules sans espaces de bord, apostrophe de neutralisation des
    // formules retirée (règle du domaine).
    let cells: Vec<&str> = record
        .iter()
        .map(|cell| restore_csv_cell(cell.trim()))
        .collect();
    let mut input = ContactInput::default();
    let mut labels: HashMap<(bool, u32), (String, bool)> = HashMap::new();
    let mut site_labels: HashMap<u32, String> = HashMap::new();
    for (column, cell) in columns.iter().zip(cells.iter().copied()) {
        match column {
            Some(Column::GoogleEmailLabel(n)) => {
                labels.insert((true, *n), google_label(cell));
            }
            Some(Column::GooglePhoneLabel(n)) => {
                labels.insert((false, *n), google_label(cell));
            }
            Some(Column::GoogleWebsiteLabel(n)) => {
                site_labels.insert(*n, google_label(cell).0);
            }
            _ => {}
        }
    }
    // Adresses postales dans l'ordre de leur première colonne ; éléments
    // de l'anniversaire de Thunderbird (année, mois, jour).
    let mut places: Vec<(PlaceSlot, Place)> = Vec::new();
    let mut birth: [&str; 3] = ["", "", ""];
    let set = |field: &mut String, cell: &str| {
        if field.is_empty() {
            cell.trim().clone_into(field);
        }
    };
    for (column, cell) in columns.iter().zip(cells.iter().copied()) {
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
            // Une seule étiquette compte, celle des favoris : les autres
            // ne sont pas gardées.
            Column::GoogleLabels => {
                input.favorite |= google_values(cell).any(is_favorite_google_label);
            }
            Column::Title => set(&mut input.title, cell),
            Column::Birthday(order) => {
                if input.birthday.is_empty() && !cell.is_empty() {
                    input.birthday = birthday_cell(cell, order);
                }
            }
            Column::BirthPart(i) => {
                if let Some(part) = birth.get_mut(i) {
                    *part = cell;
                }
            }
            Column::Place(slot, part) => {
                let index = match places.iter().position(|(s, _)| *s == slot) {
                    Some(index) => index,
                    None => {
                        let label = match slot {
                            PlaceSlot::Fixed(label) => label,
                            PlaceSlot::Google(_) => "",
                        };
                        places.push((
                            slot,
                            Place {
                                label: label.to_owned(),
                                ..Place::default()
                            },
                        ));
                        places.len() - 1
                    }
                };
                if let Some((_, place)) = places.get_mut(index) {
                    place.set(part, cell);
                }
            }
            Column::Website(label) => {
                if !cell.is_empty() {
                    input.websites.push(WebsiteInput {
                        url: cell.to_owned(),
                        label: label.to_owned(),
                    });
                }
            }
            Column::GoogleWebsiteValue(n) => {
                let label = site_labels.get(&n).cloned().unwrap_or_default();
                for url in google_values(cell) {
                    input.websites.push(WebsiteInput {
                        url: url.to_owned(),
                        label: label.clone(),
                    });
                }
            }
            Column::GoogleEmailLabel(_)
            | Column::GooglePhoneLabel(_)
            | Column::GoogleWebsiteLabel(_) => {}
        }
        let counts = [
            input.emails.len(),
            input.phones.len(),
            input.websites.len(),
            places.len(),
        ];
        if counts.iter().any(|&n| n > MAX_VALUES_PER_CARD) {
            return ImportedCard::Unreadable;
        }
    }
    input.postal_addresses = places
        .into_iter()
        .filter_map(|(_, place)| place.finish())
        .collect();
    if input.birthday.is_empty() {
        input.birthday = birth_parts(birth);
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
    let mut columns: Vec<Option<Column>> = headers.iter().map(|h| column(h)).collect();
    let format = format(&headers, &columns).ok_or(ContactCodecError::UnknownFormat)?;
    // Les étiquettes ne sont celles de Google que dans un CSV de Google.
    if format != ContactFormat::GoogleCsv {
        for c in &mut columns {
            if *c == Some(Column::GoogleLabels) {
                *c = None;
            }
        }
    }
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

    #[test]
    fn cells_neutralized_against_formulas_are_restored() {
        let d = decoded(
            "Name,E-mail 1 - Label,E-mail 1 - Value,Phone 1 - Label,Phone 1 - Value,Notes\n\
             '=Anne,'\u{2212}x,anne@example.com,Mobile,'+33 6 12,''=garde une apostrophe\n",
        );
        let [ImportedCard::Contact(c)] = &d.cards[..] else {
            panic!("{:?}", d.cards)
        };
        assert_eq!(c.display_name, "=Anne");
        assert_eq!(c.emails[0].label, "\u{2212}x");
        assert_eq!(c.phones[0].number, "+33 6 12");
        assert_eq!(c.notes, "'=garde une apostrophe");
    }

    fn only(text: &str) -> ContactInput {
        match &decoded(text).cards[..] {
            [ImportedCard::Contact(c)] => c.clone(),
            other => panic!("{other:?}"),
        }
    }

    fn places(c: &ContactInput) -> Vec<[&str; 6]> {
        c.postal_addresses
            .iter()
            .map(|a| {
                [
                    a.street.as_str(),
                    a.locality.as_str(),
                    a.region.as_str(),
                    a.postal_code.as_str(),
                    a.country.as_str(),
                    a.label.as_str(),
                ]
            })
            .collect()
    }

    fn sites(c: &ContactInput) -> Vec<(&str, &str)> {
        c.websites
            .iter()
            .map(|w| (w.url.as_str(), w.label.as_str()))
            .collect()
    }

    #[test]
    fn google_details_are_read() {
        let c = only(
            "Name,Organization Title,Birthday,E-mail 1 - Value,\
             Address 1 - Label,Address 1 - Street,Address 1 - Extended Address,Address 1 - PO Box,\
             Address 1 - City,Address 1 - Region,Address 1 - Postal Code,Address 1 - Country,\
             Address 2 - Type,Address 2 - Formatted,\
             Website 1 - Label,Website 1 - Value\n\
             Anne,Cheffe,--04-12,anne@example.com,\
             * Home,1 rue A,Bât. B,BP 3,Lyon,Rhône,69001,France,\
             Work,\"2 rue B\n44000 Nantes\",\
             Blog,https://example.com ::: https://example.org\n",
        );
        assert_eq!(c.title, "Cheffe");
        assert_eq!(c.birthday, "--04-12");
        assert_eq!(
            places(&c),
            [
                [
                    "1 rue A\nBât. B\nBP 3",
                    "Lyon",
                    "Rhône",
                    "69001",
                    "France",
                    "Home"
                ],
                ["2 rue B\n44000 Nantes", "", "", "", "", "Work"],
            ]
        );
        assert_eq!(
            sites(&c),
            [
                ("https://example.com", "Blog"),
                ("https://example.org", "Blog")
            ]
        );
    }

    #[test]
    fn outlook_details_are_read_in_english_and_french() {
        let c = only(
            "First Name,Title,Job Title,E-mail Address,Business Street,Business Street 2,\
             Business City,Business State,Business Postal Code,Business Country/Region,\
             Home Street,Home City,Web Page,Birthday\n\
             Paul,M.,Gérant,paul@example.com,1 rue A,Bât. B,Paris,IDF,75001,France,\
             2 rue B,Lyon,www.example.com,4/12/1985\n",
        );
        assert_eq!(c.title, "Gérant");
        assert_eq!(c.birthday, "1985-04-12");
        assert_eq!(
            places(&c),
            [
                ["1 rue A\nBât. B", "Paris", "IDF", "75001", "France", "work"],
                ["2 rue B", "Lyon", "", "", "", "home"],
            ]
        );
        assert_eq!(sites(&c), [("www.example.com", "")]);

        let c = only(
            "Prénom;Fonction;Adresse de messagerie;Rue (domicile);Ville (domicile);\
             Code postal (domicile);Pays/Région (domicile);Page Web;Anniversaire\n\
             François;Boulanger;f@example.fr;3 rue C;Brest;29200;France;example.fr;12/04/1985\n",
        );
        assert_eq!(c.title, "Boulanger");
        assert_eq!(c.birthday, "1985-04-12");
        assert_eq!(
            places(&c),
            [["3 rue C", "Brest", "", "29200", "France", "home"]]
        );
        assert_eq!(sites(&c), [("example.fr", "")]);
        // Date vide d'Outlook : aucun anniversaire.
        let c = only("E-mail Address,Birthday\na@example.com,0/0/00\n");
        assert_eq!(c.birthday, "0/0/00");
    }

    #[test]
    fn thunderbird_details_are_read() {
        let c = only(
            "Primary Email,Job Title,Home Address,Home Address 2,Home City,Home ZipCode,\
             Work City,Web Page 1,Web Page 2,Birth Year,Birth Month,Birth Day\n\
             a@example.com,Chef,1 rue A,Bât. B,Lyon,69001,Paris,https://work.example,\
             https://home.example,,4,12\n",
        );
        assert_eq!(c.title, "Chef");
        assert_eq!(c.birthday, "--04-12");
        assert_eq!(
            places(&c),
            [
                ["1 rue A\nBât. B", "Lyon", "", "69001", "", "home"],
                ["", "Paris", "", "", "", "work"],
            ]
        );
        assert_eq!(
            sites(&c),
            [
                ("https://work.example", "work"),
                ("https://home.example", "home")
            ]
        );
        let c = only("Primary Email,Birth Year,Birth Month,Birth Day\na@example.com,1985,4,12\n");
        assert_eq!(c.birthday, "1985-04-12");
    }

    #[test]
    fn the_starred_label_marks_a_favorite() {
        for (header, labels) in [
            ("Labels", "* myContacts ::: * starred"),
            ("Labels", "* starred"),
            ("labels", "Famille ::: *  Starred ::: Amis"),
            // Ancien export de Google.
            ("Group Membership", "* myContacts ::: * Starred"),
        ] {
            let d = decoded(&format!(
                "{header},Name,E-mail 1 - Label,E-mail 1 - Value,Notes\n\
                 \"{labels}\",Anne,Home,anne@example.com,\n"
            ));
            let [ImportedCard::Contact(c)] = &d.cards[..] else {
                panic!("{:?}", d.cards)
            };
            assert!(c.favorite, "{header} : {labels}");
            // Le favori ne laisse aucune autre trace.
            assert_eq!(c.display_name, "Anne");
            assert_eq!(c.emails[0].label, "Home");
            assert_eq!(c.notes, "");
        }
    }

    #[test]
    fn labels_of_other_programs_never_mark_a_favorite() {
        // Une colonne « Labels » d'Outlook ou de Thunderbird est une
        // donnée de l'utilisateur, pas le groupe système de Google.
        for header in ["E-mail Address", "Primary Email"] {
            let d = decoded(&format!(
                "Labels,{header}\n\"* starred\",anne@example.com\n"
            ));
            assert_ne!(d.format, ContactFormat::GoogleCsv);
            let [ImportedCard::Contact(c)] = &d.cards[..] else {
                panic!("{:?}", d.cards)
            };
            assert!(!c.favorite, "{header}");
        }
    }

    #[test]
    fn non_favorite_google_labels_leave_the_card_unchanged() {
        // « Labels » en premier : seule l'étiquette des favoris y est lue ;
        // toute autre valeur ne laisse aucune trace dans la fiche.
        let without = decoded("Name,E-mail 1 - Value\nAnne,anne@example.com\n");
        for labels in [
            "* myContacts",
            "* myContacts ::: Famille",
            "starred ::: Starred",
            "* myContacts * starred",
            "=cmd|' /C calc'!A0",
        ] {
            let with = decoded(&format!(
                "Labels,Name,E-mail 1 - Value\n\"{labels}\",Anne,anne@example.com\n"
            ));
            assert_eq!(with, without, "{labels}");
        }
    }
}
