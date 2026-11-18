//! Lecture LDIF (RFC 2849), écrite pour le projet, telle qu'exportée par
//! Thunderbird : enregistrements séparés par une ligne vide, lignes
//! repliées (continuation par une espace), commentaires `#`, valeurs en
//! base64 (`attr:: …`). Les valeurs désignées par une URL (`attr:< …`)
//! sont ignorées : rien n'est lu hors du fichier. Un groupe
//! (`groupOfNames`, `groupOfUniqueNames`) est une liste : nom (`cn`),
//! description, membres (`member`, `uniqueMember`) désignés par le nom
//! distinctif (`dn`) d'un autre enregistrement du fichier, rendu par sa
//! première adresse, ou à défaut par l'attribut `mail` de ce nom ; copie
//! cachée (`x-norelly-bcc: TRUE`, propre à Norelly).

use std::collections::HashMap;

use norelly_domain::contact_import::{ImportedCard, ImportedList, MAX_IMPORT_CARDS};
use norelly_domain::contacts::{
    ContactInput, EmailInput, PhoneInput, PostalAddressInput, WebsiteInput,
};
use norelly_domain::ports::ContactCodecError;

use crate::{
    MAX_FIELD_BYTES, MAX_LIST_MEMBERS_READ, MAX_VALUES_PER_CARD, birthday_of_parts, street_lines,
    text,
};

/// Le texte commence-t-il comme un fichier LDIF (`dn:` ou `version:`) ?
pub(crate) fn looks_like(text: &str) -> bool {
    text.lines()
        .map(str::trim_start)
        .find(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .is_some_and(|line| {
            let lower = line.get(..8).unwrap_or(line).to_ascii_lowercase();
            lower.starts_with("dn:") || lower.starts_with("version:")
        })
}

/// Valeur d'un attribut : texte, ou illisible (base64 ou UTF-8 invalide,
/// valeur démesurée).
type Value = Option<String>;

/// Enregistrements : attributs (nom en minuscules, options retirées) et
/// leurs valeurs, dans l'ordre. Les lignes sans `:` sont ignorées.
fn records(text: &str) -> Vec<Vec<(String, Value)>> {
    // Lignes logiques, `None` pour une fin d'enregistrement.
    let mut lines: Vec<Option<String>> = Vec::new();
    let mut in_comment = false;
    for physical in text.split('\n') {
        let physical = physical.strip_suffix('\r').unwrap_or(physical);
        if let Some(rest) = physical.strip_prefix(' ') {
            if !in_comment && let Some(Some(last)) = lines.last_mut() {
                last.push_str(rest);
            }
            continue;
        }
        in_comment = physical.starts_with('#');
        if in_comment {
            continue;
        }
        if physical.trim().is_empty() {
            lines.push(None);
        } else {
            lines.push(Some(physical.to_owned()));
        }
    }

    let mut records = Vec::new();
    let mut current: Vec<(String, Value)> = Vec::new();
    for line in lines {
        let Some(line) = line else {
            if !current.is_empty() {
                records.push(std::mem::take(&mut current));
            }
            continue;
        };
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let name = name
            .split(';')
            .next()
            .unwrap_or(name)
            .trim()
            .to_ascii_lowercase();
        let value = if let Some(encoded) = rest.strip_prefix(':') {
            text::base64(encoded).and_then(|bytes| String::from_utf8(bytes).ok())
        } else if rest.starts_with('<') {
            continue;
        } else {
            Some(rest.trim_start().to_owned())
        };
        let value = value.filter(|v| v.len() <= MAX_FIELD_BYTES);
        current.push((name, value));
    }
    if !current.is_empty() {
        records.push(current);
    }
    records
}

/// Étiquette d'un attribut de téléphone (comme les types vCard).
fn phone_label(attribute: &str) -> Option<&'static str> {
    Some(match attribute {
        "telephonenumber" => "work",
        "homephone" => "home",
        "mobile" | "cellphone" => "cell",
        "facsimiletelephonenumber" | "fax" => "fax",
        "pager" => "pager",
        _ => return None,
    })
}

/// Attribut d'une adresse postale de Thunderbird : emplacement (0 :
/// domicile, 1 : travail) et élément (rue, suite de la rue, ville, région,
/// code postal, pays).
fn place_attribute(attribute: &str) -> Option<(usize, usize)> {
    Some(match attribute {
        "mozillahomestreet" => (0, 0),
        "mozillahomestreet2" => (0, 1),
        "mozillahomelocalityname" => (0, 2),
        "mozillahomestate" => (0, 3),
        "mozillahomepostalcode" => (0, 4),
        "mozillahomecountryname" => (0, 5),
        "street" | "streetaddress" => (1, 0),
        "mozillaworkstreet2" => (1, 1),
        "l" | "locality" => (1, 2),
        "st" => (1, 3),
        "postalcode" => (1, 4),
        "c" | "countryname" => (1, 5),
        _ => return None,
    })
}

/// Étiquette d'un emplacement d'adresse ou de site.
const PLACE_LABELS: [&str; 2] = ["home", "work"];

/// Emplacement d'un attribut de site web (0 : domicile, 1 : travail).
fn website_attribute(attribute: &str) -> Option<usize> {
    match attribute {
        "mozillahomeurl" | "homeurl" => Some(0),
        "mozillaworkurl" | "workurl" => Some(1),
        _ => None,
    }
}

/// Groupe lu : nom, description, copie cachée et membres tels que lus
/// (noms distinctifs) ; illisible si une valeur lue l'est, ou au-delà de
/// [`MAX_LIST_MEMBERS_READ`] membres.
fn group(record: &[(String, Value)]) -> Read {
    let mut list = ImportedList::default();
    let mut members = Vec::new();
    let mut notes = Vec::new();
    for (name, value) in record {
        let known = matches!(
            name.as_str(),
            "cn" | "description" | "member" | "uniquemember" | "x-norelly-bcc"
        );
        if !known {
            continue;
        }
        let Some(value) = value else {
            return Read::Ready(ImportedCard::Unreadable);
        };
        match name.as_str() {
            "cn" if list.name.is_empty() => value.clone_into(&mut list.name),
            "description" => notes.push(value.clone()),
            "x-norelly-bcc" => list.bcc = value.trim().eq_ignore_ascii_case("true"),
            "member" | "uniquemember" => {
                if members.len() == MAX_LIST_MEMBERS_READ {
                    return Read::Ready(ImportedCard::Unreadable);
                }
                members.push(value.trim().to_owned());
            }
            _ => {}
        }
    }
    list.description = notes.join("\n");
    Read::List(list, members)
}

/// Enregistrement lu, avant que les membres des groupes soient retrouvés.
enum Read {
    Ready(ImportedCard),
    /// Contact, son nom distinctif et sa première adresse.
    Contact(ContactInput, Option<(String, String)>),
    /// Groupe et ses membres (noms distinctifs).
    List(ImportedList, Vec<String>),
}

/// Nom distinctif comparable : sans espaces de bord, en minuscules.
fn dn_key(dn: &str) -> String {
    dn.trim().to_lowercase()
}

/// Valeur de l'attribut `mail` d'un nom distinctif (RFC 4514 : RDN
/// séparés par `,` ou `+` non échappés, `\c` et `\HH` décodés) ; `None`
/// s'il n'en a pas ou s'il est illisible.
fn dn_mail(dn: &str) -> Option<String> {
    let mut parts: Vec<Vec<u8>> = vec![Vec::new()];
    let bytes = dn.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let current = parts.last_mut()?;
        match b {
            b'\\' => {
                let hex = dn
                    .get(i + 1..i + 3)
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                if let Some(byte) = hex {
                    current.push(byte);
                    i += 3;
                    continue;
                }
                current.push(*bytes.get(i + 1)?);
                i += 2;
                continue;
            }
            b',' | b'+' => parts.push(Vec::new()),
            _ => current.push(b),
        }
        i += 1;
    }
    parts.into_iter().find_map(|part| {
        let text = String::from_utf8(part).ok()?;
        let (attribute, value) = text.split_once('=')?;
        attribute
            .trim()
            .eq_ignore_ascii_case("mail")
            .then(|| value.trim().to_owned())
            .filter(|v| !v.is_empty())
    })
}

/// Fiches lues, membres des groupes retrouvés : par le nom distinctif
/// d'un enregistrement du fichier (sa première adresse), sinon par
/// l'attribut `mail` du nom ; sinon comptés introuvables.
fn resolve(read: Vec<Read>) -> Vec<ImportedCard> {
    let mut dns: HashMap<String, String> = HashMap::new();
    for card in &read {
        if let Read::Contact(_, Some((dn, address))) = card {
            dns.entry(dn_key(dn)).or_insert_with(|| address.clone());
        }
    }
    read.into_iter()
        .map(|card| match card {
            Read::Ready(card) => card,
            Read::Contact(input, _) => ImportedCard::Contact(input),
            Read::List(mut list, members) => {
                for member in members {
                    match dns
                        .get(&dn_key(&member))
                        .cloned()
                        .or_else(|| dn_mail(&member))
                    {
                        Some(address) => list.members.push(address),
                        None => list.unresolved += 1,
                    }
                }
                ImportedCard::List(list)
            }
        })
        .collect()
}

fn card(record: &[(String, Value)]) -> Option<Read> {
    let is_group = record.iter().any(|(name, value)| {
        name == "objectclass"
            && value.as_deref().is_some_and(|v| {
                let v = v.trim().to_ascii_lowercase();
                v == "groupofnames" || v == "groupofuniquenames"
            })
    });
    if is_group {
        return Some(group(record));
    }
    // Enregistrement d'en-tête (`version: 1`) ou sans nom distinctif.
    if !record.iter().any(|(name, _)| name == "dn") {
        return None;
    }
    let mut input = ContactInput::default();
    let mut notes = Vec::new();
    let mut primary_seen = false;
    // Adresses postales (domicile, travail) dans l'ordre de leur premier
    // attribut, et éléments de l'anniversaire (année, mois, jour).
    let mut places: Vec<(usize, [String; 6])> = Vec::new();
    let mut birth: [String; 3] = Default::default();
    for (name, value) in record {
        let known = matches!(
            name.as_str(),
            "cn" | "givenname"
                | "sn"
                | "surname"
                | "mozillanickname"
                | "xmozillanickname"
                | "o"
                | "description"
                | "mail"
                | "mozillasecondemail"
                | "xmozillasecondemail"
                | "title"
                | "birthyear"
                | "birthmonth"
                | "birthday"
        ) || phone_label(name).is_some()
            || place_attribute(name).is_some()
            || website_attribute(name).is_some();
        if !known {
            continue;
        }
        let Some(value) = value else {
            return Some(Read::Ready(ImportedCard::Unreadable));
        };
        let set = |field: &mut String| {
            if field.is_empty() {
                value.clone_into(field);
            }
        };
        match name.as_str() {
            "cn" => set(&mut input.display_name),
            "givenname" => set(&mut input.given_name),
            "sn" | "surname" => set(&mut input.family_name),
            "mozillanickname" | "xmozillanickname" => set(&mut input.nickname),
            "o" => set(&mut input.organization),
            "title" => set(&mut input.title),
            "birthyear" => set(&mut birth[0]),
            "birthmonth" => set(&mut birth[1]),
            "birthday" => set(&mut birth[2]),
            other if place_attribute(other).is_some() => {
                if let Some((slot, part)) = place_attribute(other) {
                    let index = match places.iter().position(|(s, _)| *s == slot) {
                        Some(index) => index,
                        None => {
                            places.push((slot, Default::default()));
                            places.len() - 1
                        }
                    };
                    if let Some(field) = places.get_mut(index).and_then(|(_, p)| p.get_mut(part)) {
                        set(field);
                    }
                }
            }
            other if website_attribute(other).is_some() => {
                let url = value.trim();
                if let (Some(slot), false) = (website_attribute(other), url.is_empty()) {
                    input.websites.push(WebsiteInput {
                        url: url.to_owned(),
                        label: PLACE_LABELS[slot].to_owned(),
                    });
                }
            }
            "description" => notes.push(value.clone()),
            "mail" | "mozillasecondemail" | "xmozillasecondemail" => {
                let address = value.trim();
                if !address.is_empty() {
                    let preferred = name == "mail" && !primary_seen;
                    primary_seen |= preferred;
                    input.emails.push(EmailInput {
                        address: address.to_owned(),
                        label: String::new(),
                        preferred,
                    });
                }
            }
            phone => {
                let number = value.trim();
                if !number.is_empty() {
                    input.phones.push(PhoneInput {
                        number: number.to_owned(),
                        label: phone_label(phone).unwrap_or_default().to_owned(),
                    });
                }
            }
        }
        if input.emails.len() > MAX_VALUES_PER_CARD
            || input.phones.len() > MAX_VALUES_PER_CARD
            || input.websites.len() > MAX_VALUES_PER_CARD
        {
            return Some(Read::Ready(ImportedCard::Unreadable));
        }
    }
    input.notes = notes.join("\n");
    input.postal_addresses = places
        .into_iter()
        .map(
            |(slot, [street, street2, locality, region, postal_code, country])| {
                PostalAddressInput {
                    street: street_lines([street.as_str(), street2.as_str()]),
                    locality: locality.trim().to_owned(),
                    region: region.trim().to_owned(),
                    postal_code: postal_code.trim().to_owned(),
                    country: country.trim().to_owned(),
                    label: PLACE_LABELS[slot].to_owned(),
                }
            },
        )
        .collect();
    input.birthday = birthday_of_parts(&birth[0], &birth[1], &birth[2]);
    let dn = record
        .iter()
        .find(|(name, _)| name == "dn")
        .and_then(|(_, dn)| dn.clone());
    let id = dn.and_then(|dn| input.emails.first().map(|e| (dn, e.address.clone())));
    Some(Read::Contact(input, id))
}

/// Fiches d'un fichier LDIF, dans son ordre.
pub(crate) fn decode(text: &str) -> Result<Vec<ImportedCard>, ContactCodecError> {
    let mut cards = Vec::new();
    for record in records(text) {
        if let Some(card) = card(&record) {
            cards.push(card);
            if cards.len() > MAX_IMPORT_CARDS {
                return Err(ContactCodecError::TooManyCards);
            }
        }
    }
    Ok(resolve(cards))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cards(text: &str) -> Vec<ImportedCard> {
        decode(text).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn looks_like_needs_dn_or_version_first() {
        assert!(looks_like("# export\n\nversion: 1\ndn: cn=a"));
        assert!(looks_like("DN: cn=a"));
        assert!(!looks_like("BEGIN:VCARD"));
        assert!(!looks_like("First Name,dn:"));
    }

    #[test]
    fn version_record_urls_and_options_are_handled() {
        let c = cards(
            "version: 1\n\ndn: cn=A\ncn;lang-fr: Anne\njpegPhoto:< file:///etc/passwd\nmail: a@example.com\n",
        );
        assert_eq!(c.len(), 1);
        let ImportedCard::Contact(a) = &c[0] else {
            panic!("{c:?}")
        };
        assert_eq!(a.display_name, "Anne");
        assert_eq!(a.emails.len(), 1);
        assert!(a.emails[0].preferred);
    }

    #[test]
    fn bad_base64_or_utf8_makes_the_card_unreadable() {
        for value in [
            "cn:: ***",
            "cn:: /w==",
            &format!("cn: {}", "x".repeat(MAX_FIELD_BYTES + 1)),
        ] {
            assert_eq!(
                cards(&format!("dn: cn=x\n{value}\n")),
                [ImportedCard::Unreadable],
                "{}",
                &value[..value.len().min(20)]
            );
        }
        // Un attribut ignoré illisible ne gêne pas.
        assert!(matches!(
            cards("dn: cn=x\njpegPhoto:: ***\nmail: a@example.com\n")[..],
            [ImportedCard::Contact(_)]
        ));
    }

    #[test]
    fn thunderbird_details_are_read() {
        let c = cards(
            "dn: cn=x\nmail: a@example.com\ntitle: Chef\nbirthyear: 1985\nbirthmonth: 4\n\
             birthday: 12\nstreet: 1 rue A\nmozillaWorkStreet2: Bat. B\nl: Paris\nst: IDF\n\
             postalCode: 75001\nc: France\nmozillaHomeStreet: 2 rue B\n\
             mozillaHomeLocalityName: Lyon\nmozillaHomePostalCode: 69001\n\
             mozillaWorkUrl: https://work.example\nmozillaHomeUrl: https://home.example\n",
        );
        let [ImportedCard::Contact(a)] = &c[..] else {
            panic!("{c:?}")
        };
        assert_eq!(a.title, "Chef");
        assert_eq!(a.birthday, "1985-04-12");
        let places: Vec<_> = a
            .postal_addresses
            .iter()
            .map(|p| {
                (
                    p.street.as_str(),
                    p.locality.as_str(),
                    p.region.as_str(),
                    p.postal_code.as_str(),
                    p.country.as_str(),
                    p.label.as_str(),
                )
            })
            .collect();
        assert_eq!(
            places,
            [
                ("1 rue A\nBat. B", "Paris", "IDF", "75001", "France", "work"),
                ("2 rue B", "Lyon", "", "69001", "", "home"),
            ]
        );
        let sites: Vec<_> = a
            .websites
            .iter()
            .map(|w| (w.url.as_str(), w.label.as_str()))
            .collect();
        assert_eq!(
            sites,
            [
                ("https://work.example", "work"),
                ("https://home.example", "home")
            ]
        );
        let c = cards("dn: cn=x\nbirthmonth: 02\nbirthday: 29\n");
        let [ImportedCard::Contact(a)] = &c[..] else {
            panic!("{c:?}")
        };
        assert_eq!(a.birthday, "--02-29");
        assert!(a.postal_addresses.is_empty());
    }

    #[test]
    fn comments_and_their_continuations_are_ignored() {
        let c = cards("dn: cn=x\n# commentaire\n  replié\nmail: a@example.com\n");
        let ImportedCard::Contact(a) = &c[0] else {
            panic!("{c:?}")
        };
        assert_eq!(a.emails[0].address, "a@example.com");
    }

    #[test]
    fn too_many_values_or_records_are_refused() {
        let mails: String = (0..=MAX_VALUES_PER_CARD)
            .map(|i| format!("mail: a{i}@example.com\n"))
            .collect();
        assert_eq!(
            cards(&format!("dn: cn=x\n{mails}")),
            [ImportedCard::Unreadable]
        );
        let record = "dn: cn=x\n\n";
        assert_eq!(
            decode(&record.repeat(MAX_IMPORT_CARDS)).map(|c| c.len()),
            Ok(MAX_IMPORT_CARDS)
        );
        assert_eq!(
            decode(&record.repeat(MAX_IMPORT_CARDS + 1)),
            Err(ContactCodecError::TooManyCards)
        );
    }

    #[test]
    fn a_group_names_its_members_by_their_dn() {
        let c = cards(
            "dn: cn=Club\\, lecture\nobjectclass: groupOfNames\ncn: Club, lecture\n\
             description: Le jeudi\nmember: cn=Paul,mail=paul@example.com\n\
             member: CN=Anne Martin,MAIL=anne@example.org\nmember:: Y249SW5jb25udQ==\n\
             uniquemember: mail=o\\27brien@example.org\n\
             member: cn=Sans\\, mail\\=x@example.com\nx-norelly-bcc: TRUE\n\n\
             dn: cn=Anne Martin,mail=anne@example.org\nobjectclass: person\nmail: anne@example.org\n\
             mail: anne@work.example\n\n\
             dn: cn=Paul,mail=paul@example.com\nmail: paul.autre@example.com\n",
        );
        assert_eq!(
            c[0],
            ImportedCard::List(ImportedList {
                name: "Club, lecture".into(),
                description: "Le jeudi".into(),
                // Fiche du fichier désignée par son nom distinctif, sinon
                // l'attribut `mail` du nom.
                members: vec![
                    "paul.autre@example.com".into(),
                    "anne@example.org".into(),
                    "o'brien@example.org".into(),
                ],
                unresolved: 2,
                bcc: true,
            })
        );
    }

    #[test]
    fn a_group_with_too_many_members_is_unreadable() {
        let members: String = (0..=MAX_LIST_MEMBERS_READ)
            .map(|i| format!("member: mail=m{i}@example.com\n"))
            .collect();
        assert_eq!(
            cards(&format!(
                "dn: cn=L\nobjectclass: groupOfNames\ncn: L\n{members}"
            )),
            [ImportedCard::Unreadable]
        );
    }
}
