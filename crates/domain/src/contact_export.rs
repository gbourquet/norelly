//! Export du carnet d'adresses (ADR-012) : formats proposés, contacts
//! visés, et règles de sortie ([`export_card`]) qui disent ce que chaque
//! format retient d'une fiche.
//!
//! L'adaptateur `contacts-io` met en forme les fiches préparées ici
//! ([`ExportCard`]) sans rien décider : l'ordre des adresses, les
//! étiquettes écrites, le nom complet d'une vCard et la neutralisation des
//! cellules CSV ([`neutralize_csv_cell`]) sont fixés par le domaine. Les
//! bornes de l'export sont celles de l'import : tout fichier exporté se
//! réimporte.

use std::borrow::Cow;

use crate::contact_import::{MAX_IMPORT_CARDS, MAX_IMPORT_FILE_BYTES};
use crate::contacts::{
    ContactDraft, ContactEmail, ContactId, ContactSelectionError, MAX_CONTACT_NAME_CHARS,
    MAX_CONTACT_SELECTION, contact_selection,
};

/// Nombre maximal de contacts d'un export (celui d'un import).
pub const MAX_EXPORT_CONTACTS: usize = MAX_IMPORT_CARDS;
/// Taille maximale d'un fichier exporté, en octets (celle d'un import).
pub const MAX_EXPORT_FILE_BYTES: usize = MAX_IMPORT_FILE_BYTES;

/// Format d'un export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// vCard 4.0 (RFC 6350).
    VCard4,
    /// vCard 3.0 (RFC 2426), pour les clients anciens.
    VCard3,
    /// CSV aux colonnes de Google Contacts.
    GoogleCsv,
    /// LDIF tel que l'exporte Thunderbird.
    Ldif,
}

impl ExportFormat {
    /// Extension du fichier proposé.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::VCard4 | Self::VCard3 => "vcf",
            Self::GoogleCsv => "csv",
            Self::Ldif => "ldif",
        }
    }
}

/// Contacts à exporter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactSelection {
    /// Tout le carnet.
    All,
    /// Ces contacts (au moins un, au plus [`MAX_CONTACT_SELECTION`],
    /// chacun une fois).
    Contacts(Vec<ContactId>),
}

/// Sélection refusée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SelectionError {
    #[error("aucun contact choisi")]
    Empty,
    #[error("plus de {MAX_CONTACT_SELECTION} contacts choisis")]
    TooMany,
    #[error("identifiant de contact invalide")]
    InvalidId,
}

impl ContactSelection {
    /// Sélection reçue de l'interface : celle d'une action sur plusieurs
    /// fiches ([`contact_selection`]), non vide.
    pub fn of<T: Copy + Into<i64>>(ids: &[T]) -> Result<Self, SelectionError> {
        let ids = contact_selection(ids).map_err(|e| match e {
            ContactSelectionError::InvalidId => SelectionError::InvalidId,
            ContactSelectionError::TooMany => SelectionError::TooMany,
        })?;
        if ids.is_empty() {
            return Err(SelectionError::Empty);
        }
        Ok(Self::Contacts(ids))
    }
}

/// Adresse écrite dans un export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportEmail {
    pub address: String,
    pub label: Option<String>,
    pub preferred: bool,
}

/// Téléphone écrit dans un export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPhone {
    pub number: String,
    pub label: Option<String>,
}

/// Fiche prête à écrire dans un format : ce que le format retient de la
/// fiche, adresse préférée en tête.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportCard {
    /// Nom complet (`FN` vCard, `cn` LDIF, colonne `Name` CSV).
    pub full_name: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub nickname: Option<String>,
    pub organization: Option<String>,
    pub notes: Option<String>,
    pub emails: Vec<ExportEmail>,
    pub phones: Vec<ExportPhone>,
}

/// Ce que `format` retient de `draft`.
#[must_use]
pub fn export_card(draft: &ContactDraft, format: ExportFormat) -> ExportCard {
    let owned = |v: Option<&str>| v.map(str::to_owned);
    let vcard = matches!(format, ExportFormat::VCard4 | ExportFormat::VCard3);
    let email_label = |label: Option<&str>| match format {
        ExportFormat::VCard4 | ExportFormat::VCard3 => label.and_then(vcard_type),
        ExportFormat::GoogleCsv => label.and_then(google_label),
        ExportFormat::Ldif => None,
    };
    let phone_label = |label: Option<&str>| match format {
        ExportFormat::VCard4 | ExportFormat::VCard3 => label.and_then(vcard_type),
        ExportFormat::GoogleCsv => label.and_then(google_label),
        ExportFormat::Ldif => Some(ldif_phone_label(label).to_owned()),
    };
    let preferred = draft.preferred_email();
    let others = draft.emails().iter().filter(|e| !e.preferred());
    ExportCard {
        full_name: if vcard {
            vcard_full_name(draft)
        } else {
            owned(draft.display_name_field())
        },
        given_name: owned(draft.given_name()),
        family_name: owned(draft.family_name()),
        nickname: owned(draft.nickname()),
        organization: owned(draft.organization()),
        notes: owned(draft.notes()),
        emails: std::iter::once(preferred)
            .chain(others)
            .map(|e: &ContactEmail| ExportEmail {
                address: e.address().to_string(),
                label: email_label(e.label()),
                preferred: e.preferred(),
            })
            .collect(),
        phones: draft
            .phones()
            .iter()
            .map(|p| ExportPhone {
                number: p.number().to_owned(),
                label: phone_label(p.label()),
            })
            .collect(),
    }
}

/// Nom complet d'une vCard (propriété obligatoire) : le nom affiché de la
/// fiche, sauf s'il se replie sur l'adresse (l'import retire un nom qui
/// imite une adresse) ou si le nom composé dépasse la longueur d'un nom
/// (l'import refuserait la fiche) : vide alors.
fn vcard_full_name(draft: &ContactDraft) -> Option<String> {
    let named = draft.display_name_field().is_some()
        || draft.given_name().is_some()
        || draft.family_name().is_some()
        || draft.nickname().is_some();
    let name = draft.display_name();
    (named && name.chars().count() <= MAX_CONTACT_NAME_CHARS).then_some(name)
}

/// Types vCard qu'une étiquette ne doit pas devenir : ils changent la
/// lecture de la carte (préférence, encodage) ou sont des types
/// techniques que l'import écarte.
const RESERVED_VCARD_TYPES: &[&str] = &[
    "pref", "internet", "x400", "voice", "text", "base64", "b", "8bit", "7bit",
];

/// Étiquette écrite en type vCard : en minuscules, les caractères de la
/// syntaxe vCard (`"`, `,`, `;`, `:`, `\`, `=`) remplacés par une espace,
/// espaces resserrées ; aucune si elle est vide ou réservée.
fn vcard_type(label: &str) -> Option<String> {
    let cleaned: String = label
        .to_lowercase()
        .chars()
        .map(|c| match c {
            '"' | ',' | ';' | ':' | '\\' | '=' => ' ',
            c => c,
        })
        .collect();
    let value = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let reserved = RESERVED_VCARD_TYPES.contains(&value.as_str())
        || value.starts_with("x-")
        || value.contains("quoted-printable");
    (!value.is_empty() && !reserved).then_some(value)
}

/// Étiquette Google : sans « * » en tête (marque de l'adresse principale).
fn google_label(label: &str) -> Option<String> {
    let value = label.trim_start_matches('*').trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Attribut Thunderbird d'un téléphone, par son étiquette : `work`
/// (`telephoneNumber`, aussi pour une étiquette inconnue ou absente),
/// `home`, `cell`, `fax` ou `pager`.
fn ldif_phone_label(label: Option<&str>) -> &'static str {
    let label = label.map(str::to_lowercase).unwrap_or_default();
    match label.as_str() {
        "home" | "maison" | "domicile" | "perso" | "personnel" => "home",
        "cell" | "mobile" | "portable" => "cell",
        "fax" | "télécopie" | "telecopie" => "fax",
        "pager" | "bipeur" | "radiomessagerie" => "pager",
        _ => "work",
    }
}

/// Premiers caractères qu'un tableur lit comme le début d'une formule.
const FORMULA_STARTS: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

/// La valeur, apostrophes de tête passées, commence-t-elle comme une
/// formule ?
fn formula_like(cell: &str) -> bool {
    cell.trim_start_matches('\'').starts_with(FORMULA_STARTS)
}

/// Neutralise une cellule CSV qu'un tableur lirait comme une formule :
/// une apostrophe est ajoutée devant une valeur qui commence, après
/// d'éventuelles apostrophes, par `=`, `+`, `-`, `@`, une tabulation ou un
/// retour chariot. [`restore_csv_cell`] la retire.
#[must_use]
pub fn neutralize_csv_cell(cell: &str) -> Cow<'_, str> {
    if formula_like(cell) {
        Cow::Owned(format!("'{cell}"))
    } else {
        Cow::Borrowed(cell)
    }
}

/// Valeur d'une cellule CSV neutralisée par [`neutralize_csv_cell`] (ou
/// par un tableur) : l'apostrophe ajoutée est retirée.
#[must_use]
pub fn restore_csv_cell(cell: &str) -> &str {
    match cell.strip_prefix('\'') {
        Some(rest) if formula_like(rest) => rest,
        _ => cell,
    }
}

#[cfg(test)]
mod tests;
