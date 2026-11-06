//! Commandes IPC des actions sur les messages.
//!
//! L'interface désigne des conversations et la portée affichée ; les
//! messages visés sont choisis ici par le domaine
//! (`conversation_targets`), puis l'action est revalidée et appliquée par
//! le cas d'usage `PerformAction`. Rien n'est décidé côté TypeScript.

use std::collections::BTreeSet;

use norelly_application::account_actor::unix_now;
use norelly_application::actions::{PerformAction, UndoActions};
use norelly_domain::account::AccountId;
use norelly_domain::actions::{MAX_ACTION_TARGETS, MessageAction, conversation_targets};
use norelly_domain::mailbox::MailboxId;
use norelly_domain::ports::{ConversationId, PendingOpId, PendingOpStore};
use tauri::State;

use crate::ipc::{ActionDto, ActionResultDto, ErrorCodeDto, FieldDto, IpcError, ScopeDto, id32};
use crate::reading::scope_of;
use crate::state::AppState;

/// Action du domaine, identifiants revalidés.
fn action_of(dto: ActionDto) -> Result<MessageAction, IpcError> {
    Ok(match dto {
        ActionDto::MarkSeen { value } => MessageAction::MarkSeen(value),
        ActionDto::MarkFlagged { value } => MessageAction::MarkFlagged(value),
        ActionDto::Move { mailbox } => MessageAction::Move(
            MailboxId::new(i64::from(mailbox))
                .ok_or_else(|| IpcError::invalid(FieldDto::Mailbox))?,
        ),
        ActionDto::Delete => MessageAction::Delete,
        ActionDto::Archive => MessageAction::Archive,
    })
}

/// Applique une action aux conversations `conversations` affichées dans
/// `scope` : effet local immédiat, rejeu sur le serveur après la fenêtre
/// d'annulation. Renvoie les opérations (pour « annuler »).
#[tauri::command]
#[specta::specta]
pub async fn perform_action(
    state: State<'_, AppState>,
    conversations: Vec<u32>,
    scope: ScopeDto,
    action: ActionDto,
) -> Result<ActionResultDto, IpcError> {
    if conversations.len() > MAX_ACTION_TARGETS {
        return Err(IpcError::new(ErrorCodeDto::TooManyMessages));
    }
    let scope = scope_of(&scope)?;
    let action = action_of(action)?;
    let store = &*state.mail.store;
    let mut messages = BTreeSet::new();
    for raw in conversations {
        let id = ConversationId::new(i64::from(raw))
            .ok_or_else(|| IpcError::invalid(FieldDto::Conversation))?;
        let members = store
            .conversation_members(id)
            .await
            .map_err(IpcError::storage)?;
        messages.extend(conversation_targets(action, &members, &scope));
    }
    let messages: Vec<_> = messages.into_iter().collect();
    let report = PerformAction { store }
        .execute(&messages, action, unix_now())
        .await?;
    for account in &report.accounts {
        state.mail.replay_soon(*account).await;
    }
    Ok(ActionResultDto {
        ops: report
            .recorded
            .iter()
            .map(|op| id32(op.get()))
            .collect::<Result<_, _>>()?,
        messages: u32::try_from(messages.len()).unwrap_or(u32::MAX),
    })
}

/// Annule des opérations encore en attente (dernière action) ; renvoie le
/// nombre d'opérations annulées (0 si elles ont déjà été rejouées).
#[tauri::command]
#[specta::specta]
pub async fn undo_actions(state: State<'_, AppState>, ops: Vec<u32>) -> Result<u32, IpcError> {
    if ops.len() > MAX_ACTION_TARGETS {
        return Err(IpcError::new(ErrorCodeDto::TooManyMessages));
    }
    let ids = ops
        .into_iter()
        .map(|raw| {
            PendingOpId::new(i64::from(raw)).ok_or_else(|| IpcError::invalid(FieldDto::Message))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let undone = UndoActions {
        store: &*state.mail.store,
    }
    .execute(&ids)
    .await
    .map_err(IpcError::storage)?;
    Ok(u32::try_from(undone).unwrap_or(u32::MAX))
}

/// Dossier de repli de « archiver » pour un compte sans dossier
/// `\Archive` (`None` : aucun).
#[tauri::command]
#[specta::specta]
pub async fn set_archive_folder(
    state: State<'_, AppState>,
    account: u32,
    mailbox: Option<u32>,
) -> Result<(), IpcError> {
    let account =
        AccountId::new(i64::from(account)).map_err(|_| IpcError::invalid(FieldDto::Account))?;
    let mailbox = mailbox
        .map(|m| MailboxId::new(i64::from(m)).ok_or_else(|| IpcError::invalid(FieldDto::Mailbox)))
        .transpose()?;
    let store = &*state.mail.store;
    // Le dossier doit être un dossier sélectionnable du compte.
    if let Some(m) = mailbox {
        let folders = store
            .action_folders(account)
            .await
            .map_err(IpcError::storage)?;
        if !folders.iter().any(|f| f.id == m && f.selectable) {
            return Err(IpcError::invalid(FieldDto::Mailbox));
        }
    }
    store
        .set_archive_fallback(account, mailbox)
        .await
        .map_err(IpcError::storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_revalidated() {
        assert!(matches!(
            action_of(ActionDto::Move { mailbox: 0 }),
            Err(IpcError {
                code: ErrorCodeDto::InvalidInput,
                ..
            })
        ));
        assert_eq!(
            action_of(ActionDto::MarkSeen { value: true }).ok(),
            Some(MessageAction::MarkSeen(true))
        );
        assert!(action_of(ActionDto::Move { mailbox: 3 }).is_ok());
    }
}
