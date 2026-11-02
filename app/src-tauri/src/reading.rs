//! Commandes IPC de lecture : dossiers, conversations, message ouvert,
//! images distantes, préférences, liens externes, pièces jointes.
//!
//! Toute entrée de l'interface est revalidée ici (identifiants, portée,
//! chemin de partie, lien) ; les décisions de sécurité (schéma d'un lien,
//! autorisation d'un expéditeur, type exécutable, dossier d'écriture) sont
//! prises en Rust.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use norelly_application::reading::allow_remote_images as allow_images;
use norelly_domain::account::AccountId;
use norelly_domain::conversation::ConversationScope;
use norelly_domain::fetch_policy::PartPath;
use norelly_domain::mailbox::{MailboxId, MailboxRole};
use norelly_domain::ports::{
    ConversationCursor, ConversationId, ConversationQuery, FolderTreeQuery, MessageId,
    ReadingPreferences,
};
use norelly_domain::reading::{
    ExternalLink, is_unsafe_to_open, numbered_file_name, safe_file_name,
};
use norelly_render::BodyMode;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::ipc::{
    AttachmentDto, BodyModeDto, ConversationCursorDto, ConversationDto, ConversationMessageDto,
    ConversationPageDto, CorrespondentDto, FolderDto, IpcError, MessageViewDto,
    ReadingPreferencesDto, ScopeDto, date_dto, date_from_dto, id32,
};
use crate::state::AppState;

fn message_id(raw: u32) -> Result<MessageId, IpcError> {
    MessageId::new(i64::from(raw)).ok_or_else(|| IpcError::new("message invalide"))
}

fn part_path(raw: &str) -> Result<PartPath, IpcError> {
    PartPath::parse(raw).ok_or_else(|| IpcError::new("partie invalide"))
}

/// Portée revalidée par le domaine.
pub fn scope_of(dto: &ScopeDto) -> Result<ConversationScope, IpcError> {
    match dto {
        ScopeDto::Mailbox { id } => MailboxId::new(i64::from(*id))
            .map(ConversationScope::Mailbox)
            .ok_or_else(|| IpcError::new("dossier invalide")),
        ScopeDto::Unified { role } => {
            let role = MailboxRole::parse(role).ok_or_else(|| IpcError::new("rôle inconnu"))?;
            ConversationScope::unified(role).map_err(IpcError::new)
        }
    }
}

/// Dossiers d'un compte.
#[tauri::command]
#[specta::specta]
pub async fn list_folders(
    state: State<'_, AppState>,
    account: u32,
) -> Result<Vec<FolderDto>, IpcError> {
    let account = AccountId::new(i64::from(account)).map_err(IpcError::new)?;
    let tree = state
        .mail
        .store
        .tree(account)
        .await
        .map_err(IpcError::new)?;
    tree.iter()
        .map(|f| {
            let depth = f
                .delimiter
                .map_or(0, |d| f.name.as_str().matches(d).count());
            Ok(FolderDto {
                id: id32(f.id.get())?,
                name: f.name.to_string(),
                leaf: f.name.leaf(f.delimiter).to_owned(),
                depth: u32::try_from(depth).unwrap_or(u32::MAX),
                role: f.role.as_str().to_owned(),
                selectable: f.selectable,
                total: f.total,
                unread: f.unread,
            })
        })
        .collect()
}

/// Une page de conversations.
#[tauri::command]
#[specta::specta]
pub async fn list_conversations(
    state: State<'_, AppState>,
    scope: ScopeDto,
    cursor: Option<ConversationCursorDto>,
    limit: u32,
) -> Result<ConversationPageDto, IpcError> {
    let scope = scope_of(&scope)?;
    let cursor = cursor
        .map(|c| {
            Ok::<_, IpcError>(ConversationCursor {
                date: date_from_dto(c.date),
                id: ConversationId::new(i64::from(c.id))
                    .ok_or_else(|| IpcError::new("curseur invalide"))?,
            })
        })
        .transpose()?;
    let page = state
        .mail
        .store
        .conversations(scope, cursor, limit)
        .await
        .map_err(IpcError::new)?;
    let items = page
        .items
        .iter()
        .map(|c| {
            Ok(ConversationDto {
                id: id32(c.id.get())?,
                account: id32(c.account.get())?,
                mailbox: id32(c.mailbox.get())?,
                date: date_dto(c.date),
                subject: c.subject.clone(),
                senders: c.senders.iter().map(CorrespondentDto::from).collect(),
                messages: c.messages,
                unread: c.unread,
                flagged: c.flagged,
                has_attachments: c.has_attachments,
            })
        })
        .collect::<Result<_, IpcError>>()?;
    let next = page
        .next
        .map(|n| {
            Ok::<_, IpcError>(ConversationCursorDto {
                date: date_dto(n.date),
                id: id32(n.id.get())?,
            })
        })
        .transpose()?;
    Ok(ConversationPageDto { items, next })
}

/// Messages d'une conversation, du plus ancien au plus récent.
#[tauri::command]
#[specta::specta]
pub async fn conversation_messages(
    state: State<'_, AppState>,
    conversation: u32,
) -> Result<Vec<ConversationMessageDto>, IpcError> {
    let id = ConversationId::new(i64::from(conversation))
        .ok_or_else(|| IpcError::new("conversation invalide"))?;
    let messages = state
        .mail
        .store
        .conversation_messages(id)
        .await
        .map_err(IpcError::new)?;
    messages
        .iter()
        .map(|m| {
            Ok(ConversationMessageDto {
                id: id32(m.id.get())?,
                account: id32(m.account.get())?,
                mailbox: id32(m.mailbox.get())?,
                date: date_dto(m.date),
                from: m.from.as_ref().map(CorrespondentDto::from),
                to: m.to.iter().map(CorrespondentDto::from).collect(),
                cc: m.cc.iter().map(CorrespondentDto::from).collect(),
                subject: m.subject.clone(),
                unread: !m.flags.seen,
                flagged: m.flags.flagged,
                has_attachments: m.has_attachments,
            })
        })
        .collect()
}

/// Ouvre un message : rendu (corps téléchargé au besoin) et pièces
/// jointes. Le document est ensuite servi par `mailview://`.
#[tauri::command]
#[specta::specta]
pub async fn open_message(
    state: State<'_, AppState>,
    message: u32,
) -> Result<MessageViewDto, IpcError> {
    let id = message_id(message)?;
    let rendered = state
        .mail
        .render(id)
        .await
        .map_err(IpcError::new)?
        .ok_or_else(|| IpcError::new("message introuvable"))?;
    Ok(MessageViewDto {
        id: message,
        mode: match rendered.mode {
            BodyMode::Html => BodyModeDto::Html,
            BodyMode::Text => BodyModeDto::Text,
            BodyMode::Empty => BodyModeDto::Empty,
        },
        has_html: rendered.has_html,
        blocked_images: u32::try_from(rendered.blocked_images).unwrap_or(u32::MAX),
        attachments: rendered
            .attachments
            .iter()
            .filter(|a| !a.inline)
            .map(|a| AttachmentDto {
                part: a.path.to_string(),
                name: a.name.clone(),
                media_type: a.media_type.clone(),
                size: u32::try_from(a.size).unwrap_or(u32::MAX),
                available: a.available,
                can_open: !is_unsafe_to_open(&a.name),
            })
            .collect(),
    })
}

/// Autorise les images distantes de l'expéditeur du message (décision
/// enregistrée en Rust, vérifiée à chaque image servie).
#[tauri::command]
#[specta::specta]
pub async fn allow_remote_images(state: State<'_, AppState>, message: u32) -> Result<(), IpcError> {
    let id = message_id(message)?;
    allow_images(&*state.mail.store, &state.mail.prefs, id)
        .await
        .map_err(IpcError::new)
}

#[tauri::command]
#[specta::specta]
pub async fn reading_preferences(
    state: State<'_, AppState>,
) -> Result<ReadingPreferencesDto, IpcError> {
    Ok(ReadingPreferencesDto {
        prefer_plain_text: state
            .mail
            .prefs
            .prefer_plain_text()
            .await
            .map_err(IpcError::new)?,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn set_prefer_plain_text(
    state: State<'_, AppState>,
    prefer: bool,
) -> Result<(), IpcError> {
    state
        .mail
        .prefs
        .set_prefer_plain_text(prefer)
        .await
        .map_err(IpcError::new)
}

/// Demande une synchronisation complète d'un compte.
#[tauri::command]
#[specta::specta]
pub async fn sync_account(state: State<'_, AppState>, account: u32) -> Result<(), IpcError> {
    let account = AccountId::new(i64::from(account)).map_err(IpcError::new)?;
    state.mail.sync_now(account).await;
    Ok(())
}

/// Ouvre un lien dans le navigateur (ou le client de messagerie) du
/// système : `http`, `https` et `mailto` seulement.
#[tauri::command]
#[specta::specta]
pub async fn open_external(url: String) -> Result<(), IpcError> {
    let link = ExternalLink::parse(&url).map_err(IpcError::new)?;
    open::that_detached(link.as_str())
        .map_err(|e| IpcError::new(format!("ouverture impossible : {e}")))
}

/// Écrit `bytes` dans `dir` sous un nom assaini, sans jamais écraser ni
/// suivre un lien existant (création exclusive) ; renvoie le chemin.
pub fn write_new_file(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let safe = safe_file_name(name);
    for n in 1..=1000u32 {
        let candidate = if n == 1 {
            safe.clone()
        } else {
            safe_file_name(&numbered_file_name(&safe, n))
        };
        let path = dir.join(&candidate);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(bytes)?;
                file.sync_all()?;
                mark_from_internet(&path)?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("trop de fichiers homonymes"))
}

/// Marque « provenant d'Internet » (Mark-of-the-Web, flux NTFS
/// `Zone.Identifier`, zone 3) : SmartScreen, le Mode protégé d'Office et le
/// blocage des macros s'appliquent au fichier. Sans objet hors de Windows.
#[cfg(windows)]
fn mark_from_internet(path: &Path) -> std::io::Result<()> {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    std::fs::write(PathBuf::from(stream), b"[ZoneTransfer]\r\nZoneId=3\r\n")
}

#[cfg(not(windows))]
#[allow(clippy::unnecessary_wraps)]
fn mark_from_internet(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

async fn attachment(
    state: &AppState,
    message: u32,
    part: &str,
) -> Result<norelly_render::PartContent, IpcError> {
    let id = message_id(message)?;
    let path = part_path(part)?;
    state
        .mail
        .part(id, &path)
        .await
        .map_err(IpcError::new)?
        .ok_or_else(|| IpcError::new("pièce jointe introuvable"))
}

/// Enregistre une pièce jointe dans un dossier choisi par l'utilisateur
/// (sélecteur du système) ; renvoie le nom du fichier écrit, `None` si
/// l'utilisateur a annulé.
#[tauri::command]
#[specta::specta]
pub async fn save_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    message: u32,
    part: String,
) -> Result<Option<String>, IpcError> {
    let content = attachment(&state, message, &part).await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Enregistrer la pièce jointe dans…")
        .pick_folder(move |folder| {
            let _ = tx.send(folder);
        });
    let Some(folder) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let dir = folder
        .into_path()
        .map_err(|_| IpcError::new("dossier invalide"))?;
    let written = tauri::async_runtime::spawn_blocking(move || {
        write_new_file(&dir, &content.name, &content.bytes)
    })
    .await
    .map_err(IpcError::new)?
    .map_err(|e| IpcError::new(format!("écriture impossible : {e}")))?;
    Ok(written
        .file_name()
        .map(|n| n.to_string_lossy().into_owned()))
}

/// Ouvre une pièce jointe avec l'application du système, depuis un
/// dossier privé. Les types exécutables sont refusés (à enregistrer).
#[tauri::command]
#[specta::specta]
pub async fn open_attachment(
    state: State<'_, AppState>,
    message: u32,
    part: String,
) -> Result<(), IpcError> {
    let content = attachment(&state, message, &part).await?;
    if is_unsafe_to_open(&content.name) {
        return Err(IpcError::new(
            "type de fichier exécutable : enregistrez-le plutôt que de l'ouvrir",
        ));
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = state.mail.open_dir.join(format!("{message}-{nanos}"));
    let written = tauri::async_runtime::spawn_blocking(move || {
        crate::state::create_private_dir(&dir)?;
        write_new_file(&dir, &content.name, &content.bytes)
    })
    .await
    .map_err(IpcError::new)?
    .map_err(|e| IpcError::new(format!("écriture impossible : {e}")))?;
    open::that_detached(&written).map_err(|e| IpcError::new(format!("ouverture impossible : {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_is_revalidated() {
        assert!(scope_of(&ScopeDto::Mailbox { id: 0 }).is_err());
        assert!(
            scope_of(&ScopeDto::Unified {
                role: "inbox".into()
            })
            .is_ok()
        );
        assert!(
            scope_of(&ScopeDto::Unified {
                role: "normal".into()
            })
            .is_err()
        );
        assert!(scope_of(&ScopeDto::Unified { role: "all".into() }).is_err());
        assert!(
            scope_of(&ScopeDto::Unified {
                role: "x' OR 1".into()
            })
            .is_err()
        );
        assert!(part_path("1.2").is_ok());
        assert!(part_path("../1").is_err());
        assert!(message_id(0).is_err());
    }

    #[test]
    fn files_are_written_only_inside_the_chosen_folder() {
        let dir = std::env::temp_dir().join(format!("norelly-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        for hostile in ["../../evil.sh", "/etc/passwd", "..", "CON", "a/b\\c.txt"] {
            let path = write_new_file(&dir, hostile, b"x").unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(path.parent(), Some(dir.as_path()), "{hostile}");
        }
        // Jamais d'écrasement : numérotation.
        let first = write_new_file(&dir, "r.pdf", b"1").unwrap_or_else(|e| panic!("{e}"));
        let second = write_new_file(&dir, "r.pdf", b"2").unwrap_or_else(|e| panic!("{e}"));
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap_or_default(), b"1");
        assert!(second.ends_with("r (2).pdf"));
        // Lien symbolique existant : jamais suivi.
        #[cfg(unix)]
        {
            let target = dir.join("cible");
            std::fs::write(&target, b"original").unwrap_or_else(|e| panic!("{e}"));
            let link = dir.join("lien.txt");
            std::os::unix::fs::symlink(&target, &link).unwrap_or_else(|e| panic!("{e}"));
            let written =
                write_new_file(&dir, "lien.txt", b"pirate").unwrap_or_else(|e| panic!("{e}"));
            assert_ne!(written, link);
            assert_eq!(std::fs::read(&target).unwrap_or_default(), b"original");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn written_files_carry_the_mark_of_the_web() {
        let dir = std::env::temp_dir().join(format!("norelly-motw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let path = write_new_file(&dir, "facture.doc", b"x").unwrap_or_else(|e| panic!("{e}"));
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        let mark = std::fs::read_to_string(PathBuf::from(stream)).unwrap_or_default();
        assert!(mark.contains("ZoneId=3"), "{mark}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
