//! Listes de diffusion du carnet (ADR-012) : liste validée, nom unique
//! sans la casse, expansion en destinataires.
//!
//! Une liste regroupe des contacts du carnet, pas des adresses : à chaque
//! envoi, chaque membre est écrit à son adresse préférée du moment
//! ([`expand`]). Une liste peut être vide (contacts supprimés depuis) ;
//! elle ne peut pas être envoyée vide. Une liste « en Cci » se déplie
//! toujours en copie cachée : les membres ne voient pas les adresses des
//! autres.

use std::collections::HashSet;
use std::fmt;

use crate::compose::draft::{MAX_RECIPIENTS, Recipient};
use crate::connection::is_format_char;
use crate::contacts::{Contact, ContactId, MAX_CONTACT_FILTER_CHARS, MAX_CONTACT_NAME_CHARS};
use crate::mailbox::header_text;
use crate::ports::ContactListSummary;
use crate::sender::contact_name;

/// Longueur maximale du nom d'une liste, en caractères.
pub const MAX_CONTACT_LIST_NAME_CHARS: usize = MAX_CONTACT_NAME_CHARS;
/// Longueur maximale de la description d'une liste, en caractères.
pub const MAX_CONTACT_LIST_DESCRIPTION_CHARS: usize = 1000;
/// Listes proposées au plus pendant la saisie d'un destinataire.
pub const MAX_LIST_SUGGESTIONS: usize = 5;
/// Membres au plus : une liste plus longue ne pourrait pas partir en un
/// message ([`MAX_RECIPIENTS`]).
pub const MAX_CONTACT_LIST_MEMBERS: usize = MAX_RECIPIENTS;

/// Identifiant local d'une liste, attribué par le carnet (strictement
/// positif).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContactListId(i64);

impl ContactListId {
    #[must_use]
    pub fn new(raw: i64) -> Option<Self> {
        (raw > 0).then_some(Self(raw))
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl fmt::Display for ContactListId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Champ d'une liste, pour situer une erreur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactListField {
    Name,
    Description,
}

/// Liste refusée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContactListError {
    #[error("une liste a un nom")]
    NoName,
    #[error("champ {0:?} trop long")]
    TooLong(ContactListField),
    #[error("champ {0:?} avec un caractère interdit")]
    InvalidCharacter(ContactListField),
    #[error("identifiant de membre invalide")]
    InvalidMember,
    #[error("plus de {MAX_CONTACT_LIST_MEMBERS} membres")]
    TooManyMembers,
    #[error("liste sans membre")]
    Empty,
}

/// Liste saisie, non validée. Les membres sont des identifiants de
/// contacts bruts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactListInput {
    pub name: String,
    pub description: String,
    pub members: Vec<i64>,
    pub bcc: bool,
}

/// Liste valide, pas encore enregistrée (ou contenu d'une liste
/// enregistrée).
///
/// Invariants : nom non vide, sans espaces de bord, borné, sans caractère
/// de contrôle ni de mise en forme invisible ; description de même (vide :
/// absente) ; membres valides, chacun une fois, au plus
/// [`MAX_CONTACT_LIST_MEMBERS`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactListDraft {
    name: String,
    description: Option<String>,
    members: Vec<ContactId>,
    bcc: bool,
}

impl ContactListDraft {
    /// Valide une liste. Un membre cité plusieurs fois l'est une fois, à
    /// sa première place.
    pub fn new(input: ContactListInput) -> Result<Self, ContactListError> {
        let name = text(
            &input.name,
            ContactListField::Name,
            MAX_CONTACT_LIST_NAME_CHARS,
        )?
        .ok_or(ContactListError::NoName)?;
        let description = text(
            &input.description,
            ContactListField::Description,
            MAX_CONTACT_LIST_DESCRIPTION_CHARS,
        )?;
        // Plus de deux fois la borne (doublons compris) : refusé sans
        // parcourir la liste.
        if input.members.len() > 2 * MAX_CONTACT_LIST_MEMBERS {
            return Err(ContactListError::TooManyMembers);
        }
        let ids = input
            .members
            .iter()
            .map(|&raw| ContactId::new(raw).ok_or(ContactListError::InvalidMember))
            .collect::<Result<Vec<_>, _>>()?;
        let members = append_members(Vec::new(), &ids)?;
        Ok(Self {
            name,
            description,
            members,
            bcc: input.bcc,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Contacts membres, dans l'ordre de la liste.
    #[must_use]
    pub fn members(&self) -> &[ContactId] {
        &self.members
    }

    /// Membres dépliés en copie cachée.
    #[must_use]
    pub fn bcc(&self) -> bool {
        self.bcc
    }

    /// Clé d'unicité et de tri : le nom en minuscules. Deux listes de même
    /// clé ne peuvent pas coexister dans le carnet.
    #[must_use]
    pub fn name_key(&self) -> String {
        self.name.to_lowercase()
    }

    /// Liste augmentée des contacts `added` absents, à la suite ; refusée
    /// au-delà de [`MAX_CONTACT_LIST_MEMBERS`].
    pub fn with_members(&self, added: &[ContactId]) -> Result<Self, ContactListError> {
        Ok(Self {
            members: append_members(self.members.clone(), added)?,
            ..self.clone()
        })
    }

    /// Liste à modifier (formulaire pré-rempli) : `ContactListDraft::new`
    /// rend la même liste.
    #[must_use]
    pub fn to_input(&self) -> ContactListInput {
        ContactListInput {
            name: self.name.clone(),
            description: self.description.clone().unwrap_or_default(),
            members: self.members.iter().map(|id| id.get()).collect(),
            bcc: self.bcc,
        }
    }

    /// Liste enregistrée sous `id`.
    #[must_use]
    pub fn with_id(self, id: ContactListId) -> ContactList {
        ContactList { id, draft: self }
    }
}

/// Liste enregistrée dans le carnet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactList {
    id: ContactListId,
    draft: ContactListDraft,
}

impl ContactList {
    #[must_use]
    pub fn id(&self) -> ContactListId {
        self.id
    }

    #[must_use]
    pub fn draft(&self) -> &ContactListDraft {
        &self.draft
    }
}

/// Filtre de la liste des listes : texte cherché dans le nom.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactListFilter {
    text: String,
}

impl ContactListFilter {
    /// Filtre sur `text` (sans espaces de bord, sans la casse, borné à
    /// [`MAX_CONTACT_FILTER_CHARS`] ; vide : toutes).
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            text: header_text(text, MAX_CONTACT_FILTER_CHARS).to_lowercase(),
        }
    }

    /// Texte recherché, en minuscules ; cherché tel quel dans
    /// [`ContactListDraft::name_key`].
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn matches(&self, list: &ContactListDraft) -> bool {
        self.text.is_empty() || list.name_key().contains(&self.text)
    }
}

/// Destinataires d'une liste dépliée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    /// Un par adresse, dans l'ordre de la liste.
    pub recipients: Vec<Recipient>,
    /// À mettre en copie cachée.
    pub bcc: bool,
}

/// Déplie `list` : chaque membre encore dans `contacts` (fiches lues du
/// carnet), à son adresse préférée, sous son nom du carnet (vide s'il n'en
/// a pas ou s'il imite une adresse) ; chaque adresse une fois
/// ([`crate::contacts::AddressKey`]). Un membre absent de `contacts`
/// (supprimé) est ignoré ; sans destinataire : [`ContactListError::Empty`].
pub fn expand(
    list: &ContactListDraft,
    contacts: &[Contact],
) -> Result<Expansion, ContactListError> {
    let mut seen = HashSet::new();
    let mut recipients = Vec::new();
    for &member in &list.members {
        let Some(contact) = contacts.iter().find(|c| c.id() == member) else {
            continue;
        };
        let email = contact.draft().preferred_email();
        if !seen.insert(email.key().clone()) {
            continue;
        }
        recipients.push(Recipient::new(
            &contact_name(contact).unwrap_or_default(),
            email.address().clone(),
        ));
    }
    if recipients.is_empty() {
        return Err(ContactListError::Empty);
    }
    Ok(Expansion {
        recipients,
        bcc: list.bcc,
    })
}

/// Listes proposées pendant la saisie de `text` : celles de `lists` (déjà
/// filtrées par le nom, dans l'ordre du carnet) qui ont au moins un membre
/// (une liste vide ne peut pas être envoyée), au plus `limit` (borné à
/// [`MAX_LIST_SUGGESTIONS`]) ; texte vide : aucune.
#[must_use]
pub fn list_suggestions(
    text: &str,
    lists: &[ContactListSummary],
    limit: usize,
) -> Vec<ContactListSummary> {
    if ContactListFilter::new(text).text().is_empty() {
        return Vec::new();
    }
    lists
        .iter()
        .filter(|l| l.members > 0)
        .take(limit.min(MAX_LIST_SUGGESTIONS))
        .cloned()
        .collect()
}

/// `members` suivis des contacts `added` absents, chacun une fois ;
/// refusé au-delà de [`MAX_CONTACT_LIST_MEMBERS`].
fn append_members(
    mut members: Vec<ContactId>,
    added: &[ContactId],
) -> Result<Vec<ContactId>, ContactListError> {
    let mut seen: HashSet<ContactId> = members.iter().copied().collect();
    for &id in added {
        if seen.insert(id) {
            if members.len() == MAX_CONTACT_LIST_MEMBERS {
                return Err(ContactListError::TooManyMembers);
            }
            members.push(id);
        }
    }
    Ok(members)
}

/// Champ d'une ligne : sans espaces de bord, borné, sans caractère de
/// contrôle ni de mise en forme ; vide : absent.
fn text(
    raw: &str,
    field: ContactListField,
    max: usize,
) -> Result<Option<String>, ContactListError> {
    let value = raw.trim();
    if value.chars().any(|c| c.is_control() || is_format_char(c)) {
        return Err(ContactListError::InvalidCharacter(field));
    }
    if value.chars().count() > max {
        return Err(ContactListError::TooLong(field));
    }
    Ok((!value.is_empty()).then(|| value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::{ContactDraft, ContactInput, EmailInput};

    fn input(name: &str, members: &[i64]) -> ContactListInput {
        ContactListInput {
            name: name.to_owned(),
            members: members.to_vec(),
            ..ContactListInput::default()
        }
    }

    fn draft(input: ContactListInput) -> ContactListDraft {
        ContactListDraft::new(input).unwrap_or_else(|e| panic!("liste refusée : {e}"))
    }

    fn cid(raw: i64) -> ContactId {
        ContactId::new(raw).unwrap_or_else(|| panic!("identifiant {raw}"))
    }

    fn contact(id: i64, name: &str, emails: &[(&str, bool)]) -> Contact {
        ContactDraft::new(ContactInput {
            display_name: name.to_owned(),
            emails: emails
                .iter()
                .map(|(a, preferred)| EmailInput {
                    address: (*a).to_owned(),
                    preferred: *preferred,
                    ..EmailInput::default()
                })
                .collect(),
            ..ContactInput::default()
        })
        .unwrap_or_else(|e| panic!("fiche refusée : {e}"))
        .with_id(cid(id))
    }

    fn sent(e: &Expansion) -> Vec<(&str, String)> {
        e.recipients
            .iter()
            .map(|r| (r.name.as_str(), r.address.to_string()))
            .collect()
    }

    #[test]
    fn list_id_is_strictly_positive() {
        assert_eq!(ContactListId::new(0), None);
        assert_eq!(ContactListId::new(-1), None);
        assert_eq!(ContactListId::new(4).map(ContactListId::get), Some(4));
        assert_eq!(
            ContactListId::new(4).map(|i| i.to_string()),
            Some("4".into())
        );
    }

    #[test]
    fn valid_list_is_trimmed_and_keeps_its_fields() {
        let d = draft(ContactListInput {
            name: "  Club de lecture ".into(),
            description: " Le jeudi soir ".into(),
            members: vec![3, 1],
            bcc: true,
        });
        assert_eq!(d.name(), "Club de lecture");
        assert_eq!(d.description(), Some("Le jeudi soir"));
        assert_eq!(d.members(), [cid(3), cid(1)]);
        assert!(d.bcc());
        assert_eq!(ContactListDraft::new(d.to_input()), Ok(d));
    }

    #[test]
    fn empty_description_is_absent_and_empty_list_is_valid() {
        let d = draft(input("Famille", &[]));
        assert_eq!(d.description(), None);
        assert!(d.members().is_empty());
        assert!(!d.bcc());
    }

    #[test]
    fn name_is_required() {
        assert_eq!(
            ContactListDraft::new(input("   ", &[1])),
            Err(ContactListError::NoName)
        );
    }

    #[test]
    fn texts_are_bounded_and_without_control_characters() {
        let long = "a".repeat(MAX_CONTACT_LIST_NAME_CHARS + 1);
        assert_eq!(
            ContactListDraft::new(input(&long, &[])),
            Err(ContactListError::TooLong(ContactListField::Name))
        );
        let exact = "é".repeat(MAX_CONTACT_LIST_NAME_CHARS);
        assert!(ContactListDraft::new(input(&exact, &[])).is_ok());
        for bad in ["Fa\nmille", "Fa\u{202E}mille", "Fa\u{7}mille"] {
            assert_eq!(
                ContactListDraft::new(input(bad, &[])),
                Err(ContactListError::InvalidCharacter(ContactListField::Name)),
                "{bad:?}"
            );
        }
        let described = |description: String| ContactListInput {
            description,
            ..input("Famille", &[])
        };
        assert_eq!(
            ContactListDraft::new(described(
                "d".repeat(MAX_CONTACT_LIST_DESCRIPTION_CHARS + 1)
            )),
            Err(ContactListError::TooLong(ContactListField::Description))
        );
        assert!(
            ContactListDraft::new(described("d".repeat(MAX_CONTACT_LIST_DESCRIPTION_CHARS)))
                .is_ok()
        );
        assert_eq!(
            ContactListDraft::new(described("une\nligne".into())),
            Err(ContactListError::InvalidCharacter(
                ContactListField::Description
            ))
        );
    }

    #[test]
    fn members_are_valid_ids_each_once_and_bounded() {
        assert_eq!(
            ContactListDraft::new(input("L", &[1, 0])),
            Err(ContactListError::InvalidMember)
        );
        assert_eq!(
            ContactListDraft::new(input("L", &[-2])),
            Err(ContactListError::InvalidMember)
        );
        assert_eq!(
            draft(input("L", &[2, 1, 2, 1, 3])).members(),
            [cid(2), cid(1), cid(3)]
        );
        let full: Vec<i64> = (1..=MAX_CONTACT_LIST_MEMBERS as i64).collect();
        assert_eq!(
            draft(input("L", &full)).members().len(),
            MAX_CONTACT_LIST_MEMBERS
        );
        let mut repeated = full.clone();
        repeated.extend(&full);
        assert!(
            ContactListDraft::new(input("L", &repeated)).is_ok(),
            "doublons comptés une fois"
        );
        let mut over = full;
        over.push(MAX_CONTACT_LIST_MEMBERS as i64 + 1);
        assert_eq!(
            ContactListDraft::new(input("L", &over)),
            Err(ContactListError::TooManyMembers)
        );
    }

    #[test]
    fn oversized_input_is_refused_without_reading_it() {
        // Plus de deux fois la borne d'identifiants reçus (doublons
        // compris) : refusé d'emblée.
        let flood = vec![1; 2 * MAX_CONTACT_LIST_MEMBERS + 1];
        assert_eq!(
            ContactListDraft::new(input("L", &flood)),
            Err(ContactListError::TooManyMembers)
        );
    }

    #[test]
    fn name_key_ignores_case() {
        let a = draft(input("Club ÉTÉ", &[]));
        let b = draft(input("club été", &[]));
        assert_eq!(a.name_key(), "club été");
        assert_eq!(a.name_key(), b.name_key());
        assert_ne!(a.name_key(), draft(input("Club hiver", &[])).name_key());
    }

    #[test]
    fn with_members_appends_absent_contacts_within_the_bound() {
        let d = draft(input("L", &[1, 2]));
        let more = d
            .with_members(&[cid(2), cid(3), cid(3), cid(4)])
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(more.members(), [cid(1), cid(2), cid(3), cid(4)]);
        assert_eq!(more.name(), "L");
        let full: Vec<i64> = (1..=MAX_CONTACT_LIST_MEMBERS as i64).collect();
        let full = draft(input("L", &full));
        assert_eq!(
            full.with_members(&[cid(1)]).as_ref(),
            Ok(&full),
            "déjà membre"
        );
        assert_eq!(
            full.with_members(&[cid(1000)]),
            Err(ContactListError::TooManyMembers)
        );
    }

    #[test]
    fn filter_searches_the_name_without_case() {
        let d = ContactListDraft::new(ContactListInput {
            description: "voisins".into(),
            ..input("Club de Lecture", &[])
        })
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(ContactListFilter::new("").matches(&d));
        assert!(ContactListFilter::new(" LECT ").matches(&d));
        assert!(!ContactListFilter::new("voisins").matches(&d));
        assert!(!ContactListFilter::new("famille").matches(&d));
        assert_eq!(ContactListFilter::new("  Club ").text(), "club");
    }

    fn summary(id: i64, name: &str, members: usize) -> ContactListSummary {
        ContactListSummary {
            id: ContactListId::new(id).unwrap_or_else(|| panic!("{id}")),
            name: name.to_owned(),
            members,
            bcc: false,
        }
    }

    #[test]
    fn suggested_lists_have_members_and_are_bounded() {
        let lists = [
            summary(1, "Club", 3),
            summary(2, "Club vide", 0),
            summary(3, "Club bis", 1),
        ];
        let ids = |found: Vec<ContactListSummary>| -> Vec<i64> {
            found.iter().map(|l| l.id.get()).collect()
        };
        assert_eq!(ids(list_suggestions("cl", &lists, 10)), [1, 3]);
        assert_eq!(ids(list_suggestions("cl", &lists, 1)), [1]);
        assert!(list_suggestions("  ", &lists, 10).is_empty());
        let many: Vec<ContactListSummary> = (1..=20).map(|i| summary(i, "L", 1)).collect();
        assert_eq!(
            list_suggestions("l", &many, 100).len(),
            MAX_LIST_SUGGESTIONS
        );
    }

    #[test]
    fn expansion_writes_each_member_at_its_preferred_address() {
        let list = draft(input("Famille", &[2, 1]));
        let contacts = [
            contact(
                1,
                "Alice",
                &[("alice@example.com", false), ("alice@maison.fr", true)],
            ),
            contact(2, "Bob", &[("bob@example.net", false)]),
        ];
        let e = expand(&list, &contacts).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            sent(&e),
            [
                ("Bob", "bob@example.net".to_owned()),
                ("Alice", "alice@maison.fr".to_owned())
            ]
        );
        assert!(!e.bcc);
    }

    #[test]
    fn expansion_keeps_the_bcc_choice() {
        let list = draft(ContactListInput {
            bcc: true,
            ..input("Famille", &[1])
        });
        let e = expand(&list, &[contact(1, "Alice", &[("a@example.com", true)])]);
        assert_eq!(e.map(|e| e.bcc), Ok(true));
    }

    #[test]
    fn expansion_skips_deleted_members_and_refuses_an_empty_list() {
        let list = draft(input("Famille", &[1, 9]));
        let e = expand(&list, &[contact(1, "Alice", &[("a@example.com", true)])])
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(sent(&e), [("Alice", "a@example.com".to_owned())]);
        assert_eq!(expand(&list, &[]), Err(ContactListError::Empty));
        assert_eq!(
            expand(
                &draft(input("Vide", &[])),
                &[contact(1, "A", &[("a@example.com", true)])]
            ),
            Err(ContactListError::Empty)
        );
    }

    #[test]
    fn expansion_gives_each_address_once() {
        // Deux fiches de même identifiant (lecture incohérente) ou deux
        // adresses de même clé : une seule puce par adresse.
        let list = draft(input("L", &[1, 2]));
        let contacts = [
            contact(1, "Alice", &[("Alice@Example.com", true)]),
            contact(2, "Alice bis", &[("alice@example.com", true)]),
            contact(1, "Alice", &[("Alice@Example.com", true)]),
        ];
        let e = expand(&list, &contacts).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(sent(&e), [("Alice", "Alice@example.com".to_owned())]);
    }

    #[test]
    fn expansion_drops_names_that_imitate_an_address_or_are_missing() {
        let list = draft(input("L", &[1, 2]));
        let contacts = [
            contact(1, "patron@corp.com", &[("x@example.com", true)]),
            contact(2, "", &[("y@example.com", true)]),
        ];
        let e = expand(&list, &contacts).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            sent(&e),
            [
                ("", "x@example.com".to_owned()),
                ("", "y@example.com".to_owned())
            ]
        );
    }
}
