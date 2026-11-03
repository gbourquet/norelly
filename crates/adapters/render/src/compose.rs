//! Messages rédigés (phase 7, ADR-010) : réassainissement du HTML de
//! l'éditeur, version texte, construction MIME avec `lettre`.
//!
//! Le HTML produit par l'éditeur (Tiptap) est traité comme non fiable : il
//! est réassaini ici avec une liste blanche minimale (paragraphes, gras,
//! italique, liens `http`/`https`/`mailto`, listes, citations). La version
//! texte du `multipart/alternative` est dérivée de ce HTML assaini, pour
//! que les deux versions disent la même chose.
//!
//! Structure produite :
//! - texte seul : `text/plain` ;
//! - avec HTML : `multipart/alternative` (texte, puis HTML) ;
//! - avec pièces jointes : `multipart/mixed` (corps, puis pièces jointes).
//!
//! `Bcc` n'est écrit que dans les copies locales (brouillon, Envoyés),
//! jamais dans le message transmis ([`BccHeader`]).

use std::time::{Duration, SystemTime};

use lettre::address::Envelope;
use lettre::message::header::{self, ContentType};
use lettre::message::{Attachment, Mailbox, Mailboxes, Message, MultiPart, SinglePart};
use mail_parser::{HeaderValue, MessageParser};
use norelly_domain::address::EmailAddress;
use norelly_domain::compose::ReplySource;
use norelly_domain::compose::draft::Recipient;
use norelly_domain::mailbox::{Correspondent, MAX_ADDRESSES, MessageHeaders, RawHeaders};
use norelly_domain::ports::{
    ComposeBuildError, ComposedMessage, Composition, MessageComposer, OutgoingMessage,
};
use norelly_domain::reading::safe_file_name;

use crate::escape_html;
use crate::sanitize::nesting_depth;

/// Écrire l'en-tête `Bcc` ?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BccHeader {
    /// Copie locale (brouillon, Envoyés) : l'utilisateur retrouve ses Cci.
    Keep,
    /// Message transmis : les Cci restent invisibles des destinataires.
    Omit,
}

/// Échec de construction (port du domaine).
pub type BuildError = ComposeBuildError;

/// Liste blanche du HTML rédigé : ce que l'éditeur sait produire.
fn composed_builder() -> ammonia::Builder<'static> {
    let mut b = ammonia::Builder::empty();
    b.add_tags([
        "p",
        "br",
        "strong",
        "b",
        "em",
        "i",
        "s",
        "a",
        "ul",
        "ol",
        "li",
        "blockquote",
    ])
    .add_tag_attributes("a", ["href"])
    .url_schemes(["http", "https", "mailto"].into())
    .url_relative(ammonia::UrlRelative::Deny)
    .link_rel(None)
    .strip_comments(true)
    .clean_content_tags(["script", "style"].into());
    b
}

/// Réassainit le HTML de l'éditeur (liste blanche minimale).
#[must_use]
pub fn sanitize_composed_html(html: &str) -> String {
    if nesting_depth(html) > MAX_COMPOSED_NESTING {
        // Jamais produit par l'éditeur : réduit à son texte, échappé
        // (analyse et version texte restent linéaires et peu profondes).
        return format!("<p>{}</p>", escape_html(&strip_tags(html)));
    }
    composed_builder().clean(html).to_string()
}

/// Taille maximale du texte cité dans une réponse (le brouillon reste
/// sous [`norelly_domain::compose::draft::MAX_BODY_BYTES`]).
pub const MAX_QUOTED_BYTES: usize = 1024 * 1024;

/// Profondeur d'imbrication maximale du HTML rédigé (l'éditeur en produit
/// bien moins) ; au-delà, il est réduit à son texte.
pub const MAX_COMPOSED_NESTING: usize = 64;

/// Texte d'un HTML, balises retirées (sans interprétation des entités).
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Version texte d'un HTML (réassaini d'abord) : paragraphes séparés
/// d'une ligne vide, listes à puces ou numérotées, citations précédées de
/// `> `, liens suivis de leur adresse.
#[must_use]
pub fn html_to_text(html: &str) -> String {
    text::of(&sanitize_composed_html(html))
}

mod text {
    //! Version texte du HTML **assaini** : la sortie du sérialiseur
    //! d'`ammonia` est régulière (balises de la liste blanche, seul
    //! attribut `href`, texte échappé, éléments tous fermés). Un petit
    //! analyseur à pile suffit ; une entrée inattendue ne fait que perdre
    //! de la mise en forme, jamais paniquer.

    /// Nœud de l'arbre relu.
    enum Node {
        Text(String),
        Element {
            name: String,
            href: Option<String>,
            children: Vec<Node>,
        },
    }

    fn unescape(raw: &str) -> String {
        raw.replace("&nbsp;", " ")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&amp;", "&")
    }

    /// Lit `<name attrs>` ou `</name>` ; renvoie (fermante, nom, href).
    fn read_tag(inner: &str) -> (bool, String, Option<String>) {
        let (closing, inner) = match inner.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, inner),
        };
        let inner = inner.trim_end_matches('/');
        let name_end = inner.find(char::is_whitespace).unwrap_or(inner.len());
        let name = inner
            .get(..name_end)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let href = inner
            .find("href=\"")
            .and_then(|start| {
                let value = inner.get(start + 6..)?;
                value.find('"').and_then(|end| value.get(..end))
            })
            .map(unescape);
        (closing, name, href)
    }

    fn parse(html: &str) -> Vec<Node> {
        const VOID: &[&str] = &["br"];
        // Pile des éléments ouverts : (nom, href, enfants).
        let mut stack: Vec<(String, Option<String>, Vec<Node>)> =
            vec![(String::new(), None, Vec::new())];
        let mut rest = html;
        while !rest.is_empty() {
            let (text, after) = match rest.find('<') {
                Some(i) => (
                    rest.get(..i).unwrap_or_default(),
                    rest.get(i..).unwrap_or_default(),
                ),
                None => (rest, ""),
            };
            if !text.is_empty()
                && let Some(top) = stack.last_mut()
            {
                top.2.push(Node::Text(unescape(text)));
            }
            if after.is_empty() {
                break;
            }
            let Some(end) = after.find('>') else { break };
            let (closing, name, href) = read_tag(after.get(1..end).unwrap_or_default());
            rest = after.get(end + 1..).unwrap_or_default();
            if closing {
                // Ferme jusqu'à l'élément de même nom (s'il est ouvert).
                if stack.iter().skip(1).any(|(n, _, _)| *n == name) {
                    while stack.len() > 1 {
                        let Some((n, h, children)) = stack.pop() else {
                            break;
                        };
                        let done = n == name;
                        if let Some(parent) = stack.last_mut() {
                            parent.2.push(Node::Element {
                                name: n,
                                href: h,
                                children,
                            });
                        }
                        if done {
                            break;
                        }
                    }
                }
            } else if VOID.contains(&name.as_str()) {
                if let Some(top) = stack.last_mut() {
                    top.2.push(Node::Element {
                        name,
                        href,
                        children: Vec::new(),
                    });
                }
            } else {
                stack.push((name, href, Vec::new()));
            }
        }
        while stack.len() > 1 {
            let Some((n, h, children)) = stack.pop() else {
                break;
            };
            if let Some(parent) = stack.last_mut() {
                parent.2.push(Node::Element {
                    name: n,
                    href: h,
                    children,
                });
            }
        }
        stack.pop().map(|(_, _, c)| c).unwrap_or_default()
    }

    fn trim_end_spaces(out: &mut String) {
        while out.ends_with(' ') {
            out.pop();
        }
    }

    /// Texte en ligne : espaces réduites, `<br>` en saut de ligne, liens
    /// suivis de leur adresse.
    fn inline(node: &Node, out: &mut String) {
        match node {
            Node::Text(text) => {
                if text.starts_with(char::is_whitespace)
                    && !out.is_empty()
                    && !out.ends_with([' ', '\n'])
                {
                    out.push(' ');
                }
                let words: Vec<&str> = text.split_whitespace().collect();
                out.push_str(&words.join(" "));
                if !words.is_empty() && text.ends_with(char::is_whitespace) {
                    out.push(' ');
                }
            }
            Node::Element {
                name,
                href,
                children,
            } => match name.as_str() {
                "br" => {
                    trim_end_spaces(out);
                    out.push('\n');
                }
                "a" => {
                    let mut label = String::new();
                    for child in children {
                        inline(child, &mut label);
                    }
                    out.push_str(&label);
                    if let Some(href) = href
                        && !href.is_empty()
                        && href != label.trim()
                    {
                        trim_end_spaces(out);
                        out.push_str(" <");
                        out.push_str(href);
                        out.push('>');
                    }
                }
                "p" | "li" => {
                    if !out.is_empty() && !out.ends_with('\n') {
                        trim_end_spaces(out);
                        out.push('\n');
                    }
                    for child in children {
                        inline(child, out);
                    }
                }
                "ul" | "ol" => {
                    let mut lines = Vec::new();
                    list(node, &mut lines);
                    trim_end_spaces(out);
                    for line in lines {
                        out.push_str("\n  ");
                        out.push_str(&line);
                    }
                }
                _ => {
                    for child in children {
                        inline(child, out);
                    }
                }
            },
        }
    }

    fn clean(raw: &str) -> String {
        raw.lines()
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n")
            .trim_matches('\n')
            .to_owned()
    }

    /// Lignes d'une liste (`- ` ou `n. `).
    fn list(node: &Node, lines: &mut Vec<String>) {
        let Node::Element { name, children, .. } = node else {
            return;
        };
        let ordered = name == "ol";
        let mut n = 0_u32;
        for child in children {
            let Node::Element {
                name,
                children: content,
                ..
            } = child
            else {
                continue;
            };
            if name != "li" {
                continue;
            }
            n = n.saturating_add(1);
            let mut text = String::new();
            for c in content {
                inline(c, &mut text);
            }
            let marker = if ordered {
                format!("{n}. ")
            } else {
                "- ".to_owned()
            };
            let mut item: Vec<String> = clean(&text).lines().map(str::to_owned).collect();
            match item.first_mut() {
                Some(first) => first.insert_str(0, &marker),
                None => item.push(marker.trim_end().to_owned()),
            }
            lines.extend(item);
        }
    }

    /// Blocs (paragraphes, listes, citations) d'une suite de nœuds.
    fn blocks(nodes: &[Node], out: &mut Vec<String>) {
        let mut loose = String::new();
        let flush = |loose: &mut String, out: &mut Vec<String>| {
            let text = clean(loose);
            if !text.is_empty() {
                out.push(text);
            }
            loose.clear();
        };
        for node in nodes {
            let name = match node {
                Node::Element { name, .. } => name.as_str(),
                Node::Text(_) => "",
            };
            match (name, node) {
                ("p", _) => {
                    flush(&mut loose, out);
                    let mut text = String::new();
                    inline(node, &mut text);
                    flush(&mut text, out);
                }
                ("ul" | "ol", _) => {
                    flush(&mut loose, out);
                    let mut lines = Vec::new();
                    list(node, &mut lines);
                    if !lines.is_empty() {
                        out.push(lines.join("\n"));
                    }
                }
                ("blockquote", Node::Element { children, .. }) => {
                    flush(&mut loose, out);
                    let mut inner = Vec::new();
                    blocks(children, &mut inner);
                    let quoted: Vec<String> = inner
                        .join("\n\n")
                        .lines()
                        .map(|line| {
                            if line.is_empty() || line.starts_with('>') {
                                format!(">{line}")
                            } else {
                                format!("> {line}")
                            }
                        })
                        .collect();
                    if !quoted.is_empty() {
                        out.push(quoted.join("\n"));
                    }
                }
                _ => inline(node, &mut loose),
            }
        }
        flush(&mut loose, out);
    }

    /// Texte d'un HTML assaini.
    pub(super) fn of(html: &str) -> String {
        let mut out = Vec::new();
        blocks(&parse(html), &mut out);
        let mut text = out.join("\n\n");
        text.push('\n');
        text
    }
}

/// `Message-ID` aléatoire (128 bits) dans le domaine de l'expéditeur, sans
/// chevrons ni nom de la machine.
pub fn new_message_id(from: &EmailAddress) -> Result<String, BuildError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| BuildError::Random)?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("{hex}@{}", from.domain().as_str()))
}

fn lettre_address(address: &EmailAddress) -> Result<lettre::Address, BuildError> {
    address
        .to_string()
        .parse::<lettre::Address>()
        .map_err(|_| BuildError::Address(address.to_string()))
}

fn mailbox(r: &Recipient) -> Result<Mailbox, BuildError> {
    let name = (!r.name.trim().is_empty()).then(|| r.name.clone());
    Ok(Mailbox::new(name, lettre_address(&r.address)?))
}

fn mailboxes(list: &[Recipient]) -> Result<Option<Mailboxes>, BuildError> {
    if list.is_empty() {
        return Ok(None);
    }
    let all: Vec<Mailbox> = list.iter().map(mailbox).collect::<Result<_, _>>()?;
    Ok(Some(all.into_iter().collect()))
}

fn mime_error(e: impl std::fmt::Display) -> BuildError {
    BuildError::Mime(e.to_string())
}

/// Message RFC 5322 brut.
pub fn build_message(c: &Composition, bcc: BccHeader) -> Result<Vec<u8>, BuildError> {
    let d = &c.draft;
    let from = mailbox(&c.from)?;
    // Enveloppe imposée (la vraie est calculée par [`outgoing`]) : un
    // brouillon peut n'avoir aucun destinataire.
    let envelope =
        Envelope::new(Some(from.email.clone()), vec![from.email.clone()]).map_err(mime_error)?;
    let date =
        SystemTime::UNIX_EPOCH + Duration::from_secs(u64::try_from(c.date).unwrap_or_default());
    let mut builder = Message::builder()
        .from(from)
        .subject(d.subject.clone())
        .date(date)
        .message_id(Some(format!("<{}>", c.message_id)))
        .envelope(envelope);
    if let Some(to) = mailboxes(&d.to)? {
        builder = builder.header(header::To::from(to));
    }
    if let Some(cc) = mailboxes(&d.cc)? {
        builder = builder.header(header::Cc::from(cc));
    }
    if bcc == BccHeader::Keep
        && let Some(list) = mailboxes(&d.bcc)?
    {
        builder = builder.header(header::Bcc::from(list)).keep_bcc();
    }
    if let Some(parent) = &d.threading.in_reply_to {
        builder = builder.in_reply_to(format!("<{parent}>"));
    }
    if !d.threading.references.is_empty() {
        let refs: Vec<String> = d
            .threading
            .references
            .iter()
            .map(|r| format!("<{r}>"))
            .collect();
        builder = builder.references(refs.join(" "));
    }

    // Corps : texte seul, ou alternative texte + HTML réassaini.
    let body = match &d.html {
        Some(html) => {
            let clean = sanitize_composed_html(html);
            let text = html_to_text(&clean);
            Body::Alternative(
                MultiPart::alternative()
                    .singlepart(SinglePart::plain(text))
                    .singlepart(SinglePart::html(clean)),
            )
        }
        None => Body::Plain(SinglePart::plain(d.text.clone())),
    };
    let message = if c.attachments.is_empty() {
        match body {
            Body::Plain(part) => builder.singlepart(part),
            Body::Alternative(part) => builder.multipart(part),
        }
    } else {
        let mut mixed = match body {
            Body::Plain(part) => MultiPart::mixed().singlepart(part),
            Body::Alternative(part) => MultiPart::mixed().multipart(part),
        };
        for a in &c.attachments {
            let content_type = ContentType::parse(&a.content_type)
                .or_else(|_| ContentType::parse("application/octet-stream"))
                .map_err(mime_error)?;
            mixed = mixed.singlepart(
                Attachment::new(safe_file_name(&a.file_name)).body(a.bytes.clone(), content_type),
            );
        }
        builder.multipart(mixed)
    }
    .map_err(mime_error)?;
    Ok(message.formatted())
}

enum Body {
    Plain(SinglePart),
    Alternative(MultiPart),
}

/// Enveloppe SMTP et message transmis (sans `Bcc`) : tous les
/// destinataires, Cci compris, sont dans l'enveloppe.
pub fn outgoing(c: &Composition) -> Result<OutgoingMessage, BuildError> {
    let d = &c.draft;
    Ok(OutgoingMessage {
        from: c.from.address.to_string(),
        recipients: d
            .to
            .iter()
            .chain(&d.cc)
            .chain(&d.bcc)
            .map(|r| r.address.to_string())
            .collect(),
        raw: build_message(c, BccHeader::Omit)?,
    })
}

fn pairs(list: &[(String, String)]) -> Vec<(&str, &str)> {
    list.iter().map(|(n, a)| (n.as_str(), a.as_str())).collect()
}

/// Implémentation du port [`MessageComposer`].
#[derive(Debug, Clone, Copy, Default)]
pub struct MimeComposer;

impl MessageComposer for MimeComposer {
    fn new_message_id(&self, from: &EmailAddress) -> Result<String, ComposeBuildError> {
        new_message_id(from)
    }

    fn compose(&self, composition: &Composition) -> Result<ComposedMessage, ComposeBuildError> {
        Ok(ComposedMessage {
            outgoing: outgoing(composition)?,
            local_copy: build_message(composition, BccHeader::Keep)?,
        })
    }

    fn reply_source(&self, raw: &[u8]) -> Option<ReplySource> {
        let message = MessageParser::default().parse(raw)?;
        let people = |value: Option<&mail_parser::Address<'_>>| -> Vec<(String, String)> {
            value
                .map(|a| {
                    a.iter()
                        .map(|p| {
                            (
                                p.name().unwrap_or_default().to_owned(),
                                p.address().unwrap_or_default().to_owned(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let ids = |value: &HeaderValue<'_>| -> Vec<String> {
            match value {
                HeaderValue::Text(t) => vec![t.to_string()],
                HeaderValue::TextList(list) => list.iter().map(ToString::to_string).collect(),
                _ => Vec::new(),
            }
        };
        let (from, to, cc) = (
            people(message.from()),
            people(message.to()),
            people(message.cc()),
        );
        let (in_reply_to, references) = (ids(message.in_reply_to()), ids(message.references()));

        let headers = MessageHeaders::sanitize(&RawHeaders {
            subject: message.subject().unwrap_or_default(),
            from: pairs(&from),
            to: pairs(&to),
            cc: pairs(&cc),
            date: None,
            message_id: message.message_id(),
            in_reply_to: in_reply_to.iter().map(String::as_str).collect(),
            references: references.iter().map(String::as_str).collect(),
        });
        let reply_to = people(message.reply_to())
            .iter()
            .take(MAX_ADDRESSES)
            .map(|(n, a)| Correspondent::new(n, a))
            .collect();
        // Message HTML seul : texte dérivé du HTML réassaini (sans script
        // ni style) ; sinon sa partie texte.
        let html_only = !message.html_body.is_empty() && message.text_body == message.html_body;
        let mut body = if html_only {
            message
                .body_html(0)
                .map(|h| html_to_text(&h))
                .unwrap_or_default()
        } else {
            message
                .body_text(0)
                .map(|t| t.into_owned())
                .unwrap_or_default()
        };
        if body.len() > MAX_QUOTED_BYTES {
            let mut cut = MAX_QUOTED_BYTES;
            while !body.is_char_boundary(cut) {
                cut -= 1;
            }
            body.truncate(cut);
        }
        Some(ReplySource {
            subject: headers.subject,
            from: headers.from,
            to: headers.to,
            cc: headers.cc,
            reply_to,
            message_id: headers.message_id,
            in_reply_to: headers.in_reply_to,
            references: headers.references,
            body,
        })
    }

    fn quote_html(&self, text: &str, attribution: &str) -> String {
        let mut html = format!("<p>{}</p><blockquote>", escape_html(attribution));
        let normalized = text.replace("\r\n", "\n");
        for paragraph in normalized.split("\n\n").filter(|p| !p.trim().is_empty()) {
            let lines: Vec<String> = paragraph
                .trim_matches('\n')
                .split('\n')
                .map(escape_html)
                .collect();
            html.push_str("<p>");
            html.push_str(&lines.join("<br>"));
            html.push_str("</p>");
        }
        html.push_str("</blockquote>");
        html
    }
}

#[cfg(test)]
mod tests;
