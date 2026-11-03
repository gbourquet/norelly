//! Rédaction (phase 7) : réponses, transferts, citation, brouillons et
//! file d'envoi.
//!
//! - [`reply_subject`] et [`forward_subject`] produisent toujours `Re:` et
//!   `Fwd:` (ADR-011), sans empiler les préfixes : ceux reconnus par
//!   [`crate::conversation`] (`RE :`, `AW:`, `TR:`, `SV:`, `Antw:`…) sont
//!   retirés d'abord.
//! - [`reply_recipients`] calcule les destinataires de « répondre » et
//!   « répondre à tous » : `Reply-To` prioritaire, ses propres adresses
//!   exclues, doublons retirés, adresses invalides ignorées.
//! - [`reply_threading`] produit `In-Reply-To` et `References`.
//! - [`quote`] cite un texte ; l'en-tête de citation (« Le … a écrit : »)
//!   est construit par l'interface dans la langue de l'utilisateur et
//!   reçu ici comme une donnée.

pub mod draft;
pub mod outbox;

use crate::address::EmailAddress;
use crate::conversation::strip_prefix;
use crate::mailbox::Correspondent;

/// Préfixe produit pour une réponse.
pub const REPLY_PREFIX: &str = "Re: ";
/// Préfixe produit pour un transfert.
pub const FORWARD_PREFIX: &str = "Fwd: ";
/// Nombre maximal d'identifiants gardés dans `References` : le premier
/// (racine de la conversation) et les plus récents.
pub const MAX_REFERENCES: usize = 20;

/// Sujet sans ses préfixes de réponse ou de transfert de tête.
#[must_use]
pub fn strip_reply_prefixes(subject: &str) -> &str {
    // Chaque tour consomme au moins un préfixe : coût linéaire, sans
    // plafond (un plafond rendrait `reply_subject` non idempotent).
    let mut rest = subject.trim();
    while let Some(after) = strip_prefix(rest) {
        rest = after.trim();
    }
    rest
}

/// Sujet d'une réponse : `Re: ` suivi du sujet sans préfixes.
#[must_use]
pub fn reply_subject(subject: &str) -> String {
    format!("{REPLY_PREFIX}{}", strip_reply_prefixes(subject))
}

/// Sujet d'un transfert : `Fwd: ` suivi du sujet sans préfixes.
#[must_use]
pub fn forward_subject(subject: &str) -> String {
    format!("{FORWARD_PREFIX}{}", strip_reply_prefixes(subject))
}

/// Message auquel on répond (ou que l'on transfère), tel que relu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplySource {
    pub subject: String,
    pub from: Vec<Correspondent>,
    pub to: Vec<Correspondent>,
    pub cc: Vec<Correspondent>,
    pub reply_to: Vec<Correspondent>,
    pub message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    /// Texte du message (sa meilleure version texte), pour la citation.
    pub body: String,
}

/// Nature d'une rédaction à partir d'un message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    Reply,
    ReplyAll,
    Forward,
}

/// Destinataires proposés.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recipients {
    pub to: Vec<Correspondent>,
    pub cc: Vec<Correspondent>,
}

/// Destinataires d'une réponse. `own` : adresses de l'utilisateur
/// (comptes), jamais reprises, sauf pour répondre à son propre message
/// (on répond alors à ses destinataires).
#[must_use]
pub fn reply_recipients(source: &ReplySource, kind: ReplyKind, own: &[EmailAddress]) -> Recipients {
    let own: Vec<String> = own.iter().map(|a| a.to_string().to_lowercase()).collect();
    let is_own = |c: &Correspondent| address_key(c).is_some_and(|k| own.contains(&k));
    let from_self = source.from.iter().any(is_own);
    // Destinataires principaux : ceux de son propre message, sinon
    // `Reply-To`, sinon l'expéditeur.
    let primary: &[Correspondent] = if from_self {
        &source.to
    } else if source.reply_to.is_empty() {
        &source.from
    } else {
        &source.reply_to
    };
    let (to, cc): (Vec<&Correspondent>, Vec<&Correspondent>) = match kind {
        ReplyKind::Forward => return Recipients::default(),
        ReplyKind::Reply => (primary.iter().collect(), Vec::new()),
        ReplyKind::ReplyAll => {
            let mut to: Vec<&Correspondent> = primary.iter().collect();
            if !from_self {
                to.extend(&source.to);
            }
            (to, source.cc.iter().collect())
        }
    };
    let mut seen: Vec<String> = own;
    let mut keep = |list: Vec<&Correspondent>| -> Vec<Correspondent> {
        let mut out = Vec::new();
        for c in list {
            // Adresse invalide : impossible d'y écrire, ignorée.
            let Some(key) = address_key(c) else { continue };
            if !seen.contains(&key) {
                seen.push(key);
                out.push(c.clone());
            }
        }
        out
    };
    let to = keep(to);
    let cc = keep(cc);
    Recipients { to, cc }
}

/// Clé de comparaison d'une adresse (valide) : sans distinction de casse.
fn address_key(c: &Correspondent) -> Option<String> {
    EmailAddress::parse(&c.address)
        .ok()
        .map(|a| a.to_string().to_lowercase())
}

/// En-têtes de fil d'une réponse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Threading {
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

/// `In-Reply-To` et `References` d'une réponse (RFC 5322 § 3.6.4).
#[must_use]
pub fn reply_threading(source: &ReplySource) -> Threading {
    let parents = if source.references.is_empty() {
        &source.in_reply_to
    } else {
        &source.references
    };
    let mut references: Vec<String> = Vec::new();
    for id in parents.iter().chain(&source.message_id) {
        if !references.contains(id) {
            references.push(id.clone());
        }
    }
    if references.len() > MAX_REFERENCES {
        // La racine, puis les plus récents.
        let tail = references.split_off(references.len() - (MAX_REFERENCES - 1));
        references.truncate(1);
        references.extend(tail);
    }
    Threading {
        in_reply_to: source.message_id.clone(),
        references,
    }
}

/// Cite `body` sous l'en-tête `attribution` : chaque ligne est précédée
/// de `> ` (`>` seul pour une ligne vide).
#[must_use]
pub fn quote(body: &str, attribution: &str) -> String {
    let mut out = String::with_capacity(body.len() + body.len() / 8 + attribution.len() + 2);
    out.push_str(attribution);
    out.push('\n');
    for line in body.trim_end_matches(['\r', '\n']).split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        // Ligne déjà citée ou vide : `>` collé, sans espace.
        if line.starts_with('>') || line.is_empty() {
            out.push('>');
        } else {
            out.push_str("> ");
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests;
