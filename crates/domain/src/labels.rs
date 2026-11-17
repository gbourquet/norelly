//! Étiquettes : mots-clés IMAP (RFC 3501, `flag-keyword`) présentés à
//! l'utilisateur sous un nom, avec une couleur locale.
//!
//! - **Nom et mot-clé** : un nom d'étiquette ([`LabelName`]) est encodé de
//!   façon réversible dans un mot-clé, qui est un atom ASCII. L'encodage
//!   suit l'UTF-7 modifié des noms de dossiers (RFC 3501, 5.1.3) : les
//!   caractères ASCII permis dans un atom passent tels quels, `&` devient
//!   `&-`, le reste est codé en UTF-16 big-endian, en base64 modifiée,
//!   entre `&` et `-`. Deux extensions : les caractères interdits dans un
//!   atom (espace, `( ) { " \ % * ]`) sont codés comme les caractères non
//!   ASCII, et un `$` en tête aussi, pour qu'un nom ne produise jamais un
//!   mot-clé `$…` (espace des mots-clés de protocole et de Thunderbird).
//!   Un mot-clé reçu n'est décodé que s'il est la forme canonique de son
//!   nom ([`decode_label`]) ; sinon il est affiché tel quel.
//! - **Mots-clés réservés** : les mots-clés de protocole (`$Junk`,
//!   `$Forwarded`, `NonJunk`…) ne sont jamais des étiquettes
//!   ([`is_protocol_keyword`]). `$label1` à `$label5` sont les étiquettes
//!   de Thunderbird ([`LabelDisplay::Builtin`]), nommées par l'interface.
//! - **Liste d'un compte** ([`account_labels`]) : étiquettes enregistrées,
//!   enrichies des mots-clés vus sur le serveur ; une étiquette supprimée
//!   de la liste reste masquée, une étiquette en cours de réécriture
//!   (renommage) disparaît sous son ancien nom.
//! - **Couleur** ([`LabelColor`]) : `#rrggbb` seulement ; le texte de la
//!   pastille est blanc ou noir selon le fond ([`LabelColor::text`]), pour
//!   un contraste d'au moins 4,5:1 quel que soit le thème.
//! - **Mots-clés permanents** ([`KeywordPermission`]) : d'après
//!   `PERMANENTFLAGS`, ce qu'un dossier accepte de garder.
//! - **Renommage et suppression** ([`LabelRewrite`]) : réécriture du
//!   mot-clé sur tous les messages du serveur, par lots d'UID
//!   ([`rewrite_batches`]).

use std::collections::BTreeSet;

use crate::actions::Member;
use crate::contacts::is_invisible;
use crate::conversation::ConversationScope;
use crate::mailbox::{Flags, Keyword, MAX_KEYWORD_LEN, MailboxId, Uid};
use crate::search::fold;

/// Longueur maximale d'un nom d'étiquette, en caractères (le mot-clé
/// encodé est en outre borné à [`MAX_KEYWORD_LEN`] octets).
pub const MAX_LABEL_NAME_CHARS: usize = 40;

/// Nombre maximal d'étiquettes listées pour un compte.
pub const MAX_LABELS: usize = 500;

/// Nombre maximal d'UID par `UID STORE` d'une réécriture.
pub const REWRITE_BATCH: usize = 500;

/// Étiquettes de Thunderbird (`$label1` à `$label5`).
pub const BUILTIN_LABELS: u8 = 5;

/// Noms des étiquettes de Thunderbird, en français et en anglais, pour
/// reconnaître une recherche (`étiquette:travail`) : l'interface les
/// affiche par ses catalogues (clés `label.builtin.N`), qui doivent porter
/// les mêmes noms (vérifié par un test).
pub const BUILTIN_NAMES: [[&str; 2]; BUILTIN_LABELS as usize] = [
    ["Important", "Important"],
    ["Travail", "Work"],
    ["Personnel", "Personal"],
    ["À faire", "To Do"],
    ["Plus tard", "Later"],
];

/// Étiquette refusée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LabelError {
    #[error("nom d'étiquette vide")]
    Empty,
    #[error("nom d'étiquette trop long")]
    TooLong,
    #[error("caractère interdit dans un nom d'étiquette")]
    InvalidChar,
    #[error("étiquette déjà existante")]
    AlreadyExists,
    #[error("étiquette inconnue")]
    Unknown,
    #[error("nom inchangé")]
    Unchanged,
    #[error("trop d'étiquettes sur ce compte")]
    TooMany,
}

// ---------------------------------------------------------------------------
// Nom et encodage
// ---------------------------------------------------------------------------

/// Nom d'étiquette validé : sans blancs de bord, non vide, sans caractère
/// de contrôle ni invisible, borné (en caractères et une fois encodé).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelName(String);

impl LabelName {
    /// # Errors
    /// [`LabelError::Empty`], [`LabelError::InvalidChar`] ou
    /// [`LabelError::TooLong`].
    pub fn parse(raw: &str) -> Result<Self, LabelError> {
        let name = raw.trim();
        if name.is_empty() {
            return Err(LabelError::Empty);
        }
        if name.chars().any(|c| c.is_control() || is_invisible(c)) {
            return Err(LabelError::InvalidChar);
        }
        if name.chars().count() > MAX_LABEL_NAME_CHARS || encode(name).len() > MAX_KEYWORD_LEN {
            return Err(LabelError::TooLong);
        }
        Ok(Self(name.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Mot-clé IMAP de ce nom.
    #[must_use]
    pub fn keyword(&self) -> Keyword {
        // Le nom a été validé : son encodage est un atom borné.
        Keyword::parse(&encode(&self.0)).unwrap_or_else(|| unreachable!("encodage validé"))
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

/// Caractère écrit tel quel dans le mot-clé (`first` : en tête).
fn is_direct(c: char, first: bool) -> bool {
    c.is_ascii_graphic()
        && !matches!(c, '(' | ')' | '{' | '"' | '\\' | '%' | '*' | ']' | '&')
        && !(first && c == '$')
}

fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending: Vec<u16> = Vec::new();
    let flush = |pending: &mut Vec<u16>, out: &mut String| {
        if pending.is_empty() {
            return;
        }
        out.push('&');
        let bytes: Vec<u8> = pending.iter().flat_map(|u| u.to_be_bytes()).collect();
        for chunk in bytes.chunks(3) {
            let b = [
                chunk.first().copied().unwrap_or(0),
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for i in 0..=chunk.len() {
                let idx = (n >> (18 - 6 * i)) & 0x3f;
                out.push(char::from(ALPHABET[idx as usize]));
            }
        }
        out.push('-');
        pending.clear();
    };
    for (i, c) in name.chars().enumerate() {
        if c == '&' {
            flush(&mut pending, &mut out);
            out.push_str("&-");
        } else if is_direct(c, i == 0) {
            flush(&mut pending, &mut out);
            out.push(c);
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

fn value(b: u8) -> Option<u32> {
    ALPHABET
        .iter()
        .position(|&a| a == b)
        .and_then(|p| u32::try_from(p).ok())
}

/// Décodage brut (sans contrôle de forme canonique).
fn decode(raw: &[u8]) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let b = *raw.get(i)?;
        if b != b'&' {
            out.push(char::from(b));
            i += 1;
            continue;
        }
        let end = raw.get(i + 1..)?.iter().position(|&c| c == b'-')? + i + 1;
        let encoded = raw.get(i + 1..end)?;
        if encoded.is_empty() {
            out.push('&');
        } else {
            let mut bits: u32 = 0;
            let mut nbits = 0;
            let mut bytes = Vec::new();
            for &c in encoded {
                bits = ((bits << 6) | value(c)?) & 0x00ff_ffff;
                nbits += 6;
                if nbits >= 8 {
                    nbits -= 8;
                    bytes.push(u8::try_from((bits >> nbits) & 0xff).ok()?);
                }
            }
            if nbits >= 6 || bits & ((1 << nbits) - 1) != 0 || bytes.len() % 2 != 0 {
                return None;
            }
            let units: Vec<u16> = bytes
                .chunks(2)
                .map(|p| u16::from_be_bytes([p[0], p[1]]))
                .collect();
            out.push_str(&String::from_utf16(&units).ok()?);
        }
        i = end + 1;
    }
    Some(out)
}

/// Nom d'un mot-clé encodé par [`LabelName::keyword`] ; `None` si ce n'est
/// pas la forme canonique d'un nom valide (mot-clé d'un autre logiciel,
/// encodage invalide) : il est alors affiché tel quel.
#[must_use]
pub fn decode_label(keyword: &Keyword) -> Option<String> {
    let decoded = decode(keyword.as_str().as_bytes())?;
    let name = LabelName::parse(&decoded).ok()?;
    (name.as_str() == decoded && encode(&decoded) == keyword.as_str()).then_some(decoded)
}

/// Présentation d'un mot-clé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelDisplay {
    /// `$labelN` de Thunderbird (1 à 5) : nom donné par les catalogues.
    Builtin(u8),
    /// Nom décodé.
    Named(String),
    /// Mot-clé illisible, à afficher tel quel (en texte échappé).
    Raw(String),
}

impl LabelDisplay {
    /// Noms sous lesquels l'étiquette est reconnue (recherche, doublons),
    /// pliés.
    fn folded_names(&self) -> Vec<String> {
        match self {
            Self::Builtin(n) => BUILTIN_NAMES
                .get(usize::from(*n).saturating_sub(1))
                .map(|names| names.iter().map(|s| fold(s)).collect())
                .unwrap_or_default(),
            Self::Named(s) | Self::Raw(s) => vec![fold(s)],
        }
    }
}

/// Numéro d'un `$labelN` de Thunderbird (casse indifférente).
fn builtin_number(keyword: &Keyword) -> Option<u8> {
    let raw = keyword.as_str();
    let prefix = raw.get(..6)?;
    if !prefix.eq_ignore_ascii_case("$label") {
        return None;
    }
    let n: u8 = match raw.get(6..)? {
        d @ ("1" | "2" | "3" | "4" | "5") => d.parse().ok()?,
        _ => return None,
    };
    Some(n)
}

#[must_use]
pub fn display(keyword: &Keyword) -> LabelDisplay {
    if let Some(n) = builtin_number(keyword) {
        return LabelDisplay::Builtin(n);
    }
    decode_label(keyword).map_or_else(
        || LabelDisplay::Raw(keyword.as_str().to_owned()),
        LabelDisplay::Named,
    )
}

/// Mots-clés de protocole sans `$` (filtres antispam).
const PROTOCOL_PLAIN: [&str; 5] = ["Junk", "NonJunk", "NotJunk", "Forwarded", "Redirected"];

/// Vrai pour un mot-clé de protocole, jamais présenté comme une étiquette :
/// tout mot-clé `$…` (registre IANA, RFC 5788 : `$Junk`, `$Forwarded`,
/// `$MDNSent`…) sauf `$label1` à `$label5`, et les mots-clés des filtres
/// antispam (`Junk`, `NonJunk`…), casse indifférente.
#[must_use]
pub fn is_protocol_keyword(keyword: &Keyword) -> bool {
    let raw = keyword.as_str();
    (raw.starts_with('$') && builtin_number(keyword).is_none())
        || PROTOCOL_PLAIN.iter().any(|p| p.eq_ignore_ascii_case(raw))
}

/// Étiquettes d'un message à afficher : ses mots-clés, sans ceux de
/// protocole ni ceux masqués sur le compte ([`hidden_keywords`]).
#[must_use]
pub fn visible_labels(flags: &Flags, hidden: &BTreeSet<Keyword>) -> Vec<Keyword> {
    flags
        .keywords()
        .iter()
        .filter(|k| !is_protocol_keyword(k) && !hidden.contains(*k))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Couleur
// ---------------------------------------------------------------------------

/// Couleur d'une pastille, `#rrggbb` (validée strictement : c'est la seule
/// forme qui atteint le CSS de l'interface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelColor {
    r: u8,
    g: u8,
    b: u8,
}

/// Teinte du texte d'une pastille.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTone {
    /// Blanc (`#ffffff`).
    Light,
    /// Noir (`#000000`).
    Dark,
}

impl TextTone {
    #[must_use]
    pub const fn color(self) -> LabelColor {
        match self {
            Self::Light => LabelColor::new(255, 255, 255),
            Self::Dark => LabelColor::new(0, 0, 0),
        }
    }
}

impl LabelColor {
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Lit `#rrggbb` (chiffres hexadécimaux ASCII, casse indifférente).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let hex = raw.strip_prefix('#')?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        Some(Self::new(byte(0)?, byte(2)?, byte(4)?))
    }

    /// Forme `#rrggbb`, en minuscules.
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Luminance relative (WCAG 2.x).
    fn luminance(self) -> f64 {
        let channel = |v: u8| {
            let s = f64::from(v) / 255.0;
            if s <= 0.040_45 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.0722_f64.mul_add(
            channel(self.b),
            0.2126_f64.mul_add(channel(self.r), 0.7152 * channel(self.g)),
        )
    }

    /// Texte de la pastille : celui des deux (blanc, noir) qui contraste le
    /// plus avec ce fond (au moins 4,5:1 pour toute couleur).
    #[must_use]
    pub fn text(self) -> TextTone {
        let light = contrast_ratio(self, TextTone::Light.color());
        let dark = contrast_ratio(self, TextTone::Dark.color());
        if light >= dark {
            TextTone::Light
        } else {
            TextTone::Dark
        }
    }
}

/// Rapport de contraste WCAG entre deux couleurs (1 à 21).
#[must_use]
pub fn contrast_ratio(a: LabelColor, b: LabelColor) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

// ---------------------------------------------------------------------------
// Mots-clés permanents
// ---------------------------------------------------------------------------

/// Mots-clés qu'un dossier garde (`PERMANENTFLAGS` de `SELECT`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum KeywordPermission {
    /// Jamais lu (ou réponse absente) : tout est permanent (RFC 3501, 7.1).
    #[default]
    Unknown,
    /// `\*` : tout mot-clé, y compris nouveau.
    Any,
    /// Seulement ces mots-clés.
    Listed(BTreeSet<Keyword>),
}

impl KeywordPermission {
    /// D'après les jetons de `PERMANENTFLAGS` (`\*`, `\Seen`, mots-clés).
    pub fn from_tokens<I, S>(tokens: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut listed = BTreeSet::new();
        for token in tokens {
            let token = token.as_ref();
            if token == "\\*" {
                return Self::Any;
            }
            if !token.starts_with('\\')
                && let Some(k) = Keyword::parse(token)
            {
                listed.insert(k);
            }
        }
        Self::Listed(listed)
    }

    /// Un mot-clé nouveau peut être créé.
    #[must_use]
    pub fn can_create(&self) -> bool {
        matches!(self, Self::Unknown | Self::Any)
    }

    /// `keyword` peut être posé de façon permanente (casse indifférente).
    #[must_use]
    pub fn can_store(&self, keyword: &Keyword) -> bool {
        match self {
            Self::Unknown | Self::Any => true,
            Self::Listed(set) => set
                .iter()
                .any(|k| k.as_str().eq_ignore_ascii_case(keyword.as_str())),
        }
    }

    /// Forme stockée : `None` (inconnu), `*`, ou mots-clés séparés par des
    /// espaces (préfixés de `=` pour distinguer une liste vide).
    #[must_use]
    pub fn to_storage(&self) -> Option<String> {
        match self {
            Self::Unknown => None,
            Self::Any => Some("*".to_owned()),
            Self::Listed(set) => Some(format!(
                "={}",
                set.iter()
                    .map(Keyword::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            )),
        }
    }

    /// Inverse de [`Self::to_storage`] ; une valeur illisible est lue
    /// comme une liste vide (prudence : rien de nouveau n'est proposé).
    #[must_use]
    pub fn from_storage(raw: Option<&str>) -> Self {
        match raw {
            None => Self::Unknown,
            Some("*") => Self::Any,
            Some(other) => {
                let Some(list) = other.strip_prefix('=') else {
                    return Self::Listed(BTreeSet::new());
                };
                let mut set = BTreeSet::new();
                for word in list.split(' ').filter(|w| !w.is_empty()) {
                    match Keyword::parse(word) {
                        Some(k) => {
                            set.insert(k);
                        }
                        None => return Self::Listed(BTreeSet::new()),
                    }
                }
                Self::Listed(set)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Liste d'un compte
// ---------------------------------------------------------------------------

/// Étiquette enregistrée pour un compte (couleur locale, masquage).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredLabel {
    pub keyword: Keyword,
    pub color: Option<LabelColor>,
    /// Supprimée de la liste sans être retirée des messages : ni listée,
    /// ni affichée.
    pub hidden: bool,
}

/// Étiquette d'un compte, telle que présentée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub keyword: Keyword,
    pub display: LabelDisplay,
    pub color: Option<LabelColor>,
}

/// Réécriture d'un mot-clé sur tous les messages du serveur : renommage
/// (`to`), ou retrait (`None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelRewrite {
    pub from: Keyword,
    pub to: Option<Keyword>,
}

impl LabelRewrite {
    /// La réécriture peut s'appliquer à un dossier : un retrait toujours,
    /// un renommage si le dossier garde le nouveau mot-clé (sinon le
    /// dossier est laissé tel quel, plutôt que d'y perdre l'étiquette).
    #[must_use]
    pub fn applies_to(&self, permission: &KeywordPermission) -> bool {
        self.to.as_ref().is_none_or(|to| permission.can_store(to))
    }
}

/// Ordre de la liste : étiquettes de Thunderbird par numéro, puis par nom
/// plié, puis par mot-clé.
fn sort_key(label: &Label) -> (u8, String, String) {
    match &label.display {
        LabelDisplay::Builtin(n) => (*n, String::new(), String::new()),
        LabelDisplay::Named(s) | LabelDisplay::Raw(s) => (
            BUILTIN_LABELS + 1,
            fold(s),
            label.keyword.as_str().to_owned(),
        ),
    }
}

/// Étiquettes d'un compte : enregistrées (`stored`) et vues sur le serveur
/// (`seen`), sans les mots-clés de protocole, masqués ou en cours de
/// réécriture (`rewrites`) ; au plus [`MAX_LABELS`].
#[must_use]
pub fn account_labels(
    stored: &[StoredLabel],
    seen: &BTreeSet<Keyword>,
    rewrites: &[LabelRewrite],
) -> Vec<Label> {
    let hidden = hidden_keywords(stored, rewrites);
    let mut keywords: BTreeSet<&Keyword> = stored
        .iter()
        .filter(|s| !s.hidden)
        .map(|s| &s.keyword)
        .collect();
    keywords.extend(seen.iter());
    let mut labels: Vec<Label> = keywords
        .into_iter()
        .filter(|k| !is_protocol_keyword(k) && !hidden.contains(*k))
        .map(|k| Label {
            keyword: k.clone(),
            display: display(k),
            color: stored
                .iter()
                .find(|s| &s.keyword == k)
                .and_then(|s| s.color),
        })
        .collect();
    labels.sort_by_key(sort_key);
    labels.truncate(MAX_LABELS);
    labels
}

/// Mots-clés jamais affichés sur ce compte : étiquettes supprimées de la
/// liste et anciens mots-clés en cours de réécriture.
#[must_use]
pub fn hidden_keywords(stored: &[StoredLabel], rewrites: &[LabelRewrite]) -> BTreeSet<Keyword> {
    stored
        .iter()
        .filter(|s| s.hidden)
        .map(|s| s.keyword.clone())
        .chain(rewrites.iter().map(|r| r.from.clone()))
        .collect()
}

/// L'étiquette `keyword`, ou une autre du même nom (casse et accents
/// indifférents), est déjà dans `existing`.
fn collides(
    keyword: &Keyword,
    name: &LabelName,
    existing: &[Label],
    except: Option<&Keyword>,
) -> bool {
    let folded = fold(name.as_str());
    existing
        .iter()
        .filter(|l| Some(&l.keyword) != except)
        .any(|l| &l.keyword == keyword || l.display.folded_names().contains(&folded))
}

/// Mot-clé d'une nouvelle étiquette `name`.
///
/// # Errors
/// [`LabelError::AlreadyExists`] si une étiquette du même nom existe ;
/// [`LabelError::TooMany`] au-delà de [`MAX_LABELS`].
pub fn plan_create(name: &LabelName, existing: &[Label]) -> Result<Keyword, LabelError> {
    let keyword = name.keyword();
    if collides(&keyword, name, existing, None) {
        return Err(LabelError::AlreadyExists);
    }
    if existing.len() >= MAX_LABELS {
        return Err(LabelError::TooMany);
    }
    Ok(keyword)
}

/// Renommage de l'étiquette `from` en `to`.
///
/// # Errors
/// [`LabelError::Unknown`] si `from` n'est pas listée,
/// [`LabelError::Unchanged`] si le mot-clé ne change pas,
/// [`LabelError::AlreadyExists`] si une autre étiquette porte ce nom.
pub fn plan_rename(
    from: &Keyword,
    to: &LabelName,
    existing: &[Label],
) -> Result<LabelRewrite, LabelError> {
    if !existing.iter().any(|l| &l.keyword == from) {
        return Err(LabelError::Unknown);
    }
    let keyword = to.keyword();
    if &keyword == from {
        return Err(LabelError::Unchanged);
    }
    if collides(&keyword, to, existing, Some(from)) {
        return Err(LabelError::AlreadyExists);
    }
    Ok(LabelRewrite {
        from: from.clone(),
        to: Some(keyword),
    })
}

/// Lots d'UID d'une réécriture (au plus [`REWRITE_BATCH`] chacun).
#[must_use]
pub fn rewrite_batches(uids: &BTreeSet<Uid>) -> Vec<Vec<Uid>> {
    let all: Vec<Uid> = uids.iter().copied().collect();
    all.chunks(REWRITE_BATCH).map(<[Uid]>::to_vec).collect()
}

/// Réécriture locale des mots-clés d'un message : `from` remplacé par `to`
/// (ou retiré) ; sans effet si le message ne porte pas `from`.
pub fn rewrite_flags(flags: &mut Flags, from: &Keyword, to: Option<&Keyword>) {
    if flags.remove_keyword(from)
        && let Some(to) = to
    {
        flags.insert_keyword(to.clone());
    }
}

// ---------------------------------------------------------------------------
// Menu « Étiqueter… »
// ---------------------------------------------------------------------------

/// Étiquette proposée par le menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelChoice {
    pub keyword: Keyword,
    /// Tous les messages visés la portent.
    pub checked: bool,
    /// Le serveur accepte de la poser (ou elle est cochée : la retirer
    /// reste possible).
    pub allowed: bool,
}

/// Menu « Étiqueter… » d'une sélection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelMenu {
    pub choices: Vec<LabelChoice>,
    /// Une nouvelle étiquette peut être créée et posée.
    pub can_create: bool,
}

impl LabelMenu {
    /// L'action « Étiqueter » a un effet possible (sinon elle est
    /// désactivée, avec une explication).
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.can_create || self.choices.iter().any(|c| c.allowed)
    }
}

/// Menu « Étiqueter… » pour les conversations `conversations` (leurs
/// membres) affichées dans `scope` : même portée que Lu et Suivi, les
/// messages de la conversation dans le dossier affiché. `permission` :
/// mots-clés permanents d'un dossier.
pub fn label_menu(
    conversations: &[Vec<Member>],
    scope: &ConversationScope,
    labels: &[Keyword],
    permission: impl Fn(MailboxId) -> KeywordPermission,
) -> LabelMenu {
    let targets: Vec<&Member> = conversations
        .iter()
        .flatten()
        .filter(|m| in_scope(m, scope))
        .collect();
    let folders: BTreeSet<MailboxId> = targets.iter().map(|m| m.mailbox).collect();
    let permissions: Vec<KeywordPermission> = folders.into_iter().map(permission).collect();
    let choices = labels
        .iter()
        .map(|k| {
            let checked =
                !targets.is_empty() && targets.iter().all(|m| m.flags.keywords().contains(k));
            LabelChoice {
                keyword: k.clone(),
                checked,
                allowed: checked || permissions.iter().all(|p| p.can_store(k)),
            }
        })
        .collect();
    LabelMenu {
        choices,
        can_create: permissions.iter().all(KeywordPermission::can_create),
    }
}

/// Le membre est dans la portée affichée.
pub(crate) fn in_scope(member: &Member, scope: &ConversationScope) -> bool {
    match scope {
        ConversationScope::Mailbox(id) => member.mailbox == *id,
        ConversationScope::Unified(role) => member.role == *role,
    }
}

// ---------------------------------------------------------------------------
// Recherche
// ---------------------------------------------------------------------------

/// Mots-clés cherchés par `étiquette:name` : l'encodage de `name`, puis
/// les mots-clés connus (`known`) dont le nom affiché est `name` (casse et
/// accents indifférents ; noms de Thunderbird en français et en anglais).
#[must_use]
pub fn label_keywords(name: &str, known: &[Keyword]) -> Vec<Keyword> {
    let Ok(name) = LabelName::parse(name) else {
        return Vec::new();
    };
    let folded = fold(name.as_str());
    let mut out = vec![name.keyword()];
    let builtins = (1..=BUILTIN_LABELS).filter_map(|n| Keyword::parse(&format!("$label{n}")));
    for k in builtins.chain(known.iter().cloned()) {
        if !out.contains(&k) && display(&k).folded_names().contains(&folded) {
            out.push(k);
        }
    }
    out
}

#[cfg(test)]
mod tests;
