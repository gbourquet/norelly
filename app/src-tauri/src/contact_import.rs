//! Commandes IPC de l'import d'un carnet (ADR-012) : aperçu d'un fichier
//! choisi dans le sélecteur du système, puis import confirmé.
//!
//! Le fichier est choisi et lu en Rust (taille bornée avant lecture) ;
//! l'interface ne reçoit qu'un aperçu chiffré et un jeton, qu'elle rend
//! pour confirmer. Les fiches lues restent ici entre les deux, une seule
//! fois : un jeton périmé ou rejoué est refusé.

use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

use norelly_application::contact_import::{
    ApplyContactImport, ImportFailure, ImportPreview, PreviewContactImport,
};
use norelly_domain::connection::server_text;
use norelly_domain::contact_import::{
    ContactFormat, DuplicatePolicy, ImportPlan, ImportedCard, MAX_IMPORT_FILE_BYTES,
    RejectReason,
};
use norelly_domain::contacts::ContactError;
use norelly_domain::ports::{ContactCodecError, ContactStoreError};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::ipc::{
    ContactImportFormatDto, ContactImportOutcomeDto, ContactImportPreviewDto,
    DuplicatePolicyDto, ErrorCodeDto, ImportCountsDto, ImportRejectReasonDto,
    ImportRejectionDto, IpcError,
};
use crate::state::AppState;

/// Fiches rejetées détaillées au plus dans un aperçu.
pub const MAX_LISTED_REJECTIONS: usize = 100;

/// Fiches d'un aperçu en attente de confirmation (une à la fois).
#[derive(Debug, Default)]
pub struct PendingImport {
    /// Dernier jeton attribué (jamais réutilisé pendant la session).
    last: AtomicU32,
    slot: Mutex<Option<(u32, Vec<ImportedCard>)>>,
}

impl PendingImport {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<(u32, Vec<ImportedCard>)>> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Garde les fiches d'un nouvel aperçu (l'aperçu précédent est
    /// périmé) ; renvoie son jeton (jamais 0).
    pub fn remember(&self, cards: Vec<ImportedCard>) -> u32 {
        let token = self
            .last
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1)
            .max(1);
        *self.slot() = Some((token, cards));
        token
    }

    /// Fiches de l'aperçu `token`, une seule fois.
    pub fn take(&self, token: u32) -> Option<Vec<ImportedCard>> {
        let mut slot = self.slot();
        match slot.take() {
            Some((t, cards)) if t == token => Some(cards),
            other => {
                *slot = other;
                None
            }
        }
    }

    /// Rend les fiches d'un import échoué, si aucun autre aperçu ne les a
    /// remplacées entre-temps.
    pub fn restore(&self, token: u32, cards: Vec<ImportedCard>) {
        let mut slot = self.slot();
        if slot.is_none() && self.last.load(Ordering::Relaxed) == token {
            *slot = Some((token, cards));
        }
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
        RejectReason::List => ImportRejectReasonDto::List,
        RejectReason::Invalid(e) => match e {
            ContactError::NoAddress => ImportRejectReasonDto::NoAddress,
            ContactError::InvalidAddress { .. }
            | ContactError::DuplicateAddress { .. }
            | ContactError::SeveralPreferred => ImportRejectReasonDto::InvalidAddress,
            ContactError::TooLong(_) => ImportRejectReasonDto::TooLong,
            ContactError::InvalidCharacter(_) | ContactError::EmptyPhone { .. } => {
                ImportRejectReasonDto::InvalidField
            }
            ContactError::TooManyAddresses | ContactError::TooManyPhones => {
                ImportRejectReasonDto::TooManyValues
            }
        },
    }
}

fn counts(plan: &ImportPlan) -> ImportCountsDto {
    ImportCountsDto {
        create: count(plan.create.len()),
        complete: count(plan.complete.len()),
        duplicates: count(plan.duplicates),
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
    let file = std::fs::File::open(path).map_err(|e| read_failed(&e))?;
    if file.metadata().map_err(|e| read_failed(&e))?.len() > limit {
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
    let bytes = tokio::task::spawn_blocking(move || read_contact_file(&path))
        .await
        .map_err(IpcError::internal)??;
    let imports = &state.imports;
    let preview = PreviewContactImport {
        codec: &imports.codec,
        directory: &state.contacts,
        normalizer: &imports.nfc,
    }
    .execute(&bytes)
    .await
    .map_err(import_error)?;
    let token = imports.pending.remember(preview.cards.clone());
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
    let outcome = ApplyContactImport {
        book: &state.contacts,
        normalizer: &imports.nfc,
    }
    .execute(&cards, policy)
    .await;
    match outcome {
        Ok(outcome) => Ok(ContactImportOutcomeDto {
            created: count(outcome.created),
            completed: count(outcome.completed),
        }),
        Err(e) => {
            imports.pending.restore(token, cards);
            Err(import_error(e))
        }
    }
}

/// Extensions proposées par le sélecteur.
const CONTACT_FILE_EXTENSIONS: &[&str] = &["vcf", "vcard", "csv", "ldif", "ldi", "txt"];

#[cfg(test)]
mod tests;
