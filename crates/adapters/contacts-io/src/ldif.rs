//! Lecture LDIF (RFC 2849), écrite pour le projet, telle qu'exportée par
//! Thunderbird : enregistrements séparés par une ligne vide, lignes
//! repliées (continuation par une espace), commentaires `#`, valeurs en
//! base64 (`attr:: …`). Les valeurs désignées par une URL (`attr:< …`)
//! sont ignorées : rien n'est lu hors du fichier.

use norelly_domain::contact_import::{ImportedCard, MAX_IMPORT_CARDS};
use norelly_domain::contacts::{ContactInput, EmailInput, PhoneInput};
use norelly_domain::ports::ContactCodecError;

use crate::{MAX_FIELD_BYTES, MAX_VALUES_PER_CARD, text};

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

fn card(record: &[(String, Value)]) -> Option<ImportedCard> {
    let is_group = record.iter().any(|(name, value)| {
        name == "objectclass"
            && value.as_deref().is_some_and(|v| {
                let v = v.trim().to_ascii_lowercase();
                v == "groupofnames" || v == "groupofuniquenames"
            })
    });
    if is_group {
        return Some(ImportedCard::List);
    }
    // Enregistrement d'en-tête (`version: 1`) ou sans nom distinctif.
    if !record.iter().any(|(name, _)| name == "dn") {
        return None;
    }
    let mut input = ContactInput::default();
    let mut notes = Vec::new();
    let mut primary_seen = false;
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
        ) || phone_label(name).is_some();
        if !known {
            continue;
        }
        let Some(value) = value else {
            return Some(ImportedCard::Unreadable);
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
        if input.emails.len() > MAX_VALUES_PER_CARD || input.phones.len() > MAX_VALUES_PER_CARD {
            return Some(ImportedCard::Unreadable);
        }
    }
    input.notes = notes.join("\n");
    Some(ImportedCard::Contact(input))
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
    Ok(cards)
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
}
