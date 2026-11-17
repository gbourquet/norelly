//! Commandes IPC des étiquettes : liste d'un compte, menu « Étiqueter… »
//! d'une sélection, création, couleur, renommage et suppression. Poser ou
//! retirer une étiquette est une action (`perform_action`).
//!
//! Toute entrée est revalidée par le domaine : nom (`LabelName`), mot-clé
//! (`Keyword`), couleur (`LabelColor`, `#rrggbb` seulement). Les noms, les
//! cases cochées et les refus du serveur sont décidés par le domaine.

use std::collections::{BTreeMap, BTreeSet};

use norelly_application::labels::{
    CreateLabel, DeleteLabel, LabelMenuFor, ListLabels, RenameLabel, SetLabelColor,
};
use norelly_domain::account::AccountId;
use norelly_domain::actions::MAX_ACTION_TARGETS;
use norelly_domain::labels::{LabelColor, LabelError, LabelName, visible_labels};
use norelly_domain::mailbox::Keyword;
use norelly_domain::ports::ConversationId;
use tauri::State;

use crate::ipc::{
    ErrorCodeDto, FieldDto, IpcError, LabelChoiceDto, LabelDto, LabelMenuDto, ScopeDto, id32,
};
use crate::reading::scope_of;
use crate::state::AppState;

fn account_of(raw: u32) -> Result<AccountId, IpcError> {
    AccountId::new(i64::from(raw)).map_err(|_| IpcError::invalid(FieldDto::Account))
}

/// Mot-clé reçu de l'interface, revalidé.
pub(crate) fn keyword_of(raw: &str) -> Result<Keyword, IpcError> {
    Keyword::parse(raw).ok_or_else(|| IpcError::invalid(FieldDto::Label))
}

fn color_of(raw: Option<&str>) -> Result<Option<LabelColor>, IpcError> {
    raw.map(|c| LabelColor::parse(c).ok_or_else(|| IpcError::invalid(FieldDto::LabelColor)))
        .transpose()
}

fn name_of(raw: &str) -> Result<LabelName, IpcError> {
    LabelName::parse(raw).map_err(IpcError::from)
}

/// Étiquette `keyword` du compte, telle que listée.
async fn label_dto(
    state: &AppState,
    account: AccountId,
    keyword: &Keyword,
) -> Result<LabelDto, IpcError> {
    ListLabels {
        store: &*state.mail.store,
    }
    .execute(account)
    .await
    .map_err(IpcError::storage)?
    .iter()
    .find(|l| &l.keyword == keyword)
    .map(LabelDto::from)
    .ok_or_else(|| IpcError::not_found(FieldDto::Label))
}

/// Mots-clés masqués de chaque compte (étiquettes supprimées de la liste,
/// anciens noms en cours de réécriture), pour filtrer les pastilles.
pub(crate) struct HiddenLabels {
    by_account: BTreeMap<AccountId, BTreeSet<Keyword>>,
}

impl HiddenLabels {
    pub(crate) async fn of(
        state: &AppState,
        accounts: BTreeSet<AccountId>,
    ) -> Result<Self, IpcError> {
        let list = ListLabels {
            store: &*state.mail.store,
        };
        let mut by_account = BTreeMap::new();
        for account in accounts {
            if let std::collections::btree_map::Entry::Vacant(e) = by_account.entry(account) {
                e.insert(list.hidden(account).await.map_err(IpcError::storage)?);
            }
        }
        Ok(Self { by_account })
    }

    /// Étiquettes à afficher parmi `keywords` (décision du domaine).
    pub(crate) fn visible(&self, account: AccountId, keywords: &BTreeSet<Keyword>) -> Vec<String> {
        let none = BTreeSet::new();
        let hidden = self.by_account.get(&account).unwrap_or(&none);
        visible_labels(keywords, hidden)
            .iter()
            .map(|k| k.as_str().to_owned())
            .collect()
    }
}

/// Étiquettes d'un compte (réglages, pastilles).
#[tauri::command]
#[specta::specta]
pub async fn list_labels(
    state: State<'_, AppState>,
    account: u32,
) -> Result<Vec<LabelDto>, IpcError> {
    let account = account_of(account)?;
    Ok(ListLabels {
        store: &*state.mail.store,
    }
    .execute(account)
    .await
    .map_err(IpcError::storage)?
    .iter()
    .map(LabelDto::from)
    .collect())
}

/// Menu « Étiqueter… » des conversations `conversations` affichées dans
/// `scope` : étiquettes cochées si tous les messages visés les portent,
/// désactivées si le serveur les refuse.
#[tauri::command]
#[specta::specta]
pub async fn label_menu(
    state: State<'_, AppState>,
    conversations: Vec<u32>,
    scope: ScopeDto,
) -> Result<LabelMenuDto, IpcError> {
    if conversations.len() > MAX_ACTION_TARGETS {
        return Err(IpcError::new(ErrorCodeDto::TooManyMessages));
    }
    let scope = scope_of(&scope)?;
    let ids = conversations
        .into_iter()
        .map(|raw| {
            ConversationId::new(i64::from(raw))
                .ok_or_else(|| IpcError::invalid(FieldDto::Conversation))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let selection = LabelMenuFor {
        store: &*state.mail.store,
    }
    .execute(&ids, &scope)
    .await
    .map_err(IpcError::storage)?;
    let available = selection.menu.is_available();
    let choices = selection
        .menu
        .choices
        .iter()
        .filter_map(|c| {
            let label = selection.labels.iter().find(|l| l.keyword == c.keyword)?;
            Some(LabelChoiceDto {
                label: LabelDto::from(label),
                checked: c.checked,
                allowed: c.allowed,
            })
        })
        .collect();
    Ok(LabelMenuDto {
        choices,
        can_create: selection.menu.can_create,
        available,
        accounts: selection
            .accounts
            .iter()
            .map(|a| id32(a.get()))
            .collect::<Result<_, _>>()?,
    })
}

/// Crée une étiquette (réglages, ou « Nouvelle étiquette » du menu).
#[tauri::command]
#[specta::specta]
pub async fn create_label(
    state: State<'_, AppState>,
    account: u32,
    name: String,
    color: Option<String>,
) -> Result<LabelDto, IpcError> {
    let account = account_of(account)?;
    let name = name_of(&name)?;
    let color = color_of(color.as_deref())?;
    let keyword = CreateLabel {
        store: &*state.mail.store,
    }
    .execute(account, &name, color)
    .await?;
    label_dto(&state, account, &keyword).await
}

/// Couleur d'une étiquette (`None` : pastille neutre).
#[tauri::command]
#[specta::specta]
pub async fn set_label_color(
    state: State<'_, AppState>,
    account: u32,
    keyword: String,
    color: Option<String>,
) -> Result<(), IpcError> {
    let account = account_of(account)?;
    let keyword = keyword_of(&keyword)?;
    let color = color_of(color.as_deref())?;
    SetLabelColor {
        store: &*state.mail.store,
    }
    .execute(account, &keyword, color)
    .await?;
    Ok(())
}

/// Renomme une étiquette : tout de suite en local, puis sur tous les
/// messages du serveur (réécriture rejouée par l'acteur du compte).
#[tauri::command]
#[specta::specta]
pub async fn rename_label(
    state: State<'_, AppState>,
    account: u32,
    keyword: String,
    name: String,
) -> Result<LabelDto, IpcError> {
    let account = account_of(account)?;
    let keyword = keyword_of(&keyword)?;
    let name = name_of(&name)?;
    let renamed = match (RenameLabel {
        store: &*state.mail.store,
    })
    .execute(account, &keyword, &name)
    .await
    {
        Ok(renamed) => renamed,
        // Même nom : rien à faire.
        Err(norelly_application::labels::LabelFailure::Invalid(LabelError::Unchanged)) => keyword,
        Err(e) => return Err(e.into()),
    };
    state.mail.replay_soon(account).await;
    label_dto(&state, account, &renamed).await
}

/// Supprime une étiquette de la liste ; avec `remove_from_messages`, la
/// retire aussi de tous les messages (en local, puis sur le serveur).
#[tauri::command]
#[specta::specta]
pub async fn delete_label(
    state: State<'_, AppState>,
    account: u32,
    keyword: String,
    remove_from_messages: bool,
) -> Result<(), IpcError> {
    let account = account_of(account)?;
    let keyword = keyword_of(&keyword)?;
    DeleteLabel {
        store: &*state.mail.store,
    }
    .execute(account, &keyword, remove_from_messages)
    .await?;
    if remove_from_messages {
        state.mail.replay_soon(account).await;
    }
    Ok(())
}
