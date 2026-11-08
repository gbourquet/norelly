//! Lecture vCard 2.1, 3.0 et 4.0 (RFC 2426, RFC 6350), écrite pour le
//! projet : dépliage des lignes, sauts souples quoted-printable,
//! paramètres (`TYPE`, `PREF`, `CHARSET`, `ENCODING`), échappements,
//! plusieurs cartes par fichier. Seuls `FN`, `N`, `NICKNAME`, `ORG`,
//! `NOTE`, `EMAIL`, `TEL` et la nature de la carte (`KIND`) sont lus ; le
//! reste (photos comprises) est ignoré sans être décodé.

use norelly_domain::contact_import::{ImportedCard, MAX_IMPORT_CARDS};
use norelly_domain::contacts::{ContactInput, EmailInput, PhoneInput};
use norelly_domain::ports::ContactCodecError;

use crate::{MAX_FIELD_BYTES, MAX_VALUES_PER_CARD, text};

/// Le texte commence-t-il par une carte vCard ?
pub(crate) fn looks_like(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .and_then(|line| line.get(..11))
        .is_some_and(|start| start.eq_ignore_ascii_case("BEGIN:VCARD"))
}

/// Partie d'une ligne avant le premier `:` hors guillemets (nom et
/// paramètres), et sa valeur.
fn split_line(line: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => return Some((&line[..i], &line[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Ligne logique encodée en quoted-printable (saut souple possible).
fn is_quoted_printable(line: &str) -> bool {
    split_line(line).is_some_and(|(head, _)| head.to_ascii_uppercase().contains("QUOTED-PRINTABLE"))
}

/// Lignes logiques : une ligne qui commence par une espace ou une
/// tabulation continue la précédente (RFC 6350 § 3.2) ; une valeur
/// quoted-printable terminée par `=` continue sur la ligne suivante
/// (vCard 2.1), saut gardé pour le décodage. L'encodage d'une ligne
/// logique est lu une fois, sur sa première ligne : le dépliage reste
/// linéaire quelle que soit la longueur des lignes repliées.
fn logical_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    // La ligne logique en cours est-elle en quoted-printable ?
    let mut quoted_printable = false;
    for physical in text.split('\n') {
        let physical = physical.strip_suffix('\r').unwrap_or(physical);
        if let Some(last) = lines.last_mut() {
            if quoted_printable && last.ends_with('=') {
                last.push('\n');
                last.push_str(physical);
                continue;
            }
            if let Some(rest) = physical
                .strip_prefix(' ')
                .or_else(|| physical.strip_prefix('\t'))
            {
                last.push_str(rest);
                continue;
            }
        }
        quoted_printable = is_quoted_printable(physical);
        lines.push(physical.to_owned());
    }
    lines
}

/// Propriété d'une ligne : nom (groupe retiré, en majuscules), paramètres
/// (nom en majuscules, valeur), valeur brute.
#[derive(Debug)]
struct Property<'a> {
    name: String,
    params: Vec<(String, String)>,
    value: &'a str,
}

/// Morceaux de `text` séparés par `separator` hors guillemets.
fn split_unquoted(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c == separator && !quoted {
            parts.push(&text[start..i]);
            start = i + c.len_utf8();
        }
    }
    parts.push(&text[start..]);
    parts
}

fn property(line: &str) -> Option<Property<'_>> {
    let (head, value) = split_line(line)?;
    let mut parts = split_unquoted(head, ';').into_iter();
    let name = parts.next()?;
    let name = name.rsplit('.').next().unwrap_or(name).to_ascii_uppercase();
    let mut params = Vec::new();
    for part in parts {
        match part.split_once('=') {
            Some((key, values)) => {
                let key = key.trim().to_ascii_uppercase();
                // `TYPE="voice,cell"` (4.0) comme `TYPE=voice,cell`.
                for v in values.trim().trim_matches('"').split(',') {
                    params.push((key.clone(), v.trim().to_owned()));
                }
            }
            // vCard 2.1 : paramètres sans nom (`WORK`, `PREF`,
            // `QUOTED-PRINTABLE`).
            None => {
                let bare = part.trim().to_ascii_uppercase();
                let key = match bare.as_str() {
                    "QUOTED-PRINTABLE" | "BASE64" | "B" | "8BIT" | "7BIT" => "ENCODING",
                    _ => "TYPE",
                };
                params.push((key.to_owned(), bare));
            }
        }
    }
    Some(Property {
        name,
        params,
        value,
    })
}

impl Property<'_> {
    fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn types(&self) -> impl Iterator<Item = &str> {
        self.params
            .iter()
            .filter(|(k, _)| k == "TYPE")
            .map(|(_, v)| v.as_str())
    }

    /// Adresse ou numéro préféré (`TYPE=PREF`, ou `PREF=n` en 4.0).
    fn preferred(&self) -> bool {
        self.param("PREF").is_some() || self.types().any(|t| t.eq_ignore_ascii_case("pref"))
    }

    /// Étiquette : le premier type qui n'est ni technique ni `PREF`, en
    /// minuscules (`work`, `home`, `cell`).
    fn label(&self) -> String {
        self.types()
            .map(str::to_ascii_lowercase)
            .find(|t| {
                !matches!(t.as_str(), "internet" | "pref" | "x400" | "voice" | "text")
                    && !t.starts_with("x-")
            })
            .unwrap_or_default()
    }

    /// Valeur décodée (quoted-printable ou base64, jeu de caractères
    /// déclaré), échappements vCard encore en place. `None` si elle est
    /// illisible ou démesurée.
    fn text(&self) -> Option<String> {
        if self.value.len() > MAX_FIELD_BYTES {
            return None;
        }
        let charset = self.param("CHARSET");
        match self
            .param("ENCODING")
            .map(str::to_ascii_uppercase)
            .as_deref()
        {
            Some("QUOTED-PRINTABLE") => {
                text::charset_text(&text::quoted_printable(self.value)?, charset)
            }
            Some("B" | "BASE64") => text::charset_text(&text::base64(self.value)?, charset),
            _ => Some(self.value.to_owned()),
        }
    }
}

/// Composants d'une valeur structurée séparés par `separator` non
/// échappé (aucun : valeur simple), échappements résolus (`\n`, `\,`,
/// `\;`, `\:`, `\\`) ; un autre `\` est gardé tel quel.
fn components(value: &str, separator: Option<char>) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if Some(c) == separator {
            parts.push(String::new());
            continue;
        }
        let Some(current) = parts.last_mut() else {
            continue;
        };
        if c != '\\' {
            current.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => current.push('\n'),
            Some(escaped @ (',' | ';' | '\\' | ':')) => current.push(escaped),
            Some(other) => {
                current.push('\\');
                current.push(other);
            }
            None => current.push('\\'),
        }
    }
    parts
}

/// Valeur simple, échappements résolus.
fn unescape(value: &str) -> String {
    components(value, None).concat()
}

/// Carte en cours de lecture.
#[derive(Debug, Default)]
struct Card {
    input: ContactInput,
    notes: Vec<String>,
    list: bool,
    unreadable: bool,
}

impl Card {
    fn read(&mut self, p: &Property<'_>) {
        let kept = matches!(
            p.name.as_str(),
            "FN" | "N" | "NICKNAME" | "ORG" | "NOTE" | "EMAIL" | "TEL"
        );
        if p.name == "KIND" || p.name == "X-ADDRESSBOOKSERVER-KIND" {
            self.list |= p.value.trim().eq_ignore_ascii_case("group");
            return;
        }
        if !kept {
            return;
        }
        let Some(value) = p.text() else {
            self.unreadable = true;
            return;
        };
        let first = |sep| {
            components(&value, Some(sep))
                .into_iter()
                .next()
                .unwrap_or_default()
        };
        let input = &mut self.input;
        match p.name.as_str() {
            "FN" if input.display_name.is_empty() => input.display_name = unescape(&value),
            "N" if input.family_name.is_empty() && input.given_name.is_empty() => {
                let mut parts = components(&value, Some(';')).into_iter();
                input.family_name = parts.next().unwrap_or_default();
                input.given_name = parts.next().unwrap_or_default();
            }
            "NICKNAME" if input.nickname.is_empty() => input.nickname = first(','),
            "ORG" if input.organization.is_empty() => input.organization = first(';'),
            "NOTE" => self.notes.push(unescape(&value)),
            "EMAIL" => {
                let address = unescape(&value).trim().to_owned();
                if !address.is_empty() {
                    input.emails.push(EmailInput {
                        address,
                        label: p.label(),
                        preferred: p.preferred(),
                    });
                }
            }
            "TEL" => {
                let raw = unescape(&value);
                let raw = raw.trim();
                let number = raw
                    .get(..4)
                    .filter(|scheme| scheme.eq_ignore_ascii_case("tel:"))
                    .map_or(raw, |_| &raw[4..]);
                if !number.is_empty() {
                    input.phones.push(PhoneInput {
                        number: number.to_owned(),
                        label: p.label(),
                    });
                }
            }
            _ => {}
        }
        if input.emails.len() > MAX_VALUES_PER_CARD || input.phones.len() > MAX_VALUES_PER_CARD {
            self.unreadable = true;
        }
    }

    fn finish(self) -> ImportedCard {
        if self.unreadable {
            ImportedCard::Unreadable
        } else if self.list {
            ImportedCard::List
        } else {
            let mut input = self.input;
            input.notes = self.notes.join("\n");
            ImportedCard::Contact(input)
        }
    }
}

/// Cartes d'un fichier vCard, dans son ordre ; une carte non fermée en
/// fin de fichier est gardée. Les cartes imbriquées (`AGENT`) sont
/// ignorées.
pub(crate) fn decode(text: &str) -> Result<Vec<ImportedCard>, ContactCodecError> {
    let mut cards = Vec::new();
    let mut current: Option<Card> = None;
    let mut depth = 0_usize;
    for line in logical_lines(text) {
        let Some(p) = property(&line) else {
            continue;
        };
        let card_value = p.value.trim().eq_ignore_ascii_case("VCARD");
        if p.name == "BEGIN" && card_value {
            if depth == 0 {
                current = Some(Card::default());
            }
            depth += 1;
        } else if p.name == "END" && card_value {
            depth = depth.saturating_sub(1);
            if depth == 0
                && let Some(card) = current.take()
            {
                cards.push(card.finish());
            }
        } else if depth == 1
            && let Some(card) = current.as_mut()
        {
            card.read(&p);
        }
        if cards.len() > MAX_IMPORT_CARDS {
            return Err(ContactCodecError::TooManyCards);
        }
    }
    if let Some(card) = current {
        cards.push(card.finish());
    }
    if cards.len() > MAX_IMPORT_CARDS {
        return Err(ContactCodecError::TooManyCards);
    }
    Ok(cards)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> ImportedCard {
        let mut cards = decode(text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cards.len(), 1, "{cards:?}");
        cards.remove(0)
    }

    fn contact(text: &str) -> ContactInput {
        match one(text) {
            ImportedCard::Contact(input) => input,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn looks_like_needs_begin_vcard_first() {
        assert!(looks_like("\r\n  begin:vcard\r\n"));
        assert!(!looks_like("dn: cn=x\nBEGIN:VCARD"));
        assert!(!looks_like("é"));
    }

    #[test]
    fn escapes_and_structured_values() {
        let c = contact(
            "BEGIN:VCARD\nN:O\\;Brien;Seán\\, dit \"Jo\";;;\nNOTE:a\\\\b\\nc\\:d\\x\nORG:Un;Deux\nEND:VCARD",
        );
        assert_eq!(c.family_name, "O;Brien");
        assert_eq!(c.given_name, "Seán, dit \"Jo\"");
        assert_eq!(c.notes, "a\\b\nc:d\\x");
        assert_eq!(c.organization, "Un");
    }

    #[test]
    fn tel_uri_pref_parameter_and_quoted_parameters() {
        let c = contact(
            "BEGIN:VCARD\nVERSION:4.0\nTEL;VALUE=uri;TYPE=\"voice,cell\":tel:+33-6-00\nEMAIL;PREF=1;TYPE=\"work\":a@example.com\nEND:VCARD",
        );
        assert_eq!(c.phones[0].number, "+33-6-00");
        assert_eq!(c.phones[0].label, "cell");
        assert!(c.emails[0].preferred);
        assert_eq!(c.emails[0].label, "work");
    }

    #[test]
    fn several_notes_are_joined_and_first_names_kept() {
        let c = contact("BEGIN:VCARD\nFN:Un\nFN:Deux\nNOTE:a\nNOTE:b\nEND:VCARD");
        assert_eq!(c.display_name, "Un");
        assert_eq!(c.notes, "a\nb");
    }

    #[test]
    fn nested_cards_and_text_outside_cards_are_ignored() {
        let cards = decode(
            "FN:dehors\nBEGIN:VCARD\nFN:A\nAGENT:BEGIN:VCARD\nBEGIN:VCARD\nFN:B\nEND:VCARD\nEMAIL:a@example.com\nEND:VCARD\nBEGIN:VCARD\nFN:C",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cards.len(), 2);
        let ImportedCard::Contact(a) = &cards[0] else {
            panic!("{cards:?}")
        };
        assert_eq!(a.display_name, "A");
        assert_eq!(a.emails.len(), 1);
        // Carte non fermée en fin de fichier : gardée.
        let ImportedCard::Contact(c) = &cards[1] else {
            panic!("{cards:?}")
        };
        assert_eq!(c.display_name, "C");
    }

    #[test]
    fn bad_encodings_and_oversized_fields_make_the_card_unreadable() {
        for line in [
            "NOTE;ENCODING=QUOTED-PRINTABLE:=ZZ".to_owned(),
            "FN;ENCODING=b:***".to_owned(),
            "FN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:=E9".to_owned(),
            format!("NOTE:{}", "x".repeat(MAX_FIELD_BYTES + 1)),
        ] {
            assert_eq!(
                one(&format!("BEGIN:VCARD\n{line}\nEND:VCARD")),
                ImportedCard::Unreadable,
                "{}",
                &line[..line.len().min(40)]
            );
        }
        // Une propriété ignorée démesurée (photo) ne gêne pas.
        let c = contact(&format!(
            "BEGIN:VCARD\nPHOTO;ENCODING=b:{}\nEMAIL:a@example.com\nEND:VCARD",
            "A".repeat(MAX_FIELD_BYTES * 2)
        ));
        assert_eq!(c.emails.len(), 1);
    }

    #[test]
    fn too_many_values_make_the_card_unreadable() {
        let emails: String = (0..=MAX_VALUES_PER_CARD)
            .map(|i| format!("EMAIL:a{i}@example.com\n"))
            .collect();
        assert_eq!(
            one(&format!("BEGIN:VCARD\n{emails}END:VCARD")),
            ImportedCard::Unreadable
        );
    }

    #[test]
    fn too_many_cards_refuse_the_file() {
        let card = "BEGIN:VCARD\nEND:VCARD\n";
        assert_eq!(
            decode(&card.repeat(MAX_IMPORT_CARDS)).map(|c| c.len()),
            Ok(MAX_IMPORT_CARDS)
        );
        assert_eq!(
            decode(&card.repeat(MAX_IMPORT_CARDS + 1)),
            Err(ContactCodecError::TooManyCards)
        );
    }

    #[test]
    fn unfolding_stays_linear_on_hostile_soft_breaks() {
        // Ligne sans « : » terminée par « = », suivie de lignes repliées
        // qui finissent toutes par « = » : chaque ligne relirait toute la
        // ligne accumulée (coût quadratique).
        let hostile = format!("BEGIN:VCARD\nx={}", "\n =".repeat(50_000));
        let start = std::time::Instant::now();
        let _ = decode(&hostile);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "dépliage trop lent : {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn lines_without_colon_are_skipped() {
        let c = contact("BEGIN:VCARD\nn'importe quoi\nEMAIL:a@example.com\nEND:VCARD");
        assert_eq!(c.emails.len(), 1);
    }
}
