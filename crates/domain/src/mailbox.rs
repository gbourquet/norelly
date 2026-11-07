//! Boîtes aux lettres et messages : identifiants IMAP (`Uid`,
//! `UidValidity`, `ModSeq`), drapeaux, rôle d'une boîte (SPECIAL-USE ou
//! heuristique de nom) et métadonnées d'un message.
//!
//! Tout ce qui vient du serveur est hostile par nature : les textes sont
//! assainis (caractères de contrôle et de mise en forme retirés, longueur
//! bornée) et les mots-clés validés avant d'entrer dans le domaine.

use std::collections::BTreeSet;
use std::fmt;
use std::num::{NonZeroU32, NonZeroU64};

use serde::{Deserialize, Serialize};

use crate::connection::is_format_char;
use crate::fetch_policy::MimePart;
use crate::ports::FolderNode;

// ---------------------------------------------------------------------------
// Identifiants IMAP
// ---------------------------------------------------------------------------

/// UID d'un message dans une boîte (RFC 3501, 2.3.1.1) : strictement
/// positif, croissant, valable tant que l'UIDVALIDITY ne change pas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Uid(NonZeroU32);

impl Uid {
    #[must_use]
    pub fn new(raw: u32) -> Option<Self> {
        NonZeroU32::new(raw).map(Self)
    }

    #[must_use]
    pub const fn from_nonzero(raw: NonZeroU32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }

    /// UID suivant, `None` au-delà de `u32::MAX`.
    #[must_use]
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

impl fmt::Display for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// UIDVALIDITY d'une boîte : si elle change, tous les UID connus sont
/// caducs (resynchronisation complète).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UidValidity(NonZeroU32);

impl UidValidity {
    #[must_use]
    pub fn new(raw: u32) -> Option<Self> {
        NonZeroU32::new(raw).map(Self)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl fmt::Display for UidValidity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Numéro de modification (CONDSTORE, RFC 7162) : strictement positif.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ModSeq(NonZeroU64);

impl ModSeq {
    /// Valeur maximale d'un mod-sequence (RFC 7162 : 63 bits).
    pub const MAX: u64 = (1 << 63) - 1;

    /// `None` pour 0 ou au-delà de 2^63 - 1.
    #[must_use]
    pub fn new(raw: u64) -> Option<Self> {
        if raw > Self::MAX {
            return None;
        }
        NonZeroU64::new(raw).map(Self)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Display for ModSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

// ---------------------------------------------------------------------------
// Drapeaux
// ---------------------------------------------------------------------------

/// Longueur maximale d'un mot-clé.
pub const MAX_KEYWORD_LEN: usize = 64;
/// Nombre maximal de mots-clés conservés par message.
pub const MAX_KEYWORDS: usize = 32;

/// Mot-clé IMAP (`$Forwarded`, `$Junk`, `NonJunk`…) : atome ASCII
/// imprimable, sans `\`, borné. Les drapeaux système inconnus (`\Foo`)
/// sont ignorés.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Keyword(String);

impl Keyword {
    /// `None` si le mot-clé n'est pas un atome valide.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let valid = !raw.is_empty()
            && raw.len() <= MAX_KEYWORD_LEN
            && raw.bytes().all(|b| {
                b.is_ascii_graphic()
                    && !matches!(b, b'(' | b')' | b'{' | b'"' | b'\\' | b'%' | b'*' | b']')
            });
        valid.then(|| Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Drapeaux d'un message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Flags {
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub deleted: bool,
    pub draft: bool,
    keywords: BTreeSet<Keyword>,
}

impl Flags {
    /// Drapeaux à partir des jetons du serveur (`\Seen`, `$Forwarded`…),
    /// insensibles à la casse pour les drapeaux système. Jetons invalides
    /// ignorés ; au plus [`MAX_KEYWORDS`] mots-clés.
    pub fn from_tokens<I, S>(tokens: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut flags = Self::default();
        for token in tokens {
            let token = token.as_ref();
            match token.to_ascii_lowercase().as_str() {
                "\\seen" => flags.seen = true,
                "\\answered" => flags.answered = true,
                "\\flagged" => flags.flagged = true,
                "\\deleted" => flags.deleted = true,
                "\\draft" => flags.draft = true,
                // \Recent est une information de session, pas un état.
                _ if token.starts_with('\\') => {}
                _ => {
                    if flags.keywords.len() < MAX_KEYWORDS
                        && let Some(k) = Keyword::parse(token)
                    {
                        flags.keywords.insert(k);
                    }
                }
            }
        }
        flags
    }

    /// Jetons IMAP, dans un ordre stable.
    #[must_use]
    pub fn to_tokens(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (on, name) in [
            (self.seen, "\\Seen"),
            (self.answered, "\\Answered"),
            (self.flagged, "\\Flagged"),
            (self.deleted, "\\Deleted"),
            (self.draft, "\\Draft"),
        ] {
            if on {
                out.push(name.to_owned());
            }
        }
        out.extend(self.keywords.iter().map(|k| k.0.clone()));
        out
    }

    #[must_use]
    pub fn keywords(&self) -> &BTreeSet<Keyword> {
        &self.keywords
    }

    /// Ajoute un mot-clé (sans effet au-delà de [`MAX_KEYWORDS`]).
    pub fn insert_keyword(&mut self, keyword: Keyword) {
        if self.keywords.len() < MAX_KEYWORDS {
            self.keywords.insert(keyword);
        }
    }

    /// Drapeaux système sous forme de masque (stockage compact).
    #[must_use]
    pub fn system_bits(&self) -> u8 {
        u8::from(self.seen)
            | u8::from(self.answered) << 1
            | u8::from(self.flagged) << 2
            | u8::from(self.deleted) << 3
            | u8::from(self.draft) << 4
    }

    /// Inverse de [`Flags::system_bits`] (bits inconnus ignorés), avec des
    /// mots-clés revalidés.
    pub fn from_parts<I, S>(bits: u8, keywords: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut flags = Self {
            seen: bits & 1 != 0,
            answered: bits & 2 != 0,
            flagged: bits & 4 != 0,
            deleted: bits & 8 != 0,
            draft: bits & 16 != 0,
            keywords: BTreeSet::new(),
        };
        for k in keywords {
            if let Some(k) = Keyword::parse(k.as_ref()) {
                flags.insert_keyword(k);
            }
        }
        flags
    }
}

// ---------------------------------------------------------------------------
// Boîtes
// ---------------------------------------------------------------------------

/// Longueur maximale d'un nom de boîte, en octets.
pub const MAX_MAILBOX_NAME_LEN: usize = 1024;

/// Erreur de nom de boîte.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MailboxNameError {
    #[error("nom de boîte vide")]
    Empty,
    #[error("nom de boîte trop long")]
    TooLong,
    #[error("nom de boîte avec un caractère interdit")]
    InvalidCharacter,
}

/// Nom complet d'une boîte (déjà décodé de l'UTF-7 modifié si besoin),
/// avec ses séparateurs de hiérarchie. `INBOX` est normalisé en
/// majuscules (RFC 3501, 5.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MailboxName(String);

impl MailboxName {
    pub fn parse(raw: &str) -> Result<Self, MailboxNameError> {
        if raw.is_empty() {
            return Err(MailboxNameError::Empty);
        }
        if raw.len() > MAX_MAILBOX_NAME_LEN {
            return Err(MailboxNameError::TooLong);
        }
        if raw.chars().any(|c| c.is_control() || is_format_char(c)) {
            return Err(MailboxNameError::InvalidCharacter);
        }
        if raw.eq_ignore_ascii_case("INBOX") {
            return Ok(Self::inbox());
        }
        Ok(Self(raw.to_owned()))
    }

    #[must_use]
    pub fn inbox() -> Self {
        Self("INBOX".to_owned())
    }

    /// Ce nom après le renommage du dossier `from` en `to` : `to` s'il est
    /// `from`, `to` suivi du reste s'il est un de ses sous-dossiers (selon
    /// `delimiter`) ; `None` s'il n'est pas concerné.
    #[must_use]
    pub fn relocated(&self, from: &Self, to: &Self, delimiter: Option<char>) -> Option<Self> {
        if self == from {
            return Some(to.clone());
        }
        let d = delimiter?;
        let rest = self.0.strip_prefix(from.as_str())?.strip_prefix(d)?;
        Self::parse(&format!("{}{d}{rest}", to.as_str())).ok()
    }

    #[must_use]
    pub fn is_inbox(&self) -> bool {
        self.0 == "INBOX"
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Dernier segment du nom (affichage).
    #[must_use]
    pub fn leaf(&self, delimiter: Option<char>) -> &str {
        match delimiter {
            Some(d) => self.0.rsplit(d).next().unwrap_or(&self.0),
            None => &self.0,
        }
    }
}

impl fmt::Display for MailboxName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Rôle d'une boîte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MailboxRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
    /// `\All` (Gmail « Tous les messages »).
    All,
    /// `\Flagged` (dossier virtuel des messages suivis).
    Flagged,
    /// Boîte ordinaire.
    Normal,
}

impl MailboxRole {
    /// Nom stable (stockage).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Sent => "sent",
            Self::Drafts => "drafts",
            Self::Trash => "trash",
            Self::Junk => "junk",
            Self::Archive => "archive",
            Self::All => "all",
            Self::Flagged => "flagged",
            Self::Normal => "normal",
        }
    }

    /// Rang d'affichage dans l'arborescence : boîte de réception,
    /// brouillons, envoyés, archives, indésirables, corbeille, puis
    /// dossiers virtuels et ordinaires.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Inbox => 0,
            Self::Drafts => 1,
            Self::Sent => 2,
            Self::Archive => 3,
            Self::Junk => 4,
            Self::Trash => 5,
            Self::All => 6,
            Self::Flagged => 7,
            Self::Normal => 8,
        }
    }

    /// Inverse de [`MailboxRole::as_str`].
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "inbox" => Self::Inbox,
            "sent" => Self::Sent,
            "drafts" => Self::Drafts,
            "trash" => Self::Trash,
            "junk" => Self::Junk,
            "archive" => Self::Archive,
            "all" => Self::All,
            "flagged" => Self::Flagged,
            "normal" => Self::Normal,
            _ => return None,
        })
    }

    /// Rôle annoncé par un attribut SPECIAL-USE (RFC 6154).
    #[must_use]
    pub fn from_special_use(attribute: &str) -> Option<Self> {
        Some(match attribute.to_ascii_lowercase().as_str() {
            "\\sent" => Self::Sent,
            "\\drafts" => Self::Drafts,
            "\\trash" => Self::Trash,
            "\\junk" => Self::Junk,
            "\\archive" => Self::Archive,
            "\\all" => Self::All,
            "\\flagged" => Self::Flagged,
            _ => return None,
        })
    }

    /// Rôle deviné d'après le dernier segment du nom, pour les serveurs
    /// sans SPECIAL-USE. Noms usuels en anglais, français, allemand,
    /// espagnol et italien ; comparaison insensible à la casse.
    #[must_use]
    pub fn from_name(leaf: &str) -> Option<Self> {
        let lower = leaf.trim().to_lowercase();
        let role = match lower.as_str() {
            "sent"
            | "sent items"
            | "sent mail"
            | "sent messages"
            | "envoyés"
            | "envoyes"
            | "éléments envoyés"
            | "messages envoyés"
            | "gesendet"
            | "gesendete elemente"
            | "enviados"
            | "inviata"
            | "posta inviata" => Self::Sent,
            "drafts" | "draft" | "brouillons" | "entwürfe" | "entwuerfe" | "borradores"
            | "bozze" => Self::Drafts,
            "trash"
            | "deleted"
            | "deleted items"
            | "deleted messages"
            | "bin"
            | "corbeille"
            | "éléments supprimés"
            | "papierkorb"
            | "gelöschte elemente"
            | "papelera"
            | "cestino" => Self::Trash,
            "junk"
            | "spam"
            | "junk e-mail"
            | "junk email"
            | "courrier indésirable"
            | "indésirables"
            | "pourriel"
            | "pourriels"
            | "bulk mail" => Self::Junk,
            "archive" | "archives" | "archiv" | "archivio" | "archivo" => Self::Archive,
            _ => return None,
        };
        Some(role)
    }

    /// Rôle d'une boîte : `INBOX`, puis attributs SPECIAL-USE, puis nom.
    pub fn detect<I, S>(name: &MailboxName, delimiter: Option<char>, attributes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        if name.is_inbox() {
            return Self::Inbox;
        }
        if let Some(role) = attributes
            .into_iter()
            .find_map(|a| Self::from_special_use(a.as_ref()))
        {
            return role;
        }
        // Heuristique de nom pour les boîtes de premier niveau ou
        // directement sous INBOX (Courier, Cyrus : « INBOX.Sent »).
        let depth_ok = match delimiter {
            None => true,
            Some(d) => {
                let parts: Vec<&str> = name.as_str().split(d).collect();
                parts.len() == 1
                    || (parts.len() == 2
                        && parts
                            .first()
                            .is_some_and(|p| p.eq_ignore_ascii_case("INBOX")))
            }
        };
        if depth_ok && let Some(role) = Self::from_name(name.leaf(delimiter)) {
            return role;
        }
        Self::Normal
    }
}

/// Boîte telle que listée par le serveur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMailbox {
    pub name: MailboxName,
    pub delimiter: Option<char>,
    pub role: MailboxRole,
    /// Faux pour `\Noselect` et `\NonExistent`.
    pub selectable: bool,
}

/// Boîte brute renvoyée par `LIST` : nom, séparateur, attributs
/// (`\\Noselect`, `\\Sent`…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedMailbox {
    pub name: MailboxName,
    pub delimiter: Option<char>,
    pub attributes: Vec<String>,
}

impl ListedMailbox {
    fn has_attribute(&self, wanted: &str) -> bool {
        self.attributes
            .iter()
            .any(|a| a.eq_ignore_ascii_case(wanted))
    }

    fn special_use(&self) -> bool {
        self.attributes
            .iter()
            .any(|a| MailboxRole::from_special_use(a).is_some())
    }
}

/// Nombre maximal de boîtes conservées par compte.
pub const MAX_MAILBOXES: usize = 10_000;

/// Classe les boîtes listées : rôle détecté, sélectionnable ou non,
/// doublons de nom retirés, et rôle spécial unique par compte. Si
/// plusieurs boîtes revendiquent un même rôle, celles qui l'annoncent par
/// SPECIAL-USE passent avant celles devinées par leur nom ; à égalité, la
/// première dans l'ordre du serveur l'emporte, les autres deviennent
/// ordinaires.
#[must_use]
pub fn classify_mailboxes(listed: Vec<ListedMailbox>) -> Vec<RemoteMailbox> {
    let mut seen = BTreeSet::new();
    let listed: Vec<ListedMailbox> = listed
        .into_iter()
        .filter(|m| seen.insert(m.name.clone()))
        .take(MAX_MAILBOXES)
        .collect();
    let mut out: Vec<RemoteMailbox> = listed
        .iter()
        .map(|m| RemoteMailbox {
            name: m.name.clone(),
            delimiter: m.delimiter,
            role: MailboxRole::detect(&m.name, m.delimiter, &m.attributes),
            selectable: !m.has_attribute("\\Noselect") && !m.has_attribute("\\NonExistent"),
        })
        .collect();
    let mut taken = BTreeSet::new();
    for pass in [true, false] {
        for (mailbox, raw) in out.iter_mut().zip(&listed) {
            if raw.special_use() != pass || mailbox.role == MailboxRole::Normal {
                continue;
            }
            if !taken.insert(mailbox.role) {
                mailbox.role = MailboxRole::Normal;
            }
        }
    }
    out
}

/// Longueur maximale du nom saisi pour un nouveau dossier, en caractères.
pub const MAX_FOLDER_LABEL_CHARS: usize = 100;

/// Nom de nouveau dossier refusé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NewFolderError {
    #[error("nom de dossier vide")]
    Empty,
    #[error("nom de dossier trop long")]
    TooLong,
    #[error("nom de dossier avec un caractère interdit")]
    InvalidCharacter,
    #[error("nom de dossier avec le séparateur de hiérarchie « {0} »")]
    ContainsDelimiter(char),
    #[error("un dossier de ce nom existe déjà")]
    AlreadyExists,
    #[error("dossier parent inconnu")]
    UnknownParent,
    #[error("le serveur n'accepte pas de sous-dossiers")]
    NoHierarchy,
    #[error("dossier virtuel : rien ne peut y être rangé")]
    VirtualParent,
}

impl From<MailboxNameError> for NewFolderError {
    fn from(e: MailboxNameError) -> Self {
        match e {
            MailboxNameError::Empty => Self::Empty,
            MailboxNameError::TooLong => Self::TooLong,
            MailboxNameError::InvalidCharacter => Self::InvalidCharacter,
        }
    }
}

/// Nom complet d'un nouveau dossier `label`, sous `parent` (ou au premier
/// niveau), d'après les boîtes `existing` du compte.
///
/// Le nom saisi est débarrassé de ses espaces de bord ; il ne peut
/// contenir ni caractère de contrôle ou de mise en forme, ni joker IMAP
/// (`*`, `%`), ni le séparateur de hiérarchie (un nom ne crée qu'un seul
/// niveau). Au premier niveau, si tous les dossiers du compte sont rangés
/// sous la boîte de réception (Courier, Cyrus : « INBOX.Envoyés »), le
/// nouveau dossier l'est aussi. Un nom déjà pris, à la casse près, est
/// refusé : certains serveurs ne distinguent pas la casse. Un dossier
/// virtuel (`\All`, `\Flagged`) ne peut pas servir d'emplacement.
pub fn new_folder_name(
    label: &str,
    parent: Option<&MailboxName>,
    existing: &[RemoteMailbox],
) -> Result<MailboxName, NewFolderError> {
    let label = folder_label(label)?;
    let (prefix, delimiter) = destination(parent, existing)?;
    if let Some(d) = delimiter
        && label.contains(d)
    {
        return Err(NewFolderError::ContainsDelimiter(d));
    }
    let full = MailboxName::parse(&format!("{prefix}{label}")).map_err(NewFolderError::from)?;
    let wanted = full.as_str().to_lowercase();
    if existing
        .iter()
        .any(|m| m.name.as_str().to_lowercase() == wanted)
    {
        return Err(NewFolderError::AlreadyExists);
    }
    Ok(full)
}

/// Nom saisi pour un nouveau dossier, sans ses espaces de bord, vérifié
/// seul (sans la liste des dossiers) : non vide, borné, sans caractère de
/// contrôle ou de mise en forme ni joker IMAP. Voir [`new_folder_name`].
pub fn folder_label(label: &str) -> Result<&str, NewFolderError> {
    let label = label.trim();
    if label.is_empty() {
        return Err(NewFolderError::Empty);
    }
    if label.chars().count() > MAX_FOLDER_LABEL_CHARS {
        return Err(NewFolderError::TooLong);
    }
    if label
        .chars()
        .any(|c| c.is_control() || is_format_char(c) || c == '*' || c == '%')
    {
        return Err(NewFolderError::InvalidCharacter);
    }
    Ok(label)
}

/// Renommage ou suppression d'un dossier refusé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FolderChangeError {
    #[error("dossier inconnu")]
    UnknownFolder,
    #[error("dossier protégé : boîte de réception ou dossier spécial")]
    Protected,
    #[error("le dossier contient des sous-dossiers")]
    HasSubfolders,
    #[error("un dossier ne peut pas aller dans lui-même ni dans un de ses sous-dossiers")]
    IntoItself,
    #[error(transparent)]
    Name(#[from] NewFolderError),
}

/// Nouveau nom complet du dossier `folder` renommé en `label`, à la même
/// place (même parent), d'après les boîtes `existing` du compte.
///
/// Le nom saisi suit les règles d'un nouveau dossier ([`folder_label`],
/// sans séparateur, nom libre à la casse près) ; seule la casse du dossier
/// lui-même peut changer. La boîte de réception et les dossiers spéciaux
/// (envoyés, brouillons, corbeille, indésirables, archives, dossiers
/// virtuels) ne se renomment pas.
pub fn renamed_folder_name(
    folder: &MailboxName,
    label: &str,
    existing: &[RemoteMailbox],
) -> Result<MailboxName, FolderChangeError> {
    let mailbox = changeable(folder, existing)?;
    let label = folder_label(label)?;
    if let Some(d) = mailbox.delimiter
        && label.contains(d)
    {
        return Err(NewFolderError::ContainsDelimiter(d).into());
    }
    let prefix = mailbox
        .delimiter
        .and_then(|d| folder.as_str().rfind(d).map(|i| i + d.len_utf8()))
        .and_then(|end| folder.as_str().get(..end))
        .unwrap_or("");
    let renamed = MailboxName::parse(&format!("{prefix}{label}")).map_err(NewFolderError::from)?;
    let wanted = renamed.as_str().to_lowercase();
    if existing
        .iter()
        .any(|m| m.name != *folder && m.name.as_str().to_lowercase() == wanted)
    {
        return Err(NewFolderError::AlreadyExists.into());
    }
    Ok(renamed)
}

/// Nouveau nom complet du dossier `folder` déplacé sous `parent` (ou au
/// premier niveau), avec son nom actuel, d'après les boîtes `existing` du
/// compte. Ses sous-dossiers le suivent (`RENAME` IMAP).
///
/// L'emplacement suit les règles d'un nouveau dossier ([`new_folder_name`] :
/// parent connu, non virtuel, serveur avec hiérarchie, préfixe « INBOX »
/// au premier niveau s'il y a lieu). Un dossier ne va ni dans lui-même ni
/// dans un de ses sous-dossiers ; un dossier du même nom à l'arrivée, à la
/// casse près, est refusé. Déjà à cet endroit : son nom est renvoyé tel
/// quel. Les dossiers protégés (voir [`renamed_folder_name`]) ne se
/// déplacent pas.
pub fn moved_folder_name(
    folder: &MailboxName,
    parent: Option<&MailboxName>,
    existing: &[RemoteMailbox],
) -> Result<MailboxName, FolderChangeError> {
    let mailbox = changeable(folder, existing)?;
    if let Some(parent) = parent
        && is_same_or_under(parent, folder, mailbox.delimiter)
    {
        return Err(FolderChangeError::IntoItself);
    }
    let (prefix, delimiter) = destination(parent, existing)?;
    let leaf = folder.leaf(mailbox.delimiter);
    if let Some(d) = delimiter
        && leaf.contains(d)
    {
        return Err(NewFolderError::ContainsDelimiter(d).into());
    }
    let moved = MailboxName::parse(&format!("{prefix}{leaf}")).map_err(NewFolderError::from)?;
    let wanted = moved.as_str().to_lowercase();
    if existing
        .iter()
        .any(|m| m.name != *folder && m.name.as_str().to_lowercase() == wanted)
    {
        return Err(NewFolderError::AlreadyExists.into());
    }
    Ok(moved)
}

/// Vrai si `name` est `folder` ou l'un de ses sous-dossiers.
fn is_same_or_under(name: &MailboxName, folder: &MailboxName, delimiter: Option<char>) -> bool {
    name == folder
        || delimiter.is_some_and(|d| {
            name.as_str()
                .strip_prefix(folder.as_str())
                .is_some_and(|rest| rest.starts_with(d))
        })
}

/// Préfixe du nom complet d'un dossier rangé sous `parent` (ou au premier
/// niveau) et séparateur de hiérarchie qui s'y applique.
fn destination(
    parent: Option<&MailboxName>,
    existing: &[RemoteMailbox],
) -> Result<(String, Option<char>), NewFolderError> {
    match parent {
        Some(parent) => {
            let mailbox = existing
                .iter()
                .find(|m| m.name == *parent)
                .ok_or(NewFolderError::UnknownParent)?;
            if matches!(mailbox.role, MailboxRole::All | MailboxRole::Flagged) {
                return Err(NewFolderError::VirtualParent);
            }
            let delimiter = mailbox.delimiter.ok_or(NewFolderError::NoHierarchy)?;
            Ok((format!("{}{delimiter}", parent.as_str()), Some(delimiter)))
        }
        None => {
            let delimiter = existing
                .iter()
                .find(|m| m.name.is_inbox())
                .and_then(|m| m.delimiter)
                .or_else(|| existing.iter().find_map(|m| m.delimiter));
            Ok((root_prefix(existing, delimiter), delimiter))
        }
    }
}

/// Vérifie que le dossier `folder` peut être supprimé : il existe, n'est
/// ni la boîte de réception ni un dossier spécial, et n'a pas de
/// sous-dossier (les serveurs traitent ce cas chacun à leur façon).
pub fn deletable_folder(
    folder: &MailboxName,
    existing: &[RemoteMailbox],
) -> Result<(), FolderChangeError> {
    let mailbox = changeable(folder, existing)?;
    if let Some(d) = mailbox.delimiter {
        let prefix = format!("{}{d}", folder.as_str());
        if existing
            .iter()
            .any(|m| m.name.as_str().starts_with(&prefix))
        {
            return Err(FolderChangeError::HasSubfolders);
        }
    }
    Ok(())
}

/// Boîte `folder` du compte, si elle peut être renommée ou supprimée.
fn changeable<'a>(
    folder: &MailboxName,
    existing: &'a [RemoteMailbox],
) -> Result<&'a RemoteMailbox, FolderChangeError> {
    let mailbox = existing
        .iter()
        .find(|m| m.name == *folder)
        .ok_or(FolderChangeError::UnknownFolder)?;
    if folder.is_inbox() || mailbox.role != MailboxRole::Normal {
        return Err(FolderChangeError::Protected);
    }
    Ok(mailbox)
}

/// Préfixe des dossiers de premier niveau : « INBOX » suivi du séparateur
/// si tous les autres dossiers du compte sont rangés dessous, rien sinon.
fn root_prefix(existing: &[RemoteMailbox], delimiter: Option<char>) -> String {
    let Some(d) = delimiter else {
        return String::new();
    };
    let prefix = format!("INBOX{d}");
    let mut others = existing.iter().filter(|m| !m.name.is_inbox()).peekable();
    let nested = others.peek().is_some()
        && others.all(|m| {
            m.name
                .as_str()
                .get(..prefix.len())
                .is_some_and(|p| p.eq_ignore_ascii_case(&prefix))
        });
    if nested { prefix } else { String::new() }
}

/// Identifiant local d'une boîte (attribué par le stockage).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MailboxId(i64);

impl MailboxId {
    #[must_use]
    pub fn new(raw: i64) -> Option<Self> {
        (raw > 0).then_some(Self(raw))
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl fmt::Display for MailboxId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

// ---------------------------------------------------------------------------
// Métadonnées de message
// ---------------------------------------------------------------------------

/// Longueur maximale d'un sujet conservé, en caractères.
pub const MAX_SUBJECT_CHARS: usize = 998;
/// Longueur maximale d'un nom ou d'une adresse de correspondant.
pub const MAX_ADDRESS_CHARS: usize = 320;
/// Longueur maximale d'un identifiant de message (`Message-ID`).
pub const MAX_MESSAGE_ID_CHARS: usize = 998;
/// Nombre maximal d'adresses conservées par champ.
pub const MAX_ADDRESSES: usize = 100;
/// Nombre maximal de références conservées.
pub const MAX_REFERENCES: usize = 100;

/// Texte affichable issu d'un en-tête : contrôles et caractères de mise en
/// forme remplacés par des espaces, espaces consécutives réduites,
/// longueur bornée.
#[must_use]
pub fn header_text(raw: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(raw.len().min(max_chars));
    let mut count = 0;
    let mut space = false;
    for c in raw.chars() {
        let c = if c.is_control() || is_format_char(c) {
            ' '
        } else {
            c
        };
        if c.is_whitespace() {
            if space || out.is_empty() {
                continue;
            }
            space = true;
            out.push(' ');
        } else {
            space = false;
            out.push(c);
        }
        count += 1;
        if count >= max_chars {
            break;
        }
    }
    out.trim_end().to_owned()
}

/// Correspondant (champ `From`, `To`…) : textes assainis, non validés
/// (une adresse reçue peut être invalide, on l'affiche telle quelle).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Correspondent {
    pub name: String,
    pub address: String,
}

impl Correspondent {
    #[must_use]
    pub fn new(name: &str, address: &str) -> Self {
        Self {
            name: header_text(name, MAX_ADDRESS_CHARS),
            address: header_text(address, MAX_ADDRESS_CHARS).replace(' ', ""),
        }
    }
}

/// Identifiant de message (`Message-ID`, `In-Reply-To`, `References`),
/// sans chevrons, assaini.
fn message_id(raw: &str) -> Option<String> {
    let id = header_text(
        raw.trim().trim_start_matches('<').trim_end_matches('>'),
        MAX_MESSAGE_ID_CHARS,
    )
    .replace(' ', "");
    (!id.is_empty()).then_some(id)
}

/// En-têtes utiles d'un message, assainis.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageHeaders {
    pub subject: String,
    pub from: Vec<Correspondent>,
    pub to: Vec<Correspondent>,
    pub cc: Vec<Correspondent>,
    /// Champ `Date`, en secondes Unix.
    pub date: Option<i64>,
    pub message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
}

/// En-têtes bruts (déjà décodés des mots encodés RFC 2047), avant
/// assainissement.
#[derive(Debug, Clone, Default)]
pub struct RawHeaders<'a> {
    pub subject: &'a str,
    pub from: Vec<(&'a str, &'a str)>,
    pub to: Vec<(&'a str, &'a str)>,
    pub cc: Vec<(&'a str, &'a str)>,
    pub date: Option<i64>,
    pub message_id: Option<&'a str>,
    pub in_reply_to: Vec<&'a str>,
    pub references: Vec<&'a str>,
}

impl MessageHeaders {
    /// Assainit des en-têtes reçus.
    #[must_use]
    pub fn sanitize(raw: &RawHeaders<'_>) -> Self {
        let people = |list: &[(&str, &str)]| {
            list.iter()
                .take(MAX_ADDRESSES)
                .map(|(n, a)| Correspondent::new(n, a))
                .collect()
        };
        let ids = |list: &[&str]| {
            list.iter()
                .filter_map(|id| message_id(id))
                .take(MAX_REFERENCES)
                .collect()
        };
        Self {
            subject: header_text(raw.subject, MAX_SUBJECT_CHARS),
            from: people(&raw.from),
            to: people(&raw.to),
            cc: people(&raw.cc),
            date: raw.date,
            message_id: raw.message_id.and_then(message_id),
            in_reply_to: ids(&raw.in_reply_to),
            references: ids(&raw.references),
        }
    }
}

/// Métadonnées d'un message récupérées du serveur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageMeta {
    pub uid: Uid,
    pub flags: Flags,
    pub modseq: Option<ModSeq>,
    /// `INTERNALDATE`, en secondes Unix.
    pub internal_date: i64,
    /// `RFC822.SIZE`, en octets.
    pub size: u32,
    pub headers: MessageHeaders,
    /// Structure MIME (`BODYSTRUCTURE`) ; `None` si le serveur ne l'a pas
    /// fournie ou si elle est inexploitable.
    pub structure: Option<MimePart>,
}

impl MessageMeta {
    /// Date de tri : champ `Date` si plausible, sinon `INTERNALDATE`
    /// (une date d'en-tête est falsifiable : on refuse celles qui sont
    /// très postérieures à la réception).
    #[must_use]
    pub fn sort_date(&self) -> i64 {
        const TOLERANCE: i64 = 24 * 3600;
        match self.headers.date {
            Some(d) if d <= self.internal_date.saturating_add(TOLERANCE) && d > 0 => d,
            _ => self.internal_date,
        }
    }

    /// Vrai si la structure annonce au moins une pièce jointe.
    #[must_use]
    pub fn has_attachments(&self) -> bool {
        self.structure
            .as_ref()
            .is_some_and(MimePart::has_attachments)
    }
}

/// Ordre d'affichage des dossiers d'un compte, en arbre : chaque dossier
/// est suivi de ses sous-dossiers, rangés par nom.
///
/// Au premier niveau viennent d'abord les dossiers spéciaux, dans l'ordre
/// de leur rôle ([`MailboxRole::rank`]), puis les autres par nom. Un
/// dossier spécial reste au premier niveau même s'il est rangé dans un
/// autre (`[Gmail]/Sent Mail`, `INBOX/Sent`) : il garde sa place parmi les
/// dossiers spéciaux, suivi de ses propres sous-dossiers. Un sous-dossier
/// dont le parent n'est pas listé suit son plus proche ancêtre listé, ou
/// se place au premier niveau s'il n'en a aucun.
#[must_use]
pub fn folder_display_order(folders: Vec<FolderNode>) -> Vec<FolderNode> {
    // Clé de hiérarchie : segments du nom joints par `\0`, absent des noms
    // (caractère de contrôle refusé), `INBOX` normalisé. Triées, les clés
    // placent chaque dossier juste avant ses descendants : une pile suffit
    // pour trouver le plus proche ancêtre listé, en un seul passage.
    let keys: Vec<String> = folders.iter().map(hierarchy_key).collect();
    let mut sorted: Vec<usize> = (0..folders.len()).collect();
    sorted.sort_by(|a, b| keys[*a].cmp(&keys[*b]));
    let is_ancestor = |ancestor: &str, key: &str| {
        key.len() > ancestor.len()
            && key.starts_with(ancestor)
            && key.as_bytes()[ancestor.len()] == 0
    };

    let mut roots = Vec::new();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); folders.len()];
    let mut ancestors: Vec<usize> = Vec::new();
    for i in sorted {
        while let Some(&top) = ancestors.last() {
            if is_ancestor(&keys[top], &keys[i]) {
                break;
            }
            ancestors.pop();
        }
        match ancestors.last() {
            Some(&parent) if folders[i].role == MailboxRole::Normal => children[parent].push(i),
            _ => roots.push(i),
        }
        ancestors.push(i);
    }
    let name = |i: &usize| folders[*i].name.as_str();
    roots.sort_by(|a, b| {
        (folders[*a].role.rank(), name(a)).cmp(&(folders[*b].role.rank(), name(b)))
    });
    for siblings in &mut children {
        siblings.sort_by(|a, b| name(a).cmp(name(b)));
    }

    // Parcours en profondeur, pile inversée pour garder l'ordre des frères.
    let mut order = Vec::with_capacity(folders.len());
    let mut stack: Vec<usize> = roots.into_iter().rev().collect();
    while let Some(i) = stack.pop() {
        order.push(i);
        stack.extend(children[i].iter().rev().copied());
    }
    let mut slots: Vec<Option<FolderNode>> = folders.into_iter().map(Some).collect();
    order.into_iter().filter_map(|i| slots[i].take()).collect()
}

/// Clé de hiérarchie d'un dossier pour [`folder_display_order`].
fn hierarchy_key(folder: &FolderNode) -> String {
    let name = folder.name.as_str();
    let Some(delimiter) = folder.delimiter else {
        return name.to_owned();
    };
    let mut key = name.replace(delimiter, "\0");
    let first = key.find('\0').unwrap_or(key.len());
    if key[..first].eq_ignore_ascii_case("INBOX") {
        key.replace_range(..first, "INBOX");
    }
    key
}

#[cfg(test)]
mod tests;
