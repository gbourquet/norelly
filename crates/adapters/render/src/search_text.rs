//! Texte cherchable d'un message (port
//! [`norelly_domain::ports::SearchTextExtractor`]) : partie texte, sinon
//! HTML réduit au texte après réassainissement ; borné.

use mail_parser::MessageParser;
use norelly_domain::fetch_policy::MimePart;
use norelly_domain::ports::{FetchedBody, MAX_INDEXED_TEXT_CHARS, SearchTextExtractor};

use crate::compose::html_to_text;
use crate::mime::{MAX_PARSE_LEN, Source, parse_body};
use crate::sanitize::MAX_HTML_LEN;

/// Implémentation du port avec `mail-parser`.
#[derive(Debug, Clone, Copy, Default)]
pub struct MimeTextExtractor;

impl SearchTextExtractor for MimeTextExtractor {
    fn searchable_text(&self, body: &FetchedBody, structure: Option<&MimePart>) -> String {
        let text = match body {
            FetchedBody::Full(raw) => full_text(raw),
            FetchedBody::Partial { header, parts } => {
                let parsed = parse_body(
                    &Source::Partial {
                        header,
                        parts: parts
                            .iter()
                            .map(|(p, b)| (p.clone(), b.as_slice()))
                            .collect(),
                        deferred: Vec::new(),
                    },
                    structure,
                );
                parsed
                    .text
                    .or_else(|| parsed.html.as_deref().map(html_text))
                    .unwrap_or_default()
            }
        };
        text.chars().take(MAX_INDEXED_TEXT_CHARS).collect()
    }
}

/// Texte d'un message complet : ses parties texte ; pour un message HTML
/// seul, son HTML réduit au texte.
fn full_text(raw: &[u8]) -> String {
    if raw.len() > MAX_PARSE_LEN {
        return String::new();
    }
    let Some(message) = MessageParser::default().parse(raw) else {
        return String::new();
    };
    let html_only = !message.html_body.is_empty() && message.text_body == message.html_body;
    if html_only {
        return message
            .body_html(0)
            .map(|h| html_text(&h))
            .unwrap_or_default();
    }
    (0..message.text_body_count())
        .filter_map(|i| message.body_text(i))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// HTML reçu réduit au texte (réassaini d'abord : ni script ni style),
/// borné comme pour la lecture (coût linéaire et limité).
fn html_text(html: &str) -> String {
    let mut cut = html.len().min(MAX_HTML_LEN);
    while !html.is_char_boundary(cut) {
        cut -= 1;
    }
    html_to_text(html.get(..cut).unwrap_or_default())
}

#[cfg(test)]
mod tests;
