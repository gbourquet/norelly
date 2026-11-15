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
use std::time::Duration;

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
/// syntaxe vCard (`"`, `,`, `;`, `:`, `\`, `=`, et `^` qui échappe les
/// paramètres en 4.0, RFC 6868) remplacés par une espace,
/// espaces resserrées ; aucune si elle est vide ou réservée.
fn vcard_type(label: &str) -> Option<String> {
    let cleaned: String = label
        .to_lowercase()
        .chars()
        .map(|c| match c {
            '"' | ',' | ';' | ':' | '\\' | '=' | '^' => ' ',
            c => c,
        })
        .collect();
    let value = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let reserved = RESERVED_VCARD_TYPES.contains(&value.as_str())
        || value.starts_with("x-")
        || value.contains("quoted-printable");
    (!value.is_empty() && !reserved).then_some(value)
}

/// Étiquette Google : sans « * » en tête (marque de l'adresse principale),
/// même répétée ou entrecoupée d'espaces.
fn google_label(label: &str) -> Option<String> {
    let value = label.trim_start_matches(|c: char| c == '*' || c.is_whitespace());
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

/// Premiers caractères qu'un tableur lit comme le début d'une formule,
/// avec leurs formes pleine chasse, que certains assimilent. Les autres
/// caractères que la normalisation de compatibilité (NFKC) ramène à `=`,
/// `+`, `-`, `@` ou au signe moins `−` (petites formes, exposants,
/// indices, plus hébreu), et `−` lui-même, qu'aucun tableur courant
/// n'évalue, sont traités de même à titre préventif.
const FORMULA_STARTS: [char; 22] = [
    '=', '+', '-', '@', '\t', '\r', '\u{FF1D}', '\u{FF0B}', '\u{FF0D}', '\u{FF20}', '\u{FE62}',
    '\u{FE63}', '\u{FE66}', '\u{FE6B}', '\u{2212}', '\u{207A}', '\u{208A}', '\u{FB29}', '\u{207C}',
    '\u{208C}', '\u{207B}', '\u{208B}',
];

/// La valeur, apostrophes de tête passées, commence-t-elle comme une
/// formule ?
fn formula_like(cell: &str) -> bool {
    cell.trim_start_matches('\'').starts_with(FORMULA_STARTS)
}

/// Neutralise une cellule CSV qu'un tableur lirait comme une formule :
/// une apostrophe est ajoutée devant une valeur qui commence, après
/// d'éventuelles apostrophes, par `=`, `+`, `-`, `@` (ou leur forme pleine
/// chasse, petite, en exposant ou en indice), le signe moins `−`, une
/// tabulation ou un retour chariot. [`restore_csv_cell`] la retire.
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

/// Âge à partir duquel un temporaire d'export libre est jugé abandonné :
/// plus récent, il peut appartenir à un export qui vient de le créer et
/// n'a pas encore posé son verrou, ou qui vient de le relâcher avant de le
/// renommer.
pub const STALE_EXPORT_TEMPORARY_AGE: Duration = Duration::from_secs(5 * 60);

/// Nombre maximal d'entrées du dossier cible examinées pour y chercher des
/// temporaires abandonnés : au-delà, le nettoyage s'arrête (il est
/// facultatif) et l'export continue.
pub const MAX_EXPORT_FOLDER_SCAN: usize = 50_000;

/// État du verrou consultatif d'un temporaire d'export, que tout export
/// tient pendant qu'il écrit le sien.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporaryLock {
    /// Personne ne le tient.
    Free,
    /// Un export en cours le tient.
    Held,
    /// Impossible à vérifier (système de fichiers sans verrou, erreur).
    Unknown,
}

/// `candidate` est-il le nom d'un fichier temporaire laissé par l'écriture
/// d'un export dans `target` (même dossier) ? Seule la forme
/// `.<target>.<pid>-<n>.tmp` est reconnue, `pid` et `n` étant des `u32`
/// écrits en 1 à 10 chiffres ASCII. Le nom de la cible est comparé sans
/// tenir compte de la casse (systèmes de fichiers insensibles à la casse) :
/// en minuscules Unicode si les deux noms sont de l'UTF-8, sinon octet par
/// octet aux lettres ASCII près.
#[must_use]
pub fn is_export_temporary_of(target: &[u8], candidate: &[u8]) -> bool {
    fn number(digits: &[u8]) -> bool {
        (1..=10).contains(&digits.len())
            && digits.iter().all(u8::is_ascii_digit)
            && std::str::from_utf8(digits).is_ok_and(|d| d.parse::<u32>().is_ok())
    }
    fn same_name(a: &[u8], b: &[u8]) -> bool {
        match (std::str::from_utf8(a), std::str::from_utf8(b)) {
            (Ok(a), Ok(b)) => a
                .chars()
                .flat_map(char::to_lowercase)
                .eq(b.chars().flat_map(char::to_lowercase)),
            _ => a.eq_ignore_ascii_case(b),
        }
    }
    // Lu depuis la fin : `.tmp`, puis `<pid>-<n>` après le dernier point ;
    // ce qui précède ce point est le nom de la cible.
    let Some(rest) = candidate
        .strip_prefix(b".")
        .and_then(|rest| rest.strip_suffix(b".tmp"))
    else {
        return false;
    };
    let Some(dot) = rest.iter().rposition(|&b| b == b'.') else {
        return false;
    };
    let (name, numbers) = rest.split_at(dot);
    let mut parts = numbers[1..].split(|&b| b == b'-');
    let (Some(pid), Some(n), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !target.is_empty() && number(pid) && number(n) && same_name(name, target)
}

/// Un temporaire d'export dont le verrou est dans l'état `lock`, modifié
/// il y a `age` (inconnu, ou dans le futur : `None`), est-il abandonné ?
/// Seulement s'il est libre et assez ancien : dans le doute, il est gardé.
#[must_use]
pub fn is_stale_export_temporary(lock: TemporaryLock, age: Option<Duration>) -> bool {
    lock == TemporaryLock::Free && age.is_some_and(|age| age >= STALE_EXPORT_TEMPORARY_AGE)
}

#[cfg(test)]
mod tests;
