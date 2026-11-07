//! Nom affiché d'un correspondant (expéditeur, destinataire) : celui du
//! carnet d'adresses quand son adresse y figure (ADR-012), sinon celui de
//! l'en-tête, écarté s'il imite une adresse.
//!
//! Le nom du carnet s'affiche aussi pour une adresse `From` falsifiée : il
//! ne vaut pas preuve d'identité.

use std::collections::HashMap;

use crate::contacts::{AddressKey, Contact, ContactId};
use crate::mailbox::Correspondent;
use crate::ports::MAX_CONTACT_LOOKUP;

/// Correspondant tel qu'il s'affiche.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SenderLabel {
    /// Nom à afficher ; vide : l'adresse seule s'affiche.
    pub name: String,
    /// Adresse de l'en-tête, telle quelle.
    pub address: String,
    /// Contact du carnet qui porte l'adresse.
    pub contact: Option<ContactId>,
}

/// Caractères qui servent, ou ressemblent, à `@`, `<` et `>` (formes
/// pleine largeur, petites formes, guillemets et chevrons).
const ADDRESS_LOOK_ALIKES: [char; 17] = [
    '@', '<', '>', '\u{FF20}', '\u{FE6B}', '\u{FF1C}', '\u{FF1E}', '\u{FE64}', '\u{FE65}',
    '\u{2039}', '\u{203A}', '\u{2329}', '\u{232A}', '\u{27E8}', '\u{27E9}', '\u{3008}', '\u{3009}',
];

/// Vrai si un nom d'en-tête imite une adresse (`@`, `<`, `>` ou un
/// caractère qui leur ressemble) : il pourrait faire passer une adresse
/// pour une autre.
#[must_use]
pub fn imitates_address(name: &str) -> bool {
    name.contains(ADDRESS_LOOK_ALIKES)
}

/// Nom affiché d'un correspondant : celui de `contact` s'il porte
/// exactement l'adresse de l'en-tête (même clé), sinon le nom de
/// l'en-tête, vide s'il imite une adresse. Un contact sans nom (ni nom
/// affiché, ni prénom, ni nom, ni surnom) laisse l'adresse seule.
#[must_use]
pub fn sender_label(header: &Correspondent, contact: Option<&Contact>) -> SenderLabel {
    let known = contact.filter(|c| {
        AddressKey::parse(&header.address).is_ok_and(|key| c.draft().has_address(&key))
    });
    let name = match known {
        Some(c) => contact_name(c).unwrap_or_default(),
        None if imitates_address(&header.name) => String::new(),
        None => header.name.clone(),
    };
    SenderLabel {
        name,
        address: header.address.clone(),
        contact: known.map(Contact::id),
    }
}

/// Nom d'un contact, s'il en porte un (sinon son nom affiché serait son
/// adresse préférée, pas forcément celle du correspondant).
fn contact_name(contact: &Contact) -> Option<String> {
    let d = contact.draft();
    let named = d.display_name_field().is_some()
        || d.given_name().is_some()
        || d.family_name().is_some()
        || d.nickname().is_some();
    named.then(|| d.display_name())
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

    /// Nom affiché de `header` ([`sender_label`]).
    #[must_use]
    pub fn label(&self, header: &Correspondent) -> SenderLabel {
        sender_label(header, self.contact_for(header))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::{ContactDraft, ContactInput, EmailInput};

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
        let draft = ContactDraft::new(input).unwrap_or_else(|e| panic!("fiche refusée : {e}"));
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
        ] {
            assert!(imitates_address(name), "{name}");
        }
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
            }
        );
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
