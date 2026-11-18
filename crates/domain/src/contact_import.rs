//! Import d'un carnet d'adresses (ADR-012) : plan d'import des fiches et
//! des listes de diffusion lues dans un fichier vCard, CSV ou LDIF
//! ([`plan_import`]).
//!
//! L'adaptateur `contacts-io` traduit le fichier en fiches brutes
//! ([`ImportedCard`]) sans rien décider ; le plan décide : noms en forme
//! NFC, nom qui imite une adresse retiré, adresses vides ou répétées dans
//! une fiche écartées, caractère invisible refusé, fiche revalidée par [`ContactDraft::new`] (rejet
//! motivé sinon), doublons reconnus par [`AddressKey`] contre le carnet et
//! entre fiches du fichier, ignorés ou versés dans le contact existant
//! selon la [`DuplicatePolicy`]. Une liste est validée comme à la saisie
//! ([`ContactListDraft::new`]), ses membres retrouvés par leur adresse
//! parmi les contacts du carnet et les fiches importées ; une liste de
//! même nom qu'une liste du carnet (ou du fichier) est un doublon, ignoré
//! ou complété des membres qui lui manquent.

use std::collections::{HashMap, HashSet};

use crate::address::EmailAddressError;
use crate::contact_details::{Birthday, MAX_CONTACT_WEBSITES, MAX_POSTAL_ADDRESSES, website_url};
use crate::contact_lists::{
    ContactList, ContactListDraft, ContactListError, ContactListField, ContactListInput,
    MAX_CONTACT_LIST_MEMBERS,
};
use crate::contacts::{
    AddressKey, Contact, ContactDraft, ContactError, ContactField, ContactId, ContactInput,
    ContactPostalAddress, EmailInput, MAX_CONTACT_EMAILS, MAX_CONTACT_PHONES, PhoneInput,
    PostalAddressInput, WebsiteInput, is_invisible,
};
use crate::ports::{ContactStoreError, TextNormalizer};

/// Taille maximale d'un fichier de contacts importé, en octets.
pub const MAX_IMPORT_FILE_BYTES: usize = 20 * 1024 * 1024;
/// Nombre maximal de fiches d'un fichier importé.
pub const MAX_IMPORT_CARDS: usize = 25_000;
/// Nombre maximal de tentatives d'un import : un carnet qui a changé entre
/// la lecture et l'écriture ([`plan_outdated`]) fait relire le carnet et
/// recalculer le plan ; au-delà, l'import est abandonné sans rien écrire.
pub const MAX_IMPORT_ATTEMPTS: usize = 3;
/// Longueur maximale du repère d'une fiche rejetée, en caractères.
pub const MAX_IMPORT_HINT_CHARS: usize = 80;

/// Format reconnu d'un fichier de contacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactFormat {
    /// vCard 2.1, 3.0 ou 4.0 (Apple, Android, Outlook, Gmail).
    VCard,
    /// CSV de Google Contacts.
    GoogleCsv,
    /// CSV d'Outlook.
    OutlookCsv,
    /// CSV de Thunderbird.
    ThunderbirdCsv,
    /// LDIF (Thunderbird).
    Ldif,
}

/// Liste de diffusion lue dans un fichier (groupe vCard, `groupOfNames`
/// LDIF), non validée.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportedList {
    pub name: String,
    pub description: String,
    /// Adresses des membres, telles que lues (celle d'un membre désigné
    /// par une autre fiche du fichier : la première adresse de cette
    /// fiche).
    pub members: Vec<String>,
    /// Membres désignés que le fichier ne permet pas de retrouver (fiche
    /// absente ou sans adresse).
    pub unresolved: usize,
    /// Membres à mettre en copie cachée (propriété propre à Norelly).
    pub bcc: bool,
}

/// Fiche lue dans un fichier, non validée.
// Presque toutes les fiches lues sont des contacts : mettre le contact en
// boîte ajouterait une allocation par fiche sans rien économiser.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportedCard {
    /// Un contact.
    Contact(ContactInput),
    /// Une liste de diffusion.
    List(ImportedList),
    /// Une fiche que le format ne permet pas de lire (encodage invalide,
    /// champ démesuré).
    Unreadable,
}

/// Contenu d'un fichier de contacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedContacts {
    pub format: ContactFormat,
    pub cards: Vec<ImportedCard>,
}

/// Que faire d'une fiche dont une adresse est déjà dans le carnet (ou dans
/// une fiche précédente du fichier).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DuplicatePolicy {
    /// L'ignorer.
    #[default]
    Skip,
    /// Compléter le contact existant (champs vides, adresses et téléphones
    /// manquants) sans rien écraser.
    Complete,
}

/// Raison du rejet d'une fiche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// Fiche illisible dans le fichier.
    Unreadable,
    /// Liste refusée par le domaine (sans nom, nom trop long, trop de
    /// membres…).
    InvalidList(ContactListError),
    /// Fiche refusée par le domaine (sans adresse, adresse invalide, champ
    /// trop long…).
    Invalid(ContactError),
}

/// Fiche rejetée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRejection {
    /// Rang de la fiche dans le fichier, à partir de 1.
    pub card: usize,
    /// Repère pour la reconnaître : son nom ou sa première adresse, sans
    /// caractère de contrôle, au plus [`MAX_IMPORT_HINT_CHARS`] caractères
    /// (vide si elle n'a ni l'un ni l'autre).
    pub hint: String,
    pub reason: RejectReason,
}

/// Complément d'un contact du carnet : la fiche telle qu'elle a été lue
/// pour calculer le plan, et son nouveau contenu. L'import ne l'écrit que
/// si la fiche n'a pas changé depuis cette lecture
/// ([`crate::ports::ContactBook::import`]) : une modification faite
/// entre-temps n'est jamais écrasée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactCompletion {
    read: Contact,
    draft: ContactDraft,
}

impl ContactCompletion {
    /// Complément de la fiche `read` par le contenu `draft`.
    #[must_use]
    pub fn new(read: Contact, draft: ContactDraft) -> Self {
        Self { read, draft }
    }

    /// Identifiant du contact complété.
    #[must_use]
    pub fn id(&self) -> ContactId {
        self.read.id()
    }

    /// Fiche telle qu'elle a été lue pour calculer le complément.
    #[must_use]
    pub fn read(&self) -> &Contact {
        &self.read
    }

    /// Nouveau contenu de la fiche.
    #[must_use]
    pub fn draft(&self) -> &ContactDraft {
        &self.draft
    }
}

/// `true` si le refus d'un import vient d'un carnet qui a changé depuis la
/// lecture qui a servi au plan : contact à compléter modifié (`Conflict`)
/// ou supprimé (`NotFound`), adresse prise entre-temps par un contact
/// (`DuplicateAddress`, le plan n'en écrit jamais une déjà portée). Un plan
/// recalculé sur le carnet relu en tient compte.
#[must_use]
pub fn plan_outdated(error: &ContactStoreError) -> bool {
    matches!(
        error,
        ContactStoreError::Conflict
            | ContactStoreError::NotFound
            | ContactStoreError::DuplicateAddress(_)
    )
}

/// Membre d'une liste importée : un contact du carnet, ou une fiche que
/// l'import crée (rang dans [`ImportPlan::create`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListMember {
    Existing(ContactId),
    New(usize),
}

/// Identifiants des membres `members`, `created` étant ceux des fiches
/// créées par l'import, dans l'ordre de [`ImportPlan::create`].
fn member_ids(
    members: &[ListMember],
    created: &[ContactId],
) -> Result<Vec<ContactId>, ContactListError> {
    members
        .iter()
        .map(|m| match *m {
            ListMember::Existing(id) => Ok(id),
            ListMember::New(i) => created
                .get(i)
                .copied()
                .ok_or(ContactListError::InvalidMember),
        })
        .collect()
}

/// Liste à créer : validée sans ses membres (nom et description en NFC),
/// et ses membres, chacun une fois, au plus [`MAX_CONTACT_LIST_MEMBERS`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListImport {
    draft: ContactListDraft,
    members: Vec<ListMember>,
}

impl ListImport {
    /// Nom, description et Cci de la liste (sans membres).
    #[must_use]
    pub fn draft(&self) -> &ContactListDraft {
        &self.draft
    }

    #[must_use]
    pub fn members(&self) -> &[ListMember] {
        &self.members
    }

    /// Liste à écrire, `created` étant les identifiants des fiches créées
    /// par l'import, dans l'ordre de [`ImportPlan::create`].
    pub fn resolve(&self, created: &[ContactId]) -> Result<ContactListDraft, ContactListError> {
        self.draft
            .with_members(&member_ids(&self.members, created)?)
    }
}

/// Complément d'une liste du carnet : la liste telle qu'elle a été lue
/// pour calculer le plan, et les membres à lui ajouter à la suite.
/// L'import ne l'écrit que si elle n'a pas changé depuis cette lecture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListCompletion {
    read: ContactList,
    added: Vec<ListMember>,
}

impl ListCompletion {
    /// Liste telle qu'elle a été lue pour calculer le complément.
    #[must_use]
    pub fn read(&self) -> &ContactList {
        &self.read
    }

    /// Membres ajoutés, absents de la liste, dans la borne d'une liste.
    #[must_use]
    pub fn added(&self) -> &[ListMember] {
        &self.added
    }

    /// Nouveau contenu de la liste (voir [`ListImport::resolve`]).
    pub fn resolve(&self, created: &[ContactId]) -> Result<ContactListDraft, ContactListError> {
        self.read
            .draft()
            .with_members(&member_ids(&self.added, created)?)
    }
}

/// Plan d'import : ce que l'import écrira, et ce qu'il laisse de côté.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportPlan {
    /// Nouvelles fiches, dans l'ordre du fichier.
    pub create: Vec<ContactDraft>,
    /// Contacts du carnet complétés (fiche lue, nouveau contenu).
    pub complete: Vec<ContactCompletion>,
    /// Nouvelles listes, dans l'ordre du fichier.
    pub lists: Vec<ListImport>,
    /// Listes du carnet complétées de membres.
    pub complete_lists: Vec<ListCompletion>,
    /// Fiches et listes en double ignorées (ou qui n'apportaient rien).
    pub duplicates: usize,
    /// Membres des listes gardées qui ne sont ni dans le carnet ni parmi
    /// les fiches importées : laissés de côté.
    pub missing_members: usize,
    /// Fiches dont un nom imitant une adresse a été retiré.
    pub renamed: usize,
    /// Fiches rejetées, dans l'ordre du fichier.
    pub rejected: Vec<ImportRejection>,
}

/// Clés des adresses des fiches et des membres des listes, une fois
/// chacune, dans l'ordre du fichier (adresses invalides omises) : les
/// contacts du carnet à lire avant [`plan_import`].
#[must_use]
pub fn import_keys(cards: &[ImportedCard], normalizer: &impl TextNormalizer) -> Vec<AddressKey> {
    let mut seen = HashSet::new();
    cards
        .iter()
        .flat_map(|card| -> Box<dyn Iterator<Item = &str>> {
            match card {
                ImportedCard::Contact(input) => {
                    Box::new(input.emails.iter().map(|e| e.address.as_str()))
                }
                ImportedCard::List(list) => Box::new(list.members.iter().map(String::as_str)),
                ImportedCard::Unreadable => Box::new(std::iter::empty()),
            }
        })
        .filter_map(|address| AddressKey::parse(normalizer.nfc(address.trim()).as_str()).ok())
        .filter(|key| seen.insert(key.clone()))
        .collect()
}

/// Clés de nom ([`ContactListDraft::name_key`]) des listes valides du
/// fichier, une fois chacune, dans l'ordre du fichier : les listes du
/// carnet à lire avant [`plan_import`].
#[must_use]
pub fn import_list_keys(cards: &[ImportedCard], normalizer: &impl TextNormalizer) -> Vec<String> {
    let mut seen = HashSet::new();
    cards
        .iter()
        .filter_map(|card| match card {
            ImportedCard::List(list) => list_draft(list, normalizer).ok(),
            ImportedCard::Contact(_) | ImportedCard::Unreadable => None,
        })
        .map(|draft| draft.name_key())
        .filter(|key| seen.insert(key.clone()))
        .collect()
}

/// Liste lue validée sans ses membres : nom et description en NFC, sans
/// caractère invisible ([`is_invisible`], refusé à l'import comme pour une
/// fiche), puis [`ContactListDraft::new`].
fn list_draft(
    list: &ImportedList,
    normalizer: &impl TextNormalizer,
) -> Result<ContactListDraft, ContactListError> {
    let fields = [
        (&list.name, ContactListField::Name),
        (&list.description, ContactListField::Description),
    ];
    if let Some((_, field)) = fields
        .iter()
        .find(|(text, _)| text.chars().any(is_invisible))
    {
        return Err(ContactListError::InvalidCharacter(*field));
    }
    ContactListDraft::new(
        ContactListInput {
            name: list.name.clone(),
            description: normalizer.nfc(&list.description),
            members: Vec::new(),
            bcc: list.bcc,
        },
        normalizer,
    )
}

/// Destination d'une adresse déjà rencontrée : un contact du carnet ou une
/// fiche créée par l'import (rang dans `existing` ou dans `create`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    Existing(usize),
    New(usize),
}

/// Plan d'import des fiches `cards` dans un carnet dont `existing` sont les
/// contacts qui portent l'une de leurs adresses ([`import_keys`]), et
/// `existing_lists` les listes qui portent le nom d'une liste du fichier
/// ([`import_list_keys`]).
///
/// Chaque fiche est préparée (noms en NFC par `normalizer`, nom imitant
/// une adresse retiré, adresses vides ou répétées écartées, une seule
/// préférée) puis validée. Une fiche dont une adresse appartient déjà à un
/// contact du carnet ou à une fiche précédente est un doublon : ignorée
/// ([`DuplicatePolicy::Skip`]) ou versée dans ce contact
/// ([`DuplicatePolicy::Complete`], voir [`complete_contact`]) ; versée
/// dans une fiche du fichier, elle compte aussi comme doublon.
#[must_use]
pub fn plan_import(
    existing: &[Contact],
    existing_lists: &[ContactList],
    cards: &[ImportedCard],
    policy: DuplicatePolicy,
    normalizer: &impl TextNormalizer,
) -> ImportPlan {
    let mut plan = ImportPlan::default();
    let mut owners: HashMap<AddressKey, Owner> = HashMap::new();
    for (index, contact) in existing.iter().enumerate() {
        for e in contact.draft().emails() {
            owners
                .entry(e.key().clone())
                .or_insert(Owner::Existing(index));
        }
    }
    let mut completed: Vec<Option<ContactDraft>> = vec![None; existing.len()];

    for (index, card) in cards.iter().enumerate() {
        let reject = |hint: String, reason| ImportRejection {
            card: index + 1,
            hint,
            reason,
        };
        let input = match card {
            ImportedCard::Contact(input) => input,
            ImportedCard::List(_) => continue,
            ImportedCard::Unreadable => {
                plan.rejected
                    .push(reject(String::new(), RejectReason::Unreadable));
                continue;
            }
        };
        let (input, renamed) = prepare(input, normalizer);
        if let Some(index) = input
            .emails
            .iter()
            .position(|e| e.address.chars().any(is_invisible))
        {
            plan.rejected.push(reject(
                hint(&input),
                RejectReason::Invalid(ContactError::InvalidAddress {
                    index,
                    error: EmailAddressError::InvalidLocalCharacter,
                }),
            ));
            continue;
        }
        if let Some(field) = invisible_field(&input) {
            plan.rejected.push(reject(
                hint(&input),
                RejectReason::Invalid(ContactError::InvalidCharacter(field)),
            ));
            continue;
        }
        let draft = match ContactDraft::new(input.clone()) {
            Ok(draft) => draft,
            Err(e) => {
                plan.rejected
                    .push(reject(hint(&input), RejectReason::Invalid(e)));
                continue;
            }
        };
        if renamed {
            plan.renamed += 1;
        }
        let owner = draft
            .emails()
            .iter()
            .find_map(|e| owners.get(e.key()).copied());
        let Some(owner) = owner else {
            for e in draft.emails() {
                owners.insert(e.key().clone(), Owner::New(plan.create.len()));
            }
            plan.create.push(draft);
            continue;
        };
        if policy == DuplicatePolicy::Skip {
            plan.duplicates += 1;
            continue;
        }
        let base = match owner {
            Owner::Existing(i) => completed[i].as_ref().unwrap_or(existing[i].draft()),
            Owner::New(i) => &plan.create[i],
        };
        let merged = complete_contact(base, &draft, |key| {
            owners.get(key).is_some_and(|o| *o != owner)
        });
        match merged {
            Some(merged) => {
                for e in merged.emails() {
                    owners.insert(e.key().clone(), owner);
                }
                match owner {
                    Owner::Existing(i) => completed[i] = Some(merged),
                    Owner::New(i) => {
                        plan.create[i] = merged;
                        plan.duplicates += 1;
                    }
                }
            }
            None => plan.duplicates += 1,
        }
        // Un nouveau contact est favori si l'une de ses fiches du fichier
        // l'est ; celui d'un contact du carnet ne change pas.
        if let Owner::New(i) = owner
            && draft.favorite()
            && let Some(created) = plan.create.get_mut(i)
            && !created.favorite()
        {
            let mut input = created.to_input();
            input.favorite = true;
            if let Ok(favorite) = ContactDraft::new(input) {
                *created = favorite;
            }
        }
    }

    plan_lists(
        &mut plan,
        existing,
        existing_lists,
        cards,
        &owners,
        policy,
        normalizer,
    );
    // Listes traitées après les fiches : rejets remis dans l'ordre du fichier.
    plan.rejected.sort_by_key(|r| r.card);
    plan.complete = existing
        .iter()
        .zip(completed)
        .filter_map(|(contact, draft)| draft.map(|d| ContactCompletion::new(contact.clone(), d)))
        .collect();
    plan
}

/// Destination d'une liste déjà rencontrée : une liste du carnet ou une
/// liste créée par l'import (rang dans `existing_lists` ou dans
/// `plan.lists`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListOwner {
    Existing(usize),
    New(usize),
}

/// Listes du plan, une fois les fiches traitées (une liste peut précéder
/// ses membres dans le fichier) : chaque liste validée sans ses membres
/// ([`list_draft`]), ses membres retrouvés par leur adresse (`owners` :
/// contacts du carnet et fiches créées), chacun une fois ; une adresse
/// introuvable compte dans `missing_members`. Une liste de même clé de nom
/// qu'une liste du carnet ou qu'une liste précédente du fichier est un
/// doublon : ignorée, ou complétée des membres qui lui manquent dans la
/// borne d'une liste.
fn plan_lists(
    plan: &mut ImportPlan,
    existing: &[Contact],
    existing_lists: &[ContactList],
    cards: &[ImportedCard],
    owners: &HashMap<AddressKey, Owner>,
    policy: DuplicatePolicy,
    normalizer: &impl TextNormalizer,
) {
    let mut names: HashMap<String, ListOwner> = HashMap::new();
    for (index, list) in existing_lists.iter().enumerate() {
        names
            .entry(list.draft().name_key())
            .or_insert(ListOwner::Existing(index));
    }
    // Membres ajoutés à chaque liste du carnet.
    let mut added: Vec<Vec<ListMember>> = vec![Vec::new(); existing_lists.len()];

    for (index, card) in cards.iter().enumerate() {
        let ImportedCard::List(list) = card else {
            continue;
        };
        let reject = |reason| ImportRejection {
            card: index + 1,
            hint: clean_hint(&list.name),
            reason: RejectReason::InvalidList(reason),
        };
        let draft = match list_draft(list, normalizer) {
            Ok(draft) => draft,
            Err(e) => {
                plan.rejected.push(reject(e));
                continue;
            }
        };
        let mut missing = list.unresolved;
        let mut members: Vec<ListMember> = Vec::new();
        let mut seen = HashSet::new();
        for address in &list.members {
            let owner = AddressKey::parse(normalizer.nfc(address.trim()).as_str())
                .ok()
                .and_then(|key| owners.get(&key).copied());
            let member = match owner {
                Some(Owner::Existing(i)) => ListMember::Existing(existing[i].id()),
                Some(Owner::New(i)) => ListMember::New(i),
                None => {
                    missing += 1;
                    continue;
                }
            };
            if seen.insert(member) {
                members.push(member);
            }
        }
        if members.len() > MAX_CONTACT_LIST_MEMBERS {
            plan.rejected.push(reject(ContactListError::TooManyMembers));
            continue;
        }
        let key = draft.name_key();
        let Some(&owner) = names.get(&key) else {
            names.insert(key, ListOwner::New(plan.lists.len()));
            plan.lists.push(ListImport { draft, members });
            plan.missing_members += missing;
            continue;
        };
        if policy == DuplicatePolicy::Skip {
            plan.duplicates += 1;
            continue;
        }
        let (current, extra): (Vec<ListMember>, &mut Vec<ListMember>) = match owner {
            ListOwner::Existing(i) => (
                existing_lists[i]
                    .draft()
                    .members()
                    .iter()
                    .map(|&id| ListMember::Existing(id))
                    .chain(added[i].iter().copied())
                    .collect(),
                &mut added[i],
            ),
            ListOwner::New(i) => (Vec::new(), &mut plan.lists[i].members),
        };
        let mut present: HashSet<ListMember> =
            current.iter().chain(extra.iter()).copied().collect();
        let mut count = present.len();
        let before = extra.len();
        for member in members {
            if count >= MAX_CONTACT_LIST_MEMBERS {
                break;
            }
            if present.insert(member) {
                extra.push(member);
                count += 1;
            }
        }
        let brought = extra.len() > before;
        if brought {
            plan.missing_members += missing;
        }
        if !brought || matches!(owner, ListOwner::New(_)) {
            plan.duplicates += 1;
        }
    }

    plan.complete_lists = existing_lists
        .iter()
        .zip(added)
        .filter(|(_, added)| !added.is_empty())
        .map(|(list, added)| ListCompletion {
            read: list.clone(),
            added,
        })
        .collect();
}

/// Fiche prête à valider : textes en NFC, nom imitant une adresse retiré
/// (`true` si un nom l'a été), adresses vides ou répétées (à la casse
/// près) écartées, la première adresse préférée seule gardée préférée.
fn prepare(input: &ContactInput, normalizer: &impl TextNormalizer) -> (ContactInput, bool) {
    let nfc = |text: &str| normalizer.nfc(text);
    let mut keys = HashSet::new();
    let mut preferred_seen = false;
    let mut emails = Vec::with_capacity(input.emails.len());
    for e in &input.emails {
        // Même adresse en forme composée ou décomposée : une seule clé.
        let address = nfc(e.address.trim());
        let address = address.as_str();
        if address.is_empty() {
            continue;
        }
        // Adresse invalide gardée : la validation la refuse, motif à l'appui.
        if let Ok(key) = AddressKey::parse(address)
            && !keys.insert(key)
        {
            continue;
        }
        let preferred = e.preferred && !preferred_seen;
        preferred_seen |= preferred;
        emails.push(EmailInput {
            address: address.to_owned(),
            label: nfc(&e.label),
            preferred,
        });
    }
    let phones = input
        .phones
        .iter()
        .map(|p| PhoneInput {
            number: nfc(&p.number),
            label: nfc(&p.label),
        })
        .collect();
    let mut prepared = ContactInput {
        display_name: nfc(&input.display_name),
        given_name: nfc(&input.given_name),
        family_name: nfc(&input.family_name),
        nickname: nfc(&input.nickname),
        organization: nfc(&input.organization),
        notes: nfc(&input.notes),
        title: nfc(&input.title),
        // Date illisible d'un autre logiciel : écartée, la fiche gardée.
        birthday: Birthday::parse(&input.birthday)
            .map(|b| b.iso())
            .unwrap_or_default(),
        emails,
        phones,
        postal_addresses: input
            .postal_addresses
            .iter()
            .map(|a| PostalAddressInput {
                street: nfc(&a.street),
                locality: nfc(&a.locality),
                region: nfc(&a.region),
                postal_code: nfc(&a.postal_code),
                country: nfc(&a.country),
                label: nfc(&a.label),
            })
            // Adresse sans aucun élément (étiquette seule) : écartée.
            .filter(|a| {
                [
                    &a.street,
                    &a.locality,
                    &a.region,
                    &a.postal_code,
                    &a.country,
                ]
                .iter()
                .any(|part| !part.trim().is_empty())
            })
            // Même lieu répété (casse et étiquette ignorées) : une fois.
            .filter({
                let mut seen = HashSet::new();
                move |a| {
                    seen.insert(
                        [
                            &a.street,
                            &a.locality,
                            &a.region,
                            &a.postal_code,
                            &a.country,
                        ]
                        .map(|part| part.trim().to_lowercase()),
                    )
                }
            })
            // Au-delà de ce qu'une fiche admet : le surplus est laissé.
            .take(MAX_POSTAL_ADDRESSES)
            .collect(),
        // Site d'un schéma refusé ou mal formé : écarté, la fiche gardée ;
        // un caractère invisible rejette la fiche, comme ailleurs.
        websites: input
            .websites
            .iter()
            .map(|w| WebsiteInput {
                url: nfc(&w.url),
                label: nfc(&w.label),
            })
            .filter({
                let mut seen = HashSet::new();
                move |w| {
                    if w.url.chars().any(is_invisible) {
                        return true;
                    }
                    // Même site répété (casse ignorée) : une fois.
                    website_url(&w.url).is_some_and(|url| seen.insert(url.to_lowercase()))
                }
            })
            .take(MAX_CONTACT_WEBSITES)
            .collect(),
        favorite: input.favorite,
    };
    // Vérifié sur les noms normalisés, comme à la relecture d'une fiche.
    let renamed = prepared.drop_names_imitating_address();
    (prepared, renamed)
}

/// Premier champ d'une fiche importée qui contient un caractère
/// invisible ([`is_invisible`]), espaces de bord compris.
fn invisible_field(input: &ContactInput) -> Option<ContactField> {
    let has = |text: &str| text.chars().any(is_invisible);
    let names = [
        (&input.display_name, ContactField::DisplayName),
        (&input.given_name, ContactField::GivenName),
        (&input.family_name, ContactField::FamilyName),
        (&input.nickname, ContactField::Nickname),
        (&input.organization, ContactField::Organization),
        (&input.title, ContactField::Title),
        (&input.notes, ContactField::Notes),
    ];
    let postal = input
        .postal_addresses
        .iter()
        .enumerate()
        .flat_map(|(i, a)| {
            [
                (&a.street, ContactField::PostalAddress(i)),
                (&a.locality, ContactField::PostalAddress(i)),
                (&a.region, ContactField::PostalAddress(i)),
                (&a.postal_code, ContactField::PostalAddress(i)),
                (&a.country, ContactField::PostalAddress(i)),
                (&a.label, ContactField::PostalAddressLabel(i)),
            ]
        });
    let websites = input.websites.iter().enumerate().flat_map(|(i, w)| {
        [
            (&w.url, ContactField::Website(i)),
            (&w.label, ContactField::WebsiteLabel(i)),
        ]
    });
    let emails = input
        .emails
        .iter()
        .enumerate()
        .map(|(i, e)| (&e.label, ContactField::EmailLabel(i)));
    let phones = input.phones.iter().enumerate().flat_map(|(i, p)| {
        [
            (&p.number, ContactField::Phone(i)),
            (&p.label, ContactField::PhoneLabel(i)),
        ]
    });
    names
        .into_iter()
        .chain(emails)
        .chain(phones)
        .chain(postal)
        .chain(websites)
        .find(|(text, _)| has(text))
        .map(|(_, field)| field)
}

/// Repère d'une fiche rejetée : nom affiché, sinon prénom et nom, sinon
/// surnom, sinon première adresse ; nettoyé et borné.
fn hint(input: &ContactInput) -> String {
    let full_name = [input.given_name.trim(), input.family_name.trim()]
        .into_iter()
        .filter(|n| !n.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let raw = [
        input.display_name.as_str(),
        full_name.as_str(),
        input.nickname.as_str(),
    ]
    .into_iter()
    .chain(input.emails.iter().map(|e| e.address.as_str()))
    .map(str::trim)
    .find(|text| !text.is_empty())
    .unwrap_or_default();
    clean_hint(raw)
}

/// Repère nettoyé (sans caractère de contrôle ni invisible) et borné à
/// [`MAX_IMPORT_HINT_CHARS`] caractères.
fn clean_hint(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|&c| !c.is_control() && !is_invisible(c))
        .take(MAX_IMPORT_HINT_CHARS)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Clé de comparaison d'un numéro : ses chiffres et `+` (« 01 23 » et
/// « 0123 » sont le même numéro), sinon le numéro en minuscules.
pub(crate) fn phone_key(number: &str) -> String {
    let digits: String = number
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();
    if digits.is_empty() {
        number.trim().to_lowercase()
    } else {
        digits
    }
}

/// `base` complété par `extra` : champs vides de `base` remplis, adresses
/// de `extra` absentes de `base` et non `taken` ajoutées (non préférées),
/// téléphones absents ajoutés, détails versés ([`DetailsMerge`]), dans les
/// limites d'une fiche ; rien n'est écrasé. `None` si `extra` n'apporte
/// rien.
#[must_use]
pub fn complete_contact(
    base: &ContactDraft,
    extra: &ContactDraft,
    taken: impl Fn(&AddressKey) -> bool,
) -> Option<ContactDraft> {
    let mut input = base.to_input();
    let mut changed = false;
    let mut fill = |field: &mut String, value: Option<&str>| {
        if let Some(value) = value
            && field.is_empty()
        {
            value.clone_into(field);
            changed = true;
        }
    };
    fill(&mut input.display_name, extra.display_name_field());
    fill(&mut input.given_name, extra.given_name());
    fill(&mut input.family_name, extra.family_name());
    fill(&mut input.nickname, extra.nickname());
    fill(&mut input.organization, extra.organization());
    fill(&mut input.notes, extra.notes());

    for e in extra.emails() {
        if input.emails.len() >= MAX_CONTACT_EMAILS {
            break;
        }
        if base.has_address(e.key()) || taken(e.key()) {
            continue;
        }
        input.emails.push(EmailInput {
            address: e.address().to_string(),
            label: e.label().unwrap_or_default().to_owned(),
            preferred: false,
        });
        changed = true;
    }
    let mut numbers: HashSet<String> = base
        .phones()
        .iter()
        .map(|p| phone_key(p.number()))
        .collect();
    for p in extra.phones() {
        if input.phones.len() >= MAX_CONTACT_PHONES {
            break;
        }
        if !numbers.insert(phone_key(p.number())) {
            continue;
        }
        input.phones.push(PhoneInput {
            number: p.number().to_owned(),
            label: p.label().unwrap_or_default().to_owned(),
        });
        changed = true;
    }
    changed |= DetailsMerge::new(base).add(&mut input, extra, true);
    if !changed {
        return None;
    }
    ContactDraft::new(input).ok()
}

/// Réunion des détails de plusieurs fiches dans une saisie : titre et
/// anniversaire manquants remplis, adresses postales (même lieu, casse et
/// étiquette ignorées) et sites web (même adresse, casse ignorée) absents
/// ajoutés ; rien n'est écrasé.
pub(crate) struct DetailsMerge {
    places: Vec<ContactPostalAddress>,
    sites: HashSet<String>,
}

impl DetailsMerge {
    /// Réunion partant des détails de `base`.
    pub(crate) fn new(base: &ContactDraft) -> Self {
        Self {
            places: base.postal_addresses().to_vec(),
            sites: base
                .websites()
                .iter()
                .map(|w| w.url().to_lowercase())
                .collect(),
        }
    }

    /// Verse les détails de `extra` dans `input` ; avec `bounded`, sans
    /// dépasser les bornes d'une fiche (le surplus est laissé). Vrai si
    /// `input` a changé.
    pub(crate) fn add(
        &mut self,
        input: &mut ContactInput,
        extra: &ContactDraft,
        bounded: bool,
    ) -> bool {
        let mut changed = false;
        if input.title.is_empty()
            && let Some(title) = extra.title()
        {
            title.clone_into(&mut input.title);
            changed = true;
        }
        if input.birthday.is_empty()
            && let Some(birthday) = extra.birthday()
        {
            input.birthday = birthday.iso();
            changed = true;
        }
        for place in extra.postal_addresses() {
            if bounded && input.postal_addresses.len() >= MAX_POSTAL_ADDRESSES {
                break;
            }
            if self.places.iter().any(|known| known.same_place(place)) {
                continue;
            }
            self.places.push(place.clone());
            input.postal_addresses.push(place.to_input());
            changed = true;
        }
        for site in extra.websites() {
            if bounded && input.websites.len() >= MAX_CONTACT_WEBSITES {
                break;
            }
            if !self.sites.insert(site.url().to_lowercase()) {
                continue;
            }
            input.websites.push(WebsiteInput {
                url: site.url().to_owned(),
                label: site.label().unwrap_or_default().to_owned(),
            });
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests;
