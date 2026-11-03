//! Commandes IPC de la rédaction (phase 7).
//!
//! Toute entrée de l'interface est revalidée ici par le domaine (adresses,
//! identifiants de contenus, sujet, tailles) ; le HTML de l'éditeur est
//! réassaini en Rust avant tout envoi (ADR-010). Les chemins des fichiers
//! joints ne viennent jamais de l'interface : sélecteur ouvert par Rust,
//! ou fichiers déposés sur la fenêtre (événement natif) désignés par un
//! jeton.

use std::path::PathBuf;

use norelly_application::account_actor::unix_now;
use norelly_application::compose::{ComposeFailure, PrepareReply, QueueMessage, SaveDraft};
use norelly_domain::compose::ReplyKind;
use norelly_domain::compose::draft::MAX_ATTACHMENTS_BYTES;
use norelly_domain::compose::outbox::OutboxEntry;
use norelly_domain::connection::server_text;
use norelly_domain::ports::{
    AccountRepository, BlobStore, ContactQuery, DraftStore, MessageId, OutboxStore,
};
use norelly_render::compose::MimeComposer;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::state::AppState;
use norelly_domain::account::AccountId;
use norelly_domain::address::EmailAddress;
use norelly_domain::compose::Threading;
use norelly_domain::compose::draft::{
    ComposeError, Draft, DraftAttachment, MAX_RECIPIENTS, Recipient,
};
use norelly_domain::compose::outbox::{OutboxState, SendFailure};
use norelly_domain::ports::{BlobId, DraftId, OutboxItem};

use crate::ipc::{
    ComposeKindDto, DraftAttachmentDto, DraftDto, ErrorCodeDto, FieldDto, IpcError, OutboxItemDto,
    OutboxStateDto, RecipientDto, SavedDraftDto, SendFailureDto, date_dto, id32,
};

fn recipients_of(list: &[RecipientDto]) -> Result<Vec<Recipient>, IpcError> {
    list.iter()
        .map(|r| {
            EmailAddress::parse(&r.address)
                .map(|a| Recipient::new(&r.name, a))
                .map_err(|_| IpcError::invalid(FieldDto::Address))
        })
        .collect()
}

fn recipients_dto(list: &[Recipient]) -> Vec<RecipientDto> {
    list.iter()
        .map(|r| RecipientDto {
            name: r.name.clone(),
            address: r.address.to_string(),
        })
        .collect()
}

/// Compte d'un identifiant de l'interface.
pub fn account_of(raw: u32) -> Result<AccountId, IpcError> {
    AccountId::new(i64::from(raw)).map_err(|_| IpcError::invalid(FieldDto::Account))
}

/// Brouillon d'un identifiant de l'interface.
pub fn draft_id_of(raw: u32) -> Result<DraftId, IpcError> {
    DraftId::new(i64::from(raw)).ok_or_else(|| IpcError::invalid(FieldDto::Draft))
}

/// Brouillon du domaine d'un DTO de l'éditeur, revalidé.
pub fn draft_of(dto: &DraftDto) -> Result<(Option<DraftId>, Draft), IpcError> {
    let id = dto.id.map(draft_id_of).transpose()?;
    let account = account_of(dto.account)?;
    if dto.to.len() + dto.cc.len() + dto.bcc.len() > MAX_RECIPIENTS {
        return Err(ComposeFailure::from(ComposeError::TooManyRecipients).into());
    }
    let mut draft = Draft::empty(account);
    draft.to = recipients_of(&dto.to)?;
    draft.cc = recipients_of(&dto.cc)?;
    draft.bcc = recipients_of(&dto.bcc)?;
    draft.subject.clone_from(&dto.subject);
    draft.html = (!dto.html.is_empty()).then(|| dto.html.clone());
    draft.threading = Threading {
        in_reply_to: dto.in_reply_to.clone(),
        references: dto.references.clone(),
    };
    draft.attachments = dto
        .attachments
        .iter()
        .map(|a| {
            Ok(DraftAttachment {
                blob: BlobId::parse(&a.blob)
                    .ok_or_else(|| IpcError::invalid(FieldDto::Attachment))?,
                file_name: a.file_name.clone(),
                content_type: a.content_type.clone(),
                size: u64::from(a.size),
            })
        })
        .collect::<Result<_, IpcError>>()?;
    Ok((id, draft))
}

/// DTO d'un brouillon.
pub fn draft_dto(id: Option<DraftId>, draft: &Draft) -> Result<DraftDto, IpcError> {
    Ok(DraftDto {
        id: id.map(|i| id32(i.get())).transpose()?,
        account: id32(draft.account.get())?,
        to: recipients_dto(&draft.to),
        cc: recipients_dto(&draft.cc),
        bcc: recipients_dto(&draft.bcc),
        subject: draft.subject.clone(),
        html: draft.html.clone().unwrap_or_default(),
        in_reply_to: draft.threading.in_reply_to.clone(),
        references: draft.threading.references.clone(),
        attachments: draft
            .attachments
            .iter()
            .map(|a| DraftAttachmentDto {
                blob: a.blob.as_str().to_owned(),
                file_name: a.file_name.clone(),
                content_type: a.content_type.clone(),
                size: u32::try_from(a.size).unwrap_or(u32::MAX),
            })
            .collect(),
    })
}

const fn failure_dto(f: SendFailure) -> SendFailureDto {
    match f {
        SendFailure::Unreachable => SendFailureDto::Unreachable,
        SendFailure::Temporary => SendFailureDto::Temporary,
        SendFailure::Rejected => SendFailureDto::Rejected,
        SendFailure::Authentication => SendFailureDto::Authentication,
        SendFailure::Security => SendFailureDto::Security,
        SendFailure::Uncertain => SendFailureDto::Uncertain,
        SendFailure::TooManyAttempts => SendFailureDto::TooManyAttempts,
    }
}

/// DTO d'un message de la file.
pub fn outbox_dto(item: &OutboxItem) -> Result<OutboxItemDto, IpcError> {
    Ok(OutboxItemDto {
        id: id32(item.id.get())?,
        subject: item.subject.clone(),
        recipients: item.message.recipients.clone(),
        state: match item.entry.state {
            OutboxState::Queued { not_before } => OutboxStateDto::Queued {
                not_before: date_dto(not_before),
            },
            OutboxState::Sending => OutboxStateDto::Sending,
            OutboxState::Sent => OutboxStateDto::Sent,
            OutboxState::Failed(f) => OutboxStateDto::Failed {
                failure: failure_dto(f),
            },
        },
        attempts: item.entry.attempts,
        queued_at: date_dto(item.queued_at),
    })
}

impl From<ComposeFailure> for IpcError {
    fn from(e: ComposeFailure) -> Self {
        match e {
            ComposeFailure::Invalid(e) => match e {
                ComposeError::NoRecipient => Self::new(ErrorCodeDto::NoRecipient),
                ComposeError::TooManyRecipients => Self::new(ErrorCodeDto::TooManyRecipients),
                ComposeError::InvalidSubject => Self::new(ErrorCodeDto::InvalidSubject),
                ComposeError::BodyTooLarge => Self::new(ErrorCodeDto::BodyTooLarge),
                ComposeError::AttachmentsTooLarge => Self::new(ErrorCodeDto::AttachmentsTooLarge),
                ComposeError::InvalidThreading => Self::invalid(FieldDto::Message),
            },
            ComposeFailure::Store(e) => Self::storage(e),
            ComposeFailure::Build(e) => Self::internal(e),
            ComposeFailure::NotFound => Self::not_found(FieldDto::Message),
        }
    }
}

/// Type MIME d'un fichier d'après son extension (sinon
/// `application/octet-stream`).
#[must_use]
pub fn content_type_of(name: &str) -> &'static str {
    let ext = std::path::Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "zip" => "application/zip",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

/// Lit au plus `limit` octets d'un fichier ; `None` s'il est plus long
/// (même s'il a grossi depuis `metadata`, ou si sa taille annoncée est
/// fausse, comme celle d'un pseudo-fichier).
pub fn read_bounded(path: &std::path::Path, limit: u64) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    Ok((u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= limit).then_some(bytes))
}

/// Suggestions d'adresses demandées par l'éditeur.
const SUGGESTIONS: usize = 8;

/// Adresses de tous les comptes (jamais reprises dans une réponse).
async fn own_addresses(state: &AppState) -> Result<Vec<EmailAddress>, IpcError> {
    Ok(state
        .accounts
        .list()
        .await
        .map_err(IpcError::storage)?
        .iter()
        .map(|a| a.address().clone())
        .collect())
}

/// Brouillon de départ : nouveau message, réponse, réponse à tous ou
/// transfert de `message`. `attribution` : en-tête de citation construit
/// par l'interface dans la langue de l'utilisateur (ADR-011), nettoyé ici.
#[tauri::command]
#[specta::specta]
pub async fn prepare_compose(
    state: State<'_, AppState>,
    account: u32,
    kind: ComposeKindDto,
    message: Option<u32>,
    attribution: String,
) -> Result<DraftDto, IpcError> {
    let kind = match kind {
        ComposeKindDto::New => return draft_dto(None, &Draft::empty(account_of(account)?)),
        ComposeKindDto::Reply => ReplyKind::Reply,
        ComposeKindDto::ReplyAll => ReplyKind::ReplyAll,
        ComposeKindDto::Forward => ReplyKind::Forward,
    };
    let message = message
        .and_then(|m| MessageId::new(i64::from(m)))
        .ok_or_else(|| IpcError::invalid(FieldDto::Message))?;
    let own = own_addresses(&state).await?;
    let draft = PrepareReply {
        locator: &*state.mail.store,
        blobs: &*state.mail.blobs,
        composer: &MimeComposer,
    }
    .execute(message, kind, &own, &server_text(attribution.as_bytes()))
    .await?;
    draft_dto(None, &draft)
}

/// Enregistre un brouillon (même incomplet).
#[tauri::command]
#[specta::specta]
pub async fn save_draft(
    state: State<'_, AppState>,
    draft: DraftDto,
) -> Result<SavedDraftDto, IpcError> {
    let (id, draft) = draft_of(&draft)?;
    let saved = SaveDraft {
        drafts: &*state.mail.compose,
    }
    .execute(id, &draft, unix_now())
    .await?;
    state.mail.compose_soon(draft.account).await;
    Ok(SavedDraftDto {
        id: id32(saved.id.get())?,
        revision: u32::try_from(saved.sync.revision).unwrap_or(u32::MAX),
    })
}

/// Met le message en file d'envoi (le brouillon enregistré est retiré) ;
/// renvoie son identifiant dans la file.
#[tauri::command]
#[specta::specta]
pub async fn send_draft(state: State<'_, AppState>, draft: DraftDto) -> Result<u32, IpcError> {
    let (id, draft) = draft_of(&draft)?;
    let account = state
        .accounts
        .get(draft.account)
        .await
        .map_err(IpcError::storage)?
        .ok_or_else(|| IpcError::not_found(FieldDto::Account))?;
    let from = Recipient::new(account.display_name().as_str(), account.address().clone());
    let queued = QueueMessage {
        store: &*state.mail.compose,
        blobs: &*state.mail.blobs,
        composer: &MimeComposer,
    }
    .execute(id, &draft, &from, unix_now())
    .await?;
    state.mail.compose_soon(draft.account).await;
    id32(queued.get())
}

/// Abandonne un brouillon (sa copie sur le serveur sera supprimée).
#[tauri::command]
#[specta::specta]
pub async fn discard_draft(state: State<'_, AppState>, id: u32) -> Result<(), IpcError> {
    let removed = state
        .mail
        .compose
        .delete_draft(draft_id_of(id)?)
        .await
        .map_err(IpcError::storage)?;
    if let Some(removed) = removed {
        state.mail.compose_soon(removed.draft.account).await;
    }
    Ok(())
}

/// File d'envoi d'un compte.
#[tauri::command]
#[specta::specta]
pub async fn outbox(
    state: State<'_, AppState>,
    account: u32,
) -> Result<Vec<OutboxItemDto>, IpcError> {
    state
        .mail
        .compose
        .outbox(account_of(account)?)
        .await
        .map_err(IpcError::storage)?
        .iter()
        .map(outbox_dto)
        .collect()
}

async fn outbox_item(
    state: &AppState,
    account: AccountId,
    id: u32,
) -> Result<OutboxItem, IpcError> {
    state
        .mail
        .compose
        .outbox(account)
        .await
        .map_err(IpcError::storage)?
        .into_iter()
        .find(|i| i.id.get() == i64::from(id))
        .ok_or_else(|| IpcError::not_found(FieldDto::Outbox))
}

/// Relance un envoi en échec (décision de l'utilisateur, y compris pour un
/// envoi incertain).
#[tauri::command]
#[specta::specta]
pub async fn retry_send(state: State<'_, AppState>, account: u32, id: u32) -> Result<(), IpcError> {
    let account = account_of(account)?;
    let item = outbox_item(&state, account, id).await?;
    let next = item
        .entry
        .retry(unix_now())
        .map_err(|_| IpcError::new(ErrorCodeDto::OutboxBusy))?;
    if !state
        .mail
        .compose
        .transition(item.id, item.entry, next)
        .await
        .map_err(IpcError::storage)?
    {
        return Err(IpcError::new(ErrorCodeDto::OutboxBusy));
    }
    state.mail.compose_soon(account).await;
    Ok(())
}

/// Retire un message de la file, seulement s'il n'est pas en cours
/// d'envoi : en attente, ou en échec.
#[tauri::command]
#[specta::specta]
pub async fn cancel_send(
    state: State<'_, AppState>,
    account: u32,
    id: u32,
) -> Result<(), IpcError> {
    let account = account_of(account)?;
    let item = outbox_item(&state, account, id).await?;
    if !matches!(
        item.entry.state,
        OutboxState::Queued { .. } | OutboxState::Failed(_)
    ) {
        return Err(IpcError::new(ErrorCodeDto::OutboxBusy));
    }
    // Réservé d'abord (comparaison et échange) : le travailleur ne peut plus
    // le prendre entre la vérification et le retrait.
    let held = OutboxEntry {
        state: OutboxState::Failed(SendFailure::Rejected),
        attempts: item.entry.attempts,
    };
    if item.entry != held
        && !state
            .mail
            .compose
            .transition(item.id, item.entry, held)
            .await
            .map_err(IpcError::storage)?
    {
        return Err(IpcError::new(ErrorCodeDto::OutboxBusy));
    }
    state
        .mail
        .compose
        .remove(item.id)
        .await
        .map_err(IpcError::storage)?;
    Ok(())
}

/// Adresses connues correspondant à la saisie.
#[tauri::command]
#[specta::specta]
pub async fn suggest_contacts(
    state: State<'_, AppState>,
    account: u32,
    text: String,
) -> Result<Vec<RecipientDto>, IpcError> {
    let found = state
        .mail
        .store
        .suggest(account_of(account)?, &text, SUGGESTIONS)
        .await
        .map_err(IpcError::storage)?;
    Ok(found
        .into_iter()
        .map(|c| RecipientDto {
            name: c.name,
            address: c.address,
        })
        .collect())
}

/// Lit des fichiers choisis (chemins venus du sélecteur ou de l'événement
/// de dépôt, jamais de l'interface) et les range dans le stockage.
async fn store_files(
    state: &AppState,
    account: AccountId,
    paths: Vec<PathBuf>,
) -> Result<Vec<DraftAttachmentDto>, IpcError> {
    let files = tauri::async_runtime::spawn_blocking(move || {
        let mut total = 0_u64;
        let mut files = Vec::new();
        for path in paths {
            let meta = std::fs::metadata(&path)
                .map_err(|e| IpcError::new(ErrorCodeDto::OpenFailed).with_detail(e))?;
            if !meta.is_file() {
                continue;
            }
            let remaining = MAX_ATTACHMENTS_BYTES.saturating_sub(total);
            let bytes = read_bounded(&path, remaining)
                .map_err(|e| IpcError::new(ErrorCodeDto::OpenFailed).with_detail(e))?
                .ok_or_else(|| {
                    IpcError::from(ComposeFailure::from(ComposeError::AttachmentsTooLarge))
                })?;
            total = total.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            files.push((name, bytes));
        }
        Ok::<_, IpcError>(files)
    })
    .await
    .map_err(IpcError::internal)??;
    let mut out = Vec::with_capacity(files.len());
    for (name, bytes) in files {
        let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        let blob = state
            .mail
            .blobs
            .put(account, bytes)
            .await
            .map_err(IpcError::storage)?;
        out.push(DraftAttachmentDto {
            blob: blob.as_str().to_owned(),
            content_type: content_type_of(&name).to_owned(),
            file_name: name,
            size,
        });
    }
    Ok(out)
}

/// Joint des fichiers choisis dans le sélecteur du système (titre
/// localisé par l'interface) ; liste vide si l'utilisateur annule.
#[tauri::command]
#[specta::specta]
pub async fn pick_attachments(
    app: AppHandle,
    state: State<'_, AppState>,
    account: u32,
    title: String,
) -> Result<Vec<DraftAttachmentDto>, IpcError> {
    let account = account_of(account)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title(server_text(title.as_bytes()))
        .pick_files(move |files| {
            let _ = tx.send(files);
        });
    let Some(files) = rx.await.ok().flatten() else {
        return Ok(Vec::new());
    };
    let paths = files
        .into_iter()
        .filter_map(|f| f.into_path().ok())
        .collect();
    store_files(&state, account, paths).await
}

/// Joint les fichiers déposés sur la fenêtre, désignés par leur jeton
/// (événement `files_dropped`).
#[tauri::command]
#[specta::specta]
pub async fn attach_dropped(
    state: State<'_, AppState>,
    account: u32,
    token: u32,
) -> Result<Vec<DraftAttachmentDto>, IpcError> {
    let account = account_of(account)?;
    let paths = state
        .mail
        .take_dropped(token)
        .ok_or_else(|| IpcError::not_found(FieldDto::Attachment))?;
    store_files(&state, account, paths).await
}

#[cfg(test)]
mod tests {
    use norelly_domain::ports::{OutboxId, OutgoingMessage, StoreError};

    use super::*;

    fn dto() -> DraftDto {
        DraftDto {
            id: Some(4),
            account: 2,
            to: vec![RecipientDto {
                name: "Bob\r\nBcc: x@y".into(),
                address: " bob@Example.COM ".into(),
            }],
            cc: vec![],
            bcc: vec![RecipientDto {
                name: String::new(),
                address: "cache@example.com".into(),
            }],
            subject: "Bonjour".into(),
            html: "<p>Salut</p>".into(),
            in_reply_to: Some("m1@example.com".into()),
            references: vec!["m0@example.com".into(), "m1@example.com".into()],
            attachments: vec![DraftAttachmentDto {
                blob: "ab".repeat(32),
                file_name: "a.pdf".into(),
                content_type: "application/pdf".into(),
                size: 12,
            }],
        }
    }

    fn code(r: Result<(Option<DraftId>, Draft), IpcError>) -> (ErrorCodeDto, Option<FieldDto>) {
        match r {
            Err(e) => (e.code, e.field),
            Ok(_) => panic!("refus attendu"),
        }
    }

    #[test]
    fn drafts_are_revalidated_and_round_trip() {
        let (id, draft) = draft_of(&dto()).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(id.map(DraftId::get), Some(4));
        assert_eq!(draft.account.get(), 2);
        assert_eq!(draft.to[0].address.to_string(), "bob@example.com");
        assert!(!draft.to[0].name.contains(['\r', '\n']), "nom assaini");
        assert_eq!(draft.html.as_deref(), Some("<p>Salut</p>"));
        assert_eq!(draft.threading.references.len(), 2);
        assert_eq!(draft.attachments[0].size, 12);
        let back = draft_dto(id, &draft).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(back.to[0].address, "bob@example.com");
        assert_eq!(back.attachments, dto().attachments);
        assert_eq!((back.id, back.account), (Some(4), 2));
    }

    #[test]
    fn invalid_editor_input_is_refused() {
        let mut d = dto();
        d.to[0].address = "pas une adresse".into();
        assert_eq!(
            code(draft_of(&d)),
            (ErrorCodeDto::InvalidInput, Some(FieldDto::Address))
        );
        let mut d = dto();
        d.attachments[0].blob = "../../etc/passwd".into();
        assert_eq!(
            code(draft_of(&d)),
            (ErrorCodeDto::InvalidInput, Some(FieldDto::Attachment))
        );
        let mut d = dto();
        d.account = 0;
        assert_eq!(
            code(draft_of(&d)),
            (ErrorCodeDto::InvalidInput, Some(FieldDto::Account))
        );
        let mut d = dto();
        d.id = Some(0);
        assert_eq!(
            code(draft_of(&d)),
            (ErrorCodeDto::InvalidInput, Some(FieldDto::Draft))
        );
        let mut d = dto();
        d.to = (0..1000)
            .map(|n| RecipientDto {
                name: String::new(),
                address: format!("a{n}@example.com"),
            })
            .collect();
        assert_eq!(code(draft_of(&d)).0, ErrorCodeDto::TooManyRecipients);
    }

    #[test]
    fn compose_failures_have_their_codes() {
        let c = |e: ComposeFailure| IpcError::from(e).code;
        assert_eq!(
            c(ComposeError::NoRecipient.into()),
            ErrorCodeDto::NoRecipient
        );
        assert_eq!(
            c(ComposeError::TooManyRecipients.into()),
            ErrorCodeDto::TooManyRecipients
        );
        assert_eq!(
            c(ComposeError::InvalidSubject.into()),
            ErrorCodeDto::InvalidSubject
        );
        assert_eq!(
            c(ComposeError::BodyTooLarge.into()),
            ErrorCodeDto::BodyTooLarge
        );
        assert_eq!(
            c(ComposeError::AttachmentsTooLarge.into()),
            ErrorCodeDto::AttachmentsTooLarge
        );
        assert_eq!(c(ComposeFailure::NotFound), ErrorCodeDto::NotFound);
        assert_eq!(c(StoreError("x".into()).into()), ErrorCodeDto::Storage);
    }

    #[test]
    fn attached_files_are_read_within_the_limit() {
        let dir = std::env::temp_dir().join(format!("norelly-borne-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("f.bin");
        std::fs::write(&path, b"0123456789").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            read_bounded(&path, 10).unwrap_or_else(|e| panic!("{e}")),
            Some(b"0123456789".to_vec())
        );
        assert_eq!(
            read_bounded(&path, 9).unwrap_or_else(|e| panic!("{e}")),
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn content_types_follow_extensions() {
        assert_eq!(content_type_of("Rapport.PDF"), "application/pdf");
        assert_eq!(content_type_of("photo.jpeg"), "image/jpeg");
        assert_eq!(
            content_type_of("sans-extension"),
            "application/octet-stream"
        );
        assert_eq!(content_type_of("piège.exe"), "application/octet-stream");
    }

    #[test]
    fn outbox_items_are_projected() {
        let item = |state| OutboxItem {
            id: OutboxId::new(7).unwrap_or_else(|| panic!("id")),
            account: AccountId::new(1).unwrap_or_else(|_| panic!("compte")),
            message: OutgoingMessage {
                from: "moi@example.com".into(),
                recipients: vec!["bob@example.com".into()],
                raw: vec![],
            },
            sent_copy: vec![],
            subject: "Sujet".into(),
            entry: OutboxEntry { state, attempts: 2 },
            queued_at: 100,
        };
        let dto = outbox_dto(&item(OutboxState::Failed(SendFailure::Uncertain)))
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            dto,
            OutboxItemDto {
                id: 7,
                subject: "Sujet".into(),
                recipients: vec!["bob@example.com".into()],
                state: OutboxStateDto::Failed {
                    failure: SendFailureDto::Uncertain
                },
                attempts: 2,
                queued_at: 100,
            }
        );
        let queued = outbox_dto(&item(OutboxState::Queued { not_before: -5 }))
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(queued.state, OutboxStateDto::Queued { not_before: 0 });
    }
}
