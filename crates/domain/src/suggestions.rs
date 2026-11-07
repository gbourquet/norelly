//! Suggestions de destinataires (ADR-012) : contacts du carnet d'abord
//! (favoris en tête, sous le nom du carnet), puis adresses collectées dans
//! le courrier, une seule fois par adresse ([`AddressKey`]).

use std::collections::HashSet;

use crate::contacts::{AddressKey, Contact, ContactDraft, ContactFilter, ContactId};
use crate::mailbox::Correspondent;
use crate::ports::MAX_CONTACT_SUGGESTIONS;
use crate::sender::{contact_name, imitates_address};

/// Destinataire proposé pendant la saisie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipientSuggestion {
    /// Nom à afficher ; vide : l'adresse seule s'affiche.
    pub name: String,
    pub address: String,
    /// Contact du carnet qui porte l'adresse ; `None` : adresse collectée.
    pub contact: Option<ContactId>,
}

/// Suggestions pour `text` (sans distinction de casse ; vide : aucune), au
/// plus `limit` (borné à [`MAX_CONTACT_SUGGESTIONS`]).
///
/// - D'abord les `contacts` qui correspondent : favoris en tête, puis dans
///   l'ordre du carnet ([`ContactDraft::sort_key`]). Un contact dont un nom
///   (affiché, prénom, nom, surnom, organisation) contient `text` propose
///   toutes ses adresses, la préférée d'abord ; sinon, seules ses adresses
///   qui contiennent `text`. Le nom proposé est celui du carnet.
/// - Puis les adresses `collected` (déjà filtrées, dans leur ordre), sous
///   le nom de l'en-tête ; une adresse invalide est écartée.
/// - Chaque adresse une seule fois ([`AddressKey`]) : le carnet l'emporte.
/// - Un nom qui imite une adresse ([`imitates_address`]) est écarté.
#[must_use]
pub fn recipient_suggestions(
    text: &str,
    contacts: &[Contact],
    collected: &[Correspondent],
    limit: usize,
) -> Vec<RecipientSuggestion> {
    let limit = limit.min(MAX_CONTACT_SUGGESTIONS);
    let filter = ContactFilter::new(text, false);
    let needle = filter.text();
    let mut found = Vec::new();
    if needle.is_empty() || limit == 0 {
        return found;
    }

    let mut ordered: Vec<&Contact> = contacts.iter().collect();
    ordered.sort_by(|a, b| {
        let (da, db) = (a.draft(), b.draft());
        db.favorite()
            .cmp(&da.favorite())
            .then_with(|| da.sort_key().cmp(&db.sort_key()))
            .then_with(|| a.id().cmp(&b.id()))
    });
    let mut seen_contacts = HashSet::new();
    let mut seen = HashSet::new();
    for contact in ordered {
        if !seen_contacts.insert(contact.id()) {
            continue;
        }
        let draft = contact.draft();
        let name = contact_name(contact).unwrap_or_default();
        let by_name = names_contain(draft, &name, needle);
        let preferred = draft.preferred_email();
        let emails =
            std::iter::once(preferred).chain(draft.emails().iter().filter(|e| !e.preferred()));
        for email in emails {
            if !(by_name || email.key().as_str().contains(needle)) || !seen.insert(email.key().clone())
            {
                continue;
            }
            found.push(RecipientSuggestion {
                name: name.clone(),
                address: email.address().to_string(),
                contact: Some(contact.id()),
            });
            if found.len() == limit {
                return found;
            }
        }
    }

    for person in collected {
        let Ok(key) = AddressKey::parse(&person.address) else {
            continue;
        };
        if !seen.insert(key) {
            continue;
        }
        let name = if imitates_address(&person.name) {
            String::new()
        } else {
            person.name.clone()
        };
        found.push(RecipientSuggestion {
            name,
            address: person.address.clone(),
            contact: None,
        });
        if found.len() == limit {
            break;
        }
    }
    found
}

/// Un nom de la fiche (champs, ou nom affiché « prénom nom ») contient-il
/// `needle` (déjà en minuscules) ?
fn names_contain(draft: &ContactDraft, full_name: &str, needle: &str) -> bool {
    [
        draft.display_name_field(),
        draft.given_name(),
        draft.family_name(),
        draft.nickname(),
        draft.organization(),
        Some(full_name),
    ]
    .into_iter()
    .flatten()
    .any(|n| n.to_lowercase().contains(needle))
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

    fn named(id: i64, name: &str, emails: &[&str]) -> Contact {
        contact(
            id,
            ContactInput {
                display_name: name.into(),
                ..ContactInput::default()
            },
            emails,
        )
    }

    fn favorite(id: i64, name: &str, emails: &[&str]) -> Contact {
        contact(
            id,
            ContactInput {
                display_name: name.into(),
                favorite: true,
                ..ContactInput::default()
            },
            emails,
        )
    }

    fn collected(name: &str, address: &str) -> Correspondent {
        Correspondent::new(name, address)
    }

    fn addresses(found: &[RecipientSuggestion]) -> Vec<&str> {
        found.iter().map(|s| s.address.as_str()).collect()
    }

    fn id(raw: i64) -> Option<ContactId> {
        ContactId::new(raw)
    }

    #[test]
    fn contacts_come_first_under_their_carnet_name() {
        let alice = named(1, "Alice Martin", &["alice@example.com"]);
        let found = recipient_suggestions(
            "ali",
            &[alice],
            &[collected("Alicia", "alicia@example.com")],
            10,
        );
        assert_eq!(
            found,
            [
                RecipientSuggestion {
                    name: "Alice Martin".into(),
                    address: "alice@example.com".into(),
                    contact: id(1),
                },
                RecipientSuggestion {
                    name: "Alicia".into(),
                    address: "alicia@example.com".into(),
                    contact: None,
                },
            ]
        );
    }

    #[test]
    fn favorites_come_before_other_contacts_then_alphabetical_order() {
        let contacts = [
            named(1, "Zoé Martin", &["zoe@example.com"]),
            named(2, "Bob Martin", &["bob@example.com"]),
            favorite(3, "Yann Martin", &["yann@example.com"]),
        ];
        let found = recipient_suggestions("martin", &contacts, &[], 10);
        assert_eq!(
            addresses(&found),
            ["yann@example.com", "bob@example.com", "zoe@example.com"]
        );
    }

    #[test]
    fn an_address_appears_once_and_the_carnet_wins() {
        let alice = named(1, "Alice Martin", &["alice@example.com"]);
        let found = recipient_suggestions(
            "alice",
            // Le même contact peut venir de deux lectures du carnet
            // (favoris, puis tous).
            &[alice.clone(), alice],
            &[
                collected("alice", "ALICE@example.com"),
                collected("Al", "al.ice@example.com"),
                collected("Al bis", "Al.Ice@example.com"),
            ],
            10,
        );
        assert_eq!(
            addresses(&found),
            ["alice@example.com", "al.ice@example.com"]
        );
        assert_eq!(found[0].contact, id(1));
        assert_eq!(found[1].name, "Al");
    }

    #[test]
    fn a_name_match_proposes_every_address_preferred_first() {
        let alice = contact(
            1,
            ContactInput {
                given_name: "Alice".into(),
                emails: vec![],
                ..ContactInput::default()
            },
            &["a@travail.fr", "a@maison.fr"],
        );
        let alice = {
            let mut input = alice.draft().to_input();
            input.emails[0].preferred = false;
            input.emails[1].preferred = true;
            let draft = ContactDraft::new(input).unwrap_or_else(|e| panic!("{e}"));
            draft.with_id(alice.id())
        };
        let found = recipient_suggestions("alice", std::slice::from_ref(&alice), &[], 10);
        assert_eq!(addresses(&found), ["a@maison.fr", "a@travail.fr"]);
        assert!(found.iter().all(|s| s.name == "Alice"));
        // Recherche sur une adresse : seule celle qui correspond.
        let found = recipient_suggestions("travail", &[alice], &[], 10);
        assert_eq!(addresses(&found), ["a@travail.fr"]);
    }

    #[test]
    fn every_name_field_and_every_address_is_searched() {
        let robert = contact(
            1,
            ContactInput {
                given_name: "Robert".into(),
                family_name: "Durand".into(),
                nickname: "Bobby".into(),
                ..ContactInput::default()
            },
            &["rd@example.com", "robert.perso@exemple.fr"],
        );
        for text in ["robert", "DURAND", "bobby", "perso"] {
            let found = recipient_suggestions(text, std::slice::from_ref(&robert), &[], 10);
            assert!(!found.is_empty(), "{text}");
            assert_eq!(found[0].name, "Robert Durand", "{text}");
        }
        // Un contact qui ne correspond pas (lecture plus large) est écarté.
        assert!(recipient_suggestions("zoé", &[robert], &[], 10).is_empty());
    }

    #[test]
    fn an_empty_text_suggests_nothing() {
        let alice = named(1, "Alice", &["alice@example.com"]);
        let people = [collected("Alice", "alice@example.com")];
        assert!(recipient_suggestions("", std::slice::from_ref(&alice), &people, 10).is_empty());
        assert!(recipient_suggestions("   ", &[alice], &people, 10).is_empty());
    }

    #[test]
    fn suggestions_are_bounded() {
        let contacts: Vec<Contact> = (1..=15)
            .map(|i| {
                named(
                    i,
                    &format!("Martin {i:02}"),
                    &[&format!("m{i}@example.com")],
                )
            })
            .collect();
        let people: Vec<Correspondent> = (0..15)
            .map(|i| collected("Martin", &format!("c{i}@example.com")))
            .collect();
        let found = recipient_suggestions("martin", &contacts, &people, usize::MAX);
        assert_eq!(found.len(), MAX_CONTACT_SUGGESTIONS);
        assert!(found[..15].iter().all(|s| s.contact.is_some()));
        assert_eq!(
            recipient_suggestions("martin", &contacts, &people, 3).len(),
            3
        );
        assert!(recipient_suggestions("martin", &contacts, &people, 0).is_empty());
    }

    #[test]
    fn names_imitating_an_address_are_dropped() {
        let fake = named(1, "ceo@banque.fr", &["pirate@exemple.net"]);
        let found = recipient_suggestions(
            "pirate",
            &[fake],
            &[collected("support@banque.fr", "pirate2@exemple.net")],
            10,
        );
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|s| s.name.is_empty()), "{found:?}");
    }

    #[test]
    fn a_contact_without_name_shows_its_address_alone() {
        let anonymous = contact(1, ContactInput::default(), &["x@example.com"]);
        let found = recipient_suggestions("x@", &[anonymous], &[], 10);
        assert_eq!(found[0].name, "");
        assert_eq!(found[0].contact, id(1));
    }

    #[test]
    fn invalid_collected_addresses_are_skipped() {
        let found = recipient_suggestions(
            "bob",
            &[],
            &[
                collected("Bob", "bob@"),
                collected("Bob", "bob@example.com"),
            ],
            10,
        );
        assert_eq!(addresses(&found), ["bob@example.com"]);
    }
}
