//! Recherche plein texte (ADR-003) : FTS5 à contenu externe
//! (`message_texts`, migration 7), tenu à jour par des déclencheurs dans la
//! transaction de chaque écriture.
//!
//! Traduction de la requête analysée par le domaine
//! ([`norelly_domain::search`]) :
//! - chaque clause de texte devient une chaîne FTS5 entre guillemets (les
//!   guillemets du texte doublés) : le moteur la découpe en mots comme le
//!   texte indexé et la cherche comme une suite de mots consécutifs, en
//!   préfixe (`*`) sauf pour une phrase. Aucun mot de l'utilisateur ne peut
//!   donc être lu comme un opérateur FTS5 (`OR`, `NEAR`, `col:`, `^`…) ;
//! - l'expression FTS5 et toutes les valeurs passent en paramètres liés ;
//!   le texte SQL n'est assemblé qu'à partir de fragments constants ;
//! - les clauses de texte niées deviennent des `NOT IN` (FTS5 ne sait pas
//!   chercher une négation seule), les filtres des conditions SQL.

use norelly_domain::account::AccountId;
use norelly_domain::mailbox::{Correspondent, MAX_SUBJECT_CHARS, header_text};
use norelly_domain::ports::{
    ConversationId, MAX_PAGE_SIZE, MAX_SEARCH_RESULTS, MAX_SNIPPET_CHARS, MessageId, SearchHit,
    SearchIndex, SearchPage, SearchRequest, SearchScope, StoreError,
};
use norelly_domain::search::{Atom, Field, Filter, tokens};
use rusqlite::Connection;
use rusqlite::types::Value;

use crate::mail::{SqliteMailStore, corrupted, flags_from, mailbox_id, uid_of};
use crate::store_error;

/// Messages visibles dans les listes.
const VISIBLE: &str = "m.hidden = 0 AND (m.flags & 8) = 0";

/// Un message de `\All` qui a une copie visible (même `Message-ID`) dans
/// un autre dossier du compte n'est pas listé deux fois.
const NOT_DUPLICATE: &str = "NOT (b.role = 'all' AND m.message_id IS NOT NULL AND EXISTS ( \
     SELECT 1 FROM messages d JOIN mailboxes db ON db.id = d.mailbox_id \
     WHERE d.conversation_id = m.conversation_id AND d.message_id = m.message_id \
     AND db.account_id = b.account_id AND db.role <> 'all' \
     AND d.hidden = 0 AND (d.flags & 8) = 0))";

/// Poids des colonnes (sujet, expéditeur, destinataires, texte) dans le
/// classement BM25.
const RANKING: &str = "bm25(message_search, 10.0, 5.0, 3.0, 1.0)";

/// Chaîne FTS5 d'une clause de texte : `col : "texte" *`.
fn fts_term(field: Field, text: &str, phrase: bool) -> String {
    let column = match field {
        Field::Any => "",
        Field::From => "sender : ",
        Field::To => "recipients : ",
        Field::Subject => "subject : ",
    };
    let quoted = text.replace('"', "\"\"");
    // Préfixe d'une lettre : non indexé (`prefix = '2 3'`), coûteux ;
    // le mot est alors cherché tel quel.
    let short = tokens(text).last().is_none_or(|w| w.chars().count() < 2);
    let prefix = if phrase || short { "" } else { " *" };
    format!("{column}\"{quoted}\"{prefix}")
}

/// Requête SQL en construction : fragments constants et paramètres liés,
/// dans l'ordre des `?`.
#[derive(Default)]
struct Sql {
    text: Vec<&'static str>,
    params: Vec<Value>,
}

impl Sql {
    fn push(&mut self, fragment: &'static str) {
        self.text.push(fragment);
    }

    fn bind(&mut self, fragment: &'static str, value: impl Into<Value>) {
        self.text.push(fragment);
        self.params.push(value.into());
    }
}

/// Ligne lue : colonnes du message puis l'extrait (passage du texte).
type HitRow = (
    i64,
    i64,
    i64,
    Option<i64>,
    i64,
    i64,
    String,
    String,
    String,
    i64,
    String,
    bool,
    Option<String>,
);

fn hit(row: HitRow) -> Result<SearchHit, StoreError> {
    let (
        id,
        account,
        mailbox,
        conversation,
        uid,
        date,
        from_name,
        from_address,
        subject,
        bits,
        keywords,
        attachments,
        snippet,
    ) = row;
    let from = (!from_name.is_empty() || !from_address.is_empty())
        .then(|| Correspondent::new(&from_name, &from_address));
    Ok(SearchHit {
        id: MessageId::new(id).ok_or_else(|| corrupted("identifiant de message"))?,
        account: AccountId::new(account).map_err(|_| corrupted("compte"))?,
        mailbox: mailbox_id(mailbox)?,
        conversation: conversation.and_then(ConversationId::new),
        uid: uid_of(uid)?,
        date,
        from,
        subject: header_text(&subject, MAX_SUBJECT_CHARS),
        snippet: snippet_text(snippet.as_deref().unwrap_or_default()),
        flags: flags_from(bits, &keywords),
        has_attachments: attachments,
    })
}

/// Passage affichable : nettoyé comme un sujet (contrôles et caractères
/// de mise en forme retirés, blancs réduits), au plus
/// [`MAX_SNIPPET_CHARS`] caractères.
fn snippet_text(raw: &str) -> String {
    header_text(raw, MAX_SNIPPET_CHARS)
}

fn search_query(c: &Connection, request: &SearchRequest) -> Result<SearchPage, StoreError> {
    let limit = request.limit.min(MAX_PAGE_SIZE);
    if limit == 0 || request.offset >= MAX_SEARCH_RESULTS {
        return Ok(SearchPage::default());
    }
    let take = limit.min(MAX_SEARCH_RESULTS - request.offset);

    // Clauses effectives : une clause de texte sans mot est ignorée.
    let mut positive: Vec<String> = Vec::new();
    let mut negative: Vec<String> = Vec::new();
    let mut filters: Vec<(bool, Filter)> = Vec::new();
    for clause in request.query.clauses() {
        match &clause.atom {
            Atom::Text {
                field,
                text,
                phrase,
            } => {
                if tokens(text).is_empty() {
                    continue;
                }
                let term = fts_term(*field, text, *phrase);
                if clause.negated {
                    negative.push(term);
                } else {
                    positive.push(term);
                }
            }
            Atom::Filter(filter) => filters.push((clause.negated, *filter)),
        }
    }
    if positive.is_empty() && negative.is_empty() && filters.is_empty() {
        return Ok(SearchPage::default());
    }
    let ranked = !positive.is_empty();

    let mut sql = Sql::default();
    if ranked {
        sql.push(SELECT_RANKED);
        sql.bind("WHERE message_search MATCH ?", positive.join(" AND "));
    } else {
        sql.push(SELECT_BY_DATE);
        sql.push("WHERE 1 = 1");
    }
    sql.push("AND");
    sql.push(VISIBLE);
    match request.scope {
        SearchScope::All => {
            sql.push("AND");
            sql.push(NOT_DUPLICATE);
        }
        SearchScope::Account(account) => {
            sql.bind("AND b.account_id = ?", account.get());
            sql.push("AND");
            sql.push(NOT_DUPLICATE);
        }
        SearchScope::Mailbox(mailbox) => sql.bind("AND m.mailbox_id = ?", mailbox.get()),
    }
    for term in negative {
        sql.bind(
            "AND m.id NOT IN (SELECT rowid FROM message_search WHERE message_search MATCH ?)",
            term,
        );
    }
    for (negated, filter) in filters {
        sql.push(if negated { "AND NOT" } else { "AND" });
        let offset = request.utc_offset_seconds;
        match filter {
            Filter::HasAttachment => sql.push("(m.has_attachments = 1)"),
            Filter::Unread => sql.push("((m.flags & 1) = 0)"),
            Filter::Read => sql.push("((m.flags & 1) = 1)"),
            Filter::Flagged => sql.push("((m.flags & 4) = 4)"),
            Filter::Before(day) => sql.bind("(m.sort_date < ?)", day.start_timestamp(offset)),
            Filter::After(day) => sql.bind("(m.sort_date >= ?)", day.start_timestamp(offset)),
        }
    }
    if ranked {
        sql.push("ORDER BY");
        sql.push(RANKING);
        sql.push(", m.sort_date DESC, m.id DESC");
    } else {
        sql.push("ORDER BY m.sort_date DESC, m.id DESC");
    }
    // Une ligne de plus pour savoir s'il reste une page.
    sql.bind("LIMIT ?", i64::from(take) + 1);
    sql.bind("OFFSET ?", i64::from(request.offset));

    let mut stmt = c.prepare(&sql.text.join(" ")).map_err(store_error)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(sql.params.iter()), |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
                r.get(12)?,
            ))
        })
        .map_err(store_error)?
        .collect::<Result<Vec<HitRow>, _>>()
        .map_err(store_error)?;
    let more = rows.len() > take as usize;
    let items = rows
        .into_iter()
        .take(take as usize)
        .map(hit)
        .collect::<Result<Vec<_>, _>>()?;
    let end = request.offset + take;
    let next = (more && end < MAX_SEARCH_RESULTS).then_some(end);
    Ok(SearchPage { items, next })
}

/// Début de la requête classée par pertinence (texte cherché).
const SELECT_RANKED: &str = "SELECT m.id, b.account_id, m.mailbox_id, m.conversation_id, m.uid, \
     m.sort_date, m.from_name, m.from_address, m.subject, m.flags, m.keywords, \
     m.has_attachments, snippet(message_search, 3, '', '', '…', 24) \
     FROM message_search JOIN messages m ON m.id = message_search.rowid \
     JOIN mailboxes b ON b.id = m.mailbox_id";

/// Début de la requête classée par date (filtres et négations seulement).
const SELECT_BY_DATE: &str = "SELECT m.id, b.account_id, m.mailbox_id, m.conversation_id, m.uid, \
     m.sort_date, m.from_name, m.from_address, m.subject, m.flags, m.keywords, \
     m.has_attachments, substr(t.body, 1, 400) \
     FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id \
     LEFT JOIN message_texts t ON t.message_id = m.id";

/// Budget de temps d'une recherche : au-delà, elle est interrompue
/// (requête pathologique, très grosse boîte) plutôt que d'occuper un
/// lecteur.
pub const SEARCH_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

/// Recherche interrompue à l'instant `deadline`.
fn search_until(
    c: &Connection,
    request: &SearchRequest,
    deadline: std::time::Instant,
) -> Result<SearchPage, StoreError> {
    // Vérifié toutes les 8 instructions de la machine virtuelle (coût
    // négligeable devant celui d'une recherche) ; retiré ensuite
    // (connexion du pool réutilisée).
    c.progress_handler(8, Some(move || std::time::Instant::now() >= deadline));
    let result = search_query(c, request);
    c.progress_handler(0, None::<fn() -> bool>);
    result
}

impl SearchIndex for SqliteMailStore {
    async fn search(&self, request: SearchRequest) -> Result<SearchPage, StoreError> {
        self.db()
            .read(move |c| search_until(c, &request, std::time::Instant::now() + SEARCH_BUDGET))
            .await
    }
}

#[cfg(test)]
mod tests;
