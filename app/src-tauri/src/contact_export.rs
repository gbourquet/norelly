//! Commande IPC de l'export du carnet (ADR-012) : fiches préparées par le
//! domaine, fichier écrit en Rust à l'emplacement choisi dans le sélecteur
//! d'enregistrement du système. L'interface ne fournit jamais de chemin.
//!
//! Le fichier est produit en mémoire (borné à la taille d'un import)
//! avant d'ouvrir le sélecteur, puis écrit de façon atomique : fichier
//! temporaire voisin, synchronisé, renommé sur la cible. Les temporaires
//! abandonnés par un export interrompu vers la même cible sont effacés
//! avant l'écriture.

use std::ffi::OsStr;
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;

use norelly_application::contact_export::{ExportFailure, PrepareContactExport};
use norelly_domain::connection::server_text;
use norelly_domain::contact_export::{
    ContactSelection, ExportFormat, MAX_EXPORT_FILE_BYTES, MAX_EXPORT_FOLDER_SCAN, SelectionError,
    TemporaryLock, export_file_name, export_label, is_export_temporary_of,
    is_stale_export_temporary,
};
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
            ContactSelection::of(ids).map_err(|e| match e {
                SelectionError::TooMany => IpcError::new(ErrorCodeDto::ContactSelectionTooMany),
                SelectionError::Empty | SelectionError::InvalidId => {
                    IpcError::invalid(FieldDto::Contact)
                }
            })
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
/// exclusivité (droits du propriétaire seul sous Unix, ou les droits de
/// lecture et d'écriture du fichier ordinaire remplacé), écrit et synchronisé, puis renommé sur `path` (un lien à
/// cet emplacement est remplacé, jamais suivi). En cas d'échec, `path`
/// est intact et le temporaire retiré. Les temporaires d'un export vers
/// `path` interrompu par un arrêt brutal sont d'abord effacés
/// ([`remove_stale_temporaries`]).
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
    remove_stale_temporaries(dir, name, SystemTime::now());
    let (temp, mut file) = create_temporary(dir, name)?;
    let result = (|| {
        file.write_all(bytes)?;
        // Un fichier ordinaire remplacé garde ses droits de lecture et
        // d'écriture (lien non suivi : sa cible n'est pas remplacée ; ni
        // bit spécial ni exécution). Sous Windows, rien n'est repris : la
        // lecture seule empêcherait le remplacement.
        #[cfg(unix)]
        if let Ok(existing) = std::fs::symlink_metadata(path)
            && existing.file_type().is_file()
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = existing.permissions().mode() & 0o666;
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
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

/// Crée, dans `dir`, le temporaire d'un export vers `target`
/// (`.<target>.<pid>-<n>.tmp`), en exclusivité et lisible par son
/// propriétaire seulement sous Unix, et pose son verrou consultatif : tenu
/// jusqu'à la fermeture du fichier, il dit à un autre export vers la même
/// cible que ce temporaire est vivant. Le verrou est facultatif (certains
/// partages n'en ont pas).
pub(crate) fn create_temporary(
    dir: &Path,
    target: &OsStr,
) -> std::io::Result<(std::path::PathBuf, std::fs::File)> {
    for n in 0..100_u32 {
        let mut temp_name = std::ffi::OsString::from(".");
        temp_name.push(target);
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
                let _ = file.try_lock();
                return Ok((temp, file));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("trop de fichiers temporaires"))
}

/// Efface, dans `dir`, les temporaires qu'un export vers `target` a pu
/// laisser après un arrêt brutal (ils contiennent tout un carnet) : seuls
/// les fichiers ordinaires dont le nom est reconnu par
/// [`is_export_temporary_of`] et que le domaine juge abandonnés à l'heure
/// `now` ([`is_stale_export_temporary`] : verrou libre et assez ancien).
/// Un lien, un dossier ou un fichier spécial n'est ni suivi ni effacé. Au
/// plus [`MAX_EXPORT_FOLDER_SCAN`] entrées sont examinées. Un échec
/// (dossier illisible, fichier déjà parti) est ignoré : il n'empêche pas
/// l'export.
pub(crate) fn remove_stale_temporaries(dir: &Path, target: &OsStr, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.take(MAX_EXPORT_FOLDER_SCAN).flatten() {
        let name = entry.file_name();
        if !is_export_temporary_of(target.as_encoded_bytes(), name.as_encoded_bytes()) {
            continue;
        }
        let path = entry.path();
        // Fichier ordinaire seulement : un lien ou une jonction n'est pas
        // suivi, un dossier ou un fichier spécial n'est pas ouvert.
        if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file()) {
            continue;
        }
        let Ok(file) = open_temporary(&path) else {
            continue;
        };
        let Ok(meta) = file.metadata() else {
            continue;
        };
        let age = meta
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok());
        // Un temporaire trop récent est gardé quel que soit son verrou :
        // il n'est pas verrouillé ici, pour ne pas gêner l'export qui vient
        // de le créer.
        if !meta.file_type().is_file() || !is_stale_export_temporary(TemporaryLock::Free, age) {
            continue;
        }
        let lock = match file.try_lock() {
            Ok(()) => TemporaryLock::Free,
            Err(std::fs::TryLockError::WouldBlock) => TemporaryLock::Held,
            Err(std::fs::TryLockError::Error(_)) => TemporaryLock::Unknown,
        };
        if is_stale_export_temporary(lock, age) {
            // Effacer un lien qui aurait pris la place du fichier ne
            // retirerait que le lien, jamais sa cible.
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Ouvre en lecture un temporaire à examiner, sans suivre de lien qui
/// aurait pris sa place (Unix : échec ; Windows : le lien lui-même est
/// ouvert, et l'appelant le refuse car ce n'est pas un fichier ordinaire)
/// ni bloquer sur un fichier spécial (Unix).
fn open_temporary(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(
        &mut options,
        libc::O_NOFOLLOW | libc::O_NONBLOCK,
    );
    // FILE_FLAG_OPEN_REPARSE_POINT : un lien symbolique ou une jonction est
    // ouvert lui-même, jamais sa cible.
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::custom_flags(&mut options, 0x0020_0000);
    options.open(path)
}

/// Prépare et produit le fichier, ouvre le sélecteur d'enregistrement du
/// système (titre, nom du filtre et nom de base du fichier proposé
/// localisés par l'interface, le nom revalidé par le domaine) et écrit le
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
    file_name: String,
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
    let proposed = export_file_name(&file_name, format);
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title(server_text(export_label(&title).as_bytes()))
        .add_filter(server_text(export_label(&filter).as_bytes()), &[extension])
        .set_file_name(proposed)
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
