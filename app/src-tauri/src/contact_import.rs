//! Commandes IPC de l'import d'un carnet (ADR-012) : aperçu d'un fichier
//! choisi dans le sélecteur du système, puis import confirmé.
//!
//! Le fichier est choisi et lu en Rust (taille bornée avant lecture) ;
//! l'interface ne reçoit qu'un aperçu chiffré et un jeton, qu'elle rend
//! pour confirmer. Les fiches lues restent ici entre les deux, une seule
//! fois : un jeton périmé ou rejoué est refusé.

use std::io::Read;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use norelly_application::contact_import::{
    ApplyContactImport, ImportFailure, ImportPreview, PreviewContactImport,
};
use norelly_domain::connection::server_text;
use norelly_domain::contact_import::{
    ContactFormat, DuplicatePolicy, ImportPlan, ImportedCard, MAX_IMPORT_FILE_BYTES, RejectReason,
};
use norelly_domain::contact_lists::ContactListError;
use norelly_domain::contact_photo::CardPhotos;
use norelly_domain::contacts::ContactError;
use norelly_domain::ports::{ContactCodec, ContactCodecError, ContactStoreError};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::ipc::{
    ContactImportFormatDto, ContactImportOutcomeDto, ContactImportPreviewDto, DuplicatePolicyDto,
    ErrorCodeDto, ImportCountsDto, ImportRejectReasonDto, ImportRejectionDto, IpcError,
};
use crate::state::AppState;

/// Fiches rejetées détaillées au plus dans un aperçu.
pub const MAX_LISTED_REJECTIONS: usize = 100;

/// Fiches lues d'un aperçu, et leurs photos intégrées.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingCards {
    pub cards: Vec<ImportedCard>,
    pub photos: CardPhotos,
}

/// Fiches d'un aperçu en attente de confirmation (une à la fois).
#[derive(Debug, Default)]
pub struct PendingImport {
    slot: Mutex<Slot>,
}

/// État du créneau, sous un seul verrou : jeton attribué et fiches gardées
/// changent ensemble.
#[derive(Debug, Default)]
struct Slot {
    /// Dernier jeton attribué (jamais 0 ; repart à 1 après `u32::MAX`).
    last: u32,
    /// Jeton oublié pendant que ses fiches étaient prises par un import :
    /// elles ne sont pas rendues s'il échoue. L'interface empêche de
    /// fermer la fenêtre pendant un import : défense en profondeur.
    discarded: u32,
    cards: Option<(u32, PendingCards)>,
}

impl Slot {
    /// Fiches gardées si elles sont celles de l'aperçu `token`.
    fn take(&mut self, token: u32) -> Option<PendingCards> {
        match self.cards.take() {
            Some((t, cards)) if t == token => Some(cards),
            other => {
                self.cards = other;
                None
            }
        }
    }
}

impl PendingImport {
    fn slot(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Créneau dont le dernier jeton attribué est `last`.
    #[cfg(test)]
    fn with_last(last: u32) -> Self {
        Self {
            slot: Mutex::new(Slot {
                last,
                ..Slot::default()
            }),
        }
    }

    /// Garde les fiches d'un nouvel aperçu (l'aperçu précédent est
    /// périmé) ; renvoie son jeton (jamais 0).
    pub fn remember(&self, cards: PendingCards) -> u32 {
        let mut slot = self.slot();
        let token = slot.last.checked_add(1).unwrap_or(1);
        slot.last = token;
        slot.discarded = 0;
        let replaced = slot.cards.replace((token, cards));
        // Fiches remplacées libérées hors du verrou.
        drop(slot);
        drop(replaced);
        token
    }

    /// Fiches de l'aperçu `token`, une seule fois.
    pub fn take(&self, token: u32) -> Option<PendingCards> {
        self.slot().take(token)
    }

    /// Rend les fiches d'un import échoué, si aucun autre aperçu ne les a
    /// remplacées entre-temps et si l'aperçu n'a pas été oublié.
    pub fn restore(&self, token: u32, cards: PendingCards) {
        let mut slot = self.slot();
        if slot.cards.is_none() && slot.last == token && slot.discarded != token {
            slot.cards = Some((token, cards));
        }
    }

    /// Oublie l'aperçu `token` (fenêtre fermée sans importer) : ses fiches
    /// sont libérées tout de suite, ou à la fin d'un import en cours sans
    /// être rendues s'il échoue. Un jeton périmé, inconnu ou nul n'efface
    /// pas un aperçu plus récent.
    pub fn discard(&self, token: u32) {
        let mut slot = self.slot();
        if token == 0 || slot.last != token {
            return;
        }
        slot.discarded = token;
        let released = slot.take(token);
        // Fiches libérées hors du verrou : les autres commandes n'attendent
        // pas la libération.
        drop(slot);
        drop(released);
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn format_dto(format: ContactFormat) -> ContactImportFormatDto {
    match format {
        ContactFormat::VCard => ContactImportFormatDto::Vcard,
        ContactFormat::GoogleCsv => ContactImportFormatDto::GoogleCsv,
        ContactFormat::OutlookCsv => ContactImportFormatDto::OutlookCsv,
        ContactFormat::ThunderbirdCsv => ContactImportFormatDto::ThunderbirdCsv,
        ContactFormat::Ldif => ContactImportFormatDto::Ldif,
    }
}

fn reason_dto(reason: &RejectReason) -> ImportRejectReasonDto {
    match reason {
        RejectReason::Unreadable => ImportRejectReasonDto::Unreadable,
        RejectReason::ListNameLooksLikeAddress => ImportRejectReasonDto::ListNameLooksLikeAddress,
        RejectReason::ListMergeTooLarge => ImportRejectReasonDto::ListTooManyMembers,
        RejectReason::InvalidList(e) => match e {
            ContactListError::NoName => ImportRejectReasonDto::ListNoName,
            ContactListError::TooManyMembers => ImportRejectReasonDto::ListTooManyMembers,
            ContactListError::TooLong(_) => ImportRejectReasonDto::TooLong,
            ContactListError::InvalidCharacter(_)
            | ContactListError::InvalidMember
            | ContactListError::Empty => ImportRejectReasonDto::InvalidField,
        },
        RejectReason::Invalid(e) => match e {
            ContactError::NoAddress => ImportRejectReasonDto::NoAddress,
            ContactError::InvalidAddress { .. }
            | ContactError::DuplicateAddress { .. }
            | ContactError::SeveralPreferred => ImportRejectReasonDto::InvalidAddress,
            ContactError::TooLong(_) => ImportRejectReasonDto::TooLong,
            ContactError::InvalidCharacter(_)
            | ContactError::NameLooksLikeAddress(_)
            | ContactError::EmptyPhone { .. }
            | ContactError::InvalidBirthday
            | ContactError::EmptyPostalAddress { .. }
            | ContactError::InvalidWebsite { .. } => ImportRejectReasonDto::InvalidField,
            ContactError::TooManyAddresses
            | ContactError::TooManyPhones
            | ContactError::TooManyPostalAddresses
            | ContactError::TooManyWebsites => ImportRejectReasonDto::TooManyValues,
        },
    }
}

fn counts(plan: &ImportPlan) -> ImportCountsDto {
    ImportCountsDto {
        create: count(plan.create.len()),
        complete: count(plan.complete.len()),
        duplicates: count(plan.duplicates),
        lists: count(plan.lists.len()),
        missing_members: count(plan.missing_members),
    }
}

/// Aperçu chiffré pour l'interface (rejets identiques pour les deux
/// politiques : ils ne dépendent que des fiches).
pub(crate) fn preview_dto(
    token: u32,
    file_name: &str,
    preview: &ImportPreview,
) -> Result<ContactImportPreviewDto, IpcError> {
    let rejected = &preview.skip.rejected;
    Ok(ContactImportPreviewDto {
        token,
        file_name: server_text(file_name.as_bytes()),
        format: format_dto(preview.format),
        cards: count(preview.cards.len()),
        skip: counts(&preview.skip),
        complete: counts(&preview.complete),
        renamed: count(preview.skip.renamed),
        rejected_total: count(rejected.len()),
        rejected: rejected
            .iter()
            .take(MAX_LISTED_REJECTIONS)
            .map(|r| ImportRejectionDto {
                card: count(r.card),
                hint: r.hint.clone(),
                reason: reason_dto(&r.reason),
            })
            .collect(),
    })
}

/// Échec d'un import, en code.
pub(crate) fn import_error(e: ImportFailure) -> IpcError {
    match e {
        ImportFailure::File(e) => IpcError::new(match e {
            ContactCodecError::TooLarge => ErrorCodeDto::ContactImportTooLarge,
            ContactCodecError::TooManyCards => ErrorCodeDto::ContactImportTooManyCards,
            ContactCodecError::UnknownFormat => ErrorCodeDto::ContactImportUnknownFormat,
        }),
        ImportFailure::Store(e) => match e {
            ContactStoreError::DuplicateAddress(address) => {
                IpcError::new(ErrorCodeDto::ContactDuplicateAddress).with_detail(address)
            }
            ContactStoreError::NotFound => IpcError::new(ErrorCodeDto::ContactNotFound),
            // Rendu par l'import seul : la fusion a son propre code.
            ContactStoreError::Conflict => IpcError::new(ErrorCodeDto::ContactImportConflict),
            ContactStoreError::Corrupted(e) => IpcError::storage(e),
            ContactStoreError::Store(e) => IpcError::storage(e),
        },
    }
}

fn read_failed(e: &std::io::Error) -> IpcError {
    IpcError::new(ErrorCodeDto::ContactImportReadFailed).with_detail(e)
}

/// Contenu du fichier `path`, au plus `MAX_IMPORT_FILE_BYTES` octets
/// (taille vérifiée avant lecture, lecture bornée : un fichier qui grandit
/// ou un périphérique sans fin est refusé aussi).
pub(crate) fn read_contact_file(path: &Path) -> Result<Vec<u8>, IpcError> {
    let too_large = || IpcError::new(ErrorCodeDto::ContactImportTooLarge);
    let limit = u64::try_from(MAX_IMPORT_FILE_BYTES).unwrap_or(u64::MAX);
    // Fichier ordinaire seulement : une FIFO bloquerait l'ouverture, un
    // périphérique n'a pas de fin. Revérifié sur le fichier ouvert.
    let not_regular = || IpcError::new(ErrorCodeDto::ContactImportReadFailed);
    if !std::fs::metadata(path)
        .map_err(|e| read_failed(&e))?
        .is_file()
    {
        return Err(not_regular());
    }
    let file = std::fs::File::open(path).map_err(|e| read_failed(&e))?;
    let metadata = file.metadata().map_err(|e| read_failed(&e))?;
    if !metadata.is_file() {
        return Err(not_regular());
    }
    if metadata.len() > limit {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| read_failed(&e))?;
    if bytes.len() > MAX_IMPORT_FILE_BYTES {
        return Err(too_large());
    }
    Ok(bytes)
}

/// Ouvre le sélecteur du système (titre et nom du filtre localisés par
/// l'interface), lit le fichier choisi et en chiffre l'import ; `None` si
/// l'utilisateur annule. L'aperçu précédent est périmé.
#[tauri::command]
#[specta::specta]
pub async fn preview_contact_import(
    app: AppHandle,
    state: State<'_, AppState>,
    title: String,
    filter: String,
) -> Result<Option<ContactImportPreviewDto>, IpcError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title(server_text(title.as_bytes()))
        .add_filter(server_text(filter.as_bytes()), CONTACT_FILE_EXTENSIONS)
        .pick_file(move |file| {
            let _ = tx.send(file);
        });
    let Some(path) = rx.await.ok().flatten().and_then(|f| f.into_path().ok()) else {
        return Ok(None);
    };
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let imports = &state.imports;
    // Lecture et décodage hors des fils asynchrones.
    let codec = imports.codec;
    let decoded = tokio::task::spawn_blocking(move || {
        let bytes = read_contact_file(&path)?;
        codec
            .decode(&bytes)
            .map_err(|e| import_error(ImportFailure::File(e)))
    })
    .await
    .map_err(IpcError::internal)??;
    let mut preview = PreviewContactImport {
        codec: &imports.codec,
        directory: &state.contacts,
        normalizer: &imports.nfc,
    }
    .plan(decoded)
    .await
    .map_err(import_error)?;
    // Les photos ne servent pas au résumé : déplacées, pas copiées.
    let token = imports.pending.remember(PendingCards {
        cards: preview.cards.clone(),
        photos: std::mem::take(&mut preview.photos),
    });
    preview_dto(token, &file_name, &preview).map(Some)
}

/// Importe les fiches de l'aperçu `token` en une transaction, selon
/// `policy` ; le plan est recalculé contre le carnet du moment.
#[tauri::command]
#[specta::specta]
pub async fn apply_contact_import(
    state: State<'_, AppState>,
    token: u32,
    policy: DuplicatePolicyDto,
) -> Result<ContactImportOutcomeDto, IpcError> {
    let imports = &state.imports;
    let cards = imports
        .pending
        .take(token)
        .ok_or_else(|| IpcError::new(ErrorCodeDto::ContactImportExpired))?;
    let policy = match policy {
        DuplicatePolicyDto::Skip => DuplicatePolicy::Skip,
        DuplicatePolicyDto::Complete => DuplicatePolicy::Complete,
    };
    // Pas de ménage des photos pendant l'import : une photo enregistrée
    // n'est donnée à son contact qu'à la fin.
    let _photos = imports.photo_writes.lock().await;
    let outcome = ApplyContactImport {
        book: &state.contacts,
        normalizer: &imports.nfc,
        photo_codec: &imports.photo_codec,
        photos: &state.contact_photos,
    }
    .execute(&cards.cards, &cards.photos, policy)
    .await;
    match outcome {
        Ok(outcome) => Ok(ContactImportOutcomeDto {
            created: count(outcome.created),
            completed: count(outcome.completed),
            lists_created: count(outcome.lists_created),
        }),
        Err(e) => {
            imports.pending.restore(token, cards);
            Err(import_error(e))
        }
    }
}

/// Oublie l'aperçu `token` (fenêtre fermée sans importer) : ses fiches
/// sont libérées. Sans effet pour un jeton périmé ou inconnu.
#[tauri::command]
#[specta::specta]
pub async fn discard_contact_import(
    state: State<'_, AppState>,
    token: u32,
) -> Result<(), IpcError> {
    state.imports.pending.discard(token);
    Ok(())
}

/// Extensions proposées par le sélecteur.
const CONTACT_FILE_EXTENSIONS: &[&str] = &["vcf", "vcard", "csv", "ldif", "ldi", "txt"];

#[cfg(test)]
mod tests;
