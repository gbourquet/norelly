//! Nom affiché d'un correspondant (expéditeur, destinataire) : celui du
//! carnet d'adresses quand son adresse y figure (ADR-012), sinon celui de
//! l'en-tête, écarté s'il imite une adresse.
//!
//! Le nom du carnet s'affiche aussi pour une adresse `From` falsifiée : il
//! ne vaut pas preuve d'identité. La pastille « Dans vos contacts », elle,
//! n'est accordée qu'à un expéditeur authentifié par le serveur du compte
//! ([`crate::sender_auth`]).

use std::collections::HashMap;

use crate::contacts::{AddressKey, Contact, ContactId, is_invisible};
use crate::mailbox::Correspondent;
use crate::ports::MAX_CONTACT_LOOKUP;
use crate::sender_auth::SenderAuth;

/// Correspondant tel qu'il s'affiche.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SenderLabel {
    /// Nom à afficher ; vide : l'adresse seule s'affiche.
    pub name: String,
    /// Adresse de l'en-tête, telle quelle.
    pub address: String,
    /// Contact du carnet qui porte l'adresse.
    pub contact: Option<ContactId>,
    /// Afficher aussi l'adresse à côté du nom : le nom de l'en-tête
    /// contient un domaine et pourrait passer pour l'expéditeur réel.
    pub show_address: bool,
    /// Authentification de l'expéditeur ([`SenderAuth::Unknown`] pour un
    /// destinataire, ou un expéditeur sans verdict).
    pub auth: SenderAuth,
    /// Afficher la pastille « Dans vos contacts » : le contact du carnet
    /// porte l'adresse et l'expéditeur est authentifié
    /// ([`SenderAuth::Pass`]). Sans authentification, l'adresse seule ne
    /// cautionne personne.
    pub contact_badge: bool,
}

/// Caractères qui servent, ou ressemblent, à `@`, `<` et `>` (formes
/// pleine largeur, petites formes, guillemets, chevrons, lettres
/// modificatives et ornements). Les guillemets français `« »`, d'usage
/// courant, n'en font pas partie, ni les lettres d'une écriture (syllabaire
/// canadien `ᐸ ᐳ`).
const ADDRESS_LOOK_ALIKES: [char; 23] = [
    '@', '<', '>', '\u{FF20}', '\u{FE6B}', '\u{FF1C}', '\u{FF1E}', '\u{FE64}', '\u{FE65}',
    '\u{2039}', '\u{203A}', '\u{2329}', '\u{232A}', '\u{27E8}', '\u{27E9}', '\u{3008}', '\u{3009}',
    '\u{02C2}', '\u{02C3}', '\u{276C}', '\u{276D}', '\u{276E}', '\u{276F}',
];

/// Mots qui écrivent `@` en toutes lettres (`support(at)banque.fr`,
/// `support arobase banque.fr`).
const TEXTUAL_AT: [&str; 2] = ["at", "arobase"];

/// Mots qui écrivent `.` en toutes lettres (`banque dot fr`).
const TEXTUAL_DOT: [&str; 2] = ["dot", "point"];

/// Paires de délimiteurs qui encadrent une forme textuelle (`(at)`,
/// `[at]`, `{at}`).
const TEXTUAL_BRACKETS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

/// Vrai si un nom d'en-tête imite une adresse : `@`, `<`, `>` ou un
/// caractère qui leur ressemble, ou `@` écrit en toutes lettres
/// (`support(at)banque.fr`, `support at banque.fr`, `support arobase
/// banque point fr`). Un tel nom pourrait faire passer une adresse pour
/// une autre.
#[must_use]
pub fn imitates_address(name: &str) -> bool {
    name.chars().any(is_address_look_alike) || writes_at_in_words(&folded(name))
}

/// Vrai si un nom contient un nom de domaine (`banque.fr`,
/// `secure.paypal.com`) : affiché seul, il pourrait passer pour
/// l'expéditeur réel. Des initiales (`A. Dupont`, `M.Dupont`), une
/// abréviation (`Dr. Martin`) ou un nombre (`2.0`) n'en sont pas.
#[must_use]
pub fn names_a_domain(name: &str) -> bool {
    contains_domain(&widened(name))
}

/// Marques combinantes : elles ne changent pas, ou à peine, l'apparence
/// d'un nom, mais couperaient un mot pour la détection. Les caractères
/// invisibles viennent de l'ensemble partagé du domaine
/// ([`is_invisible`]).
const COMBINING: [(char, char); 9] = [
    ('\u{0300}', '\u{036F}'),
    ('\u{1AB0}', '\u{1AFF}'),
    ('\u{1DC0}', '\u{1DFF}'),
    ('\u{20D0}', '\u{20FF}'),
    ('\u{FE20}', '\u{FE2F}'),
    ('\u{1D167}', '\u{1D169}'),
    ('\u{1D17B}', '\u{1D182}'),
    ('\u{1D185}', '\u{1D18B}'),
    ('\u{1D1AA}', '\u{1D1AD}'),
];

/// Sosies du point.
const DOT_LOOK_ALIKES: [char; 16] = [
    '\u{00B7}', '\u{0387}', '\u{06D4}', '\u{0701}', '\u{0702}', '\u{2024}', '\u{2219}', '\u{22C5}',
    '\u{2E31}', '\u{2E33}', '\u{3002}', '\u{30FB}', '\u{A4F8}', '\u{FE52}', '\u{FF61}', '\u{FF65}',
];

/// Sosies des délimiteurs de [`TEXTUAL_BRACKETS`] et leur forme ASCII.
const BRACKET_LOOK_ALIKES: [(char, char); 14] = [
    ('\u{FE59}', '('),
    ('\u{FE5A}', ')'),
    ('\u{FE5B}', '{'),
    ('\u{FE5C}', '}'),
    ('\u{FE5D}', '['),
    ('\u{FE5E}', ']'),
    ('\u{207D}', '('),
    ('\u{207E}', ')'),
    ('\u{208D}', '('),
    ('\u{208E}', ')'),
    ('\u{2768}', '('),
    ('\u{2769}', ')'),
    ('\u{FF5F}', '('),
    ('\u{FF60}', ')'),
];

/// Lettres cyrilliques et grecques minuscules qui ressemblent à une lettre
/// latine, et cette lettre.
const LETTER_LOOK_ALIKES: [(char, char); 15] = [
    ('\u{0430}', 'a'),
    ('\u{0435}', 'e'),
    ('\u{043E}', 'o'),
    ('\u{0440}', 'p'),
    ('\u{0441}', 'c'),
    ('\u{0442}', 't'),
    ('\u{0445}', 'x'),
    ('\u{0456}', 'i'),
    ('\u{0501}', 'd'),
    ('\u{03B1}', 'a'),
    ('\u{03B9}', 'i'),
    ('\u{03BF}', 'o'),
    ('\u{03C1}', 'p'),
    ('\u{03C4}', 't'),
    ('\u{03BD}', 'v'),
];

fn is_invisible_or_combining(c: char) -> bool {
    is_invisible(c)
        || COMBINING
            .iter()
            .any(|&(low, high)| (low..=high).contains(&c))
}

/// Lettre ou chiffre des symboles alphanumériques mathématiques (gras,
/// italique, double barre…) ramené à l'ASCII.
fn plain_math_alphanumeric(c: char) -> Option<char> {
    let code = u32::from(c);
    let ascii = match code {
        0x1D400..=0x1D6A3 => {
            let i = (code - 0x1D400) % 52;
            if i < 26 { 0x41 + i } else { 0x61 + i - 26 }
        }
        0x1D7CE..=0x1D7FF => 0x30 + (code - 0x1D7CE) % 10,
        _ => return None,
    };
    char::from_u32(ascii)
}

/// Nom débarrassé de ce qui déguise un caractère : caractères invisibles
/// et marques combinantes retirés, formes pleine largeur et symboles
/// mathématiques ramenés à l'ASCII, sosies du point ramenés à `.`, sosies
/// des parenthèses et accolades à leur forme ASCII, espace idéographique à
/// une espace. La casse est gardée.
fn widened(name: &str) -> String {
    name.chars()
        .filter(|&c| !is_invisible_or_combining(c))
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(u32::from(c) - 0xFEE0).unwrap_or(c),
            '\u{3000}' => ' ',
            _ if DOT_LOOK_ALIKES.contains(&c) => '.',
            _ => BRACKET_LOOK_ALIKES
                .iter()
                .find(|&&(alike, _)| alike == c)
                .map(|&(_, ascii)| ascii)
                .or_else(|| plain_math_alphanumeric(c))
                .unwrap_or(c),
        })
        .collect()
}

/// [`widened`], en minuscules, les lettres cyrilliques et grecques qui
/// ressemblent à une lettre latine ramenées à celle-ci.
fn folded(name: &str) -> String {
    widened(name)
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| {
            LETTER_LOOK_ALIKES
                .iter()
                .find(|&&(alike, _)| alike == c)
                .map_or(c, |&(_, latin)| latin)
        })
        .collect()
}

/// `@` en toutes lettres dans un nom déjà ramené par [`folded`] : un mot
/// de [`TEXTUAL_AT`] entre délimiteurs et entre deux mots
/// (`support(at)banque`, pas `Hans Müller (AT)`), ou seul entre deux mots
/// quand ce qui suit ressemble à un domaine.
fn writes_at_in_words(name: &str) -> bool {
    let compact: String = name.chars().filter(|c| !c.is_whitespace()).collect();
    if TEXTUAL_AT
        .iter()
        .any(|word| bracketed_between_words(&compact, word))
    {
        return true;
    }
    // Un mot avant, puis `at` suivi d'un domaine (`banque.fr`,
    // `banque(dot)fr`) ou d'un mot puis d'un `.` en toutes lettres
    // (`banque dot fr`).
    let words: Vec<&str> = name.split_whitespace().collect();
    (1..words.len()).any(|i| {
        let at = words.get(i).is_some_and(|w| TEXTUAL_AT.contains(w));
        let next = words.get(i + 1).copied().unwrap_or_default();
        let dot_after = words.get(i + 2).is_some_and(|w| is_textual_dot(w));
        at && !next.is_empty()
            && (contains_domain(next)
                || TEXTUAL_DOT.iter().any(|dot| bracketed(next, dot))
                || dot_after)
    })
}

/// Vrai si `word` figure dans `text` entre l'une des paires de
/// [`TEXTUAL_BRACKETS`].
fn bracketed(text: &str, word: &str) -> bool {
    TEXTUAL_BRACKETS
        .iter()
        .any(|(open, close)| text.contains(&format!("{open}{word}{close}")))
}

/// Vrai si `word` figure dans `text` entre l'une des paires de
/// [`TEXTUAL_BRACKETS`], précédé et suivi d'une lettre ou d'un chiffre.
fn bracketed_between_words(text: &str, word: &str) -> bool {
    TEXTUAL_BRACKETS.iter().any(|(open, close)| {
        let pattern = format!("{open}{word}{close}");
        text.match_indices(&pattern).any(|(at, _)| {
            let before = text.get(..at).and_then(|t| t.chars().next_back());
            let after = text
                .get(at + pattern.len()..)
                .and_then(|t| t.chars().next());
            before.is_some_and(char::is_alphanumeric) && after.is_some_and(char::is_alphanumeric)
        })
    })
}

/// Mot qui écrit `.` en toutes lettres, encadré ou non (`dot`, `[dot]`).
fn is_textual_dot(word: &str) -> bool {
    let bare = word.trim_matches(|c| {
        TEXTUAL_BRACKETS
            .iter()
            .any(|&(open, close)| c == open || c == close)
    });
    TEXTUAL_DOT.contains(&bare)
}

/// Vrai si un nom déjà ramené par [`widened`] contient un mot en forme de
/// domaine : des étiquettes séparées par des points, la dernière faite de
/// deux lettres ou plus, ou de la forme ASCII d'un domaine international
/// (`xn--p1ai`).
fn contains_domain(name: &str) -> bool {
    name.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '.'))
        .any(is_domain)
}

fn is_domain(token: &str) -> bool {
    let labels: Vec<&str> = token.trim_end_matches('.').split('.').collect();
    let Some((tld, rest)) = labels.split_last() else {
        return false;
    };
    let valid_label = |label: &&str| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_alphanumeric() || c == '-')
    };
    let punycode = tld.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("xn--"))
        && tld.len() > 4
        && tld.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let letters = tld.chars().count() >= 2 && tld.chars().all(char::is_alphabetic);
    // Des initiales devant un nom propre (`M.Dupont`, `J.R.R.Tolkien`) ne
    // font pas un domaine ; `X.com`, `T.Co` ou `A.Bank` en sont un.
    let initials = rest.iter().all(|label| label.chars().count() == 1)
        && tld.chars().next().is_some_and(char::is_uppercase)
        && tld.chars().any(char::is_lowercase)
        && tld.chars().count() >= 4
        && !WORD_TLDS.iter().any(|word| tld.eq_ignore_ascii_case(word));
    !rest.is_empty() && rest.iter().all(valid_label) && (punycode || letters) && !initials
}

/// Domaines de premier niveau courants qui sont aussi des mots
/// (`A.Bank`) : jamais pris pour un nom propre après des initiales.
const WORD_TLDS: [&str; 12] = [
    "bank", "info", "shop", "site", "store", "online", "cloud", "mobi", "tech", "live", "email",
    "secure",
];

/// Vrai si `c` sert, ou ressemble, à `@`, `<` ou `>`.
#[must_use]
pub fn is_address_look_alike(c: char) -> bool {
    ADDRESS_LOOK_ALIKES.contains(&c)
}

/// Nom affiché d'un correspondant : celui de `contact` s'il porte
/// exactement l'adresse de l'en-tête (même clé), sinon le nom de
/// l'en-tête sans caractère invisible, vide s'il imite une adresse,
/// accompagné de l'adresse s'il contient un domaine ([`names_a_domain`]).
/// Un contact sans nom (ni nom affiché, ni prénom, ni nom, ni surnom)
/// laisse l'adresse seule.
/// Sans verdict d'authentification : jamais de pastille du carnet.
#[must_use]
pub fn sender_label(header: &Correspondent, contact: Option<&Contact>) -> SenderLabel {
    sender_label_with_auth(header, contact, SenderAuth::Unknown)
}

/// [`sender_label`] d'un expéditeur dont l'authentification est `auth` :
/// la pastille du carnet ([`SenderLabel::contact_badge`]) n'est accordée
/// qu'à un expéditeur authentifié ([`SenderAuth::Pass`]).
#[must_use]
pub fn sender_label_with_auth(
    header: &Correspondent,
    contact: Option<&Contact>,
    auth: SenderAuth,
) -> SenderLabel {
    let known = contact.filter(|c| {
        AddressKey::parse(&header.address).is_ok_and(|key| c.draft().has_address(&key))
    });
    let name = match known {
        Some(c) => contact_name(c).unwrap_or_default(),
        None => {
            // Sans caractère invisible : ce nom préremplit une nouvelle
            // fiche, qui les refuse.
            let name: String = header.name.chars().filter(|&c| !is_invisible(c)).collect();
            let name = name.trim();
            if imitates_address(name) {
                String::new()
            } else {
                name.to_owned()
            }
        }
    };
    let show_address = known.is_none() && names_a_domain(&name);
    SenderLabel {
        name,
        address: header.address.clone(),
        contact: known.map(Contact::id),
        show_address,
        auth,
        contact_badge: known.is_some() && auth == SenderAuth::Pass,
    }
}

/// Nom d'un contact, s'il en porte un (sinon son nom affiché serait son
/// adresse préférée, pas forcément celle du correspondant) et qu'il
/// n'imite pas une adresse.
pub(crate) fn contact_name(contact: &Contact) -> Option<String> {
    let d = contact.draft();
    let named = d.display_name_field().is_some()
        || d.given_name().is_some()
        || d.family_name().is_some()
        || d.nickname().is_some();
    named
        .then(|| d.display_name())
        .filter(|name| !imitates_address(name))
}

/// Clés des adresses valides de ces correspondants, chacune une fois dans
/// l'ordre de la première apparition, au plus [`MAX_CONTACT_LOOKUP`].
pub fn sender_keys<'a>(people: impl IntoIterator<Item = &'a Correspondent>) -> Vec<AddressKey> {
    let mut keys = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for person in people {
        if keys.len() == MAX_CONTACT_LOOKUP {
            break;
        }
        if let Ok(key) = AddressKey::parse(&person.address)
            && seen.insert(key.clone())
        {
            keys.push(key);
        }
    }
    keys
}

/// Contacts du carnet retrouvés par adresse, pour nommer des
/// correspondants.
#[derive(Debug, Clone, Default)]
pub struct SenderNames {
    contacts: Vec<Contact>,
    by_key: HashMap<AddressKey, usize>,
}

impl SenderNames {
    #[must_use]
    pub fn new(contacts: Vec<Contact>) -> Self {
        let mut by_key = HashMap::new();
        for (index, contact) in contacts.iter().enumerate() {
            for email in contact.draft().emails() {
                by_key.entry(email.key().clone()).or_insert(index);
            }
        }
        Self { contacts, by_key }
    }

    /// Contact qui porte l'adresse de `header`.
    #[must_use]
    pub fn contact_for(&self, header: &Correspondent) -> Option<&Contact> {
        let key = AddressKey::parse(&header.address).ok()?;
        self.by_key.get(&key).and_then(|&i| self.contacts.get(i))
    }

    /// Nom affiché de `header` ([`sender_label`]), sans verdict
    /// d'authentification (destinataire, listes).
    #[must_use]
    pub fn label(&self, header: &Correspondent) -> SenderLabel {
        sender_label(header, self.contact_for(header))
    }

    /// Nom affiché de l'expéditeur `header`, authentifié selon `auth`
    /// ([`sender_label_with_auth`]).
    #[must_use]
    pub fn sender(&self, header: &Correspondent, auth: SenderAuth) -> SenderLabel {
        sender_label_with_auth(header, self.contact_for(header), auth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::{ContactDraft, ContactInput, EmailInput};

    /// Fiche telle que relue du carnet.
    fn contact(id: i64, input: ContactInput, emails: &[&str]) -> Contact {
        let input = ContactInput {
            emails: emails
                .iter()
                .map(|a| EmailInput {
                    address: (*a).to_owned(),
                    ..EmailInput::default()
                })
                .collect(),
            ..input
        };
        let draft =
            ContactDraft::from_stored(input).unwrap_or_else(|e| panic!("fiche refusée : {e}"));
        draft.with_id(ContactId::new(id).unwrap_or_else(|| panic!("identifiant {id}")))
    }

    fn alice() -> Contact {
        contact(
            7,
            ContactInput {
                display_name: "Alice Martin".into(),
                ..ContactInput::default()
            },
            &["alice@example.com", "a.martin@travail.fr"],
        )
    }

    fn header(name: &str, address: &str) -> Correspondent {
        Correspondent::new(name, address)
    }

    fn id(raw: i64) -> Option<ContactId> {
        ContactId::new(raw)
    }

    #[test]
    fn a_name_with_at_or_angle_brackets_imitates_an_address() {
        assert!(imitates_address("banque@exemple.fr"));
        assert!(imitates_address("Banque <service@banque.fr>"));
        assert!(imitates_address("x > y"));
        assert!(!imitates_address("Alice Martin"));
        assert!(!imitates_address(""));
    }

    #[test]
    fn look_alikes_of_at_and_angle_brackets_also_imitate_an_address() {
        for name in [
            "service\u{FF20}banque.fr",
            "service\u{FE6B}banque.fr",
            "Banque \u{FF1C}x\u{FF1E}",
            "Banque \u{FE64}x\u{FE65}",
            "Banque \u{2039}x\u{203A}",
            "Banque \u{2329}x\u{232A}",
            "Banque \u{27E8}x\u{27E9}",
            "Banque \u{3008}x\u{3009}",
            "Banque \u{02C2}x\u{02C3}",
            "Banque \u{276E}x\u{276F}",
            "Banque \u{276C}x\u{276D}",
        ] {
            assert!(imitates_address(name), "{name}");
        }
    }

    #[test]
    fn textual_forms_of_at_imitate_an_address() {
        for name in [
            "support(at)banque.fr",
            "support (at) banque.fr",
            "support[at]banque.fr",
            "support [ AT ] banque.fr",
            "support{at}banque.fr",
            "Support(AT)Banque.fr",
            "support\u{FF08}at\u{FF09}banque.fr",
            "support\u{FF3B}\u{FF41}\u{FF54}\u{FF3D}banque.fr",
            "support(arobase)banque.fr",
            "support at banque.fr",
            "support AT banque.fr",
            "support at banque dot fr",
            "support at banque point fr",
            "support at banque(dot)fr",
            "support at banque [dot] fr",
            "support arobase banque.fr",
            "support arobase banque point fr",
            "support\u{3000}at\u{3000}banque.fr",
            "support at banque\u{FF0E}fr",
            "support(a\u{200B}t)banque.fr",
            "support a\u{2060}t banque.fr",
        ] {
            assert!(imitates_address(name), "{name}");
        }
    }

    #[test]
    fn ordinary_names_do_not_imitate_an_address() {
        for name in [
            "Jean-Pierre",
            "Dr. Martin",
            "Saint-Étienne",
            "A. Dupont",
            "J.-P. Sartre",
            "Alice (travail)",
            "Équipe [projet]",
            "Meet at noon",
            "Rendez-vous at home",
            "Kate",
            "Matt Atkinson",
            "Banque.fr",
            "Pat (atelier)",
            "",
        ] {
            assert!(!imitates_address(name), "{name}");
        }
    }

    #[test]
    fn a_name_containing_a_domain_is_detected() {
        for name in [
            "banque.fr",
            "Service client banque.fr",
            "Paypal.com Sécurité",
            "secure.paypal.com",
            "BANQUE.FR",
            "« banque.fr »",
            "(banque.fr)",
            "banque.fr,",
            "banque\u{FF0E}fr",
            "banque\u{3002}fr",
            "\u{FF42}\u{FF41}\u{FF4E}\u{FF51}\u{FF55}\u{FF45}.fr",
            "société-générale.fr",
            "banque\u{2024}fr",
            "banque\u{FE52}fr",
            "banque\u{200B}.fr",
            "X.com",
            "t.co",
            "sberbank.xn--p1ai",
        ] {
            assert!(names_a_domain(name), "{name}");
        }
        for name in [
            "Jean-Pierre",
            "Dr. Martin",
            "Saint-Étienne",
            "A. Dupont",
            "J.-P. Sartre",
            "M.Dupont",
            "J.R.R.Tolkien",
            "Version 2.0",
            "3.14",
            "Fin.",
            ".fr",
            "a..fr",
            "Alice Martin",
            "",
        ] {
            assert!(!names_a_domain(name), "{name}");
        }
    }

    #[test]
    fn a_header_name_containing_a_domain_shows_the_address_too() {
        let label = sender_label(&header("Banque.fr", "pirate@exemple.net"), None);
        assert_eq!(label.name, "Banque.fr");
        assert!(label.show_address);
        assert!(!sender_label(&header("Bob", "bob@example.com"), None).show_address);
        // Nom écarté : l'adresse s'affiche déjà seule.
        assert!(!sender_label(&header("support(at)banque.fr", "p@exemple.net"), None).show_address);
        // Nom du carnet : choisi par l'utilisateur.
        let shop = contact(
            11,
            ContactInput {
                display_name: "Boutique.fr".into(),
                ..ContactInput::default()
            },
            &["vente@boutique.fr"],
        );
        let label = sender_label(&header("Arnaque.fr", "vente@boutique.fr"), Some(&shop));
        assert_eq!(label.name, "Boutique.fr");
        assert!(!label.show_address);
    }

    #[test]
    fn a_header_name_with_a_textual_at_is_dropped() {
        let label = sender_label(&header("support(at)banque.fr", "pirate@exemple.net"), None);
        assert_eq!(label.name, "");
        assert_eq!(label.address, "pirate@exemple.net");
    }

    #[test]
    fn disguised_headers_still_show_the_address_or_drop_the_name() {
        // Chemin réel d'un en-tête : `Correspondent::new` puis le libellé.
        let shown = |name: &str| sender_label(&header(name, "pirate@exemple.net"), None);
        for name in [
            "Banque.f\u{034F}r",
            "Banque.f\u{FE0F}r",
            "Banque\u{034F}.fr",
            "Banque.f\u{E0061}r",
            "Banque.f\u{0332}r",
            "Banque.f\u{180B}r",
            "Banque.f\u{17B4}r",
            "banque\u{00B7}fr",
            "banque\u{A4F8}fr",
            "banque\u{06D4}fr",
            "banque\u{2219}fr",
            "banque\u{30FB}fr",
            "banque\u{0701}fr",
            "Paypal\u{22C5}com",
            "X.Com",
            "T.Co Support",
            "A.Bank",
            "\u{1D41B}\u{1D41A}\u{1D427}\u{1D42A}\u{1D42E}\u{1D41E}.fr",
        ] {
            let label = shown(name);
            assert!(label.show_address, "{name}");
            assert!(!label.name.is_empty(), "{name}");
        }
        for name in [
            "support(a\u{034F}t)banque",
            "support a\u{FE0F}t banque dot fr",
            "support \u{0430}t banque dot fr",
            "support \u{1D41A}\u{1D42D} banque point fr",
            "support \u{03B1}\u{03C4} banque dot fr",
            "support\u{FE59}at\u{FE5A}banque",
            "support\u{207D}at\u{207E}banque",
            "support\u{2768}at\u{2769}banque",
            "service \u{0430}t paypal d\u{043E}t com",
        ] {
            assert_eq!(shown(name).name, "", "{name}");
        }
    }

    #[test]
    fn a_bracketed_at_closing_a_name_is_a_country_not_an_address() {
        for name in ["Hans Müller (AT)", "Team [at]", "(at) Hans", "Équipe {AT}"] {
            assert!(!imitates_address(name), "{name}");
        }
    }

    #[test]
    fn the_contact_badge_needs_an_authenticated_sender() {
        let alice = alice();
        let shown =
            |auth| sender_label_with_auth(&header("", "alice@example.com"), Some(&alice), auth);
        let pass = shown(SenderAuth::Pass);
        assert!(pass.contact_badge);
        assert_eq!(pass.auth, SenderAuth::Pass);
        for auth in [SenderAuth::Fail, SenderAuth::Unknown] {
            let label = shown(auth);
            assert!(!label.contact_badge, "{auth:?}");
            assert_eq!(label.auth, auth);
            // Le nom du carnet et le menu du contact restent.
            assert_eq!(label.name, "Alice Martin");
            assert_eq!(label.contact, id(7));
        }
        // Hors du carnet : jamais de pastille, le verdict est transmis.
        let stranger =
            sender_label_with_auth(&header("Bob", "bob@example.com"), None, SenderAuth::Pass);
        assert!(!stranger.contact_badge);
        assert_eq!(stranger.auth, SenderAuth::Pass);
        // Sans verdict (destinataires, listes) : aucune pastille.
        assert!(!sender_label(&header("", "alice@example.com"), Some(&alice)).contact_badge);
        let names = SenderNames::new(vec![alice.clone()]);
        assert!(
            names
                .sender(&header("", "alice@example.com"), SenderAuth::Pass)
                .contact_badge
        );
        assert!(
            !names
                .sender(&header("", "alice@example.com"), SenderAuth::Fail)
                .contact_badge
        );
        assert!(!names.label(&header("", "alice@example.com")).contact_badge);
    }

    #[test]
    fn without_contact_the_header_name_is_kept() {
        let label = sender_label(&header("Bob", "bob@example.com"), None);
        assert_eq!(
            label,
            SenderLabel {
                name: "Bob".into(),
                address: "bob@example.com".into(),
                contact: None,
                show_address: false,
                auth: SenderAuth::Unknown,
                contact_badge: false,
            }
        );
    }

    /// Le nom de l'en-tête sert à préremplir une nouvelle fiche : il en
    /// respecte les règles, sans caractère invisible.
    #[test]
    fn header_name_loses_invisible_characters() {
        let label = sender_label(&header("Bou\u{FE0F}tique\u{2800}", "b@example.com"), None);
        assert_eq!(label.name, "Boutique");
        let blank = sender_label(&header("\u{2800}\u{3164}", "b@example.com"), None);
        assert_eq!(blank.name, "");
    }

    #[test]
    fn a_header_name_imitating_an_address_is_dropped() {
        let label = sender_label(&header("support@banque.fr", "pirate@exemple.net"), None);
        assert_eq!(label.name, "");
        assert_eq!(label.address, "pirate@exemple.net");
    }

    #[test]
    fn the_contact_name_wins_when_the_address_matches_its_key() {
        let alice = alice();
        let label = sender_label(&header("alice", "Alice@EXAMPLE.com"), Some(&alice));
        assert_eq!(label.name, "Alice Martin");
        assert_eq!(label.address, "Alice@EXAMPLE.com");
        assert_eq!(label.contact, id(7));
        // Une autre adresse de la même fiche nomme aussi le contact.
        let other = sender_label(&header("", "a.martin@travail.fr"), Some(&alice));
        assert_eq!(other.name, "Alice Martin");
        assert_eq!(other.contact, id(7));
    }

    #[test]
    fn a_contact_that_does_not_carry_the_address_is_ignored() {
        let alice = alice();
        let label = sender_label(&header("Alice", "alice@exemple.com"), Some(&alice));
        assert_eq!(label.name, "Alice");
        assert_eq!(label.contact, None);
        // Adresse d'en-tête invalide : jamais rapprochée d'un contact.
        let invalid = sender_label(&header("Alice", "alice@"), Some(&alice));
        assert_eq!(invalid.contact, None);
    }

    #[test]
    fn a_contact_without_any_name_leaves_the_address_alone() {
        // Le nom affiché par défaut d'une fiche sans nom est son adresse
        // préférée, qui peut différer de l'adresse de l'en-tête.
        let anonymous = contact(
            3,
            ContactInput::default(),
            &["prefere@example.com", "autre@example.com"],
        );
        let label = sender_label(&header("Usurpé", "autre@example.com"), Some(&anonymous));
        assert_eq!(label.name, "");
        assert_eq!(label.contact, id(3));
        let named = contact(
            4,
            ContactInput {
                nickname: "Zoé".into(),
                ..ContactInput::default()
            },
            &["zoe@example.com"],
        );
        assert_eq!(
            sender_label(&header("", "zoe@example.com"), Some(&named)).name,
            "Zoé"
        );
    }

    #[test]
    fn a_contact_name_imitating_an_address_is_dropped() {
        let fake = contact(
            5,
            ContactInput {
                display_name: "ceo@banque.fr".into(),
                ..ContactInput::default()
            },
            &["pirate@exemple.net"],
        );
        let label = sender_label(&header("", "pirate@exemple.net"), Some(&fake));
        assert_eq!(label.name, "");
        assert_eq!(label.contact, id(5));
        assert_eq!(contact_name(&fake), None);
    }

    #[test]
    fn sender_keys_are_distinct_valid_and_in_order() {
        let people = [
            header("B", "bob@example.com"),
            header("x", "pas une adresse"),
            header("A", "Alice@Example.com"),
            header("B2", "BOB@example.com"),
            header("", ""),
        ];
        let keys: Vec<String> = sender_keys(&people)
            .iter()
            .map(|k| k.as_str().to_owned())
            .collect();
        assert_eq!(keys, ["bob@example.com", "alice@example.com"]);
    }

    #[test]
    fn sender_keys_are_bounded() {
        let people: Vec<Correspondent> = (0..MAX_CONTACT_LOOKUP + 10)
            .map(|i| header("", &format!("p{i}@example.com")))
            .collect();
        let keys = sender_keys(&people);
        assert_eq!(keys.len(), MAX_CONTACT_LOOKUP);
        assert_eq!(keys[0].as_str(), "p0@example.com");
    }

    #[test]
    fn sender_names_find_contacts_by_address() {
        let bob = contact(
            9,
            ContactInput {
                given_name: "Robert".into(),
                family_name: "Durand".into(),
                ..ContactInput::default()
            },
            &["bob@example.com"],
        );
        let names = SenderNames::new(vec![alice(), bob]);
        assert_eq!(
            names.label(&header("bob", "Bob@example.com")).name,
            "Robert Durand"
        );
        assert_eq!(
            names.label(&header("", "a.martin@travail.fr")).contact,
            id(7)
        );
        let stranger = names.label(&header("Inconnu", "inconnu@example.com"));
        assert_eq!(stranger.name, "Inconnu");
        assert_eq!(stranger.contact, None);
        assert!(
            SenderNames::default()
                .contact_for(&header("", "bob@example.com"))
                .is_none()
        );
    }
}
