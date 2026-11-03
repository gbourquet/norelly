//! Brouillon : contenu, validation avant envoi et cycle de vie.
//!
//! Cycle de vie :
//! 1. l'interface enregistre le brouillon localement à chaque pause de
//!    frappe (sauvegarde locale, peu coûteuse) : chaque enregistrement
//!    incrémente sa révision ([`DraftSync::edited`]) ;
//! 2. la copie sur le serveur (`APPEND` dans « Brouillons », qui remplace
//!    l'ancienne) est périodique : [`DraftSync::remote_save_due`] ne la
//!    demande que si la révision a changé et au plus toutes les
//!    [`REMOTE_SAVE_INTERVAL_SECS`] secondes ;
//! 3. à l'envoi, le brouillon passe dans la file d'envoi
//!    ([`super::outbox`]) après [`Draft::check_sendable`] ; sa copie sur
//!    le serveur est retirée une fois le message envoyé ;
//! 4. abandonné, il est supprimé localement et sur le serveur.

use crate::account::AccountId;
use crate::address::EmailAddress;
use crate::mailbox::{MAX_ADDRESS_CHARS, Uid, UidValidity, header_text};
use crate::ports::BlobId;

use super::Threading;

/// Identifiant de message utilisable dans un en-tête : ASCII imprimable,
/// sans espace ni chevron, borné.
fn valid_message_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_MESSAGE_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'<' && b != b'>')
}

/// Destinataires au plus, tous champs confondus.
pub const MAX_RECIPIENTS: usize = 100;
/// Taille cumulée maximale des pièces jointes (25 Mio, limite usuelle des
/// serveurs).
pub const MAX_ATTACHMENTS_BYTES: u64 = 25 * 1024 * 1024;
/// Taille maximale du corps (texte et HTML, chacun).
pub const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
/// Caractères au plus dans le sujet.
pub const MAX_SUBJECT_CHARS: usize = 998;
/// Longueur maximale d'un identifiant de message (`In-Reply-To`,
/// `References`).
pub const MAX_MESSAGE_ID_LEN: usize = 250;
/// Intervalle minimal entre deux copies d'un brouillon sur le serveur.
pub const REMOTE_SAVE_INTERVAL_SECS: i64 = 30;

/// Destinataire validé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    /// Nom affiché, assaini (sans retour à la ligne ni caractère de
    /// contrôle).
    pub name: String,
    pub address: EmailAddress,
}

impl Recipient {
    #[must_use]
    pub fn new(name: &str, address: EmailAddress) -> Self {
        Self {
            name: header_text(name, MAX_ADDRESS_CHARS),
            address,
        }
    }
}

/// Pièce jointe d'un brouillon : contenu dans le stockage de blobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftAttachment {
    pub blob: BlobId,
    /// Nom de fichier proposé (assaini à la construction du MIME).
    pub file_name: String,
    pub content_type: String,
    pub size: u64,
}

/// Contenu d'un brouillon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub account: AccountId,
    pub to: Vec<Recipient>,
    pub cc: Vec<Recipient>,
    pub bcc: Vec<Recipient>,
    pub subject: String,
    /// Version texte (toujours présente : `multipart/alternative`).
    pub text: String,
    /// HTML produit par l'éditeur : non fiable, réassaini avant l'envoi.
    pub html: Option<String>,
    pub threading: Threading,
    pub attachments: Vec<DraftAttachment>,
}

/// Brouillon impossible à envoyer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ComposeError {
    #[error("aucun destinataire")]
    NoRecipient,
    #[error("trop de destinataires")]
    TooManyRecipients,
    #[error("sujet invalide (caractère de contrôle ou trop long)")]
    InvalidSubject,
    #[error("corps trop volumineux")]
    BodyTooLarge,
    #[error("pièces jointes trop volumineuses")]
    AttachmentsTooLarge,
    #[error("identifiant de message invalide (In-Reply-To, References)")]
    InvalidThreading,
}

impl Draft {
    /// Brouillon vide d'un compte.
    #[must_use]
    pub fn empty(account: AccountId) -> Self {
        Self {
            account,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: String::new(),
            text: String::new(),
            html: None,
            threading: Threading::default(),
            attachments: Vec::new(),
        }
    }

    /// Vérifie que le brouillon peut partir : au moins un destinataire,
    /// au plus [`MAX_RECIPIENTS`], sujet sans caractère de contrôle (pas
    /// d'injection d'en-tête), tailles bornées.
    pub fn check_sendable(&self) -> Result<(), ComposeError> {
        self.check_storable()?;
        let count = self.to.len() + self.cc.len() + self.bcc.len();
        if count == 0 {
            return Err(ComposeError::NoRecipient);
        }
        if count > MAX_RECIPIENTS {
            return Err(ComposeError::TooManyRecipients);
        }
        Ok(())
    }

    /// Le brouillon peut-il être enregistré (même incomplet) : seules les
    /// tailles sont bornées.
    pub fn check_storable(&self) -> Result<(), ComposeError> {
        if self.subject.chars().count() > MAX_SUBJECT_CHARS
            || self.subject.chars().any(char::is_control)
        {
            return Err(ComposeError::InvalidSubject);
        }
        if self.text.len() > MAX_BODY_BYTES
            || self.html.as_ref().is_some_and(|h| h.len() > MAX_BODY_BYTES)
        {
            return Err(ComposeError::BodyTooLarge);
        }
        let threading = &self.threading;
        if threading.references.len() > super::MAX_REFERENCES
            || !threading
                .in_reply_to
                .iter()
                .chain(&threading.references)
                .all(|id| valid_message_id(id))
        {
            return Err(ComposeError::InvalidThreading);
        }
        let total = self
            .attachments
            .iter()
            .try_fold(0_u64, |acc, a| acc.checked_add(a.size));
        if total.is_none_or(|t| t > MAX_ATTACHMENTS_BYTES) {
            return Err(ComposeError::AttachmentsTooLarge);
        }
        Ok(())
    }
}

/// Message déposé sur le serveur (`APPENDUID`) : l'UID n'a de sens
/// qu'avec l'UIDVALIDITY de sa boîte. Avant de supprimer une ancienne
/// copie, on vérifie que la boîte a toujours cette UIDVALIDITY (sinon l'UID
/// désignerait un autre message).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RemoteCopy {
    pub validity: UidValidity,
    pub uid: Uid,
}

/// Les brouillons sont-ils copiés sur ce serveur ? Seulement avec UIDPLUS :
/// sans `APPENDUID`, l'ancienne copie ne pourrait pas être retrouvée ni
/// remplacée, et les copies s'accumuleraient.
#[must_use]
pub fn remote_drafts_supported(imap: &crate::capabilities::ServerCapabilities) -> bool {
    imap.has("UIDPLUS")
}

/// État de la copie d'un brouillon sur le serveur.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DraftSync {
    /// Révision locale (incrémentée à chaque enregistrement).
    pub revision: u64,
    /// Révision copiée sur le serveur.
    pub remote_revision: Option<u64>,
    /// Copie dans « Brouillons » (à remplacer puis supprimer).
    pub remote: Option<RemoteCopy>,
    /// Instant de la dernière copie (secondes Unix).
    pub last_remote_save: Option<i64>,
}

impl DraftSync {
    /// Nouvel enregistrement local.
    #[must_use]
    pub fn edited(self) -> Self {
        Self {
            revision: self.revision.saturating_add(1),
            ..self
        }
    }

    /// Faut-il copier le brouillon sur le serveur à l'instant `now` ?
    #[must_use]
    pub fn remote_save_due(&self, now: i64) -> bool {
        let changed = self.revision > 0 && self.remote_revision != Some(self.revision);
        changed
            && self
                .last_remote_save
                .is_none_or(|last| now.saturating_sub(last) >= REMOTE_SAVE_INTERVAL_SECS)
    }

    /// Copie de la révision `revision` faite à `now` ; `copy` : la nouvelle
    /// copie (UIDPLUS `APPENDUID`), si connue. Renvoie l'état et l'ancienne
    /// copie, à supprimer.
    #[must_use]
    pub fn remote_saved(
        self,
        revision: u64,
        copy: Option<RemoteCopy>,
        now: i64,
    ) -> (Self, Option<RemoteCopy>) {
        let next = Self {
            revision: self.revision,
            remote_revision: Some(revision),
            remote: copy,
            last_remote_save: Some(now),
        };
        (next, self.remote.filter(|old| Some(*old) != copy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> AccountId {
        AccountId::new(1).unwrap_or_else(|_| panic!("compte"))
    }

    fn to(address: &str) -> Recipient {
        Recipient::new(
            "Nom",
            EmailAddress::parse(address).unwrap_or_else(|_| panic!("{address}")),
        )
    }

    fn uid(n: u32) -> RemoteCopy {
        RemoteCopy {
            validity: UidValidity::new(7).unwrap_or_else(|| panic!("validité")),
            uid: Uid::new(n).unwrap_or_else(|| panic!("uid")),
        }
    }

    fn sendable() -> Draft {
        let mut d = Draft::empty(account());
        d.to.push(to("bob@example.com"));
        d.subject = "Bonjour".into();
        d.text = "Texte".into();
        d
    }

    fn attachment(size: u64) -> DraftAttachment {
        DraftAttachment {
            blob: BlobId::parse(&"a".repeat(64)).unwrap_or_else(|| panic!("blob")),
            file_name: "a.pdf".into(),
            content_type: "application/pdf".into(),
            size,
        }
    }

    #[test]
    fn recipient_names_are_sanitized() {
        let r = Recipient::new(
            "Bob\r\nBcc: pirate@x",
            EmailAddress::parse("b@example.com").unwrap_or_else(|_| panic!()),
        );
        assert!(!r.name.contains(['\r', '\n']), "{:?}", r.name);
    }

    #[test]
    fn a_draft_needs_a_recipient_to_be_sent() {
        assert_eq!(sendable().check_sendable(), Ok(()));
        let mut d = sendable();
        d.to.clear();
        assert_eq!(d.check_sendable(), Err(ComposeError::NoRecipient));
        assert_eq!(d.check_storable(), Ok(()), "incomplet mais enregistrable");
        d.bcc.push(to("cache@example.com"));
        assert_eq!(d.check_sendable(), Ok(()), "Cci seul suffit");
    }

    #[test]
    fn recipients_are_bounded() {
        let mut d = sendable();
        d.cc = (0..MAX_RECIPIENTS)
            .map(|n| to(&format!("c{n}@example.com")))
            .collect();
        assert_eq!(d.check_sendable(), Err(ComposeError::TooManyRecipients));
        d.cc.pop();
        assert_eq!(d.check_sendable(), Ok(()));
    }

    #[test]
    fn subject_cannot_inject_headers() {
        let mut d = sendable();
        d.subject = "Salut\r\nBcc: pirate@example.com".into();
        assert_eq!(d.check_sendable(), Err(ComposeError::InvalidSubject));
        assert_eq!(d.check_storable(), Err(ComposeError::InvalidSubject));
        d.subject = "x".repeat(MAX_SUBJECT_CHARS + 1);
        assert_eq!(d.check_sendable(), Err(ComposeError::InvalidSubject));
        d.subject = "é".repeat(MAX_SUBJECT_CHARS);
        assert_eq!(d.check_sendable(), Ok(()));
    }

    #[test]
    fn threading_identifiers_cannot_inject_headers() {
        let mut d = sendable();
        d.threading = Threading {
            in_reply_to: Some("m1@example.com".into()),
            references: vec!["m0@example.com".into(), "m1@example.com".into()],
        };
        assert_eq!(d.check_storable(), Ok(()));
        for bad in [
            "a\r\nBcc: pirate@example.com",
            "a b@example.com",
            "<a@example.com>",
            "",
            "é@example.com",
        ] {
            let mut d = sendable();
            d.threading.in_reply_to = Some(bad.into());
            assert_eq!(
                d.check_storable(),
                Err(ComposeError::InvalidThreading),
                "{bad:?}"
            );
            let mut d = sendable();
            d.threading.references = vec![bad.into()];
            assert_eq!(
                d.check_storable(),
                Err(ComposeError::InvalidThreading),
                "{bad:?}"
            );
        }
        let mut d = sendable();
        d.threading.in_reply_to = Some("x".repeat(MAX_MESSAGE_ID_LEN + 1));
        assert_eq!(d.check_storable(), Err(ComposeError::InvalidThreading));
        let mut d = sendable();
        d.threading.references = (0..=crate::compose::MAX_REFERENCES)
            .map(|n| format!("r{n}@x"))
            .collect();
        assert_eq!(d.check_storable(), Err(ComposeError::InvalidThreading));
    }

    #[test]
    fn sizes_are_bounded() {
        let mut d = sendable();
        d.attachments = vec![attachment(MAX_ATTACHMENTS_BYTES - 1), attachment(1)];
        assert_eq!(d.check_sendable(), Ok(()));
        d.attachments.push(attachment(1));
        assert_eq!(d.check_sendable(), Err(ComposeError::AttachmentsTooLarge));
        d.attachments = vec![attachment(u64::MAX), attachment(u64::MAX)];
        assert_eq!(d.check_storable(), Err(ComposeError::AttachmentsTooLarge));
        let mut d = sendable();
        d.html = Some("x".repeat(MAX_BODY_BYTES + 1));
        assert_eq!(d.check_storable(), Err(ComposeError::BodyTooLarge));
        d.html = None;
        d.text = "x".repeat(MAX_BODY_BYTES + 1);
        assert_eq!(d.check_sendable(), Err(ComposeError::BodyTooLarge));
    }

    #[test]
    fn remote_drafts_need_uidplus() {
        let caps =
            |t: &[&str]| crate::capabilities::ServerCapabilities::from_imap(t.iter().copied());
        assert!(remote_drafts_supported(&caps(&["IMAP4rev1", "UIDPLUS"])));
        assert!(!remote_drafts_supported(&caps(&["IMAP4rev1"])));
    }

    #[test]
    fn remote_copy_is_periodic_and_only_when_changed() {
        let s = DraftSync::default();
        assert!(!s.remote_save_due(0), "rien à copier");
        let s = s.edited();
        assert_eq!(s.revision, 1);
        assert!(s.remote_save_due(0), "première copie aussitôt");
        let (s, old) = s.remote_saved(1, Some(uid(10)), 100);
        assert_eq!(old, None);
        assert_eq!(s.remote, Some(uid(10)));
        assert!(!s.remote_save_due(200), "inchangé");
        let s = s.edited();
        assert!(
            !s.remote_save_due(100 + REMOTE_SAVE_INTERVAL_SECS - 1),
            "trop tôt"
        );
        assert!(s.remote_save_due(100 + REMOTE_SAVE_INTERVAL_SECS));
        let (s, old) = s.remote_saved(2, Some(uid(11)), 200);
        assert_eq!(old, Some(uid(10)), "ancienne copie à supprimer");
        assert_eq!((s.remote_revision, s.remote), (Some(2), Some(uid(11))));
    }

    #[test]
    fn a_stale_remote_save_does_not_hide_newer_edits() {
        // Copie de la révision 1 terminée alors que la 2 existe déjà.
        let s = DraftSync::default().edited().edited();
        let (s, _) = s.remote_saved(1, None, 100);
        assert_eq!(s.revision, 2);
        assert_eq!(s.remote_revision, Some(1));
        assert!(s.remote_save_due(100 + REMOTE_SAVE_INTERVAL_SECS));
        // Sans APPENDUID : l'ancienne copie reste à supprimer plus tard.
        let s = DraftSync {
            remote: Some(uid(5)),
            ..DraftSync::default()
        }
        .edited();
        let (s, old) = s.remote_saved(1, None, 0);
        assert_eq!((s.remote, old), (None, Some(uid(5))));
    }
}
