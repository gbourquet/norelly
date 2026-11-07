//! Commande IPC de recherche plein texte.
//!
//! La requête saisie est analysée par le domaine (`search::parse_query`) ;
//! la portée et le fuseau sont revalidés ici. Le surlignage est calculé en
//! Rust et transmis en morceaux de texte, que l'interface affiche sans
//! jamais les interpréter comme du HTML.

use norelly_application::contacts::NameSenders;
use norelly_domain::account::AccountId;
use norelly_domain::mailbox::MailboxId;
use norelly_domain::ports::{SearchIndex, SearchRequest, SearchScope};
use norelly_domain::search::{Segment, highlight, parse_query};
use tauri::State;

use crate::ipc::{
    CorrespondentDto, FieldDto, IpcError, SearchHitDto, SearchPageDto, SearchScopeDto, SegmentDto,
    date_dto, id32,
};
use crate::state::AppState;

/// Décalage horaire maximal accepté (UTC−14 à UTC+14), en minutes.
const MAX_UTC_OFFSET_MINUTES: i32 = 14 * 60;

fn scope_of(dto: &SearchScopeDto) -> Result<SearchScope, IpcError> {
    Ok(match dto {
        SearchScopeDto::All => SearchScope::All,
        SearchScopeDto::Account { id } => SearchScope::Account(
            AccountId::new(i64::from(*id)).map_err(|_| IpcError::invalid(FieldDto::Account))?,
        ),
        SearchScopeDto::Mailbox { id } => SearchScope::Mailbox(
            MailboxId::new(i64::from(*id)).ok_or_else(|| IpcError::invalid(FieldDto::Mailbox))?,
        ),
    })
}

fn segments(list: Vec<Segment>) -> Vec<SegmentDto> {
    list.into_iter()
        .map(|s| SegmentDto {
            text: s.text,
            hit: s.hit,
        })
        .collect()
}

/// Une page de messages trouvés, hors ligne (index local).
#[tauri::command]
#[specta::specta]
pub async fn search_messages(
    state: State<'_, AppState>,
    query: String,
    scope: SearchScopeDto,
    utc_offset_minutes: i32,
    offset: u32,
    limit: u32,
) -> Result<SearchPageDto, IpcError> {
    let query = parse_query(&query).map_err(|_| IpcError::invalid(FieldDto::Query))?;
    if utc_offset_minutes.abs() > MAX_UTC_OFFSET_MINUTES {
        return Err(IpcError::invalid(FieldDto::Query));
    }
    let scope = scope_of(&scope)?;
    let terms = query.highlight_terms();
    let page = state
        .mail
        .store
        .search(SearchRequest {
            query,
            scope,
            utc_offset_seconds: utc_offset_minutes * 60,
            offset,
            limit,
        })
        .await
        .map_err(IpcError::storage)?;
    let names = NameSenders {
        directory: &state.contacts,
    }
    .execute(page.items.iter().filter_map(|h| h.from.as_ref()))
    .await
    .map_err(IpcError::storage)?;
    let items = page
        .items
        .into_iter()
        .map(|h| {
            Ok(SearchHitDto {
                message: id32(h.id.get())?,
                conversation: h.conversation.map(|c| id32(c.get())).transpose()?,
                account: id32(h.account.get())?,
                mailbox: id32(h.mailbox.get())?,
                date: date_dto(h.date),
                from: h
                    .from
                    .as_ref()
                    .map(|f| CorrespondentDto::named(&names, f))
                    .transpose()?,
                subject: segments(highlight(&h.subject, &terms)),
                snippet: segments(highlight(&h.snippet, &terms)),
                unread: !h.flags.seen,
                flagged: h.flags.flagged,
                has_attachments: h.has_attachments,
            })
        })
        .collect::<Result<_, IpcError>>()?;
    Ok(SearchPageDto {
        items,
        next: page.next,
    })
}
