//! Carnet d'adresses (ADR-012) : fiche contact validée, nom affiché, clé
//! d'adresse, filtre et ordre de la liste.
//!
//! Le modèle suit vCard (nom affiché, prénom, nom, surnom, plusieurs
//! adresses dont une préférée, téléphones, organisation, notes) pour qu'une
//! synchronisation CardDAV puisse s'ajouter plus tard. Une fiche saisie ou
//! importée ([`ContactInput`]) devient une fiche valide ([`ContactDraft`])
//! par [`ContactDraft::new`] ; le carnet lui attribue un identifiant
//! ([`Contact`]).

use std::fmt;

use crate::address::{EmailAddress, EmailAddressError};
use crate::connection::is_format_char;
use crate::mailbox::{MAX_ADDRESS_CHARS, header_text};

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

/// Identifiant local d'un contact, attribué par le carnet (strictement
/// positif).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContactId(i64);

impl ContactId {
    #[must_use]
    pub fn new(raw: i64) -> Option<Self> {
        (raw > 0).then_some(Self(raw))
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
    Notes,
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

/// Fiche saisie ou importée, non validée. Un champ vide (ou fait
/// d'espaces) est absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactInput {
    pub display_name: String,
    pub given_name: String,
    pub family_name: String,
    pub nickname: String,
    pub organization: String,
    pub notes: String,
    pub emails: Vec<EmailInput>,
    pub phones: Vec<PhoneInput>,
    pub favorite: bool,
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

/// Fiche valide, pas encore enregistrée (ou contenu d'une fiche
/// enregistrée).
///
/// Invariants : au moins une adresse, au plus [`MAX_CONTACT_EMAILS`],
/// distinctes à la casse près, exactement une préférée ; textes sans
/// espaces de bord, bornés, sans caractère de contrôle ni de mise en forme
/// invisible (les notes admettent sauts de ligne et tabulations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactDraft {
    display_name: Option<String>,
    given_name: Option<String>,
    family_name: Option<String>,
    nickname: Option<String>,
    organization: Option<String>,
    notes: Option<String>,
    emails: Vec<ContactEmail>,
    phones: Vec<ContactPhone>,
    favorite: bool,
}

impl ContactDraft {
    /// Valide une fiche. Sans adresse marquée préférée, la première l'est.
    pub fn new(input: ContactInput) -> Result<Self, ContactError> {
        if input.emails.is_empty() {
            return Err(ContactError::NoAddress);
        }
        if input.emails.len() > MAX_CONTACT_EMAILS {
            return Err(ContactError::TooManyAddresses);
        }
        if input.phones.len() > MAX_CONTACT_PHONES {
            return Err(ContactError::TooManyPhones);
        }
        let name = |raw: &str, field| text(raw, field, MAX_CONTACT_NAME_CHARS);
        let display_name = name(&input.display_name, ContactField::DisplayName)?;
        let given_name = name(&input.given_name, ContactField::GivenName)?;
        let family_name = name(&input.family_name, ContactField::FamilyName)?;
        let nickname = name(&input.nickname, ContactField::Nickname)?;
        let organization = name(&input.organization, ContactField::Organization)?;
        let notes = notes(&input.notes)?;

        if input.emails.iter().filter(|e| e.preferred).count() > 1 {
            return Err(ContactError::SeveralPreferred);
        }
        let any_preferred = input.emails.iter().any(|e| e.preferred);
        let mut emails: Vec<ContactEmail> = Vec::with_capacity(input.emails.len());
        for (index, e) in input.emails.iter().enumerate() {
            let address = EmailAddress::parse(&e.address)
                .map_err(|error| ContactError::InvalidAddress { index, error })?;
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

        Ok(Self {
            display_name,
            given_name,
            family_name,
            nickname,
            organization,
            notes,
            emails,
            phones,
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

    #[must_use]
    pub fn notes(&self) -> Option<&str> {
        self.notes.as_deref()
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

    /// Texte dans lequel cherche un filtre : noms, organisation et
    /// adresses, en minuscules, séparés par des sauts de ligne.
    #[must_use]
    pub fn search_text(&self) -> String {
        let names = [
            &self.display_name,
            &self.given_name,
            &self.family_name,
            &self.nickname,
            &self.organization,
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
            notes: owned(&self.notes),
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
            favorite: self.favorite,
        }
    }

    /// Fiche enregistrée sous `id`.
    #[must_use]
    pub fn with_id(self, id: ContactId) -> Contact {
        Contact { id, draft: self }
    }
}

/// Contact enregistré dans le carnet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    id: ContactId,
    draft: ContactDraft,
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
        (Some(given), Some(family)) => return format!("{given} {family}"),
        (Some(one), None) | (None, Some(one)) => return one.to_owned(),
        (None, None) => {}
    }
    nickname.map_or_else(|| address.to_string(), str::to_owned)
}

/// Caractère interdit dans un champ d'une ligne.
fn forbidden(c: char) -> bool {
    c.is_control() || is_format_char(c)
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
/// [`MAX_CONTACT_SELECTION`] fiches distinctes.
pub fn contact_selection(raw: &[i64]) -> Result<Vec<ContactId>, ContactSelectionError> {
    let mut ids: Vec<ContactId> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for &value in raw {
        let id = ContactId::new(value).ok_or(ContactSelectionError::InvalidId)?;
        if seen.insert(id) {
            if ids.len() == MAX_CONTACT_SELECTION {
                return Err(ContactSelectionError::TooMany);
            }
            ids.push(id);
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
