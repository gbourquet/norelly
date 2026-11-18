//! Lecture vCard 2.1, 3.0 et 4.0 (RFC 2426, RFC 6350), écrite pour le
//! projet : dépliage des lignes, sauts souples quoted-printable,
//! paramètres (`TYPE`, `PREF`, `CHARSET`, `ENCODING`), échappements,
//! plusieurs cartes par fichier. Seuls `FN`, `N`, `NICKNAME`, `ORG`,
//! `TITLE`, `NOTE`, `BDAY`, `EMAIL`, `TEL`, `ADR`, `URL`, la nature de la
//! carte (`KIND`), les libellés d'Apple (`itemN.X-ABLabel`, étiquette de
//! la valeur du même groupe) et la catégorie des favoris (`CATEGORIES`,
//! rien d'autre des catégories) sont lus ; le reste (photos comprises)
//! est ignoré sans être décodé. Une carte de groupe (`KIND:group`, ou
//! `X-ADDRESSBOOKSERVER-KIND:group` d'Apple) est une liste : nom (`FN`),
//! description (`NOTE`), membres (`MEMBER`, `X-ADDRESSBOOKSERVER-MEMBER`)
//! désignés par une adresse (`mailto:`) ou par l'identifiant (`UID`,
//! `X-ABUID` d'Apple) d'une autre carte du fichier, rendue par sa première
//! adresse ; copie cachée (`X-NORELLY-BCC:TRUE`, propre à Norelly).

use std::collections::HashMap;

use norelly_domain::contact_export::is_favorite_category;
use norelly_domain::contact_import::{ImportedCard, ImportedList, MAX_IMPORT_CARDS};
use norelly_domain::contact_labels::apple_label;
use norelly_domain::contacts::{
    ContactInput, EmailInput, PhoneInput, PostalAddressInput, WebsiteInput,
};
use norelly_domain::ports::ContactCodecError;

use crate::{
    MAX_FIELD_BYTES, MAX_LIST_MEMBERS_READ, MAX_VALUES_PER_CARD, birthday_text, street_lines, text,
};

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

/// Libellés d'Apple gardés au plus par carte (groupes distincts) : un par
/// adresse ou téléphone possible, avec de la marge pour ceux des adresses
/// postales, dates ou sites ; au-delà, ils sont ignorés.
const MAX_GROUP_LABELS: usize = MAX_VALUES_PER_CARD * 2;

/// Longueur maximale d'un nom de groupe (`item1`), en octets ; au-delà, la
/// propriété est lue sans groupe.
const MAX_GROUP_BYTES: usize = 64;

/// Propriété d'une ligne : groupe (`item1`, en minuscules), nom (groupe
/// retiré, en majuscules), paramètres (nom en majuscules, valeur), valeur
/// brute.
#[derive(Debug)]
struct Property<'a> {
    group: Option<String>,
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
    let (group, name) = match name.rsplit_once('.') {
        Some((group, name)) => (
            (group.len() <= MAX_GROUP_BYTES).then(|| group.to_ascii_lowercase()),
            name,
        ),
        None => (None, name),
    };
    let name = name.to_ascii_uppercase();
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
        group,
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

/// Valeur d'une carte qui appartient à un groupe (`item1.TEL`), par son
/// rang : son étiquette est le libellé Apple du groupe, s'il y en a un.
#[derive(Debug)]
enum Grouped {
    Email(usize),
    Phone(usize),
    Postal(usize),
    Website(usize),
}

/// Carte en cours de lecture.
#[derive(Debug, Default)]
struct Card {
    input: ContactInput,
    notes: Vec<String>,
    list: bool,
    unreadable: bool,
    /// Identifiant de la carte (`UID`, `X-ABUID`), comparé sans la casse
    /// ni le préfixe `urn:uuid:`.
    uid: Option<String>,
    /// Membres d'une carte de groupe, tels que lus.
    members: Vec<String>,
    bcc: bool,
    /// Libellés Apple retenus, par groupe (le premier valable compte).
    group_labels: HashMap<String, String>,
    /// Adresses et téléphones d'un groupe.
    grouped: Vec<(Grouped, String)>,
}

impl Card {
    fn read(&mut self, p: &Property<'_>) {
        let kept = matches!(
            p.name.as_str(),
            "FN" | "N"
                | "NICKNAME"
                | "ORG"
                | "TITLE"
                | "NOTE"
                | "BDAY"
                | "EMAIL"
                | "TEL"
                | "ADR"
                | "URL"
        );
        if p.name == "KIND" || p.name == "X-ADDRESSBOOKSERVER-KIND" {
            self.list |= p.value.trim().eq_ignore_ascii_case("group");
            return;
        }
        if p.name == "X-ABLABEL" {
            self.read_apple_label(p);
            return;
        }
        if p.name == "CATEGORIES" {
            self.read_categories(p);
            return;
        }
        if matches!(
            p.name.as_str(),
            "UID" | "X-ABUID" | "MEMBER" | "X-ADDRESSBOOKSERVER-MEMBER" | "X-NORELLY-BCC"
        ) {
            self.read_list_property(p);
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
            "TITLE" if input.title.is_empty() => input.title = unescape(&value),
            "BDAY" if input.birthday.is_empty() => {
                let raw = unescape(&value);
                let raw = raw.trim();
                // Apple marque un anniversaire sans année par une année
                // fictive (`X-APPLE-OMIT-YEAR=1604`, date `1604-04-12`).
                let without_year = p
                    .param("X-APPLE-OMIT-YEAR")
                    .filter(|year| !year.is_empty())
                    .and_then(|year| raw.strip_prefix(year))
                    .map(|rest| format!("--{}", rest.trim_start_matches('-')));
                input.birthday = birthday_text(without_year.as_deref().unwrap_or(raw));
            }
            "ADR" => {
                let parts = components(&value, Some(';'));
                let part = |i: usize| parts.get(i).map_or("", |s| s.trim()).to_owned();
                if let Some(group) = &p.group {
                    self.grouped
                        .push((Grouped::Postal(input.postal_addresses.len()), group.clone()));
                }
                // Boîte postale et complément (`ADR` : boîte ; complément ;
                // rue ; ville ; région ; code ; pays) suivent la rue.
                let street = [2, 1, 0].map(|i| parts.get(i).map_or("", String::as_str));
                input.postal_addresses.push(PostalAddressInput {
                    street: street_lines(street),
                    locality: part(3),
                    region: part(4),
                    postal_code: part(5),
                    country: part(6),
                    label: p.label(),
                });
            }
            "URL" => {
                let url = unescape(&value).trim().to_owned();
                if !url.is_empty() {
                    if let Some(group) = &p.group {
                        self.grouped
                            .push((Grouped::Website(input.websites.len()), group.clone()));
                    }
                    input.websites.push(WebsiteInput {
                        url,
                        label: p.label(),
                    });
                }
            }
            "NOTE" => self.notes.push(unescape(&value)),
            "EMAIL" => {
                let address = unescape(&value).trim().to_owned();
                if !address.is_empty() {
                    if let Some(group) = &p.group {
                        self.grouped
                            .push((Grouped::Email(input.emails.len()), group.clone()));
                    }
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
                    if let Some(group) = &p.group {
                        self.grouped
                            .push((Grouped::Phone(input.phones.len()), group.clone()));
                    }
                    input.phones.push(PhoneInput {
                        number: number.to_owned(),
                        label: p.label(),
                    });
                }
            }
            _ => {}
        }
        let counts = [
            input.emails.len(),
            input.phones.len(),
            input.postal_addresses.len(),
            input.websites.len(),
        ];
        if counts.iter().any(|&n| n > MAX_VALUES_PER_CARD) {
            self.unreadable = true;
        }
    }

    /// Identifiant de la carte, membre d'un groupe, copie cachée. Une
    /// valeur illisible rend la carte illisible ; plus de
    /// [`MAX_LIST_MEMBERS_READ`] membres aussi.
    fn read_list_property(&mut self, p: &Property<'_>) {
        let Some(value) = p.text() else {
            self.unreadable = true;
            return;
        };
        let value = unescape(&value).trim().to_owned();
        match p.name.as_str() {
            "UID" | "X-ABUID" => {
                if self.uid.is_none() && !value.is_empty() {
                    // `X-ABUID` d'Apple : `<uuid>:ABPerson`.
                    let id = value
                        .rsplit_once(':')
                        .filter(|(_, kind)| kind.eq_ignore_ascii_case("ABPerson"))
                        .map_or(value.as_str(), |(id, _)| id);
                    self.uid = Some(card_id(id));
                }
            }
            "X-NORELLY-BCC" => self.bcc = value.eq_ignore_ascii_case("true"),
            _ => {
                if self.members.len() == MAX_LIST_MEMBERS_READ {
                    self.unreadable = true;
                } else {
                    self.members.push(value);
                }
            }
        }
    }

    /// Libellé Apple d'un groupe (`item1.X-ABLabel`) : retenu s'il est
    /// lisible et qu'une fiche peut le garder ([`apple_label`]), dans la
    /// limite de [`MAX_GROUP_LABELS`] groupes ; sinon ignoré, sans rendre
    /// la carte illisible (il peut viser une propriété que Norelly ne lit
    /// pas).
    fn read_apple_label(&mut self, p: &Property<'_>) {
        let Some(group) = &p.group else {
            return;
        };
        if self.group_labels.len() >= MAX_GROUP_LABELS || self.group_labels.contains_key(group) {
            return;
        }
        if let Some(label) = p.text().and_then(|value| apple_label(&unescape(&value))) {
            self.group_labels.insert(group.clone(), label);
        }
    }

    /// Catégories (`CATEGORIES`, séparées par des virgules non
    /// échappées) : la fiche est favorite si l'une d'elles est celle des
    /// favoris ([`is_favorite_category`]). Les autres sont ignorées, et
    /// une valeur illisible ou démesurée aussi, sans rendre la carte
    /// illisible.
    fn read_categories(&mut self, p: &Property<'_>) {
        if let Some(value) = p.text()
            && components(&value, Some(','))
                .iter()
                .any(|category| is_favorite_category(category))
        {
            self.input.favorite = true;
        }
    }

    fn finish(self) -> Read {
        if self.unreadable {
            Read::Ready(ImportedCard::Unreadable)
        } else if self.list {
            Read::List(ImportedList {
                name: self.input.display_name,
                description: self.notes.join("\n"),
                members: self.members,
                unresolved: 0,
                bcc: self.bcc,
            })
        } else {
            let mut input = self.input;
            input.notes = self.notes.join("\n");
            for (value, group) in &self.grouped {
                let Some(label) = self.group_labels.get(group) else {
                    continue;
                };
                let slot = match *value {
                    Grouped::Email(i) => input.emails.get_mut(i).map(|e| &mut e.label),
                    Grouped::Phone(i) => input.phones.get_mut(i).map(|p| &mut p.label),
                    Grouped::Postal(i) => input.postal_addresses.get_mut(i).map(|a| &mut a.label),
                    Grouped::Website(i) => input.websites.get_mut(i).map(|w| &mut w.label),
                };
                if let Some(slot) = slot {
                    slot.clone_from(label);
                }
            }
            let id = self
                .uid
                .and_then(|uid| input.emails.first().map(|e| (uid, e.address.clone())));
            Read::Contact(input, id)
        }
    }
}

/// Carte lue, avant que les membres des groupes soient retrouvés.
#[derive(Debug)]
enum Read {
    Ready(ImportedCard),
    /// Contact, et son identifiant avec sa première adresse s'il en a.
    Contact(ContactInput, Option<(String, String)>),
    /// Groupe, membres tels que lus.
    List(ImportedList),
}

/// Identifiant de carte comparable : sans espaces de bord, sans le préfixe
/// `urn:uuid:`, en minuscules.
fn card_id(raw: &str) -> String {
    let raw = raw.trim();
    let id = raw
        .get(..9)
        .filter(|prefix| prefix.eq_ignore_ascii_case("urn:uuid:"))
        .map_or(raw, |_| &raw[9..]);
    id.to_lowercase()
}

/// Adresse d'un membre `mailto:` (RFC 6068) : partie avant `?`,
/// pourcentages décodés (laissée telle quelle si un pourcentage est
/// invalide ou ne donne pas de l'UTF-8) ; `None` si ce n'en est pas un.
fn mailto(member: &str) -> Option<String> {
    let rest = member
        .get(..7)
        .filter(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
        .map(|_| &member[7..])?;
    let address = rest.split('?').next().unwrap_or_default();
    Some(percent_decoded(address).unwrap_or_else(|| address.to_owned()))
}

/// Pourcentages décodés ; `None` si l'un est invalide ou si le résultat
/// n'est pas de l'UTF-8.
fn percent_decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Cartes lues, membres des groupes retrouvés : une adresse `mailto:`
/// telle quelle, un identifiant par la première adresse de la carte de
/// contact qui le porte ; sinon compté introuvable.
fn resolve(read: Vec<Read>) -> Vec<ImportedCard> {
    let mut ids: HashMap<String, String> = HashMap::new();
    for card in &read {
        if let Read::Contact(_, Some((uid, address))) = card {
            ids.entry(uid.clone()).or_insert_with(|| address.clone());
        }
    }
    read.into_iter()
        .map(|card| match card {
            Read::Ready(card) => card,
            Read::Contact(input, _) => ImportedCard::Contact(input),
            Read::List(mut list) => {
                let raw = std::mem::take(&mut list.members);
                for member in raw {
                    let address = mailto(&member).or_else(|| ids.get(&card_id(&member)).cloned());
                    match address {
                        Some(address) if !address.trim().is_empty() => list.members.push(address),
                        _ => list.unresolved += 1,
                    }
                }
                ImportedCard::List(list)
            }
        })
        .collect()
}

/// Cartes d'un fichier vCard, dans son ordre ; une carte non fermée en
/// fin de fichier est gardée. Les cartes imbriquées (`AGENT`) sont
/// ignorées.
pub(crate) fn decode(text: &str) -> Result<Vec<ImportedCard>, ContactCodecError> {
    let mut cards: Vec<Read> = Vec::new();
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
    Ok(resolve(cards))
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
    fn apple_labels_apply_to_the_value_of_their_group() {
        let c = contact(
            "BEGIN:VCARD\n\
             Item2.X-ABLabel:bureau\n\
             item1.TEL;type=CELL:01\n\
             item2.TEL:02\n\
             ITEM1.X-ABLabel:_$!<Work>!$_\n\
             item3.EMAIL;type=HOME:a@example.com\n\
             item3.X-ABLabel:  Chalet \\, été\n\
             item4.X-ABLabel:orphelin\n\
             TEL;type=HOME:03\n\
             item5.X-ABLabel;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Caf=C3=A9\n\
             item5.TEL:04\n\
             END:VCARD",
        );
        let phones: Vec<_> = c.phones.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(phones, ["work", "bureau", "home", "Café"]);
        assert_eq!(c.emails[0].label, "Chalet , été");
    }

    #[test]
    fn apple_labels_a_card_could_not_keep_leave_the_original_label() {
        // Illisible, démesuré, refusé par une fiche, ou d'un groupe que
        // Norelly ne lit pas : le libellé est ignoré, la fiche importée.
        let c = contact(&format!(
            "BEGIN:VCARD\n\
             item1.EMAIL;type=HOME:a@example.com\n\
             item1.X-ABLabel;ENCODING=QUOTED-PRINTABLE:=ZZ\n\
             item2.TEL;type=CELL:01\n\
             item2.X-ABLabel:Maison \u{2764}\u{FE0F}\n\
             item3.TEL;type=WORK:02\n\
             item3.X-ABLabel:{}\n\
             item4.ADR:;;rue;Lyon;;;\n\
             item4.X-ABLabel;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:=E9\n\
             END:VCARD",
            "x".repeat(MAX_FIELD_BYTES + 1)
        ));
        assert_eq!(c.emails[0].label, "home");
        let phones: Vec<_> = c.phones.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(phones, ["cell", "work"]);
    }

    #[test]
    fn apple_labels_beyond_the_bound_are_ignored() {
        // Groupes distincts au-delà de la borne : ignorés, pas de croissance.
        let labels: String = (0..=MAX_GROUP_LABELS)
            .map(|i| format!("item{i}.X-ABLabel:l{i}\n"))
            .collect();
        let c = contact(&format!(
            "BEGIN:VCARD\n{labels}item0.TEL:01\nitem{MAX_GROUP_LABELS}.TEL;type=CELL:02\nEND:VCARD"
        ));
        let phones: Vec<_> = c.phones.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(phones, ["l0", "cell"]);
    }

    #[test]
    fn an_overlong_group_name_is_not_a_group() {
        let group = "g".repeat(MAX_GROUP_BYTES + 1);
        let c = contact(&format!(
            "BEGIN:VCARD\n{group}.TEL;type=CELL:01\n{group}.X-ABLabel:bureau\nEND:VCARD"
        ));
        assert_eq!(c.phones[0].label, "cell");
        let group = "g".repeat(MAX_GROUP_BYTES);
        let c = contact(&format!(
            "BEGIN:VCARD\n{group}.TEL;type=CELL:01\n{group}.X-ABLabel:bureau\nEND:VCARD"
        ));
        assert_eq!(c.phones[0].label, "bureau");
    }

    #[test]
    fn the_starred_category_marks_a_favorite() {
        for categories in [
            "CATEGORIES:myContacts,starred",
            "CATEGORIES:Starred",
            "CATEGORIES:Famille\nCATEGORIES:Amis, STARRED ",
            "item1.CATEGORIES;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Caf=C3=A9,starred",
        ] {
            let c = contact(&format!(
                "BEGIN:VCARD\n{categories}\nEMAIL:a@example.com\nEND:VCARD"
            ));
            assert!(c.favorite, "{categories}");
            // Aucune autre trace : ni étiquette, ni note.
            assert_eq!(c.emails[0].label, "", "{categories}");
            assert_eq!(c.notes, "", "{categories}");
        }
    }

    #[test]
    fn other_or_unreadable_categories_leave_the_card_as_it_is() {
        for categories in [
            "CATEGORIES:myContacts,Famille".to_owned(),
            "CATEGORIES:starred2,star red".to_owned(),
            // Virgule échappée : une seule catégorie, « a,starred ».
            "CATEGORIES:a\\,starred".to_owned(),
            "CATEGORIES;ENCODING=QUOTED-PRINTABLE:=ZZ".to_owned(),
            format!("CATEGORIES:{},starred", "x".repeat(MAX_FIELD_BYTES)),
        ] {
            let c = contact(&format!(
                "BEGIN:VCARD\n{categories}\nEMAIL:a@example.com\nEND:VCARD"
            ));
            assert!(!c.favorite, "{}", &categories[..categories.len().min(40)]);
            assert_eq!(c.emails.len(), 1);
        }
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
    fn title_birthday_postal_addresses_and_websites_are_read() {
        let c = contact(
            "BEGIN:VCARD\n\
             VERSION:3.0\n\
             TITLE:Directrice\\, ventes\n\
             TITLE:Autre\n\
             BDAY:1985-04-12\n\
             ADR;TYPE=HOME:BP 12;Bât. B;1 rue des Lilas\\nEsc. 3;Lyon;Rhône;69001;France\n\
             item1.ADR:;;2 rue\\, du Port;Nantes;;44000;\n\
             item1.X-ABLabel:_$!<Work>!$_\n\
             ADR;TYPE=work:;;;;;;\n\
             URL;TYPE=work:https://example.com/a\\,b\n\
             item2.URL:www.example.org\n\
             item2.X-ABLabel:blog\n\
             END:VCARD",
        );
        assert_eq!(c.title, "Directrice, ventes");
        assert_eq!(c.birthday, "1985-04-12");
        let places: Vec<_> = c
            .postal_addresses
            .iter()
            .map(|a| {
                (
                    a.street.as_str(),
                    a.locality.as_str(),
                    a.region.as_str(),
                    a.postal_code.as_str(),
                    a.country.as_str(),
                    a.label.as_str(),
                )
            })
            .collect();
        assert_eq!(
            places,
            [
                (
                    "1 rue des Lilas\nEsc. 3\nBât. B\nBP 12",
                    "Lyon",
                    "Rhône",
                    "69001",
                    "France",
                    "home"
                ),
                ("2 rue, du Port", "Nantes", "", "44000", "", "work"),
                ("", "", "", "", "", "work"),
            ]
        );
        let sites: Vec<_> = c
            .websites
            .iter()
            .map(|w| (w.url.as_str(), w.label.as_str()))
            .collect();
        assert_eq!(
            sites,
            [
                ("https://example.com/a,b", "work"),
                ("www.example.org", "blog")
            ]
        );
    }

    #[test]
    fn apple_birthdays_without_year_drop_the_placeholder_year() {
        for (line, expected) in [
            ("BDAY;X-APPLE-OMIT-YEAR=1604:1604-04-12", "--04-12"),
            ("BDAY;X-APPLE-OMIT-YEAR=1604;VALUE=date:16040412", "--04-12"),
            ("BDAY;X-APPLE-OMIT-YEAR=1604:1985-04-12", "1985-04-12"),
            ("BDAY:--0412", "--04-12"),
            ("BDAY;VALUE=text:vers 1950", "vers 1950"),
        ] {
            let c = contact(&format!("BEGIN:VCARD\n{line}\nEND:VCARD"));
            assert_eq!(c.birthday, expected, "{line}");
        }
    }

    #[test]
    fn too_many_postal_addresses_or_websites_make_the_card_unreadable() {
        for property in ["ADR:;;rue;;;;", "URL:https://example.com"] {
            let lines = format!("{property}\n").repeat(MAX_VALUES_PER_CARD + 1);
            assert_eq!(
                one(&format!("BEGIN:VCARD\n{lines}END:VCARD")),
                ImportedCard::Unreadable,
                "{property}"
            );
        }
    }

    #[test]
    fn lines_without_colon_are_skipped() {
        let c = contact("BEGIN:VCARD\nn'importe quoi\nEMAIL:a@example.com\nEND:VCARD");
        assert_eq!(c.emails.len(), 1);
    }

    fn list(text: &str) -> ImportedList {
        let cards = decode(text).unwrap_or_else(|e| panic!("{e}"));
        let lists: Vec<_> = cards
            .into_iter()
            .filter_map(|c| match c {
                ImportedCard::List(list) => Some(list),
                _ => None,
            })
            .collect();
        // La première liste du fichier.
        lists
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("aucune liste"))
    }

    #[test]
    fn a_vcard_4_group_names_its_members_by_uid_or_mailto() {
        let l = list(
            "BEGIN:VCARD\nVERSION:4.0\nKIND:group\nFN:Club\\, lecture\nNOTE:Le jeudi\\n20 h\n\
             MEMBER:urn:uuid:A1\nMEMBER:mailto:o%27brien@example.org?subject=x\n\
             MEMBER:MAILTO:bob@example.net\nMEMBER:urn:uuid:inconnu\nMEMBER:urn:uuid:sans-adresse\n\
             MEMBER:urn:uuid:groupe\nMEMBER:mailto:\nMEMBER:mailto:%ZZ@example.com\n\
             X-NORELLY-BCC:TRUE\nEND:VCARD\n\
             BEGIN:VCARD\nVERSION:4.0\nUID:urn:uuid:a1\nFN:Alice\n\
             EMAIL:alice@example.com\nEMAIL:alice@work.example\nEND:VCARD\n\
             BEGIN:VCARD\nVERSION:4.0\nUID:urn:uuid:sans-adresse\nFN:Sans\nEND:VCARD\n\
             BEGIN:VCARD\nVERSION:4.0\nKIND:group\nUID:urn:uuid:groupe\nFN:Autre\nEND:VCARD\n",
        );
        // Dans l'ordre de la carte ; la carte désignée peut la suivre.
        assert_eq!(
            l,
            ImportedList {
                name: "Club, lecture".into(),
                description: "Le jeudi\n20 h".into(),
                members: vec![
                    "alice@example.com".into(),
                    "o'brien@example.org".into(),
                    "bob@example.net".into(),
                    "%ZZ@example.com".into(),
                ],
                unresolved: 4,
                bcc: true,
            }
        );
    }

    #[test]
    fn an_apple_group_names_its_members_by_their_card_id() {
        let l = list(
            "BEGIN:VCARD\nVERSION:3.0\nN:Martin;Alice;;;\nFN:Alice\nEMAIL:alice@example.com\n\
             X-ABUID:5C1E7F2A-0001:ABPerson\nEND:VCARD\n\
             BEGIN:VCARD\nVERSION:3.0\nUID:b2\nEMAIL:bob@example.net\nEND:VCARD\n\
             BEGIN:VCARD\nVERSION:3.0\nN:\nFN:Famille\nX-ADDRESSBOOKSERVER-KIND:group\n\
             X-ADDRESSBOOKSERVER-MEMBER:urn:uuid:5c1e7f2a-0001\n\
             X-ADDRESSBOOKSERVER-MEMBER:urn:uuid:B2\nX-NORELLY-BCC:non\nEND:VCARD\n",
        );
        assert_eq!(l.name, "Famille");
        assert_eq!(l.members, ["alice@example.com", "bob@example.net"]);
        assert_eq!(l.unresolved, 0);
        assert!(!l.bcc);
    }

    #[test]
    fn a_group_with_too_many_members_is_unreadable() {
        let members: String = (0..=MAX_LIST_MEMBERS_READ)
            .map(|i| format!("MEMBER:mailto:m{i}@example.com\n"))
            .collect();
        assert_eq!(
            one(&format!(
                "BEGIN:VCARD\nKIND:group\nFN:L\n{members}END:VCARD"
            )),
            ImportedCard::Unreadable
        );
        let members: String = (0..MAX_LIST_MEMBERS_READ)
            .map(|i| format!("MEMBER:mailto:m{i}@example.com\n"))
            .collect();
        assert_eq!(
            list(&format!(
                "BEGIN:VCARD\nKIND:group\nFN:L\n{members}END:VCARD"
            ))
            .members
            .len(),
            MAX_LIST_MEMBERS_READ
        );
    }
}
