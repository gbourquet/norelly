//! Carnet d'adresses (ADR-012) : fiche contact validée, nom affiché, clé
//! d'adresse, filtre et ordre de la liste.
//!
//! Le modèle suit vCard (nom affiché, prénom, nom, surnom, plusieurs
//! adresses dont une préférée, téléphones, organisation, titre, notes,
//! anniversaire, adresses postales, sites web) pour qu'une
//! synchronisation CardDAV puisse s'ajouter plus tard. Une fiche saisie ou
//! importée ([`ContactInput`]) devient une fiche valide ([`ContactDraft`])
//! par [`ContactDraft::new`] ; le carnet lui attribue un identifiant
//! ([`Contact`]).

use std::fmt;

use crate::address::{EmailAddress, EmailAddressError};
use crate::connection::is_format_char;
use crate::contact_details::{
    Birthday, MAX_CONTACT_WEBSITES, MAX_POSTAL_ADDRESSES, MAX_POSTAL_FIELD_CHARS,
    MAX_POSTAL_STREET_CHARS, website_url,
};
use crate::mailbox::{MAX_ADDRESS_CHARS, header_text};
use crate::ports::{BlobId, MAX_CONTACT_LOOKUP};
use crate::sender::imitates_address;

/// Longueur maximale d'un nom (affiché, prénom, nom, surnom, organisation),
/// en caractères : celle d'un nom de correspondant.
pub const MAX_CONTACT_NAME_CHARS: usize = MAX_ADDRESS_CHARS;
/// Longueur maximale d'une étiquette (« travail », « maison »).
pub const MAX_CONTACT_LABEL_CHARS: usize = 64;
/// Longueur maximale d'un numéro de téléphone.
pub const MAX_PHONE_CHARS: usize = 64;
/// Longueur maximale des notes.
pub const MAX_CONTACT_NOTES_CHARS: usize = 10_000;
/// Nombre maximal d'adresses d'un contact.
pub const MAX_CONTACT_EMAILS: usize = 20;
/// Nombre maximal de téléphones d'un contact.
pub const MAX_CONTACT_PHONES: usize = 20;
/// Longueur maximale du texte d'un filtre, en caractères.
pub const MAX_CONTACT_FILTER_CHARS: usize = 200;
/// Nombre maximal de fiches visées en une fois (suppression).
pub const MAX_CONTACT_SELECTION: usize = 1000;

/// Plus grand identifiant de contact : ceux de l'interface tiennent sur
/// 32 bits (`u32::MAX`).
pub const MAX_CONTACT_ID: i64 = u32::MAX as i64;

/// Nombre maximal de fiches du carnet, tous chemins d'écriture confondus
/// (saisie, import, synchronisation) : sans borne, une suite d'imports
/// ferait grossir la base et ralentir la liste et les suggestions sans
/// limite (bancs `contacts` de l'adaptateur SQLite). Celle d'un fichier
/// importé : un carnet plein s'exporte, se réimporte, se parcourt (doublons)
/// et se synchronise encore en entier.
pub const MAX_CONTACTS: usize = 25_000;

/// Identifiant local d'un contact, attribué par le carnet (strictement
/// positif, au plus [`MAX_CONTACT_ID`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContactId(i64);

impl ContactId {
    #[must_use]
    pub fn new(raw: i64) -> Option<Self> {
        (1..=MAX_CONTACT_ID).contains(&raw).then_some(Self(raw))
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl fmt::Display for ContactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Clé de comparaison d'une adresse : l'adresse validée, lettres ASCII en
/// minuscules. Deux adresses de même clé désignent la même boîte pour le
/// carnet (la casse de la partie locale n'est presque jamais significative).
/// Les caractères non ASCII sont gardés tels quels : le repli Unicode
/// change avec les versions et confond des boîtes distinctes (signe
/// kelvin U+212A et « k »).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AddressKey(String);

impl AddressKey {
    /// Clé d'une adresse validée.
    #[must_use]
    pub fn of(address: &EmailAddress) -> Self {
        Self(format!(
            "{}@{}",
            address.local_part().to_ascii_lowercase(),
            address.domain()
        ))
    }

    /// Clé d'une adresse brute (en-tête reçu, saisie) ; refusée si
    /// l'adresse est invalide.
    pub fn parse(raw: &str) -> Result<Self, EmailAddressError> {
        EmailAddress::parse(raw).map(|a| Self::of(&a))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Champ d'une fiche, pour situer une erreur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactField {
    DisplayName,
    GivenName,
    FamilyName,
    Nickname,
    Organization,
    /// Titre ou fonction (`TITLE`).
    Title,
    Notes,
    /// Anniversaire.
    Birthday,
    /// Adresse postale `n` (un de ses éléments).
    PostalAddress(usize),
    /// Étiquette de l'adresse postale `n`.
    PostalAddressLabel(usize),
    /// Site web `n`.
    Website(usize),
    /// Étiquette du site web `n`.
    WebsiteLabel(usize),
    /// Étiquette de l'adresse numéro `n` (à partir de 0).
    EmailLabel(usize),
    /// Numéro de téléphone `n`.
    Phone(usize),
    /// Étiquette du téléphone `n`.
    PhoneLabel(usize),
}

/// Fiche refusée.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContactError {
    #[error("un contact a au moins une adresse")]
    NoAddress,
    #[error("plus de {MAX_CONTACT_EMAILS} adresses")]
    TooManyAddresses,
    #[error("plus de {MAX_CONTACT_PHONES} téléphones")]
    TooManyPhones,
    #[error("plus de {MAX_POSTAL_ADDRESSES} adresses postales")]
    TooManyPostalAddresses,
    #[error("plus de {MAX_CONTACT_WEBSITES} sites web")]
    TooManyWebsites,
    #[error("adresse postale n° {index} vide")]
    EmptyPostalAddress { index: usize },
    #[error("anniversaire invalide")]
    InvalidBirthday,
    #[error("site web n° {index} refusé (http ou https seulement)")]
    InvalidWebsite { index: usize },
    #[error("adresse n° {index} invalide : {error}")]
    InvalidAddress {
        index: usize,
        error: EmailAddressError,
    },
    #[error("adresse n° {index} déjà présente dans la fiche")]
    DuplicateAddress { index: usize },
    #[error("plusieurs adresses préférées")]
    SeveralPreferred,
    #[error("téléphone n° {index} vide")]
    EmptyPhone { index: usize },
    #[error("champ {0:?} trop long")]
    TooLong(ContactField),
    #[error("champ {0:?} avec un caractère interdit")]
    InvalidCharacter(ContactField),
    #[error("nom {0:?} imitant une adresse")]
    NameLooksLikeAddress(ContactField),
}

/// Adresse saisie ou importée, non validée.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmailInput {
    pub address: String,
    pub label: String,
    pub preferred: bool,
}

/// Téléphone saisi ou importé, non validé.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhoneInput {
    pub number: String,
    pub label: String,
}

/// Adresse postale saisie ou importée, non validée (`ADR` vCard, sans
/// boîte postale ni complément, versés dans la rue).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostalAddressInput {
    /// Rue, sur une ou plusieurs lignes.
    pub street: String,
    pub locality: String,
    pub region: String,
    pub postal_code: String,
    pub country: String,
    pub label: String,
}

/// Site web saisi ou importé, non validé.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebsiteInput {
    pub url: String,
    pub label: String,
}

/// Fiche saisie ou importée, non validée. Un champ vide (ou fait
/// d'espaces) est absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactInput {
    pub display_name: String,
    pub given_name: String,
    pub family_name: String,
    pub nickname: String,
    pub organization: String,
    /// Titre ou fonction (`TITLE`).
    pub title: String,
    pub notes: String,
    /// Anniversaire, forme lue par [`Birthday::parse`].
    pub birthday: String,
    pub emails: Vec<EmailInput>,
    pub phones: Vec<PhoneInput>,
    pub postal_addresses: Vec<PostalAddressInput>,
    pub websites: Vec<WebsiteInput>,
    pub favorite: bool,
}

impl ContactInput {
    /// Un nom (affiché, prénom, nom, surnom) imite-t-il une adresse ?
    #[must_use]
    pub fn has_names_imitating_address(&self) -> bool {
        [
            &self.display_name,
            &self.given_name,
            &self.family_name,
            &self.nickname,
        ]
        .into_iter()
        .any(|name| imitates_address(name))
    }

    /// Retire les noms (affiché, prénom, nom, surnom) qui imitent une
    /// adresse ; vrai si un nom l'a été.
    pub fn drop_names_imitating_address(&mut self) -> bool {
        let mut dropped = false;
        for name in [
            &mut self.display_name,
            &mut self.given_name,
            &mut self.family_name,
            &mut self.nickname,
        ] {
            if imitates_address(name) {
                name.clear();
                dropped = true;
            }
        }
        dropped
    }
}

/// Adresse d'un contact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactEmail {
    address: EmailAddress,
    key: AddressKey,
    label: Option<String>,
    preferred: bool,
}

impl ContactEmail {
    #[must_use]
    pub fn address(&self) -> &EmailAddress {
        &self.address
    }

    #[must_use]
    pub fn key(&self) -> &AddressKey {
        &self.key
    }

    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    #[must_use]
    pub fn preferred(&self) -> bool {
        self.preferred
    }
}

/// Téléphone d'un contact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactPhone {
    number: String,
    label: Option<String>,
}

impl ContactPhone {
    #[must_use]
    pub fn number(&self) -> &str {
        &self.number
    }

    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }
}

/// Adresse postale d'un contact : au moins un élément renseigné.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContactPostalAddress {
    street: Option<String>,
    locality: Option<String>,
    region: Option<String>,
    postal_code: Option<String>,
    country: Option<String>,
    label: Option<String>,
}

impl ContactPostalAddress {
    /// Rue : lignes sans espaces de bord, séparées par `\n`.
    #[must_use]
    pub fn street(&self) -> Option<&str> {
        self.street.as_deref()
    }

    #[must_use]
    pub fn locality(&self) -> Option<&str> {
        self.locality.as_deref()
    }

    #[must_use]
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    #[must_use]
    pub fn postal_code(&self) -> Option<&str> {
        self.postal_code.as_deref()
    }

    #[must_use]
    pub fn country(&self) -> Option<&str> {
        self.country.as_deref()
    }

    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Même adresse, étiquette et casse ignorées.
    #[must_use]
    pub fn same_place(&self, other: &Self) -> bool {
        let key = |a: &Self| {
            [
                &a.street,
                &a.locality,
                &a.region,
                &a.postal_code,
                &a.country,
            ]
            .map(|v| v.as_deref().unwrap_or_default().to_lowercase())
        };
        key(self) == key(other)
    }

    pub(crate) fn to_input(&self) -> PostalAddressInput {
        let owned = |v: &Option<String>| v.clone().unwrap_or_default();
        PostalAddressInput {
            street: owned(&self.street),
            locality: owned(&self.locality),
            region: owned(&self.region),
            postal_code: owned(&self.postal_code),
            country: owned(&self.country),
            label: owned(&self.label),
        }
    }
}

/// Site web d'un contact : adresse `http` ou `https` acceptée par
/// [`website_url`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContactWebsite {
    url: String,
    label: Option<String>,
}

impl ContactWebsite {
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }
}

/// Sort d'un nom qui imite une adresse lors de la validation d'une fiche.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameRule {
    /// Refusé : toute fiche saisie, importée ou relue.
    RefuseImitations,
    /// Gardé : seulement pour recalculer les clés d'une fiche telle
    /// qu'enregistrée ([`ContactDraft::keys_as_stored`]), jamais rendue.
    KeepImitations,
}

/// Fiche valide, pas encore enregistrée (ou contenu d'une fiche
/// enregistrée).
///
/// Invariants : au moins une adresse, au plus [`MAX_CONTACT_EMAILS`],
/// distinctes à la casse près, exactement une préférée ; textes et parties
/// locales d'adresse sans caractère invisible ([`INVISIBLE`]) ; textes sans
/// espaces de bord, bornés, sans caractère de contrôle (les notes
/// admettent sauts de ligne et tabulations) ; noms (affiché, prénom, nom,
/// surnom) qui n'imitent pas une adresse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactDraft {
    display_name: Option<String>,
    given_name: Option<String>,
    family_name: Option<String>,
    nickname: Option<String>,
    organization: Option<String>,
    title: Option<String>,
    notes: Option<String>,
    birthday: Option<Birthday>,
    emails: Vec<ContactEmail>,
    phones: Vec<ContactPhone>,
    postal_addresses: Vec<ContactPostalAddress>,
    websites: Vec<ContactWebsite>,
    favorite: bool,
}

impl ContactDraft {
    /// Fiche relue par un adaptateur de stockage, et seulement pour cela
    /// (toute saisie passe par [`Self::new`]) : mêmes règles que `new`,
    /// sauf qu'un nom imitant une adresse est retiré au lieu d'être refusé,
    /// comme à l'import. Une fiche enregistrée avant que la règle ne
    /// l'écarte s'ouvre donc normalement, sans ce nom.
    pub fn from_stored(mut input: ContactInput) -> Result<Self, ContactError> {
        input.drop_names_imitating_address();
        Self::new(input)
    }

    /// Clés de tri et de recherche d'une fiche enregistrée qui porte un nom
    /// imitant une adresse, calculées avec ce nom : celles que le stockage a
    /// écrites avant que la règle ne l'écarte. `None` si aucun nom n'imite
    /// une adresse, ou si la fiche est invalide pour une autre raison.
    /// [`Self::from_stored`] rend, elle, les clés de la fiche sans ce nom.
    #[must_use]
    pub fn keys_as_stored(input: &ContactInput) -> Option<(String, String)> {
        if !input.has_names_imitating_address() {
            return None;
        }
        let stored = Self::validate(input.clone(), NameRule::KeepImitations).ok()?;
        Some((stored.sort_key(), stored.search_text()))
    }

    /// Migration du texte cherché au nom complet ([`Self::full_name`]) :
    /// nouveau texte cherché de la fiche relue `self`
    /// ([`Self::from_stored`] de `input`, ses champs enregistrés), si ses
    /// clés stockées `stored` (tri, texte cherché) sont celles que
    /// calculait le domaine avant d'y ajouter le nom complet, ou déjà
    /// celles d'aujourd'hui, pour la fiche relue ou telle qu'enregistrée
    /// avec un nom imitant une adresse ([`Self::keys_as_stored`]). `None`
    /// sinon (clés retouchées) : elle n'est ni réparée ni masquée.
    #[must_use]
    pub fn full_name_reindex(
        &self,
        input: &ContactInput,
        stored: &(String, String),
    ) -> Option<String> {
        let as_stored = if input.has_names_imitating_address() {
            Self::validate(input.clone(), NameRule::KeepImitations).ok()
        } else {
            None
        };
        [Some(self.clone()), as_stored]
            .into_iter()
            .flatten()
            .find_map(|draft| {
                let current = draft.search_text();
                let before = draft.search_text_with(None);
                (draft.sort_key() == stored.0 && (stored.1 == before || stored.1 == current))
                    .then_some(current)
            })
    }

    /// Valide une fiche. Sans adresse marquée préférée, la première l'est.
    pub fn new(input: ContactInput) -> Result<Self, ContactError> {
        Self::validate(input, NameRule::RefuseImitations)
    }

    /// Règles de [`Self::new`] ; `rule` dit seulement si un nom imitant une
    /// adresse est refusé (toujours, hors calcul de [`Self::keys_as_stored`]).
    fn validate(input: ContactInput, rule: NameRule) -> Result<Self, ContactError> {
        if input.emails.is_empty() {
            return Err(ContactError::NoAddress);
        }
        if input.emails.len() > MAX_CONTACT_EMAILS {
            return Err(ContactError::TooManyAddresses);
        }
        if input.phones.len() > MAX_CONTACT_PHONES {
            return Err(ContactError::TooManyPhones);
        }
        // Un nom qui imite une adresse pourrait faire passer une adresse
        // pour une autre : il est refusé (l'organisation n'est jamais
        // affichée comme nom du correspondant).
        let name = |raw: &str, field| {
            let value = text(raw, field, MAX_CONTACT_NAME_CHARS)?;
            if rule == NameRule::RefuseImitations && value.as_deref().is_some_and(imitates_address)
            {
                return Err(ContactError::NameLooksLikeAddress(field));
            }
            Ok(value)
        };
        let organization = text(
            &input.organization,
            ContactField::Organization,
            MAX_CONTACT_NAME_CHARS,
        )?;
        let display_name = name(&input.display_name, ContactField::DisplayName)?;
        let given_name = name(&input.given_name, ContactField::GivenName)?;
        let family_name = name(&input.family_name, ContactField::FamilyName)?;
        let nickname = name(&input.nickname, ContactField::Nickname)?;
        let notes = notes(&input.notes)?;

        if input.emails.iter().filter(|e| e.preferred).count() > 1 {
            return Err(ContactError::SeveralPreferred);
        }
        let any_preferred = input.emails.iter().any(|e| e.preferred);
        let mut emails: Vec<ContactEmail> = Vec::with_capacity(input.emails.len());
        for (index, e) in input.emails.iter().enumerate() {
            let address = EmailAddress::parse(&e.address)
                .map_err(|error| ContactError::InvalidAddress { index, error })?;
            // Le domaine est ASCII ; la partie locale peut être en UTF-8.
            if address.local_part().chars().any(is_invisible) {
                return Err(ContactError::InvalidAddress {
                    index,
                    error: EmailAddressError::InvalidLocalCharacter,
                });
            }
            let key = AddressKey::of(&address);
            if emails.iter().any(|known| known.key == key) {
                return Err(ContactError::DuplicateAddress { index });
            }
            emails.push(ContactEmail {
                address,
                key,
                label: text(
                    &e.label,
                    ContactField::EmailLabel(index),
                    MAX_CONTACT_LABEL_CHARS,
                )?,
                preferred: e.preferred || (!any_preferred && index == 0),
            });
        }

        let mut phones = Vec::with_capacity(input.phones.len());
        for (index, p) in input.phones.iter().enumerate() {
            let number = text(&p.number, ContactField::Phone(index), MAX_PHONE_CHARS)?
                .ok_or(ContactError::EmptyPhone { index })?;
            phones.push(ContactPhone {
                number,
                label: text(
                    &p.label,
                    ContactField::PhoneLabel(index),
                    MAX_CONTACT_LABEL_CHARS,
                )?,
            });
        }

        let (title, birthday, postal_addresses, websites) = details(&input)?;

        Ok(Self {
            display_name,
            given_name,
            family_name,
            nickname,
            organization,
            title,
            notes,
            birthday,
            emails,
            phones,
            postal_addresses,
            websites,
            favorite: input.favorite,
        })
    }

    #[must_use]
    pub fn display_name_field(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    #[must_use]
    pub fn given_name(&self) -> Option<&str> {
        self.given_name.as_deref()
    }

    #[must_use]
    pub fn family_name(&self) -> Option<&str> {
        self.family_name.as_deref()
    }

    #[must_use]
    pub fn nickname(&self) -> Option<&str> {
        self.nickname.as_deref()
    }

    #[must_use]
    pub fn organization(&self) -> Option<&str> {
        self.organization.as_deref()
    }

    /// Titre ou fonction.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    #[must_use]
    pub fn notes(&self) -> Option<&str> {
        self.notes.as_deref()
    }

    #[must_use]
    pub fn birthday(&self) -> Option<Birthday> {
        self.birthday
    }

    /// Adresses postales, dans l'ordre de la fiche.
    #[must_use]
    pub fn postal_addresses(&self) -> &[ContactPostalAddress] {
        &self.postal_addresses
    }

    /// Sites web, dans l'ordre de la fiche.
    #[must_use]
    pub fn websites(&self) -> &[ContactWebsite] {
        &self.websites
    }

    /// Adresses, dans l'ordre de la fiche.
    #[must_use]
    pub fn emails(&self) -> &[ContactEmail] {
        &self.emails
    }

    #[must_use]
    pub fn phones(&self) -> &[ContactPhone] {
        &self.phones
    }

    #[must_use]
    pub fn favorite(&self) -> bool {
        self.favorite
    }

    /// Adresse préférée (il y en a toujours une).
    #[must_use]
    pub fn preferred_email(&self) -> &ContactEmail {
        // Invariant : une adresse au moins, dont une préférée.
        self.emails
            .iter()
            .find(|e| e.preferred)
            .unwrap_or(&self.emails[0])
    }

    /// Nom à afficher : voir [`display_label`].
    #[must_use]
    pub fn display_name(&self) -> String {
        display_label(
            self.display_name.as_deref(),
            self.given_name.as_deref(),
            self.family_name.as_deref(),
            self.nickname.as_deref(),
            self.preferred_email().address(),
        )
    }

    /// Clé de tri de la liste : le nom affiché en minuscules.
    #[must_use]
    pub fn sort_key(&self) -> String {
        self.display_name().to_lowercase()
    }

    /// Nom complet « prénom nom », composé comme par [`display_label`],
    /// quand les deux champs sont renseignés.
    #[must_use]
    pub fn full_name(&self) -> Option<String> {
        match (&self.given_name, &self.family_name) {
            (Some(given), Some(family)) => Some(compose_full_name(given, family)),
            _ => None,
        }
    }

    /// Texte dans lequel cherche un filtre : noms, nom complet
    /// ([`Self::full_name`]), organisation, titre et adresses, en minuscules,
    /// séparés par des sauts de ligne.
    #[must_use]
    pub fn search_text(&self) -> String {
        self.search_text_with(self.full_name())
    }

    /// Texte cherché, avec `full_name` comme nom complet (`None` : le
    /// texte d'avant le nom complet, voir [`Self::full_name_reindex`]).
    fn search_text_with(&self, full_name: Option<String>) -> String {
        let names = [
            self.display_name.as_ref(),
            self.given_name.as_ref(),
            self.family_name.as_ref(),
            full_name.as_ref(),
            self.nickname.as_ref(),
            self.organization.as_ref(),
            self.title.as_ref(),
        ];
        names
            .into_iter()
            .flatten()
            .map(|n| n.to_lowercase())
            .chain(self.emails.iter().map(|e| e.key.0.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// La fiche contient-elle cette adresse ?
    #[must_use]
    pub fn has_address(&self, key: &AddressKey) -> bool {
        self.emails.iter().any(|e| &e.key == key)
    }

    /// Fiche à modifier (formulaire pré-rempli) : `ContactDraft::new`
    /// rend la même fiche.
    #[must_use]
    pub fn to_input(&self) -> ContactInput {
        let owned = |v: &Option<String>| v.clone().unwrap_or_default();
        ContactInput {
            display_name: owned(&self.display_name),
            given_name: owned(&self.given_name),
            family_name: owned(&self.family_name),
            nickname: owned(&self.nickname),
            organization: owned(&self.organization),
            title: owned(&self.title),
            notes: owned(&self.notes),
            birthday: self.birthday.map(|b| b.iso()).unwrap_or_default(),
            emails: self
                .emails
                .iter()
                .map(|e| EmailInput {
                    address: e.address.to_string(),
                    label: owned(&e.label),
                    preferred: e.preferred,
                })
                .collect(),
            phones: self
                .phones
                .iter()
                .map(|p| PhoneInput {
                    number: p.number.clone(),
                    label: owned(&p.label),
                })
                .collect(),
            postal_addresses: self
                .postal_addresses
                .iter()
                .map(ContactPostalAddress::to_input)
                .collect(),
            websites: self
                .websites
                .iter()
                .map(|w| WebsiteInput {
                    url: w.url.clone(),
                    label: owned(&w.label),
                })
                .collect(),
            favorite: self.favorite,
        }
    }

    /// Fiche enregistrée sous `id`.
    #[must_use]
    pub fn with_id(self, id: ContactId) -> Contact {
        Contact {
            id,
            draft: self,
            photo: None,
        }
    }
}

/// Contact enregistré dans le carnet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    id: ContactId,
    draft: ContactDraft,
    /// Photo réencodée ([`crate::contact_photo`]), dans le magasin des
    /// photos.
    photo: Option<BlobId>,
}

impl Contact {
    #[must_use]
    pub fn id(&self) -> ContactId {
        self.id
    }

    #[must_use]
    pub fn draft(&self) -> &ContactDraft {
        &self.draft
    }

    /// Photo du contact, s'il en a une.
    #[must_use]
    pub fn photo(&self) -> Option<&BlobId> {
        self.photo.as_ref()
    }

    /// Le même contact, avec la photo `photo` (relue par le stockage).
    #[must_use]
    pub fn with_photo(self, photo: Option<BlobId>) -> Self {
        Self { photo, ..self }
    }
}

/// Nom à afficher pour un contact : le nom affiché, sinon prénom et nom,
/// sinon le surnom, sinon l'adresse préférée.
#[must_use]
pub fn display_label(
    display_name: Option<&str>,
    given_name: Option<&str>,
    family_name: Option<&str>,
    nickname: Option<&str>,
    address: &EmailAddress,
) -> String {
    if let Some(name) = display_name {
        return name.to_owned();
    }
    match (given_name, family_name) {
        (Some(given), Some(family)) => return compose_full_name(given, family),
        (Some(one), None) | (None, Some(one)) => return one.to_owned(),
        (None, None) => {}
    }
    nickname.map_or_else(|| address.to_string(), str::to_owned)
}

/// Nom complet « prénom nom », affiché ([`display_label`]) comme cherché
/// ([`ContactDraft::full_name`]).
fn compose_full_name(given: &str, family: &str) -> String {
    format!("{given} {family}")
}

/// Caractères invisibles ou trompeurs, refusés dans une fiche (saisie ou
/// importée) en plus des caractères de contrôle : catégorie Unicode Cf
/// (format), séparateurs de ligne et de paragraphe, sélecteurs de
/// variante, étiquettes, remplisseurs Hangul et blanc braille, joints de
/// graphèmes ; toute la propriété Unicode Default_Ignorable_Code_Point,
/// points réservés compris (ils pourront être attribués). Ils permettent de faire passer un nom ou une adresse pour un
/// autre à l'écran.
const INVISIBLE: [(char, char); 26] = [
    ('\u{00AD}', '\u{00AD}'),
    ('\u{034F}', '\u{034F}'),
    ('\u{0600}', '\u{0605}'),
    ('\u{061C}', '\u{061C}'),
    ('\u{06DD}', '\u{06DD}'),
    ('\u{070F}', '\u{070F}'),
    ('\u{0890}', '\u{0891}'),
    ('\u{08E2}', '\u{08E2}'),
    ('\u{115F}', '\u{1160}'),
    ('\u{17B4}', '\u{17B5}'),
    ('\u{180B}', '\u{180F}'),
    ('\u{200B}', '\u{200F}'),
    ('\u{2028}', '\u{202E}'),
    ('\u{2060}', '\u{206F}'),
    ('\u{2800}', '\u{2800}'),
    ('\u{3164}', '\u{3164}'),
    ('\u{FE00}', '\u{FE0F}'),
    ('\u{FEFF}', '\u{FEFF}'),
    ('\u{FFA0}', '\u{FFA0}'),
    ('\u{FFF0}', '\u{FFFB}'),
    ('\u{110BD}', '\u{110BD}'),
    ('\u{110CD}', '\u{110CD}'),
    ('\u{13430}', '\u{1343F}'),
    ('\u{1BCA0}', '\u{1BCA3}'),
    ('\u{1D173}', '\u{1D17A}'),
    ('\u{E0000}', '\u{E0FFF}'),
];

/// Caractère invisible d'une fiche ([`INVISIBLE`], qui couvre aussi les
/// caractères de mise en forme refusés partout ailleurs).
pub(crate) fn is_invisible(c: char) -> bool {
    is_format_char(c)
        || INVISIBLE
            .iter()
            .any(|&(low, high)| (low..=high).contains(&c))
}

/// Caractère interdit dans un champ d'une ligne.
pub(crate) fn forbidden(c: char) -> bool {
    c.is_control() || is_invisible(c)
}

/// Champ d'une ligne : sans espaces de bord, borné, sans caractère de
/// contrôle ni de mise en forme ; vide : absent.
fn text(raw: &str, field: ContactField, max: usize) -> Result<Option<String>, ContactError> {
    let value = raw.trim();
    if value.chars().any(forbidden) {
        return Err(ContactError::InvalidCharacter(field));
    }
    if value.chars().count() > max {
        return Err(ContactError::TooLong(field));
    }
    Ok((!value.is_empty()).then(|| value.to_owned()))
}

/// Titre, anniversaire, adresses postales et sites web d'une fiche.
type Details = (
    Option<String>,
    Option<Birthday>,
    Vec<ContactPostalAddress>,
    Vec<ContactWebsite>,
);

/// Valide le titre, l'anniversaire, les adresses postales et les sites
/// web de `input`.
fn details(input: &ContactInput) -> Result<Details, ContactError> {
    if input.postal_addresses.len() > MAX_POSTAL_ADDRESSES {
        return Err(ContactError::TooManyPostalAddresses);
    }
    if input.websites.len() > MAX_CONTACT_WEBSITES {
        return Err(ContactError::TooManyWebsites);
    }
    let title = text(&input.title, ContactField::Title, MAX_CONTACT_NAME_CHARS)?;
    let birthday = match input.birthday.trim() {
        "" => None,
        raw => Some(Birthday::parse(raw).ok_or(ContactError::InvalidBirthday)?),
    };

    let mut postal_addresses = Vec::with_capacity(input.postal_addresses.len());
    for (index, a) in input.postal_addresses.iter().enumerate() {
        let field = ContactField::PostalAddress(index);
        let part = |raw: &str| text(raw, field, MAX_POSTAL_FIELD_CHARS);
        let address = ContactPostalAddress {
            street: street(&a.street, field)?,
            locality: part(&a.locality)?,
            region: part(&a.region)?,
            postal_code: part(&a.postal_code)?,
            country: part(&a.country)?,
            label: text(
                &a.label,
                ContactField::PostalAddressLabel(index),
                MAX_CONTACT_LABEL_CHARS,
            )?,
        };
        let parts = [
            &address.street,
            &address.locality,
            &address.region,
            &address.postal_code,
            &address.country,
        ];
        if parts.iter().all(|p| p.is_none()) {
            return Err(ContactError::EmptyPostalAddress { index });
        }
        postal_addresses.push(address);
    }

    let mut websites = Vec::with_capacity(input.websites.len());
    for (index, w) in input.websites.iter().enumerate() {
        let label = text(
            &w.label,
            ContactField::WebsiteLabel(index),
            MAX_CONTACT_LABEL_CHARS,
        )?;
        // Caractère invisible ou de contrôle : refusé comme dans un autre
        // champ, avant la règle du lien.
        if w.url.trim().chars().any(forbidden) {
            return Err(ContactError::InvalidCharacter(ContactField::Website(index)));
        }
        let url = website_url(&w.url).ok_or(ContactError::InvalidWebsite { index })?;
        websites.push(ContactWebsite { url, label });
    }
    Ok((title, birthday, postal_addresses, websites))
}

/// Rue : comme [`text`] ligne à ligne (sauts de ligne admis, `\r\n` ramené
/// à `\n`), lignes vides retirées, bornée à [`MAX_POSTAL_STREET_CHARS`].
fn street(raw: &str, field: ContactField) -> Result<Option<String>, ContactError> {
    let normalized = raw.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.iter().any(|line| line.chars().any(forbidden)) {
        return Err(ContactError::InvalidCharacter(field));
    }
    let value = lines.join("\n");
    if value.chars().count() > MAX_POSTAL_STREET_CHARS {
        return Err(ContactError::TooLong(field));
    }
    Ok((!value.is_empty()).then_some(value))
}

/// Notes : comme [`text`], sauts de ligne (`\r\n` ramené à `\n`) et
/// tabulations admis.
fn notes(raw: &str) -> Result<Option<String>, ContactError> {
    let value = raw.replace("\r\n", "\n");
    let value = value.trim();
    if value
        .chars()
        .any(|c| c != '\n' && c != '\t' && forbidden(c))
    {
        return Err(ContactError::InvalidCharacter(ContactField::Notes));
    }
    if value.chars().count() > MAX_CONTACT_NOTES_CHARS {
        return Err(ContactError::TooLong(ContactField::Notes));
    }
    Ok((!value.is_empty()).then(|| value.to_owned()))
}

/// Sort d'une fiche enregistrée avant que les caractères [`INVISIBLE`]
/// ne soient refusés à la saisie ([`clean_invisible`]).
// Valeur passagère, rendue fiche par fiche pendant une migration.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvisibleCleanup {
    /// Rien à changer : la fiche n'en contient pas, ou elle est invalide
    /// pour une autre raison (laissée telle quelle).
    Keep,
    /// Fiche nettoyée, à réécrire.
    Rewrite(ContactDraft),
    /// Plus aucune adresse utilisable : la fiche est retirée.
    Remove,
}

/// Nettoie une fiche enregistrée qui contient des caractères invisibles :
/// ils sont retirés des textes (séparateur de ligne remplacé), un champ
/// ainsi vidé devient absent, un téléphone sans numéro disparaît. Une
/// adresse qui en contient est écartée, jamais transformée (ce serait une
/// autre boîte) ; si c'était la préférée, la première restante le devient ;
/// sans adresse restante, la fiche est retirée.
#[must_use]
pub fn clean_invisible(input: ContactInput) -> InvisibleCleanup {
    let dirty = |text: &str| text.chars().any(is_invisible);
    let has_invisible = [
        &input.display_name,
        &input.given_name,
        &input.family_name,
        &input.nickname,
        &input.organization,
        &input.notes,
    ]
    .into_iter()
    .any(|t| dirty(t))
        || input
            .emails
            .iter()
            .any(|e| dirty(&e.address) || dirty(&e.label))
        || input
            .phones
            .iter()
            .any(|p| dirty(&p.number) || dirty(&p.label));
    if !has_invisible {
        return InvisibleCleanup::Keep;
    }
    // Séparateur de ligne ou de paragraphe : remplacé (espace, ou saut de
    // ligne dans les notes) pour ne pas coller les mots ; les autres
    // caractères invisibles sont retirés.
    let replace = |text: &str, separator: char| {
        text.chars()
            .filter_map(|c| match c {
                '\u{2028}' | '\u{2029}' => Some(separator),
                c if is_invisible(c) => None,
                c => Some(c),
            })
            .collect::<String>()
    };
    let strip = |text: &str| replace(text, ' ');
    // Les adresses gardées sont inchangées : leurs clés, uniques dans le
    // carnet, le restent.
    let emails: Vec<EmailInput> = input
        .emails
        .iter()
        .filter(|e| !dirty(&e.address))
        .map(|e| EmailInput {
            address: e.address.clone(),
            label: strip(&e.label),
            preferred: e.preferred,
        })
        .collect();
    if emails.is_empty() {
        return InvisibleCleanup::Remove;
    }
    let phones = input
        .phones
        .iter()
        .map(|p| PhoneInput {
            number: strip(&p.number),
            label: strip(&p.label),
        })
        .filter(|p| !p.number.trim().is_empty())
        .collect();
    let cleaned = ContactInput {
        display_name: strip(&input.display_name),
        given_name: strip(&input.given_name),
        family_name: strip(&input.family_name),
        nickname: strip(&input.nickname),
        organization: strip(&input.organization),
        notes: replace(&input.notes, '\n'),
        emails,
        phones,
        // Champs apparus après ce nettoyage (migration 18) : vides alors.
        title: input.title.clone(),
        birthday: input.birthday.clone(),
        postal_addresses: input.postal_addresses.clone(),
        websites: input.websites.clone(),
        favorite: input.favorite,
    };
    // Retirer des caractères ne rend invalide aucune fiche qui suivait les
    // règles d'avant : un refus vient d'un autre défaut de la ligne.
    ContactDraft::new(cleaned).map_or(InvisibleCleanup::Keep, InvisibleCleanup::Rewrite)
}

/// Nettoie une fiche enregistrée avant que les noms qui imitent une
/// adresse ne soient refusés à la saisie : ces noms sont retirés, comme à
/// l'import. Une fiche qui contient aussi des caractères invisibles (restée
/// telle quelle au nettoyage de [`clean_invisible`], qui la validait avec
/// ce nom) en est nettoyée du même coup. Sans tel nom : [`InvisibleCleanup::Keep`].
#[must_use]
pub fn clean_names_imitating_address(mut input: ContactInput) -> InvisibleCleanup {
    if !input.drop_names_imitating_address() {
        return InvisibleCleanup::Keep;
    }
    match clean_invisible(input.clone()) {
        InvisibleCleanup::Keep => {
            ContactDraft::new(input).map_or(InvisibleCleanup::Keep, InvisibleCleanup::Rewrite)
        }
        cleaned => cleaned,
    }
}

/// Filtre de la liste des contacts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactFilter {
    text: String,
    favorites_only: bool,
}

impl ContactFilter {
    /// Filtre sur `text` (sans espaces de bord, sans distinction de casse,
    /// borné à [`MAX_CONTACT_FILTER_CHARS`] ; vide : tous) et, au besoin,
    /// les seuls favoris.
    #[must_use]
    pub fn new(text: &str, favorites_only: bool) -> Self {
        Self {
            text: header_text(text, MAX_CONTACT_FILTER_CHARS).to_lowercase(),
            favorites_only,
        }
    }

    /// Texte recherché, normalisé (minuscules) ; vide : pas de filtre
    /// texte. Il est cherché tel quel dans [`ContactDraft::search_text`].
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn favorites_only(&self) -> bool {
        self.favorites_only
    }

    /// Le contact passe-t-il le filtre ?
    #[must_use]
    pub fn matches(&self, contact: &ContactDraft) -> bool {
        (!self.favorites_only || contact.favorite)
            && (self.text.is_empty() || contact.search_text().contains(&self.text))
    }
}

/// Sélection de fiches refusée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContactSelectionError {
    #[error("identifiant de contact invalide")]
    InvalidId,
    #[error("plus de {MAX_CONTACT_SELECTION} contacts visés")]
    TooMany,
}

/// Fiches visées par une action (suppression) : identifiants valides,
/// chacun une fois dans l'ordre de la première apparition, au plus
/// [`MAX_CONTACT_SELECTION`] fiches distinctes. Plus de deux fois cette
/// borne d'identifiants reçus (doublons compris) : refusé d'emblée, sans
/// parcourir la liste.
pub fn contact_selection<T: Copy + Into<i64>>(
    raw: &[T],
) -> Result<Vec<ContactId>, ContactSelectionError> {
    if raw.len() > 2 * MAX_CONTACT_SELECTION {
        return Err(ContactSelectionError::TooMany);
    }
    let mut ids: Vec<ContactId> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for &value in raw {
        let id = ContactId::new(value.into()).ok_or(ContactSelectionError::InvalidId)?;
        if seen.insert(id) {
            if ids.len() == MAX_CONTACT_SELECTION {
                return Err(ContactSelectionError::TooMany);
            }
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Fiches à lire pour `ids` ([`crate::ports::ContactDirectory::get_many`]) :
/// chaque identifiant une fois, dans l'ordre de sa première apparition,
/// les [`MAX_CONTACT_LOOKUP`] premiers distincts au plus.
#[must_use]
pub fn lookup_ids(ids: &[ContactId]) -> Vec<ContactId> {
    let mut kept = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for &id in ids {
        if kept.len() == MAX_CONTACT_LOOKUP {
            break;
        }
        if seen.insert(id) {
            kept.push(id);
        }
    }
    kept
}

/// Fiches qu'un carnet de `current` fiches peut encore recevoir avant
/// [`MAX_CONTACTS`] (aucune s'il en compte déjà autant ou plus).
#[must_use]
pub fn contacts_room(current: usize) -> usize {
    MAX_CONTACTS.saturating_sub(current)
}

/// Un carnet de `current` fiches peut-il en recevoir `adding` de plus sans
/// dépasser [`MAX_CONTACTS`] ? Tout ou rien : un import qui ne tient pas
/// en entier est refusé, jamais tronqué.
#[must_use]
pub fn fits_in_book(current: usize, adding: usize) -> bool {
    adding <= contacts_room(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_ids_are_distinct_in_order_and_bounded() {
        let id = |n: i64| ContactId::new(n).unwrap_or_else(|| panic!("id"));
        assert!(lookup_ids(&[]).is_empty());
        assert_eq!(
            lookup_ids(&[id(3), id(1), id(3), id(2), id(1)]),
            [id(3), id(1), id(2)]
        );
        // Borne : identifiants distincts, les doublons ne comptent pas.
        let mut many: Vec<ContactId> = (1..=MAX_CONTACT_LOOKUP as i64 + 5).map(id).collect();
        many.insert(1, id(1));
        let kept = lookup_ids(&many);
        assert_eq!(kept.len(), MAX_CONTACT_LOOKUP);
        assert_eq!(kept.first(), Some(&id(1)));
        assert_eq!(kept.last(), Some(&id(MAX_CONTACT_LOOKUP as i64)));
    }

    #[test]
    fn the_book_takes_cards_up_to_its_cap_and_no_more() {
        assert_eq!(contacts_room(0), MAX_CONTACTS);
        assert_eq!(contacts_room(MAX_CONTACTS - 3), 3);
        assert_eq!(contacts_room(MAX_CONTACTS), 0);
        // Carnet déjà au-delà (plafond abaissé, écriture étrangère) : rien.
        assert_eq!(contacts_room(MAX_CONTACTS + 10), 0);
        assert!(fits_in_book(0, MAX_CONTACTS));
        assert!(!fits_in_book(0, MAX_CONTACTS + 1));
        assert!(fits_in_book(MAX_CONTACTS - 1, 1));
        assert!(!fits_in_book(MAX_CONTACTS - 1, 2));
        assert!(fits_in_book(MAX_CONTACTS, 0), "rien à ajouter : accepté");
        assert!(!fits_in_book(MAX_CONTACTS, 1));
        assert!(!fits_in_book(usize::MAX, usize::MAX));
    }

    #[test]
    fn a_full_book_stays_exportable_scannable_and_synchronisable() {
        // Un carnet vide reçoit un fichier de taille maximale.
        assert!(MAX_CONTACTS >= crate::contact_import::MAX_IMPORT_CARDS);
        // Un carnet plein s'exporte en entier, et le fichier se réimporte.
        assert!(MAX_CONTACTS <= crate::contact_export::MAX_EXPORT_CONTACTS);
        assert!(MAX_CONTACTS <= crate::contact_import::MAX_IMPORT_CARDS);
        // La recherche des doublons le parcourt en entier.
        assert!(MAX_CONTACTS <= crate::contact_duplicates::MAX_DUPLICATE_SCAN);
        // La synchronisation CardDAV l'envoie en entier, et le carnet
        // distant qui en résulte reste lisible.
        assert!(MAX_CONTACTS <= crate::carddav::MAX_REMOTE_CARDS);
        // Les identifiants ne s'épuisent pas avant le plafond.
        assert!(i64::try_from(MAX_CONTACTS).is_ok_and(|max| max < MAX_CONTACT_ID));
    }

    fn email(address: &str) -> EmailInput {
        EmailInput {
            address: address.to_owned(),
            ..EmailInput::default()
        }
    }

    fn input(emails: &[&str]) -> ContactInput {
        ContactInput {
            emails: emails.iter().map(|a| email(a)).collect(),
            ..ContactInput::default()
        }
    }

    fn draft(input: ContactInput) -> ContactDraft {
        ContactDraft::new(input).unwrap_or_else(|e| panic!("fiche refusée : {e}"))
    }

    fn addr(raw: &str) -> EmailAddress {
        EmailAddress::parse(raw).unwrap_or_else(|e| panic!("{raw} : {e}"))
    }

    #[test]
    fn contact_id_is_strictly_positive() {
        assert_eq!(ContactId::new(0), None);
        assert_eq!(ContactId::new(-3), None);
        assert_eq!(ContactId::new(7).map(ContactId::get), Some(7));
        assert_eq!(ContactId::new(7).map(|i| i.to_string()), Some("7".into()));
    }

    /// Les identifiants exposés à l'interface tiennent sur 32 bits : un
    /// identifiant au-delà ne désigne aucune fiche valide.
    #[test]
    fn contact_id_fits_in_32_bits() {
        let max = i64::from(u32::MAX);
        assert_eq!(MAX_CONTACT_ID, max);
        assert_eq!(ContactId::new(max).map(ContactId::get), Some(max));
        assert_eq!(ContactId::new(max + 1), None);
        assert_eq!(ContactId::new(i64::MAX), None);
    }

    #[test]
    fn address_key_ignores_case() {
        assert_eq!(
            AddressKey::of(&addr("Alice.Martin@Example.COM")).as_str(),
            "alice.martin@example.com"
        );
        assert_eq!(
            AddressKey::parse(" ALICE@example.com "),
            AddressKey::parse("alice@EXAMPLE.com")
        );
        assert!(AddressKey::parse("pas une adresse").is_err());
    }

    #[test]
    fn address_key_folds_ascii_only() {
        // Repli Unicode instable d'une version à l'autre et source de
        // collisions (signe kelvin U+212A → « k ») : seul l'ASCII est plié.
        assert_eq!(
            AddressKey::of(&addr("\u{212A}evin@example.org")).as_str(),
            "\u{212A}evin@example.org"
        );
        assert_ne!(
            AddressKey::parse("\u{212A}evin@example.org"),
            AddressKey::parse("kevin@example.org")
        );
        assert_eq!(
            AddressKey::of(&addr("ÉLodie@Example.fr")).as_str(),
            "Élodie@example.fr"
        );
    }

    #[test]
    fn minimal_contact_is_one_address_preferred() {
        let d = draft(input(&["alice@example.com"]));
        assert_eq!(d.emails().len(), 1);
        assert!(d.preferred_email().preferred());
        assert_eq!(d.preferred_email().address(), &addr("alice@example.com"));
        assert_eq!(d.given_name(), None);
        assert_eq!(d.notes(), None);
        assert!(!d.favorite());
    }

    #[test]
    fn fields_are_trimmed_and_empty_ones_absent() {
        let d = draft(ContactInput {
            display_name: "  Alice Martin ".into(),
            given_name: " Alice".into(),
            family_name: "   ".into(),
            nickname: String::new(),
            organization: "ACME ".into(),
            notes: "\n  ligne 1\n\tligne 2  \n".into(),
            emails: vec![EmailInput {
                address: " alice@Example.com ".into(),
                label: " travail ".into(),
                preferred: false,
            }],
            phones: vec![PhoneInput {
                number: " +33 6 12 34 56 78 ".into(),
                label: "  ".into(),
            }],
            favorite: true,
            ..ContactInput::default()
        });
        assert_eq!(d.display_name_field(), Some("Alice Martin"));
        assert_eq!(d.given_name(), Some("Alice"));
        assert_eq!(d.family_name(), None);
        assert_eq!(d.nickname(), None);
        assert_eq!(d.organization(), Some("ACME"));
        assert_eq!(d.notes(), Some("ligne 1\n\tligne 2"));
        assert_eq!(d.emails()[0].label(), Some("travail"));
        assert_eq!(d.emails()[0].address().to_string(), "alice@example.com");
        assert_eq!(d.phones()[0].number(), "+33 6 12 34 56 78");
        assert_eq!(d.phones()[0].label(), None);
        assert!(d.favorite());
    }

    #[test]
    fn at_least_one_address() {
        assert_eq!(
            ContactDraft::new(ContactInput {
                display_name: "Alice".into(),
                ..ContactInput::default()
            }),
            Err(ContactError::NoAddress)
        );
    }

    #[test]
    fn invalid_address_is_located() {
        assert_eq!(
            ContactDraft::new(input(&["alice@example.com", "bob"])),
            Err(ContactError::InvalidAddress {
                index: 1,
                error: EmailAddressError::MissingAt
            })
        );
        assert_eq!(
            ContactDraft::new(input(&["  "])),
            Err(ContactError::InvalidAddress {
                index: 0,
                error: EmailAddressError::Empty
            })
        );
    }

    #[test]
    fn addresses_are_distinct_ignoring_case() {
        assert_eq!(
            ContactDraft::new(input(&[
                "alice@example.com",
                "bob@example.com",
                "ALICE@example.COM"
            ])),
            Err(ContactError::DuplicateAddress { index: 2 })
        );
    }

    #[test]
    fn exactly_one_preferred_address() {
        let mut two = input(&["a@example.com", "b@example.com", "c@example.com"]);
        two.emails[1].preferred = true;
        let d = draft(two.clone());
        assert_eq!(d.preferred_email().address(), &addr("b@example.com"));
        assert_eq!(
            d.emails().iter().filter(|e| e.preferred()).count(),
            1,
            "une seule préférée"
        );
        // L'ordre de la fiche est conservé.
        assert_eq!(d.emails()[0].address(), &addr("a@example.com"));

        two.emails[2].preferred = true;
        assert_eq!(ContactDraft::new(two), Err(ContactError::SeveralPreferred));

        let none = draft(input(&["a@example.com", "b@example.com"]));
        assert_eq!(none.preferred_email().address(), &addr("a@example.com"));
        assert!(!none.emails()[1].preferred());
    }

    #[test]
    fn counts_are_bounded() {
        let many: Vec<String> = (0..=MAX_CONTACT_EMAILS)
            .map(|i| format!("a{i}@example.com"))
            .collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(
            ContactDraft::new(input(&refs)),
            Err(ContactError::TooManyAddresses)
        );
        assert!(ContactDraft::new(input(&refs[..MAX_CONTACT_EMAILS])).is_ok());

        let mut phones = input(&["a@example.com"]);
        phones.phones = vec![
            PhoneInput {
                number: "01".into(),
                label: String::new()
            };
            MAX_CONTACT_PHONES + 1
        ];
        assert_eq!(
            ContactDraft::new(phones.clone()),
            Err(ContactError::TooManyPhones)
        );
        phones.phones.pop();
        assert!(ContactDraft::new(phones).is_ok());
    }

    #[test]
    fn empty_phone_is_refused() {
        let mut i = input(&["a@example.com"]);
        i.phones = vec![
            PhoneInput {
                number: "01".into(),
                label: String::new(),
            },
            PhoneInput {
                number: "  ".into(),
                label: "maison".into(),
            },
        ];
        assert_eq!(
            ContactDraft::new(i),
            Err(ContactError::EmptyPhone { index: 1 })
        );
    }

    #[test]
    fn lengths_are_bounded() {
        let long = |n: usize| "é".repeat(n);
        type Setter = Box<dyn Fn(&mut ContactInput, String)>;
        let cases: Vec<(ContactField, Setter, usize)> = vec![
            (
                ContactField::DisplayName,
                Box::new(|i, s| i.display_name = s),
                MAX_CONTACT_NAME_CHARS,
            ),
            (
                ContactField::GivenName,
                Box::new(|i, s| i.given_name = s),
                MAX_CONTACT_NAME_CHARS,
            ),
            (
                ContactField::FamilyName,
                Box::new(|i, s| i.family_name = s),
                MAX_CONTACT_NAME_CHARS,
            ),
            (
                ContactField::Nickname,
                Box::new(|i, s| i.nickname = s),
                MAX_CONTACT_NAME_CHARS,
            ),
            (
                ContactField::Organization,
                Box::new(|i, s| i.organization = s),
                MAX_CONTACT_NAME_CHARS,
            ),
            (
                ContactField::Notes,
                Box::new(|i, s| i.notes = s),
                MAX_CONTACT_NOTES_CHARS,
            ),
            (
                ContactField::EmailLabel(0),
                Box::new(|i, s| i.emails[0].label = s),
                MAX_CONTACT_LABEL_CHARS,
            ),
            (
                ContactField::Phone(0),
                Box::new(|i, s| i.phones[0].number = s),
                MAX_PHONE_CHARS,
            ),
            (
                ContactField::PhoneLabel(0),
                Box::new(|i, s| i.phones[0].label = s),
                MAX_CONTACT_LABEL_CHARS,
            ),
        ];
        for (field, set, max) in cases {
            let mut i = input(&["a@example.com"]);
            i.phones = vec![PhoneInput {
                number: "01".into(),
                label: String::new(),
            }];
            set(&mut i, long(max));
            assert!(
                ContactDraft::new(i.clone()).is_ok(),
                "{field:?} à la limite"
            );
            set(&mut i, long(max + 1));
            assert_eq!(
                ContactDraft::new(i),
                Err(ContactError::TooLong(field)),
                "{field:?}"
            );
        }
    }

    #[test]
    fn control_and_invisible_characters_are_refused() {
        for bad in [
            "a\nb",
            "a\rb",
            "a\0b",
            "a\u{202E}b",
            "a\u{200B}b",
            "a\u{7}b",
        ] {
            let mut i = input(&["a@example.com"]);
            i.display_name = bad.into();
            assert_eq!(
                ContactDraft::new(i),
                Err(ContactError::InvalidCharacter(ContactField::DisplayName)),
                "{bad:?}"
            );
            let mut i = input(&["a@example.com"]);
            i.phones = vec![PhoneInput {
                number: "01".into(),
                label: bad.into(),
            }];
            assert_eq!(
                ContactDraft::new(i),
                Err(ContactError::InvalidCharacter(ContactField::PhoneLabel(0))),
                "{bad:?}"
            );
        }
        // Les notes admettent sauts de ligne et tabulations, rien d'autre.
        let mut i = input(&["a@example.com"]);
        i.notes = "a\r\nb\tc".into();
        assert_eq!(draft(i).notes(), Some("a\nb\tc"));
        for bad in ["a\0b", "a\u{202E}b", "a\u{1B}b"] {
            let mut i = input(&["a@example.com"]);
            i.notes = bad.into();
            assert_eq!(
                ContactDraft::new(i),
                Err(ContactError::InvalidCharacter(ContactField::Notes)),
                "{bad:?}"
            );
        }
    }

    /// Caractères invisibles refusés à l'import, refusés aussi à la saisie.
    const INVISIBLES: [char; 22] = [
        '\u{FFF0}',
        '\u{E0000}',
        '\u{E0080}',
        '\u{E0FFF}',
        '\u{17B4}',
        '\u{17B5}',
        '\u{2028}',
        '\u{2029}',
        '\u{206A}',
        '\u{206F}',
        '\u{FFF9}',
        '\u{FFFB}',
        '\u{E0001}',
        '\u{E0041}',
        '\u{E007F}',
        '\u{FE00}',
        '\u{FE0F}',
        '\u{E0100}',
        '\u{115F}',
        '\u{3164}',
        '\u{FFA0}',
        '\u{2800}',
    ];

    #[test]
    fn invisible_characters_are_refused_at_entry() {
        for c in INVISIBLES {
            let bad = format!("a{c}b");
            let mut i = input(&["a@example.com"]);
            i.display_name.clone_from(&bad);
            assert_eq!(
                ContactDraft::new(i),
                Err(ContactError::InvalidCharacter(ContactField::DisplayName)),
                "{c:?}"
            );
            let mut i = input(&["a@example.com"]);
            i.phones = vec![PhoneInput {
                number: bad.clone(),
                label: String::new(),
            }];
            assert_eq!(
                ContactDraft::new(i),
                Err(ContactError::InvalidCharacter(ContactField::Phone(0))),
                "{c:?}"
            );
            let mut i = input(&["a@example.com"]);
            i.notes = format!("ligne\n{bad}");
            assert_eq!(
                ContactDraft::new(i),
                Err(ContactError::InvalidCharacter(ContactField::Notes)),
                "{c:?}"
            );
            assert_eq!(
                ContactDraft::new(input(&["a@example.com", &format!("{bad}@example.com")])),
                Err(ContactError::InvalidAddress {
                    index: 1,
                    error: EmailAddressError::InvalidLocalCharacter,
                }),
                "{c:?}"
            );
        }
    }

    fn cleanup(input: ContactInput) -> InvisibleCleanup {
        clean_invisible(input)
    }

    fn rewritten(input: ContactInput) -> ContactDraft {
        match cleanup(input) {
            InvisibleCleanup::Rewrite(draft) => draft,
            other => panic!("fiche non réécrite : {other:?}"),
        }
    }

    #[test]
    fn clean_contact_is_kept() {
        let mut i = input(&["alice@example.com"]);
        i.display_name = "Alice\u{00E9}".into();
        i.notes = "a\nb\tc".into();
        assert_eq!(cleanup(i), InvisibleCleanup::Keep);
    }

    #[test]
    fn cleanup_strips_invisible_characters_from_texts() {
        let mut i = input(&["alice@example.com"]);
        i.display_name = "Ali\u{2800}ce".into();
        i.given_name = "\u{3164}".into();
        i.family_name = "Mar\u{E0041}tin".into();
        i.nickname = "Lili\u{FE0F}".into();
        i.organization = "\u{2028}Acme\u{2029}".into();
        i.notes = "a\n\u{FFF9}b\tc".into();
        i.emails[0].label = "tra\u{206A}vail".into();
        i.phones = vec![
            PhoneInput {
                number: "01\u{115F} 02".into(),
                label: "mai\u{FFA0}son".into(),
            },
            PhoneInput {
                number: "\u{3164}".into(),
                label: "vide".into(),
            },
        ];
        let d = rewritten(i);
        assert_eq!(d.display_name_field(), Some("Alice"));
        assert_eq!(d.given_name(), None);
        assert_eq!(d.family_name(), Some("Martin"));
        assert_eq!(d.nickname(), Some("Lili"));
        assert_eq!(d.organization(), Some("Acme"));
        assert_eq!(d.notes(), Some("a\nb\tc"));
        assert_eq!(d.emails()[0].label(), Some("travail"));
        // Un téléphone sans numéro une fois nettoyé disparaît.
        assert_eq!(d.phones().len(), 1);
        assert_eq!(d.phones()[0].number(), "01 02");
        assert_eq!(d.phones()[0].label(), Some("maison"));
    }

    /// Une adresse n'est jamais transformée (ce serait une autre boîte) :
    /// celle qui contient un caractère invisible est écartée.
    #[test]
    fn cleanup_drops_addresses_with_invisible_characters() {
        let mut i = input(&[
            "ali\u{FE0F}ce@example.com",
            "bob@example.com",
            "carol@example.com",
        ]);
        i.emails[0].preferred = true;
        let d = rewritten(i);
        let keys: Vec<&str> = d.emails().iter().map(|e| e.key().as_str()).collect();
        assert_eq!(keys, ["bob@example.com", "carol@example.com"]);
        // L'adresse préférée écartée : la première restante le devient.
        assert!(d.emails()[0].preferred());
    }

    #[test]
    fn cleanup_removes_a_contact_left_without_address() {
        let mut i = input(&["ali\u{2800}ce@example.com"]);
        i.display_name = "Alice".into();
        assert_eq!(cleanup(i), InvisibleCleanup::Remove);
    }

    #[test]
    fn cleanup_turns_line_separators_into_spaces_or_line_breaks() {
        let mut i = input(&["alice@example.com"]);
        i.display_name = "Jean\u{2029}Dupont".into();
        i.notes = "Appeler\u{2028}lundi".into();
        let d = rewritten(i);
        assert_eq!(d.display_name_field(), Some("Jean Dupont"));
        assert_eq!(d.notes(), Some("Appeler\nlundi"));
    }

    #[test]
    fn cleanup_keeps_a_contact_invalid_for_another_reason() {
        let mut i = input(&["alice@example.com"]);
        i.display_name = "Ali\u{2800}ce\u{0}".into();
        assert_eq!(cleanup(i), InvisibleCleanup::Keep);
    }

    /// Remplit un champ de nom d'une saisie.
    type NameSetter = fn(&mut ContactInput, &str);

    /// Champs de nom d'une fiche, et comment les remplir.
    fn name_fields() -> Vec<(ContactField, NameSetter)> {
        vec![
            (ContactField::DisplayName, |i, s| i.display_name = s.into()),
            (ContactField::GivenName, |i, s| i.given_name = s.into()),
            (ContactField::FamilyName, |i, s| i.family_name = s.into()),
            (ContactField::Nickname, |i, s| i.nickname = s.into()),
        ]
    }

    #[test]
    fn names_imitating_an_address_are_refused() {
        for (field, set) in name_fields() {
            for bad in ["ceo@bank.example", "Banque <service@banque.fr>", "a > b"] {
                let mut i = input(&["a@example.com"]);
                set(&mut i, bad);
                assert_eq!(
                    ContactDraft::new(i),
                    Err(ContactError::NameLooksLikeAddress(field)),
                    "{field:?} : {bad:?}"
                );
            }
            // Guillemets français : un nom ordinaire.
            let mut i = input(&["a@example.com"]);
            set(&mut i, "Alice « Lili » Martin");
            assert!(ContactDraft::new(i).is_ok(), "{field:?}");
        }
        // L'organisation ne s'affiche pas comme nom du correspondant.
        let mut i = input(&["a@example.com"]);
        i.organization = "ACME <ventes>".into();
        assert_eq!(draft(i).organization(), Some("ACME <ventes>"));
    }

    #[test]
    fn a_stored_card_drops_its_names_imitating_an_address() {
        let mut i = input(&["pirate@example.net"]);
        i.display_name = "ceo@bank.example".into();
        i.given_name = "Bob".into();
        i.family_name = "<x>".into();
        i.nickname = "Lili".into();
        let stored = ContactDraft::from_stored(i).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(stored.display_name_field(), None);
        assert_eq!(stored.given_name(), Some("Bob"));
        assert_eq!(stored.family_name(), None);
        assert_eq!(stored.nickname(), Some("Lili"));
        assert_eq!(stored.display_name(), "Bob");
        assert!(
            !stored.search_text().contains("bank"),
            "{}",
            stored.search_text()
        );
        // Les autres règles s'appliquent toujours.
        let mut i = input(&[]);
        i.display_name = "ceo@bank.example".into();
        assert_eq!(ContactDraft::from_stored(i), Err(ContactError::NoAddress));
    }

    #[test]
    fn dropping_names_imitating_an_address_says_whether_one_was() {
        let mut i = input(&["a@example.com"]);
        i.given_name = "Alice".into();
        i.organization = "a@b".into();
        assert!(!i.has_names_imitating_address());
        assert!(!i.drop_names_imitating_address());
        assert_eq!(i.given_name, "Alice");
        assert_eq!(i.organization, "a@b");
        i.nickname = "x@y".into();
        assert!(i.has_names_imitating_address());
        assert!(i.drop_names_imitating_address());
        assert!(!i.has_names_imitating_address());
        assert_eq!(i.nickname, "");
        assert_eq!(i.given_name, "Alice");
    }

    #[test]
    fn stored_names_imitating_an_address_are_cleaned() {
        // Sans tel nom : rien à faire.
        let mut i = input(&["a@example.com"]);
        i.display_name = "Alice".into();
        assert_eq!(clean_names_imitating_address(i), InvisibleCleanup::Keep);
        // Nom retiré, fiche réécrite.
        let mut i = input(&["a@example.com"]);
        i.display_name = "ceo@bank.example".into();
        i.given_name = "Bob".into();
        let InvisibleCleanup::Rewrite(d) = clean_names_imitating_address(i) else {
            panic!("fiche non réécrite");
        };
        assert_eq!(
            (d.display_name_field(), d.given_name()),
            (None, Some("Bob"))
        );
        // Fiche invalide pour une autre raison : laissée telle quelle.
        let mut i = input(&["a@example.com"]);
        i.display_name = "x@y".into();
        i.notes = "a\0b".into();
        assert_eq!(clean_names_imitating_address(i), InvisibleCleanup::Keep);
    }

    #[test]
    fn stored_names_imitating_an_address_are_cleaned_with_invisible_characters() {
        // Restée telle quelle au nettoyage des invisibles (nom refusé) :
        // nettoyée des deux à la fois.
        let mut i = input(&["ali\u{3164}ce@example.com", "alice@example.com"]);
        i.display_name = "ceo@bank.example".into();
        i.given_name = "Ali\u{2800}ce".into();
        assert_eq!(clean_invisible(i.clone()), InvisibleCleanup::Keep);
        let InvisibleCleanup::Rewrite(d) = clean_names_imitating_address(i) else {
            panic!("fiche non réécrite");
        };
        assert_eq!(
            (d.display_name_field(), d.given_name()),
            (None, Some("Alice"))
        );
        let keys: Vec<&str> = d.emails().iter().map(|e| e.key().as_str()).collect();
        assert_eq!(keys, ["alice@example.com"]);
        // Plus aucune adresse utilisable : retirée, comme au nettoyage des
        // invisibles.
        let mut i = input(&["\u{2800}@example.com"]);
        i.nickname = "<x>".into();
        assert_eq!(clean_names_imitating_address(i), InvisibleCleanup::Remove);
    }

    #[test]
    fn keys_as_stored_are_computed_with_the_name_imitating_an_address() {
        let mut i = input(&["alice@example.com"]);
        i.display_name = "CEO@Bank.example".into();
        i.organization = "ACME".into();
        assert_eq!(
            ContactDraft::keys_as_stored(&i),
            Some((
                "ceo@bank.example".to_owned(),
                "ceo@bank.example\nacme\nalice@example.com".to_owned()
            ))
        );
        // Relue, la fiche n'a plus ce nom : d'autres clés.
        let read = ContactDraft::from_stored(i).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(read.sort_key(), "alice@example.com");
        // Sans nom imitant une adresse : rien de tel.
        let mut i = input(&["alice@example.com"]);
        i.display_name = "Alice".into();
        assert_eq!(ContactDraft::keys_as_stored(&i), None);
        // Invalide pour une autre raison : rien non plus.
        let mut i = input(&["alice@example.com"]);
        i.display_name = "x@y".into();
        i.notes = "a\0b".into();
        assert_eq!(ContactDraft::keys_as_stored(&i), None);
    }

    #[test]
    fn display_label_rule() {
        let a = addr("alice@example.com");
        assert_eq!(
            display_label(Some("Ali"), Some("Alice"), Some("Martin"), Some("Lili"), &a),
            "Ali"
        );
        assert_eq!(
            display_label(None, Some("Alice"), Some("Martin"), Some("Lili"), &a),
            "Alice Martin"
        );
        assert_eq!(
            display_label(None, None, Some("Martin"), None, &a),
            "Martin"
        );
        assert_eq!(display_label(None, Some("Alice"), None, None, &a), "Alice");
        assert_eq!(display_label(None, None, None, Some("Lili"), &a), "Lili");
        assert_eq!(
            display_label(None, None, None, None, &a),
            "alice@example.com"
        );
    }

    #[test]
    fn display_name_uses_the_preferred_address_last() {
        let mut i = input(&["a@example.com", "b@example.com"]);
        i.emails[1].preferred = true;
        assert_eq!(draft(i.clone()).display_name(), "b@example.com");
        i.given_name = "Bob".into();
        assert_eq!(draft(i).display_name(), "Bob");
    }

    #[test]
    fn sort_key_is_lowercase_display_name() {
        let mut i = input(&["a@example.com"]);
        i.display_name = "Élodie DURAND".into();
        assert_eq!(draft(i).sort_key(), "élodie durand");
    }

    #[test]
    fn search_text_covers_names_organization_and_addresses() {
        let d = draft(ContactInput {
            display_name: "Ali".into(),
            given_name: "Alice".into(),
            family_name: "Martin".into(),
            nickname: "Lili".into(),
            organization: "ACME".into(),
            notes: "secret".into(),
            emails: vec![email("Alice@Example.com"), email("perso@maison.fr")],
            ..ContactInput::default()
        });
        let text = d.search_text();
        for part in [
            "ali",
            "alice",
            "martin",
            "lili",
            "acme",
            "alice@example.com",
            "perso@maison.fr",
        ] {
            assert!(
                text.split('\n').any(|l| l == part),
                "{part} absent de {text:?}"
            );
        }
        assert!(!text.contains("secret"), "les notes ne sont pas cherchées");
    }

    #[test]
    fn search_text_covers_the_composed_full_name() {
        // Prénom et nom dans deux champs : « alice mar » trouve Alice
        // Martin, même avec un nom affiché saisi.
        let mut i = input(&["a@example.com"]);
        i.given_name = "Alice".into();
        i.family_name = "MARTIN".into();
        let filter = ContactFilter::new("alice mar", false);
        assert!(filter.matches(&draft(i.clone())));
        i.display_name = "Ali".into();
        assert!(filter.matches(&draft(i.clone())));
        // Un seul des deux champs : pas de nom composé.
        i.family_name = String::new();
        assert!(!draft(i).search_text().contains("alice "));
    }

    /// Texte cherché recalculé pour le nom complet : seulement depuis les
    /// clés stockées d'avant (ou déjà à jour), pour la fiche relue ou
    /// telle qu'enregistrée avec un nom imitant une adresse ; une clé
    /// retouchée n'est ni réparée ni masquée.
    #[test]
    fn full_name_reindex_only_rewrites_the_previous_keys() {
        let reindex = |i: &ContactInput, stored: &(String, String)| {
            ContactDraft::from_stored(i.clone())
                .ok()
                .and_then(|read| read.full_name_reindex(i, stored))
        };
        let mut i = input(&["alice@example.com"]);
        i.given_name = "Alice".into();
        i.family_name = "Martin".into();
        let new = "alice\nmartin\nalice martin\nalice@example.com".to_owned();
        let before = (
            "alice martin".to_owned(),
            "alice\nmartin\nalice@example.com".to_owned(),
        );
        assert_eq!(reindex(&i, &before), Some(new.clone()));
        // Déjà à jour : même texte.
        let current = ("alice martin".to_owned(), new.clone());
        assert_eq!(reindex(&i, &current), Some(new));
        // Texte ou clé de tri retouchés : rien.
        let tampered = ("alice martin".to_owned(), "zzz".to_owned());
        assert_eq!(reindex(&i, &tampered), None);
        let tampered = ("repère".to_owned(), before.1.clone());
        assert_eq!(reindex(&i, &tampered), None);
        // Nom imitant une adresse : clés telles qu'enregistrées, avec lui.
        i.nickname = "ceo@bank.example".into();
        let stored = (
            "alice martin".to_owned(),
            "alice\nmartin\nceo@bank.example\nalice@example.com".to_owned(),
        );
        assert_eq!(
            reindex(&i, &stored),
            Some("alice\nmartin\nalice martin\nceo@bank.example\nalice@example.com".to_owned())
        );
        // Fiche invalide : rien.
        i.emails.clear();
        assert_eq!(reindex(&i, &stored), None);
    }

    #[test]
    fn has_address_ignores_case() {
        let d = draft(input(&["Alice@example.com"]));
        assert!(d.has_address(&AddressKey::of(&addr("ALICE@example.com"))));
        assert!(!d.has_address(&AddressKey::of(&addr("bob@example.com"))));
    }

    #[test]
    fn to_input_round_trips() {
        let mut i = ContactInput {
            display_name: "Alice Martin".into(),
            given_name: "Alice".into(),
            family_name: "Martin".into(),
            nickname: "Lili".into(),
            organization: "ACME".into(),
            notes: "a\nb".into(),
            emails: vec![
                EmailInput {
                    address: "alice@example.com".into(),
                    label: "travail".into(),
                    preferred: false,
                },
                email("perso@example.org"),
            ],
            phones: vec![PhoneInput {
                number: "01 02".into(),
                label: "maison".into(),
            }],
            title: "Directrice".into(),
            birthday: "--02-29".into(),
            postal_addresses: vec![PostalAddressInput {
                street: "1 rue des Lilas\nBât. B".into(),
                locality: "Lyon".into(),
                postal_code: "69001".into(),
                label: "maison".into(),
                ..PostalAddressInput::default()
            }],
            websites: vec![WebsiteInput {
                url: "https://example.com/alice".into(),
                label: "blog".into(),
            }],
            favorite: true,
        };
        i.emails[1].preferred = true;
        let d = draft(i.clone());
        assert_eq!(d.to_input(), i);
        assert_eq!(draft(d.to_input()), d);
    }

    #[test]
    fn with_id_keeps_the_draft() {
        let d = draft(input(&["a@example.com"]));
        let Some(id) = ContactId::new(4) else {
            panic!("id");
        };
        let c = d.clone().with_id(id);
        assert_eq!(c.id(), id);
        assert_eq!(c.draft(), &d);
    }

    #[test]
    fn filter_normalizes_text() {
        let f = ContactFilter::new("  ÉLodie ", false);
        assert_eq!(f.text(), "élodie");
        assert!(!f.favorites_only());
        let long = "a".repeat(MAX_CONTACT_FILTER_CHARS + 10);
        assert_eq!(
            ContactFilter::new(&long, true).text().chars().count(),
            MAX_CONTACT_FILTER_CHARS
        );
    }

    #[test]
    fn filter_matches_text_and_favorites() {
        let mut i = ContactInput {
            given_name: "Élodie".into(),
            organization: "ACME".into(),
            emails: vec![email("e.durand@example.com")],
            ..ContactInput::default()
        };
        let plain = draft(i.clone());
        i.favorite = true;
        let fav = draft(i);
        assert!(ContactFilter::default().matches(&plain));
        assert!(ContactFilter::new("élo", false).matches(&plain));
        assert!(ContactFilter::new("acm", false).matches(&plain));
        assert!(ContactFilter::new("DURAND@", false).matches(&plain));
        assert!(!ContactFilter::new("bob", false).matches(&plain));
        assert!(!ContactFilter::new("", true).matches(&plain));
        assert!(ContactFilter::new("", true).matches(&fav));
        assert!(ContactFilter::new("élo", true).matches(&fav));
        // Un saut de ligne ne fait pas déborder d'un champ à l'autre.
        assert!(!ContactFilter::new("élodie\nacme", false).matches(&plain));
    }

    #[test]
    fn selection_keeps_valid_ids_once_in_order() {
        let ids = |raw: &[i64]| -> Vec<i64> {
            contact_selection(raw)
                .unwrap_or_else(|e| panic!("{e}"))
                .into_iter()
                .map(ContactId::get)
                .collect()
        };
        assert_eq!(ids(&[3, 1, 3, 2, 1]), vec![3, 1, 2]);
        assert!(ids(&[]).is_empty());
    }

    #[test]
    fn selection_refuses_invalid_ids_and_too_many() {
        assert_eq!(
            contact_selection(&[1, 0]),
            Err(ContactSelectionError::InvalidId)
        );
        assert_eq!(
            contact_selection(&[-4]),
            Err(ContactSelectionError::InvalidId)
        );
        let max = i64::try_from(MAX_CONTACT_SELECTION).unwrap_or(i64::MAX);
        let all: Vec<i64> = (1..=max).collect();
        assert_eq!(
            contact_selection(&all).map(|v| v.len()),
            Ok(MAX_CONTACT_SELECTION)
        );
        let more: Vec<i64> = (1..=max + 1).collect();
        assert_eq!(
            contact_selection(&more),
            Err(ContactSelectionError::TooMany)
        );
        // Les doublons ne comptent qu'une fois.
        let doubled: Vec<i64> = all.iter().chain(all.iter()).copied().collect();
        assert_eq!(
            contact_selection(&doubled).map(|v| v.len()),
            Ok(MAX_CONTACT_SELECTION)
        );
        // Liste reçue démesurée : refusée sans être parcourue en entier.
        let flood = vec![1_i64; 2 * MAX_CONTACT_SELECTION + 1];
        assert_eq!(
            contact_selection(&flood),
            Err(ContactSelectionError::TooMany)
        );
    }

    #[test]
    fn cursor_from_the_interface_is_revalidated() {
        use crate::ports::{ContactCursor, MAX_CONTACT_SORT_KEY_CHARS};
        let c = ContactCursor::parse("élodie".into(), 7).unwrap_or_else(|| panic!("curseur"));
        assert_eq!(c.sort_key, "élodie");
        assert_eq!(c.id.get(), 7);
        assert!(ContactCursor::parse("a".into(), 0).is_none());
        let max = "é".repeat(MAX_CONTACT_SORT_KEY_CHARS);
        assert!(ContactCursor::parse(max.clone(), 1).is_some());
        assert!(ContactCursor::parse(max + "e", 1).is_none());
        // Toute clé produite par une fiche valide est acceptée.
        let long = "İ".repeat(MAX_CONTACT_NAME_CHARS);
        let d = draft(ContactInput {
            given_name: long.clone(),
            family_name: long,
            emails: vec![email("a@example.com")],
            ..ContactInput::default()
        });
        assert!(ContactCursor::parse(d.sort_key(), 1).is_some());
    }

    fn postal(street: &str, locality: &str) -> PostalAddressInput {
        PostalAddressInput {
            street: street.into(),
            locality: locality.into(),
            ..PostalAddressInput::default()
        }
    }

    fn website(url: &str) -> WebsiteInput {
        WebsiteInput {
            url: url.into(),
            label: String::new(),
        }
    }

    #[test]
    fn details_are_trimmed_validated_and_kept() {
        let mut i = input(&["a@example.com"]);
        i.title = "  Directrice commerciale ".into();
        i.birthday = " 19850412 ".into();
        i.postal_addresses = vec![PostalAddressInput {
            street: "  1 rue des Lilas \r\n\n  Bât. B  ".into(),
            locality: " Lyon ".into(),
            region: "  ".into(),
            postal_code: "69001".into(),
            country: "France".into(),
            label: " maison ".into(),
        }];
        i.websites = vec![WebsiteInput {
            url: " www.example.com ".into(),
            label: " blog ".into(),
        }];
        let d = draft(i);
        assert_eq!(d.title(), Some("Directrice commerciale"));
        assert_eq!(d.birthday().map(|b| b.iso()), Some("1985-04-12".into()));
        let [a] = d.postal_addresses() else {
            panic!("{:?}", d.postal_addresses())
        };
        assert_eq!(a.street(), Some("1 rue des Lilas\nBât. B"));
        assert_eq!(a.locality(), Some("Lyon"));
        assert_eq!(a.region(), None);
        assert_eq!(a.postal_code(), Some("69001"));
        assert_eq!(a.country(), Some("France"));
        assert_eq!(a.label(), Some("maison"));
        let [w] = d.websites() else {
            panic!("{:?}", d.websites())
        };
        assert_eq!(w.url(), "https://www.example.com");
        assert_eq!(w.label(), Some("blog"));
        // Champs absents.
        let bare = draft(input(&["a@example.com"]));
        assert_eq!(bare.title(), None);
        assert_eq!(bare.birthday(), None);
        assert!(bare.postal_addresses().is_empty());
        assert!(bare.websites().is_empty());
    }

    #[test]
    fn invalid_details_are_refused_and_located() {
        let mut i = input(&["a@example.com"]);
        i.birthday = "1985-02-30".into();
        assert_eq!(ContactDraft::new(i), Err(ContactError::InvalidBirthday));

        let mut i = input(&["a@example.com"]);
        i.websites = vec![
            website("https://example.com"),
            website("javascript:alert(1)"),
        ];
        assert_eq!(
            ContactDraft::new(i),
            Err(ContactError::InvalidWebsite { index: 1 })
        );

        let mut i = input(&["a@example.com"]);
        i.postal_addresses = vec![
            postal("", "Lyon"),
            PostalAddressInput {
                label: "maison".into(),
                ..PostalAddressInput::default()
            },
        ];
        assert_eq!(
            ContactDraft::new(i),
            Err(ContactError::EmptyPostalAddress { index: 1 })
        );

        let mut i = input(&["a@example.com"]);
        i.postal_addresses = vec![postal("1 rue\tdes Lilas", "")];
        assert_eq!(
            ContactDraft::new(i),
            Err(ContactError::InvalidCharacter(ContactField::PostalAddress(
                0
            )))
        );
        let mut i = input(&["a@example.com"]);
        i.postal_addresses = vec![postal("", "Ly\non")];
        assert_eq!(
            ContactDraft::new(i),
            Err(ContactError::InvalidCharacter(ContactField::PostalAddress(
                0
            )))
        );
        let mut i = input(&["a@example.com"]);
        i.postal_addresses = vec![postal("1 rue\u{202E}", "")];
        assert_eq!(
            ContactDraft::new(i),
            Err(ContactError::InvalidCharacter(ContactField::PostalAddress(
                0
            )))
        );
        let mut i = input(&["a@example.com"]);
        i.title = "Dir\u{200B}ectrice".into();
        assert_eq!(
            ContactDraft::new(i),
            Err(ContactError::InvalidCharacter(ContactField::Title))
        );
    }

    #[test]
    fn details_are_bounded() {
        let mut i = input(&["a@example.com"]);
        i.postal_addresses = vec![postal("rue", ""); MAX_POSTAL_ADDRESSES + 1];
        assert_eq!(
            ContactDraft::new(i.clone()),
            Err(ContactError::TooManyPostalAddresses)
        );
        i.postal_addresses.pop();
        assert!(ContactDraft::new(i).is_ok());

        let mut i = input(&["a@example.com"]);
        i.websites = vec![website("https://example.com"); MAX_CONTACT_WEBSITES + 1];
        assert_eq!(
            ContactDraft::new(i.clone()),
            Err(ContactError::TooManyWebsites)
        );
        i.websites.pop();
        assert!(ContactDraft::new(i).is_ok());

        let long = |n: usize| "é".repeat(n);
        type Setter = Box<dyn Fn(&mut ContactInput, String)>;
        let cases: Vec<(ContactField, Setter, usize)> = vec![
            (
                ContactField::Title,
                Box::new(|i, s| i.title = s),
                MAX_CONTACT_NAME_CHARS,
            ),
            (
                ContactField::PostalAddress(0),
                Box::new(|i, s| i.postal_addresses[0].street = s),
                MAX_POSTAL_STREET_CHARS,
            ),
            (
                ContactField::PostalAddress(0),
                Box::new(|i, s| i.postal_addresses[0].country = s),
                MAX_POSTAL_FIELD_CHARS,
            ),
            (
                ContactField::PostalAddressLabel(0),
                Box::new(|i, s| i.postal_addresses[0].label = s),
                MAX_CONTACT_LABEL_CHARS,
            ),
            (
                ContactField::WebsiteLabel(0),
                Box::new(|i, s| i.websites[0].label = s),
                MAX_CONTACT_LABEL_CHARS,
            ),
        ];
        for (field, set, max) in cases {
            let mut i = input(&["a@example.com"]);
            i.postal_addresses = vec![postal("rue", "")];
            i.websites = vec![website("https://example.com")];
            set(&mut i, long(max));
            assert!(
                ContactDraft::new(i.clone()).is_ok(),
                "{field:?} à la limite"
            );
            set(&mut i, long(max + 1));
            assert_eq!(
                ContactDraft::new(i),
                Err(ContactError::TooLong(field)),
                "{field:?}"
            );
        }
    }

    #[test]
    fn search_text_includes_the_title() {
        let mut i = input(&["a@example.com"]);
        i.given_name = "Alice".into();
        let before = draft(i.clone()).search_text();
        i.title = "Directrice".into();
        let d = draft(i);
        assert!(
            d.search_text().contains("directrice"),
            "{}",
            d.search_text()
        );
        assert!(ContactFilter::new("DIRECT", false).matches(&d));
        // Sans titre, le texte cherché est celui d'avant (fiches enregistrées
        // inchangées).
        assert_eq!(before, "alice\na@example.com");
    }

    #[test]
    fn same_place_ignores_case_and_label() {
        let mut a = input(&["a@example.com"]);
        a.postal_addresses = vec![PostalAddressInput {
            label: "maison".into(),
            ..postal("1 rue des Lilas", "Lyon")
        }];
        let mut b = input(&["b@example.com"]);
        b.postal_addresses = vec![postal("1 RUE DES LILAS", "lyon")];
        let (a, b) = (draft(a), draft(b));
        assert!(a.postal_addresses()[0].same_place(&b.postal_addresses()[0]));
        let mut c = input(&["c@example.com"]);
        c.postal_addresses = vec![postal("2 rue des Lilas", "Lyon")];
        assert!(!a.postal_addresses()[0].same_place(&draft(c).postal_addresses()[0]));
    }
}
