//! Opérations de synchronisation d'une session IMAP : `ENABLE`, `LIST`,
//! `EXAMINE` (CONDSTORE/QRESYNC), `UID FETCH` (drapeaux, métadonnées,
//! corps), `UID SEARCH`, `IDLE`.
//!
//! Les réponses du serveur sont traduites en types du domaine, qui les
//! valident et les assainissent ; une donnée inexploitable est ignorée
//! (message sans UID, nom de boîte invalide…) plutôt que de faire échouer
//! toute la synchronisation.

use std::collections::BTreeSet;
use std::num::NonZeroU32;
use std::time::Duration;

use imap_next::imap_types::body::{BodyStructure, SpecificFields};
use imap_next::imap_types::command::{CommandBody, FetchModifier, SelectParameter, StoreModifier};
use imap_next::imap_types::core::{IString, Literal, NString, Vec1};
use imap_next::imap_types::extensions::binary::LiteralOrLiteral8;
use imap_next::imap_types::extensions::enable::CapabilityEnable;
use imap_next::imap_types::fetch::{
    MacroOrMessageDataItemNames, MessageDataItem, MessageDataItemName, Part, Section,
};
use imap_next::imap_types::flag::{Flag, FlagFetch, StoreResponse, StoreType};
use imap_next::imap_types::mailbox::{ListMailbox, Mailbox};
use imap_next::imap_types::response::{Code, Data, StatusKind};
use imap_next::imap_types::search::SearchKey;
use imap_next::imap_types::sequence::SequenceSet;
use mail_parser::{Address, HeaderValue, MessageParser};
use norelly_domain::actions::StoreOutcome;
use norelly_domain::connection::{ConnectionError, server_text};
use norelly_domain::fetch_policy::{
    BodyFetch, Disposition, MAX_MIME_DEPTH, MAX_MIME_PARTS, MimePart, PartPath, attachment_name,
    mime_token,
};
use norelly_domain::mailbox::{
    Flags, ListedMailbox, MailboxName, MessageHeaders, MessageMeta, ModSeq, RawHeaders, Uid,
    UidValidity,
};
use norelly_domain::ports::{FetchedBody, IdleOutcome, MAX_FETCH_BATCH};
use norelly_domain::sync::{
    FlagUpdate, SelectRequest, SelectedMailbox, ServerQuery, SyncExtensions, UidRanges,
};

use crate::connection::{BULK_DEADLINE, Connection, ensure_ok};
use crate::utf7;

fn protocol(what: impl Into<String>) -> ConnectionError {
    ConnectionError::Protocol(what.into())
}

/// Texte d'une chaîne IMAP, en remplaçant l'UTF-8 invalide.
fn text(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).into_owned()
}

fn istring(s: &IString<'_>) -> String {
    mime_token(&text(s.as_ref()))
}

/// Ensemble d'UID compact (`1:5,7,9:12`), trié.
pub(crate) fn uid_set(uids: impl IntoIterator<Item = Uid>) -> Option<SequenceSet> {
    let sorted: BTreeSet<u32> = uids.into_iter().map(Uid::get).collect();
    let mut parts: Vec<String> = Vec::new();
    let mut iter = sorted.into_iter().peekable();
    while let Some(start) = iter.next() {
        let mut end = start;
        while let Some(&next) = iter.peek() {
            if Some(next) == end.checked_add(1) {
                end = next;
                iter.next();
            } else {
                break;
            }
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}:{end}")
        });
    }
    if parts.is_empty() {
        return None;
    }
    SequenceSet::try_from(parts.join(",").as_str()).ok()
}

fn all_uids() -> Result<SequenceSet, ConnectionError> {
    SequenceSet::try_from("1:*").map_err(|_| protocol("ensemble 1:* invalide"))
}

/// Nom de boîte envoyé au serveur (UTF-7 modifié).
fn mailbox_arg(name: &MailboxName) -> Result<Mailbox<'static>, ConnectionError> {
    if name.is_inbox() {
        return Ok(Mailbox::Inbox);
    }
    Mailbox::try_from(utf7::encode(name.as_str()))
        .map_err(|_| protocol("nom de boîte non représentable"))
}

/// Nom de boîte reçu : UTF-7 modifié, ou UTF-8 (serveurs qui l'envoient
/// tel quel) ; `None` si invalide.
fn mailbox_name(mailbox: &Mailbox<'_>) -> Option<MailboxName> {
    let raw: &[u8] = match mailbox {
        Mailbox::Inbox => return Some(MailboxName::inbox()),
        Mailbox::Other(other) => other.inner().as_ref(),
    };
    // Repli sur l'UTF-8 brut seulement pour un nom 8 bits (serveur qui
    // n'encode pas) ; un nom ASCII mal encodé est ignoré.
    let decoded = match utf7::decode(raw) {
        Some(name) => name,
        None if !raw.is_ascii() => std::str::from_utf8(raw).ok()?.to_owned(),
        None => return None,
    };
    MailboxName::parse(&decoded).ok()
}

fn flags_of(list: &[FlagFetch<'_>]) -> Flags {
    Flags::from_tokens(list.iter().filter_map(|f| match f {
        FlagFetch::Flag(flag) => Some(flag.to_string()),
        FlagFetch::Recent => None,
    }))
}

/// UID, drapeaux et MODSEQ d'une réponse `FETCH`.
fn flag_update(items: &Vec1<MessageDataItem<'_>>) -> Option<FlagUpdate> {
    let mut uid = None;
    let mut flags = None;
    let mut modseq = None;
    for item in items.as_ref() {
        match item {
            MessageDataItem::Uid(u) => uid = Some(Uid::from_nonzero(*u)),
            MessageDataItem::Flags(f) => flags = Some(flags_of(f)),
            MessageDataItem::ModSeq(m) => modseq = ModSeq::new(m.get()),
            _ => {}
        }
    }
    Some(FlagUpdate {
        uid: uid?,
        flags: flags?,
        modseq,
    })
}

/// Plages d'une réponse `VANISHED`, sans dépliage (une plage peut couvrir
/// tout l'espace des UID) ; `*` est ignoré.
fn vanished_ranges(set: &SequenceSet, out: &mut Vec<(Uid, Uid)>) {
    use imap_next::imap_types::sequence::{SeqOrUid, Sequence};
    for seq in set.0.as_ref() {
        match seq {
            Sequence::Single(SeqOrUid::Value(v)) => {
                out.push((Uid::from_nonzero(*v), Uid::from_nonzero(*v)));
            }
            Sequence::Range(SeqOrUid::Value(a), SeqOrUid::Value(b)) => {
                out.push((Uid::from_nonzero(*a), Uid::from_nonzero(*b)));
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// BODYSTRUCTURE
// ---------------------------------------------------------------------------

fn param(list: &[(IString<'_>, IString<'_>)], name: &str) -> Option<String> {
    list.iter()
        .find(|(k, _)| text(k.as_ref()).eq_ignore_ascii_case(name))
        .map(|(_, v)| text(v.as_ref()))
}

/// Convertit une structure MIME ; `None` au-delà des bornes.
fn mime_part(structure: &BodyStructure<'_>, depth: usize, count: &mut usize) -> Option<MimePart> {
    if depth > MAX_MIME_DEPTH || *count >= MAX_MIME_PARTS {
        return None;
    }
    match structure {
        BodyStructure::Single {
            body,
            extension_data,
        } => {
            *count += 1;
            let size = body.basic.size;
            let (media_type, subtype) = match &body.specific {
                SpecificFields::Basic { r#type, subtype } => (istring(r#type), istring(subtype)),
                SpecificFields::Text { subtype, .. } => ("text".to_owned(), istring(subtype)),
                SpecificFields::Message { .. } => return Some(MimePart::Message { size }),
            };
            let disposition = extension_data
                .as_ref()
                .and_then(|e| e.tail.as_ref())
                .and_then(|d| d.disposition.as_ref());
            let (kind, filename) = match disposition {
                Some((kind, params)) => {
                    let kind = match text(kind.as_ref()).to_ascii_lowercase().as_str() {
                        "attachment" => Disposition::Attachment,
                        "inline" => Disposition::Inline,
                        _ => Disposition::Unspecified,
                    };
                    (kind, param(params, "filename"))
                }
                None => (Disposition::Unspecified, None),
            };
            let filename = filename
                .or_else(|| param(&body.basic.parameter_list, "name"))
                .map(|n| attachment_name(&n))
                .filter(|n| !n.is_empty());
            Some(MimePart::Leaf {
                media_type,
                subtype,
                size,
                disposition: kind,
                filename,
            })
        }
        BodyStructure::Multi {
            bodies, subtype, ..
        } => {
            let mut parts = Vec::new();
            for b in bodies.as_ref() {
                parts.push(mime_part(b, depth + 1, count)?);
            }
            Some(MimePart::Multipart {
                subtype: istring(subtype),
                parts,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// En-têtes
// ---------------------------------------------------------------------------

fn addresses<'a>(value: Option<&'a Address<'a>>) -> Vec<(&'a str, &'a str)> {
    let Some(value) = value else {
        return Vec::new();
    };
    value
        .iter()
        .map(|a| {
            (
                a.name().unwrap_or_default(),
                a.address().unwrap_or_default(),
            )
        })
        .collect()
}

fn id_list(value: &HeaderValue<'_>) -> Vec<String> {
    match value {
        HeaderValue::Text(t) => vec![t.to_string()],
        HeaderValue::TextList(list) => list.iter().map(ToString::to_string).collect(),
        _ => Vec::new(),
    }
}

/// En-têtes utiles d'un bloc d'en-têtes brut (mots encodés RFC 2047
/// décodés par `mail-parser`), assainis par le domaine.
pub(crate) fn parse_headers(raw: &[u8]) -> MessageHeaders {
    let Some(message) = MessageParser::new().parse_headers(raw) else {
        return MessageHeaders::default();
    };
    let in_reply_to = id_list(message.in_reply_to());
    let references = id_list(message.references());
    MessageHeaders::sanitize(&RawHeaders {
        subject: message.subject().unwrap_or_default(),
        from: addresses(message.from()),
        to: addresses(message.to()),
        cc: addresses(message.cc()),
        date: message.date().map(mail_parser::DateTime::to_timestamp),
        message_id: message.message_id(),
        in_reply_to: in_reply_to.iter().map(String::as_str).collect(),
        references: references.iter().map(String::as_str).collect(),
    })
}

fn nstring_bytes(data: &NString<'_>) -> Vec<u8> {
    data.0
        .as_ref()
        .map(|s| s.as_ref().to_vec())
        .unwrap_or_default()
}

fn meta_of(items: &Vec1<MessageDataItem<'_>>) -> Option<MessageMeta> {
    let mut uid = None;
    let mut flags = Flags::default();
    let mut modseq = None;
    let mut internal_date = 0;
    let mut size = 0;
    let mut headers = MessageHeaders::default();
    let mut structure = None;
    for item in items.as_ref() {
        match item {
            MessageDataItem::Uid(u) => uid = Some(Uid::from_nonzero(*u)),
            MessageDataItem::Flags(f) => flags = flags_of(f),
            MessageDataItem::ModSeq(m) => modseq = ModSeq::new(m.get()),
            MessageDataItem::InternalDate(d) => internal_date = d.as_ref().timestamp(),
            MessageDataItem::Rfc822Size(s) => size = *s,
            MessageDataItem::BodyStructure(b) | MessageDataItem::Body(b) => {
                let mut count = 0;
                structure = mime_part(b, 0, &mut count);
            }
            MessageDataItem::BodyExt {
                section: Some(Section::Header(None)),
                data,
                ..
            } => headers = parse_headers(&nstring_bytes(data)),
            _ => {}
        }
    }
    Some(MessageMeta {
        uid: uid?,
        flags,
        modseq,
        internal_date,
        size,
        headers,
        structure,
    })
}

fn part_numbers(path: &PartPath) -> Option<Part> {
    let numbers: Vec<NonZeroU32> = path
        .components()
        .iter()
        .filter_map(|n| NonZeroU32::new(*n))
        .collect();
    Vec1::try_from(numbers).ok().map(Part)
}

fn part_section(path: &PartPath) -> Option<Section<'static>> {
    part_numbers(path).map(Section::Part)
}

/// En-têtes MIME d'une partie (`BODY[1.2.MIME]`).
fn mime_section(path: &PartPath) -> Option<Section<'static>> {
    part_numbers(path).map(Section::Mime)
}

fn path_of(part: &Part) -> Option<PartPath> {
    PartPath::new(part.0.as_ref().iter().map(|n| n.get()).collect())
}

/// Entité MIME autonome d'une partie : ses en-têtes MIME puis son contenu
/// (encodé tel que reçu). En-têtes absents : ligne vide seule (le rendu
/// reprend alors ceux du message).
fn entity(mime: Option<Vec<u8>>, body: Vec<u8>) -> Vec<u8> {
    let mut out = mime.unwrap_or_default();
    if !(out.ends_with(b"\r\n\r\n") || out.ends_with(b"\n\n")) {
        if !out.is_empty() && !out.ends_with(b"\n") {
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(&body);
    out
}

fn body_item(section: Option<Section<'static>>) -> MessageDataItemName<'static> {
    MessageDataItemName::BodyExt {
        section,
        partial: None,
        peek: true,
    }
}

// ---------------------------------------------------------------------------
// Opérations
// ---------------------------------------------------------------------------

/// Forme de la demande de métadonnées, de la plus riche à la plus sûre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetaTier {
    Complete,
    NoStructure,
    Minimal,
}

impl Connection {
    pub(crate) async fn enable_sync(
        &mut self,
        condstore: bool,
        qresync: bool,
    ) -> Result<SyncExtensions, ConnectionError> {
        if !condstore {
            return Ok(SyncExtensions::default());
        }
        let mut wanted = vec![CapabilityEnable::CondStore];
        if qresync {
            wanted.push(
                CapabilityEnable::try_from("QRESYNC").map_err(|_| protocol("capacité QRESYNC"))?,
            );
        }
        let body = CommandBody::Enable {
            capabilities: Vec1::try_from(wanted).map_err(|_| protocol("ENABLE vide"))?,
        };
        let reply = self.execute(body, false).await?;
        ensure_ok(&reply.status)?;
        let enabled_qresync = reply.data.iter().any(|d| match d {
            Data::Enabled { capabilities } => capabilities
                .iter()
                .any(|c| c.to_string().eq_ignore_ascii_case("QRESYNC")),
            _ => false,
        });
        Ok(SyncExtensions {
            // CONDSTORE est actif dès qu'il est annoncé (on l'active aussi
            // par le paramètre de SELECT).
            condstore: true,
            qresync: enabled_qresync,
        })
    }

    pub(crate) async fn list(&mut self) -> Result<Vec<ListedMailbox>, ConnectionError> {
        let body = CommandBody::List {
            reference: Mailbox::try_from("").map_err(|_| protocol("référence LIST"))?,
            mailbox_wildcard: ListMailbox::try_from("*").map_err(|_| protocol("motif LIST"))?,
        };
        let reply = self.execute_bulk(body).await?;
        ensure_ok(&reply.status)?;
        let mut out = Vec::new();
        for data in reply.data {
            if let Data::List {
                items,
                delimiter,
                mailbox,
            } = data
                && let Some(name) = mailbox_name(&mailbox)
            {
                out.push(ListedMailbox {
                    name,
                    delimiter: delimiter.map(|d| d.inner()),
                    attributes: items.iter().map(ToString::to_string).collect(),
                });
            }
        }
        Ok(out)
    }

    pub(crate) async fn examine(
        &mut self,
        name: &MailboxName,
        request: &SelectRequest,
    ) -> Result<SelectedMailbox, ConnectionError> {
        let mut parameters = Vec::new();
        if let Some(q) = &request.qresync {
            let modseq =
                std::num::NonZeroU64::new(q.modseq.get()).ok_or_else(|| protocol("MODSEQ nul"))?;
            let uid_validity = NonZeroU32::new(q.uid_validity.get())
                .ok_or_else(|| protocol("UIDVALIDITY nulle"))?;
            parameters.push(SelectParameter::QResync {
                uid_validity,
                mod_sequence_value: modseq,
                // Volontairement absent : voir `QresyncParams`.
                known_uids: None,
                seq_match_data: None,
            });
        } else if request.condstore {
            parameters.push(SelectParameter::CondStore);
        }
        let body = CommandBody::Examine {
            mailbox: mailbox_arg(name)?,
            parameters,
        };
        let reply = self.execute_bulk(body).await?;
        if reply.status.kind == StatusKind::No {
            return Err(ConnectionError::Rejected(format!(
                "sélection de « {name} » refusée : {}",
                server_text(reply.status.text.as_ref().as_bytes())
            )));
        }
        if reply.status.kind != StatusKind::Ok {
            return Err(ConnectionError::Protocol(format!(
                "sélection de « {name} » invalide : {}",
                server_text(reply.status.text.as_ref().as_bytes())
            )));
        }
        let mut selected = SelectedMailbox::default();
        let mut vanished = Vec::new();
        let codes = reply
            .untagged
            .iter()
            .chain(std::iter::once(&reply.status))
            .filter_map(|s| s.code.as_ref());
        for code in codes {
            match code {
                Code::UidValidity(v) => selected.uid_validity = UidValidity::new(v.get()),
                Code::UidNext(n) => selected.uid_next = Uid::new(n.get()),
                Code::HighestModSeq(m) => selected.highest_modseq = ModSeq::new(m.get()),
                Code::Other(other) if text(other.inner()).eq_ignore_ascii_case("NOMODSEQ") => {
                    selected.highest_modseq = None;
                }
                _ => {}
            }
        }
        for data in &reply.data {
            match data {
                Data::Exists(n) => selected.exists = *n,
                Data::Fetch { items, .. } => {
                    if let Some(update) = flag_update(items) {
                        selected.changed.push(update);
                    }
                }
                Data::Vanished { known_uids, .. } => {
                    vanished_ranges(known_uids, &mut vanished);
                }
                _ => {}
            }
        }
        selected.vanished = UidRanges::new(vanished);
        Ok(selected)
    }

    pub(crate) async fn fetch_flags(
        &mut self,
        query: ServerQuery,
        condstore: bool,
    ) -> Result<Vec<FlagUpdate>, ConnectionError> {
        let mut names = vec![MessageDataItemName::Uid, MessageDataItemName::Flags];
        let mut modifiers = Vec::new();
        if condstore {
            names.push(MessageDataItemName::ModSeq);
        }
        if let ServerQuery::ChangedFlags(since) = query {
            let since =
                std::num::NonZeroU64::new(since.get()).ok_or_else(|| protocol("MODSEQ nul"))?;
            modifiers.push(FetchModifier::ChangedSince(since));
        }
        let body = CommandBody::Fetch {
            sequence_set: all_uids()?,
            macro_or_item_names: MacroOrMessageDataItemNames::MessageDataItemNames(names),
            uid: true,
            modifiers,
        };
        let reply = self.execute_bulk(body).await?;
        ensure_ok(&reply.status)?;
        Ok(reply
            .data
            .iter()
            .filter_map(|d| match d {
                Data::Fetch { items, .. } => flag_update(items),
                _ => None,
            })
            .collect())
    }

    pub(crate) async fn search_all(&mut self) -> Result<BTreeSet<Uid>, ConnectionError> {
        let body = CommandBody::search(None, Vec1::from(SearchKey::All), true);
        let reply = self.execute_bulk(body).await?;
        ensure_ok(&reply.status)?;
        let mut out = BTreeSet::new();
        for data in &reply.data {
            if let Data::Search(uids, ..) = data {
                out.extend(uids.iter().map(|u| Uid::from_nonzero(*u)));
            }
        }
        Ok(out)
    }

    /// Métadonnées d'un lot. Un message dont la réponse est écartée
    /// (BODYSTRUCTURE trop imbriquée, réponse indécodable) est redemandé
    /// sans structure, puis réduit à ses drapeaux, sa taille et sa date :
    /// un seul message hostile ne bloque pas la synchronisation.
    pub(crate) async fn fetch_meta(
        &mut self,
        uids: &[Uid],
        condstore: bool,
    ) -> Result<Vec<MessageMeta>, ConnectionError> {
        if uids.len() > MAX_FETCH_BATCH {
            return Err(protocol("lot de FETCH trop grand"));
        }
        let mut missing: BTreeSet<Uid> = uids.iter().copied().collect();
        let mut out: Vec<MessageMeta> = Vec::new();
        for tier in [MetaTier::Complete, MetaTier::NoStructure, MetaTier::Minimal] {
            let Some(set) = uid_set(missing.iter().copied()) else {
                break;
            };
            let (metas, skipped) = self.fetch_meta_tier(set, &missing, condstore, tier).await?;
            for meta in metas {
                if missing.remove(&meta.uid) {
                    out.push(meta);
                }
            }
            // Sans réponse écartée, les UID restants ont disparu.
            if skipped == 0 {
                break;
            }
            tracing::warn!(
                ?tier,
                skipped,
                "métadonnées redemandées sous une forme réduite"
            );
        }
        out.sort_by_key(|m| m.uid);
        Ok(out)
    }

    async fn fetch_meta_tier(
        &mut self,
        set: SequenceSet,
        wanted: &BTreeSet<Uid>,
        condstore: bool,
        tier: MetaTier,
    ) -> Result<(Vec<MessageMeta>, usize), ConnectionError> {
        let mut names = vec![
            MessageDataItemName::Uid,
            MessageDataItemName::Flags,
            MessageDataItemName::InternalDate,
            MessageDataItemName::Rfc822Size,
        ];
        if tier == MetaTier::Complete {
            names.push(MessageDataItemName::BodyStructure);
        }
        if tier != MetaTier::Minimal {
            names.push(body_item(Some(Section::Header(None))));
        }
        if condstore {
            names.push(MessageDataItemName::ModSeq);
        }
        let body = CommandBody::Fetch {
            sequence_set: set,
            macro_or_item_names: MacroOrMessageDataItemNames::MessageDataItemNames(names),
            uid: true,
            modifiers: Vec::new(),
        };
        // Quelques réponses par UID demandé (FETCH non sollicités tolérés),
        // pas davantage.
        let max_data = wanted.len().saturating_mul(4).saturating_add(64);
        let reply = self
            .execute_with(body, false, max_data, BULK_DEADLINE)
            .await?;
        // NO avec des données (« certains messages n'ont pu être lus ») :
        // on garde ce qui est arrivé.
        if reply.status.kind == StatusKind::Bad {
            ensure_ok(&reply.status)?;
        }
        let metas = reply
            .data
            .iter()
            .filter_map(|d| match d {
                Data::Fetch { items, .. } => meta_of(items),
                _ => None,
            })
            // Réponses non sollicitées ou hors demande ignorées.
            .filter(|m| wanted.contains(&m.uid))
            .collect();
        Ok((metas, reply.skipped))
    }

    pub(crate) async fn fetch_body(
        &mut self,
        uid: Uid,
        fetch: &BodyFetch,
    ) -> Result<Option<FetchedBody>, ConnectionError> {
        let set = uid_set([uid]).ok_or_else(|| protocol("UID invalide"))?;
        let mut names = vec![MessageDataItemName::Uid];
        match fetch {
            BodyFetch::Full => names.push(body_item(None)),
            BodyFetch::Partial { parts, .. } => {
                names.push(body_item(Some(Section::Header(None))));
                for path in parts {
                    names.push(body_item(mime_section(path)));
                    names.push(body_item(part_section(path)));
                }
            }
        }
        let body = CommandBody::Fetch {
            sequence_set: set,
            macro_or_item_names: MacroOrMessageDataItemNames::MessageDataItemNames(names),
            uid: true,
            modifiers: Vec::new(),
        };
        // Corps volumineux : échéance longue ; réponse trop longue écartée
        // (le message est alors traité comme absent).
        let reply = self.execute_with(body, false, 64, BULK_DEADLINE).await?;
        ensure_ok(&reply.status)?;
        for data in &reply.data {
            let Data::Fetch { items, .. } = data else {
                continue;
            };
            let items = items.as_ref();
            if !items
                .iter()
                .any(|i| matches!(i, MessageDataItem::Uid(u) if u.get() == uid.get()))
            {
                continue;
            }
            let mut full = None;
            let mut header = None;
            let mut parts = Vec::new();
            let mut mimes = Vec::new();
            for item in items {
                let MessageDataItem::BodyExt { section, data, .. } = item else {
                    continue;
                };
                match section {
                    None => full = Some(nstring_bytes(data)),
                    Some(Section::Header(None)) => header = Some(nstring_bytes(data)),
                    Some(Section::Part(part)) => {
                        if let Some(path) = path_of(part) {
                            parts.push((path, nstring_bytes(data)));
                        }
                    }
                    Some(Section::Mime(part)) => {
                        if let Some(path) = path_of(part) {
                            mimes.push((path, nstring_bytes(data)));
                        }
                    }
                    _ => {}
                }
            }
            return Ok(match fetch {
                BodyFetch::Full => full.map(FetchedBody::Full),
                BodyFetch::Partial { .. } => header.map(|header| FetchedBody::Partial {
                    header,
                    parts: parts
                        .into_iter()
                        .map(|(path, body)| {
                            let mime = mimes
                                .iter()
                                .position(|(p, _)| *p == path)
                                .map(|i| mimes.swap_remove(i).1);
                            (path, entity(mime, body))
                        })
                        .collect(),
                }),
            });
        }
        Ok(None)
    }

    // -----------------------------------------------------------------------
    // Actions (phase 6)
    // -----------------------------------------------------------------------

    /// `SELECT` en lecture-écriture (CONDSTORE si demandé).
    pub(crate) async fn select_rw(
        &mut self,
        name: &MailboxName,
        condstore: bool,
    ) -> Result<SelectedMailbox, ConnectionError> {
        let parameters = if condstore {
            vec![SelectParameter::CondStore]
        } else {
            Vec::new()
        };
        let body = CommandBody::Select {
            mailbox: mailbox_arg(name)?,
            parameters,
        };
        let reply = self.execute(body, false).await?;
        if reply.status.kind == StatusKind::No {
            return Err(ConnectionError::Rejected(server_text(
                reply.status.text.as_ref().as_bytes(),
            )));
        }
        ensure_ok(&reply.status)?;
        let mut selected = SelectedMailbox::default();
        for code in reply
            .untagged
            .iter()
            .chain(std::iter::once(&reply.status))
            .filter_map(|s| s.code.as_ref())
        {
            match code {
                Code::UidValidity(v) => selected.uid_validity = UidValidity::new(v.get()),
                Code::UidNext(n) => selected.uid_next = Uid::new(n.get()),
                Code::HighestModSeq(m) => selected.highest_modseq = ModSeq::new(m.get()),
                _ => {}
            }
        }
        for data in &reply.data {
            if let Data::Exists(n) = data {
                selected.exists = *n;
            }
        }
        Ok(selected)
    }

    /// Drapeaux d'un message (`UID FETCH uid (UID FLAGS)`).
    pub(crate) async fn flags_of(&mut self, uid: Uid) -> Result<Option<Flags>, ConnectionError> {
        let set = uid_set([uid]).ok_or_else(|| protocol("UID invalide"))?;
        let body = CommandBody::Fetch {
            sequence_set: set,
            macro_or_item_names: MacroOrMessageDataItemNames::MessageDataItemNames(vec![
                MessageDataItemName::Uid,
                MessageDataItemName::Flags,
            ]),
            uid: true,
            modifiers: Vec::new(),
        };
        let reply = self.execute(body, false).await?;
        ensure_ok(&reply.status)?;
        Ok(reply.data.iter().find_map(|d| match d {
            Data::Fetch { items, .. } => {
                flag_update(items).filter(|u| u.uid == uid).map(|u| u.flags)
            }
            _ => None,
        }))
    }

    /// `UID STORE uid [(UNCHANGEDSINCE m)] ±FLAGS.SILENT (flag)`.
    pub(crate) async fn store(
        &mut self,
        uid: Uid,
        flag: Flag<'static>,
        add: bool,
        unchanged_since: Option<ModSeq>,
    ) -> Result<StoreOutcome, ConnectionError> {
        let set = uid_set([uid]).ok_or_else(|| protocol("UID invalide"))?;
        let body = CommandBody::Store {
            sequence_set: set,
            kind: if add {
                StoreType::Add
            } else {
                StoreType::Remove
            },
            response: StoreResponse::Silent,
            flags: vec![flag],
            uid: true,
            modifiers: unchanged_since
                .map(|m| vec![StoreModifier::UnchangedSince(m.get())])
                .unwrap_or_default(),
        };
        let reply = self.execute(body, false).await?;
        ensure_ok(&reply.status)?;
        let modified = reply
            .untagged
            .iter()
            .chain(std::iter::once(&reply.status))
            .any(|s| matches!(s.code, Some(Code::Modified(_))));
        Ok(if modified {
            StoreOutcome::Modified
        } else {
            StoreOutcome::Stored
        })
    }

    /// `APPEND` (littéral synchrone), `\Seen` et `\Draft` si `draft` ;
    /// UID lu dans `APPENDUID`.
    pub(crate) async fn append(
        &mut self,
        to: &MailboxName,
        raw: &[u8],
        draft: bool,
    ) -> Result<Option<Uid>, ConnectionError> {
        let mut flags = vec![Flag::Seen];
        if draft {
            flags.push(Flag::Draft);
        }
        let literal = Literal::try_from(raw.to_vec())
            .map_err(|_| protocol("message impossible à transmettre (octet nul)"))?;
        let body = CommandBody::Append {
            mailbox: mailbox_arg(to)?,
            flags,
            date: None,
            message: LiteralOrLiteral8::Literal(literal),
        };
        let reply = self.execute(body, false).await?;
        ensure_ok(&reply.status)?;
        Ok(match reply.status.code {
            Some(Code::AppendUid { uid, .. }) => Some(Uid::from_nonzero(uid)),
            _ => None,
        })
    }

    /// `UID MOVE` ou `UID COPY`.
    pub(crate) async fn transfer(
        &mut self,
        uid: Uid,
        to: &MailboxName,
        keep_source: bool,
    ) -> Result<(), ConnectionError> {
        let sequence_set = uid_set([uid]).ok_or_else(|| protocol("UID invalide"))?;
        let mailbox = mailbox_arg(to)?;
        let body = if keep_source {
            CommandBody::Copy {
                sequence_set,
                mailbox,
                uid: true,
            }
        } else {
            CommandBody::Move {
                sequence_set,
                mailbox,
                uid: true,
            }
        };
        let reply = self.execute(body, false).await?;
        ensure_ok(&reply.status)
    }

    /// `UID EXPUNGE uid` (UIDPLUS), ou `EXPUNGE` (`uid` absent). Renvoie
    /// le nombre de réponses `* n EXPUNGE` (messages supprimés).
    pub(crate) async fn expunge(&mut self, uid: Option<Uid>) -> Result<u32, ConnectionError> {
        let body = match uid {
            Some(uid) => CommandBody::ExpungeUid {
                sequence_set: uid_set([uid]).ok_or_else(|| protocol("UID invalide"))?,
            },
            None => CommandBody::Expunge,
        };
        let reply = self.execute_bulk(body).await?;
        ensure_ok(&reply.status)?;
        let expunged = reply
            .data
            .iter()
            .filter(|d| matches!(d, Data::Expunge(_)))
            .count();
        Ok(u32::try_from(expunged).unwrap_or(u32::MAX))
    }

    /// `UID SEARCH DELETED`.
    pub(crate) async fn search_deleted(&mut self) -> Result<BTreeSet<Uid>, ConnectionError> {
        let body = CommandBody::search(None, Vec1::from(SearchKey::Deleted), true);
        let reply = self.execute_bulk(body).await?;
        ensure_ok(&reply.status)?;
        let mut out = BTreeSet::new();
        for data in &reply.data {
            if let Data::Search(uids, ..) = data {
                out.extend(uids.iter().map(|u| Uid::from_nonzero(*u)));
            }
        }
        Ok(out)
    }

    pub(crate) async fn idle_or_poll(
        &mut self,
        max: Duration,
        supported: bool,
    ) -> Result<IdleOutcome, ConnectionError> {
        if !supported {
            // Sondage : NOOP après l'attente pour vérifier la connexion.
            tokio::time::sleep(max).await;
            let reply = self.execute(CommandBody::Noop, false).await?;
            ensure_ok(&reply.status)?;
            return Ok(IdleOutcome::Changed);
        }
        Ok(if self.idle(max).await? {
            IdleOutcome::Changed
        } else {
            IdleOutcome::Timeout
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uids(list: &[u32]) -> Vec<Uid> {
        list.iter().filter_map(|n| Uid::new(*n)).collect()
    }

    #[test]
    fn uid_sets_are_compact() {
        let set = uid_set(uids(&[9, 1, 2, 3, 5, 10, 11, 7])).map(|s| {
            s.0.as_ref()
                .iter()
                .map(|q| format!("{q:?}"))
                .collect::<Vec<_>>()
                .len()
        });
        assert_eq!(set, Some(4), "1:3,5,7,9:11");
        assert!(uid_set(Vec::new()).is_none());
    }

    #[test]
    fn headers_are_decoded_and_sanitized() {
        let raw = b"Subject: =?UTF-8?B?UsOpdW5pb24=?=\r\n\
From: =?ISO-8859-1?Q?Ren=E9?= <rene@example.com>\r\n\
To: a@example.com, \"B\" <b@example.com>\r\n\
Date: Mon, 2 Nov 2026 10:00:00 +0000\r\n\
Message-ID: <id1@example.com>\r\n\
In-Reply-To: <id0@example.com>\r\n\
References: <r1@example.com> <id0@example.com>\r\n\r\n";
        let h = parse_headers(raw);
        assert_eq!(h.subject, "Réunion");
        assert_eq!(
            h.from
                .first()
                .map(|c| (c.name.as_str(), c.address.as_str())),
            Some(("René", "rene@example.com"))
        );
        assert_eq!(h.to.len(), 2);
        assert_eq!(h.date, Some(1_793_613_600));
        assert_eq!(h.message_id.as_deref(), Some("id1@example.com"));
        assert_eq!(h.in_reply_to, ["id0@example.com"]);
        assert_eq!(h.references, ["r1@example.com", "id0@example.com"]);
    }

    #[test]
    fn hostile_headers_do_not_panic() {
        for raw in [
            &b""[..],
            b"\r\n\r\n",
            b"Subject: \xff\xfe\r\n\r\n",
            b"From: <<<>>>@@@\r\n\r\n",
            b"Subject: =?UTF-8?B?????==?=\r\n\r\n",
            b"Date: 99 Foo 99999 99:99:99 +9999\r\n\r\n",
        ] {
            let _ = parse_headers(raw);
        }
        let long = format!("Subject: {}\r\n\r\n", "x".repeat(100_000));
        assert!(parse_headers(long.as_bytes()).subject.chars().count() <= 998);
    }
}
