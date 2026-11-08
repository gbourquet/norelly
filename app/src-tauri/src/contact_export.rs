//! Commande IPC de l'export du carnet (ADR-012) : fiches préparées par le
//! domaine, fichier écrit en Rust à l'emplacement choisi dans le sélecteur
//! d'enregistrement du système. L'interface ne fournit jamais de chemin.
//!
//! Le fichier est produit en mémoire (borné à la taille d'un import)
//! avant d'ouvrir le sélecteur, puis écrit de façon atomique : fichier
//! temporaire voisin, synchronisé, renommé sur la cible.

use std::io::Write;
use std::path::Path;

use norelly_application::contact_export::{ExportFailure, PrepareContactExport};
use norelly_domain::connection::server_text;
use norelly_domain::contact_export::{ContactSelection, ExportFormat, MAX_EXPORT_FILE_BYTES};
use norelly_domain::ports::{ContactCodec, ContactCodecError};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::ipc::{
    ContactExportFormatDto, ContactExportOutcomeDto, ContactExportSelectionDto, ErrorCodeDto,
    FieldDto, IpcError,
};
use crate::state::AppState;

/// Sélection reçue de l'interface, revalidée par le domaine.
pub(crate) fn selection_of(dto: &ContactExportSelectionDto) -> Result<ContactSelection, IpcError> {
    match dto {
        ContactExportSelectionDto::All => Ok(ContactSelection::All),
        ContactExportSelectionDto::Contacts { ids } => {
            ContactSelection::of(ids).map_err(|_| IpcError::invalid(FieldDto::Contact))
        }
    }
}

pub(crate) fn format_of(dto: ContactExportFormatDto) -> ExportFormat {
    match dto {
        ContactExportFormatDto::Vcard4 => ExportFormat::VCard4,
        ContactExportFormatDto::Vcard3 => ExportFormat::VCard3,
        ContactExportFormatDto::GoogleCsv => ExportFormat::GoogleCsv,
        ContactExportFormatDto::Ldif => ExportFormat::Ldif,
    }
}

/// Échec de la préparation, en code.
pub(crate) fn export_error(e: ExportFailure) -> IpcError {
    match e {
        ExportFailure::Empty => IpcError::new(ErrorCodeDto::ContactExportEmpty),
        ExportFailure::TooMany => IpcError::new(ErrorCodeDto::ContactExportTooLarge),
        ExportFailure::Store(e) => IpcError::storage(e),
    }
}

/// Échec de l'écriture du fichier en mémoire, en code.
pub(crate) fn codec_error(e: ContactCodecError) -> IpcError {
    match e {
        ContactCodecError::TooLarge | ContactCodecError::TooManyCards => {
            IpcError::new(ErrorCodeDto::ContactExportTooLarge)
        }
        ContactCodecError::UnknownFormat => IpcError::internal(e),
    }
}

/// Écrit `bytes` (au plus [`MAX_EXPORT_FILE_BYTES`]) dans `path`, de
/// façon atomique : un fichier temporaire du même dossier, créé en
/// exclusivité (droits du propriétaire seul sous Unix, ou ceux du fichier
/// remplacé), écrit et synchronisé, puis renommé sur `path` (un lien à
/// cet emplacement est remplacé, jamais suivi). En cas d'échec, `path`
/// est intact et le temporaire retiré.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if bytes.len() > MAX_EXPORT_FILE_BYTES {
        return Err(std::io::Error::other("export plus gros que la borne"));
    }
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("chemin sans nom de fichier"))?;
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let mut created = None;
    for n in 0..100_u32 {
        let mut temp_name = std::ffi::OsString::from(".");
        temp_name.push(name);
        temp_name.push(format!(".{}-{n}.tmp", std::process::id()));
        let temp = dir.join(temp_name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        // Le carnet est une donnée personnelle : lisible par son
        // propriétaire seulement.
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        match options.open(&temp) {
            Ok(file) => {
                created = Some((temp, file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    let (temp, mut file) =
        created.ok_or_else(|| std::io::Error::other("trop de fichiers temporaires"))?;
    let result = (|| {
        file.write_all(bytes)?;
        // Un fichier remplacé garde ses droits.
        if let Ok(existing) = std::fs::metadata(path) {
            file.set_permissions(existing.permissions())?;
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)?;
        // Le renommage atteint le disque (sans effet sous Windows, et
        // facultatif : certains dossiers, ceux des portails par exemple, ne
        // se synchronisent pas).
        #[cfg(unix)]
        let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Prépare et produit le fichier, ouvre le sélecteur d'enregistrement du
/// système (titre et nom du filtre localisés par l'interface) et écrit le
/// fichier choisi ; `None` si l'utilisateur annule.
#[tauri::command]
#[specta::specta]
pub async fn export_contacts(
    app: AppHandle,
    state: State<'_, AppState>,
    selection: ContactExportSelectionDto,
    format: ContactExportFormatDto,
    title: String,
    filter: String,
) -> Result<Option<ContactExportOutcomeDto>, IpcError> {
    let selection = selection_of(&selection)?;
    let format = format_of(format);
    let cards = PrepareContactExport {
        directory: &state.contacts,
    }
    .execute(&selection, format)
    .await
    .map_err(export_error)?;
    let exported = u32::try_from(cards.len()).unwrap_or(u32::MAX);
    let codec = state.imports.codec;
    // Mise en forme hors des fils asynchrones : tout un carnet.
    let bytes = tokio::task::spawn_blocking(move || codec.encode(&cards, format))
        .await
        .map_err(IpcError::internal)?
        .map_err(codec_error)?;

    let extension = format.extension();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title(server_text(title.as_bytes()))
        .add_filter(server_text(filter.as_bytes()), &[extension])
        .set_file_name(format!("contacts.{extension}"))
        .save_file(move |file| {
            let _ = tx.send(file);
        });
    let Some(path) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = path
        .into_path()
        .map_err(|_| IpcError::invalid(FieldDto::Folder))?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    tokio::task::spawn_blocking(move || write_atomic(&path, &bytes))
        .await
        .map_err(IpcError::internal)?
        .map_err(|e| IpcError::new(ErrorCodeDto::WriteFailed).with_detail(e))?;
    Ok(Some(ContactExportOutcomeDto {
        exported,
        file_name: server_text(file_name.as_bytes()),
    }))
}

#[cfg(test)]
mod tests;
