//! Doublons du carnet d'adresses (ADR-012) : contacts qui semblent
//! désigner la même personne ([`find_duplicates`]) et fusion de leurs
//! fiches ([`merge_contacts`]).
//!
//! Deux contacts ne partagent jamais une adresse (le carnet le refuse) :
//! sont proposés comme doublons les contacts de même nom ou de même
//! numéro de téléphone. Ce ne sont que des suggestions, que l'utilisateur
//! confirme groupe par groupe.

use std::collections::{HashMap, HashSet};

use crate::contact_import::phone_key;
use crate::contacts::{
    AddressKey, Contact, ContactDraft, ContactError, ContactId, EmailInput, MAX_CONTACT_EMAILS,
    PhoneInput,
};
use crate::ports::TextNormalizer;
use crate::sender::imitates_address;

/// Nombre minimal de chiffres d'un numéro pour rapprocher deux contacts
/// (« 112 » ou « 3615 » ne désignent personne).
pub const MIN_DUPLICATE_PHONE_DIGITS: usize = 6;

/// Nombre maximal de fiches absorbées par une fusion : chacune apporte au
/// moins une adresse, la fiche gardée en a déjà une.
pub const MAX_MERGED_CONTACTS: usize = MAX_CONTACT_EMAILS - 1;

/// Ce qui rapproche les contacts d'un groupe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DuplicateReason {
    /// Même nom affiché (mots sans casse ni ordre).
    SameName,
    /// Même numéro de téléphone.
    SamePhone,
}

/// Groupe de contacts qui semblent désigner la même personne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateGroup {
    /// Contacts du groupe (au moins deux) : la fiche proposée à garder
    /// (la plus complète) d'abord, puis les autres par identifiant.
    pub contacts: Vec<ContactId>,
    /// Raisons du rapprochement, chacune une fois, dans l'ordre de
    /// [`DuplicateReason`].
    pub reasons: Vec<DuplicateReason>,
}

/// Groupes de doublons parmi `contacts` : contacts de même [`name_key`]
/// ou qui partagent un numéro d'au moins [`MIN_DUPLICATE_PHONE_DIGITS`]
/// chiffres, regroupés par transitivité. Groupes dans l'ordre du nom de
/// leur première fiche ([`ContactDraft::sort_key`]), puis de son
/// identifiant.
#[must_use]
pub fn find_duplicates(
    contacts: &[Contact],
    normalizer: &impl TextNormalizer,
) -> Vec<DuplicateGroup> {
    let mut sets = DisjointSets::new(contacts.len());
    let mut names: HashMap<String, usize> = HashMap::new();
    let mut numbers: HashMap<String, usize> = HashMap::new();
    // Paires rapprochées, avec leur raison (pour les raisons du groupe).
    let mut links: Vec<(usize, DuplicateReason)> = Vec::new();
    for (index, contact) in contacts.iter().enumerate() {
        let draft = contact.draft();
        if let Some(key) = name_key(draft, normalizer) {
            match names.get(&key) {
                Some(&first) => {
                    sets.union(first, index);
                    links.push((index, DuplicateReason::SameName));
                }
                None => {
                    names.insert(key, index);
                }
            }
        }
        let mut own = HashSet::new();
        for p in draft.phones() {
            let key = phone_key(p.number());
            let digits = key.chars().filter(char::is_ascii_digit).count();
            if digits < MIN_DUPLICATE_PHONE_DIGITS || !own.insert(key.clone()) {
                continue;
            }
            match numbers.get(&key) {
                Some(&first) => {
                    sets.union(first, index);
                    links.push((index, DuplicateReason::SamePhone));
                }
                None => {
                    numbers.insert(key, index);
                }
            }
        }
    }

    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    for index in 0..contacts.len() {
        members.entry(sets.find(index)).or_default().push(index);
    }
    let mut reasons: HashMap<usize, Vec<DuplicateReason>> = HashMap::new();
    for (index, reason) in links {
        let found = reasons.entry(sets.find(index)).or_default();
        if !found.contains(&reason) {
            found.push(reason);
        }
    }

    let mut groups: Vec<(String, DuplicateGroup)> = members
        .into_iter()
        .filter(|(_, indexes)| indexes.len() > 1)
        .map(|(root, indexes)| {
            let mut group: Vec<&Contact> = indexes.iter().map(|&i| &contacts[i]).collect();
            group.sort_by_key(|c| c.id());
            let keep = group
                .iter()
                .enumerate()
                .max_by_key(|(_, c)| {
                    (
                        completeness(c.draft()),
                        c.draft().favorite(),
                        std::cmp::Reverse(c.id()),
                    )
                })
                .map_or(0, |(i, _)| i);
            let first = group.remove(keep);
            let mut reasons = reasons.remove(&root).unwrap_or_default();
            reasons.sort();
            let sort_key = first.draft().sort_key();
            let contacts = std::iter::once(first)
                .chain(group)
                .map(Contact::id)
                .collect();
            (sort_key, DuplicateGroup { contacts, reasons })
        })
        .collect();
    groups.sort_by(|a, b| (&a.0, a.1.contacts[0]).cmp(&(&b.0, b.1.contacts[0])));
    groups.into_iter().map(|(_, group)| group).collect()
}

/// Ensembles disjoints (union-find) sur les rangs `0..n`.
struct DisjointSets {
    parent: Vec<usize>,
}

impl DisjointSets {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent[b.max(a)] = a.min(b);
        }
    }
}

/// Clé de comparaison du nom d'une fiche : mots du nom affiché en NFC et
/// en minuscules, triés. `None` pour une fiche sans nom (seule l'adresse
/// s'afficherait) ou dont le nom imite une adresse.
#[must_use]
pub fn name_key(draft: &ContactDraft, normalizer: &impl TextNormalizer) -> Option<String> {
    let named = draft.display_name_field().is_some()
        || draft.given_name().is_some()
        || draft.family_name().is_some()
        || draft.nickname().is_some();
    if !named {
        return None;
    }
    let name = normalizer.nfc(&draft.display_name()).to_lowercase();
    if imitates_address(&name) {
        return None;
    }
    let mut words: Vec<&str> = name.split_whitespace().collect();
    words.sort_unstable();
    (!words.is_empty()).then(|| words.join(" "))
}

/// Degré de remplissage d'une fiche : champs de nom, organisation et notes
/// renseignés, plus ses adresses et téléphones.
#[must_use]
pub fn completeness(draft: &ContactDraft) -> usize {
    let fields = [
        draft.display_name_field(),
        draft.given_name(),
        draft.family_name(),
        draft.nickname(),
        draft.organization(),
        draft.notes(),
    ];
    fields.iter().flatten().count() + draft.emails().len() + draft.phones().len()
}

/// Fiche issue de la fusion de `others` dans `keep` : champs vides de
/// `keep` complétés dans l'ordre de `others` (rien n'est écrasé), adresses
/// réunies (la préférée de `keep` le reste), téléphones réunis au chiffre
/// près, notes distinctes mises bout à bout (une ligne vide entre deux),
/// favori si l'une des fiches l'est. Refusée par [`ContactDraft::new`] si
/// elle dépasse les bornes d'une fiche (adresses, téléphones, notes).
pub fn merge_contacts(
    keep: &ContactDraft,
    others: &[&ContactDraft],
) -> Result<ContactDraft, ContactError> {
    let mut input = keep.to_input();
    let fill = |field: &mut String, value: Option<&str>| {
        if let Some(value) = value
            && field.is_empty()
        {
            value.clone_into(field);
        }
    };
    let mut notes: Vec<&str> = keep.notes().into_iter().collect();
    let mut numbers: HashSet<String> = keep
        .phones()
        .iter()
        .map(|p| phone_key(p.number()))
        .collect();
    let mut keys: HashSet<AddressKey> = keep.emails().iter().map(|e| e.key().clone()).collect();
    for other in others {
        fill(&mut input.display_name, other.display_name_field());
        fill(&mut input.given_name, other.given_name());
        fill(&mut input.family_name, other.family_name());
        fill(&mut input.nickname, other.nickname());
        fill(&mut input.organization, other.organization());
        if let Some(note) = other.notes()
            && !notes.contains(&note)
        {
            notes.push(note);
        }
        for e in other.emails() {
            if keys.insert(e.key().clone()) {
                input.emails.push(EmailInput {
                    address: e.address().to_string(),
                    label: e.label().unwrap_or_default().to_owned(),
                    preferred: false,
                });
            }
        }
        for p in other.phones() {
            if numbers.insert(phone_key(p.number())) {
                input.phones.push(PhoneInput {
                    number: p.number().to_owned(),
                    label: p.label().unwrap_or_default().to_owned(),
                });
            }
        }
        input.favorite |= other.favorite();
    }
    input.notes = notes.join("\n\n");
    ContactDraft::new(input)
}

/// Fusion demandée refusée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MergeSelectionError {
    #[error("identifiant de contact invalide")]
    InvalidId,
    #[error("aucune fiche à fusionner")]
    Empty,
    #[error("plus de {MAX_MERGED_CONTACTS} fiches à fusionner")]
    TooMany,
    #[error("la fiche gardée figure parmi les fiches absorbées")]
    KeepAbsorbed,
}

/// Fusion demandée : la fiche gardée et les fiches absorbées (distinctes,
/// dans l'ordre de la première apparition).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeSelection {
    keep: ContactId,
    others: Vec<ContactId>,
}

impl MergeSelection {
    /// Valide une demande reçue de l'interface : identifiants valides, au
    /// moins une et au plus [`MAX_MERGED_CONTACTS`] fiches absorbées
    /// distinctes, sans la fiche gardée.
    pub fn new<T: Copy + Into<i64>>(keep: T, others: &[T]) -> Result<Self, MergeSelectionError> {
        let id = |raw: T| ContactId::new(raw.into()).ok_or(MergeSelectionError::InvalidId);
        let keep = id(keep)?;
        if others.len() > 2 * MAX_MERGED_CONTACTS {
            return Err(MergeSelectionError::TooMany);
        }
        let mut distinct: Vec<ContactId> = Vec::new();
        for &raw in others {
            let other = id(raw)?;
            if other == keep {
                return Err(MergeSelectionError::KeepAbsorbed);
            }
            if !distinct.contains(&other) {
                distinct.push(other);
            }
        }
        if distinct.is_empty() {
            return Err(MergeSelectionError::Empty);
        }
        if distinct.len() > MAX_MERGED_CONTACTS {
            return Err(MergeSelectionError::TooMany);
        }
        Ok(Self {
            keep,
            others: distinct,
        })
    }

    #[must_use]
    pub fn keep(&self) -> ContactId {
        self.keep
    }

    #[must_use]
    pub fn others(&self) -> &[ContactId] {
        &self.others
    }
}

#[cfg(test)]
mod tests;
