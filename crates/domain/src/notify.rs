//! Nouveaux messages à signaler (notifications natives).
//!
//! - Seuls les messages **non lus** comptent : un message déjà lu ailleurs
//!   (autre client) n'est pas signalé.
//! - Rien n'est signalé à la **première synchronisation** d'une boîte (toute
//!   la boîte arriverait d'un coup).
//! - Les arrivées d'une synchronisation sont **regroupées** : un nombre et,
//!   au plus, le message le plus récent (expéditeur, sujet).
//! - Avec l'option de confidentialité, le contenu est retiré en Rust
//!   ([`NewMail::without_content`]) avant d'atteindre l'interface.
//!
//! Le texte de la notification est composé par l'interface, dans la langue
//! de l'utilisateur (ADR-011).

use crate::mailbox::{Correspondent, MessageMeta};

/// Message le plus récent d'un groupe d'arrivées.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrival {
    pub from: Option<Correspondent>,
    pub subject: String,
    pub date: i64,
}

/// Arrivées regroupées d'une boîte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMail {
    /// Nombre de messages non lus arrivés (au moins 1).
    pub count: u32,
    /// Le plus récent ; `None` sans contenu (confidentialité).
    pub latest: Option<Arrival>,
}

/// Arrivées à signaler parmi les messages `new` d'une synchronisation ;
/// `first_sync` : la boîte n'avait encore jamais été synchronisée.
#[must_use]
pub fn new_mail(first_sync: bool, new: &[MessageMeta]) -> Option<NewMail> {
    if first_sync {
        return None;
    }
    let unread: Vec<&MessageMeta> = new.iter().filter(|m| !m.flags.seen).collect();
    let latest = unread.iter().max_by_key(|m| m.sort_date())?;
    Some(NewMail {
        count: u32::try_from(unread.len()).unwrap_or(u32::MAX),
        latest: Some(Arrival {
            from: latest.headers.from.first().cloned(),
            subject: latest.headers.subject.clone(),
            date: latest.sort_date(),
        }),
    })
}

impl NewMail {
    /// Regroupe deux lots d'arrivées (synchronisation par lots).
    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        let latest = match (self.latest, other.latest) {
            (Some(a), Some(b)) => Some(if b.date > a.date { b } else { a }),
            (a, b) => a.or(b),
        };
        Self {
            count: self.count.saturating_add(other.count),
            latest,
        }
    }

    /// Sans expéditeur ni sujet (option de confidentialité).
    #[must_use]
    pub fn without_content(self) -> Self {
        Self {
            latest: None,
            ..self
        }
    }
}

#[cfg(test)]
mod tests;
