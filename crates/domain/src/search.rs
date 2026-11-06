//! Recherche plein texte : analyse de la requête saisie par
//! l'utilisateur et surlignage des résultats.
//!
//! Syntaxe reconnue par [`parse_query`] :
//! - mots libres, cherchés partout (sujet, expéditeur, destinataires,
//!   texte du message), en préfixe (`fact` trouve « facture ») à partir de
//!   deux lettres ;
//! - `"phrase exacte"` (guillemet final facultatif) ;
//! - `from:`, `to:` (destinataires, À et Cc), `subject:`, suivis d'un mot
//!   ou d'une phrase entre guillemets ;
//! - `has:attachment`, `is:unread`, `is:read`, `is:flagged` ;
//! - `before:AAAA-MM-JJ` (strictement avant ce jour) et `after:AAAA-MM-JJ`
//!   (ce jour compris) ;
//! - `-` devant n'importe quelle clause pour l'exclure.
//!
//! Les clauses se combinent par « et ». Un opérateur inconnu ou une valeur
//! invalide (`is:bleu`, `before:hier`) est cherché comme du texte : une
//! faute de frappe ne fait jamais échouer la recherche. La requête est
//! bornée en longueur et en nombre de clauses.
//!
//! L'arbre produit ne contient que des données : sa traduction dans la
//! syntaxe d'un moteur (FTS5…) et l'échappement des termes relèvent de
//! l'adaptateur qui implémente [`crate::ports::SearchIndex`].

/// Longueur maximale d'une requête, en caractères.
pub const MAX_QUERY_CHARS: usize = 512;
/// Nombre maximal de clauses d'une requête.
pub const MAX_CLAUSES: usize = 16;

/// Champ visé par un terme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    /// Sujet, expéditeur, destinataires et texte.
    Any,
    /// Nom et adresse de l'expéditeur.
    From,
    /// Noms et adresses des destinataires (À et Cc).
    To,
    /// Sujet.
    Subject,
}

/// Jour du calendrier grégorien, validé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: i32,
    month: u8,
    day: u8,
}

/// Filtre sur les propriétés d'un message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Filter {
    HasAttachment,
    Unread,
    Read,
    Flagged,
    /// Strictement avant ce jour.
    Before(Date),
    /// Ce jour ou après.
    After(Date),
}

/// Élément d'une clause.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Atom {
    /// Texte à chercher : un mot (cherché en préfixe) ou une phrase
    /// (`phrase` vrai : mots consécutifs). `text` n'est jamais vide.
    Text {
        field: Field,
        text: String,
        phrase: bool,
    },
    Filter(Filter),
}

/// Clause de la requête, éventuellement niée.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Clause {
    pub negated: bool,
    pub atom: Atom,
}

/// Requête analysée : conjonction de clauses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Query {
    clauses: Vec<Clause>,
}

/// Requête refusée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QueryError {
    #[error("requête trop longue")]
    TooLong,
    #[error("trop de clauses")]
    TooManyClauses,
}

/// Analyse une requête saisie par l'utilisateur.
///
/// # Errors
/// [`QueryError::TooLong`] au-delà de [`MAX_QUERY_CHARS`] caractères,
/// [`QueryError::TooManyClauses`] au-delà de [`MAX_CLAUSES`] clauses.
pub fn parse_query(input: &str) -> Result<Query, QueryError> {
    if input.chars().count() > MAX_QUERY_CHARS {
        return Err(QueryError::TooLong);
    }
    let mut clauses = Vec::new();
    let mut rest = input;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let mut negated = false;
        if let Some(after) = rest.strip_prefix('-') {
            match after.chars().next() {
                // `-` isolé : ignoré.
                None => break,
                Some(c) if c.is_whitespace() => {
                    rest = after;
                    continue;
                }
                Some(_) => {
                    negated = true;
                    rest = after;
                }
            }
        }
        let (atom, after) = next_atom(rest);
        rest = after;
        if let Some(atom) = atom {
            if clauses.len() == MAX_CLAUSES {
                return Err(QueryError::TooManyClauses);
            }
            clauses.push(Clause { negated, atom });
        }
    }
    Ok(Query { clauses })
}

/// Lit une clause au début de `input` (non vide, sans blanc de tête).
/// Renvoie `None` pour une phrase vide.
fn next_atom(input: &str) -> (Option<Atom>, &str) {
    if let Some(after) = input.strip_prefix('"') {
        let (text, rest) = read_phrase(after);
        return (text_atom(Field::Any, text, true), rest);
    }
    let end = input
        .find(|c: char| c.is_whitespace() || c == '"')
        .unwrap_or(input.len());
    let (word, rest) = input.split_at(end);
    if let (Some(op), Some(after)) = (word.strip_suffix(':'), rest.strip_prefix('"'))
        && let Some(field) = field_operator(op)
    {
        let (text, rest) = read_phrase(after);
        let atom = text_atom(field, text, true).or_else(|| text_atom(Field::Any, word, false));
        return (atom, rest);
    }
    (Some(word_atom(word)), rest)
}

/// Lit une phrase jusqu'au guillemet fermant (ou la fin) ; renvoie son
/// texte et la suite.
fn read_phrase(input: &str) -> (&str, &str) {
    match input.split_once('"') {
        Some((text, rest)) => (text, rest),
        None => (input, ""),
    }
}

fn text_atom(field: Field, text: &str, phrase: bool) -> Option<Atom> {
    let text = text.trim();
    (!text.is_empty()).then(|| Atom::Text {
        field,
        text: text.to_owned(),
        phrase,
    })
}

fn field_operator(op: &str) -> Option<Field> {
    match op.to_ascii_lowercase().as_str() {
        "from" => Some(Field::From),
        "to" => Some(Field::To),
        "subject" => Some(Field::Subject),
        _ => None,
    }
}

/// Mot sans blanc ni guillemet : opérateur reconnu, sinon texte libre.
fn word_atom(word: &str) -> Atom {
    let plain = || Atom::Text {
        field: Field::Any,
        text: word.to_owned(),
        phrase: false,
    };
    let Some((op, value)) = word.split_once(':') else {
        return plain();
    };
    if value.is_empty() {
        return plain();
    }
    if let Some(field) = field_operator(op) {
        return Atom::Text {
            field,
            text: value.to_owned(),
            phrase: false,
        };
    }
    let filter = match (
        op.to_ascii_lowercase().as_str(),
        value.to_ascii_lowercase().as_str(),
    ) {
        ("has", "attachment") => Some(Filter::HasAttachment),
        ("is", "unread") => Some(Filter::Unread),
        ("is", "read") => Some(Filter::Read),
        ("is", "flagged") => Some(Filter::Flagged),
        ("before", v) => Date::parse(v).map(Filter::Before),
        ("after", v) => Date::parse(v).map(Filter::After),
        _ => None,
    };
    filter.map_or_else(plain, Atom::Filter)
}

impl Query {
    /// Clauses, dans l'ordre de saisie.
    #[must_use]
    pub fn clauses(&self) -> &[Clause] {
        &self.clauses
    }

    /// Mots pliés des clauses de texte non niées, pour le surlignage.
    #[must_use]
    pub fn highlight_terms(&self) -> Vec<String> {
        let mut terms: Vec<String> = Vec::new();
        for clause in self.clauses.iter().filter(|c| !c.negated) {
            if let Atom::Text { text, .. } = &clause.atom {
                for word in tokens(text) {
                    if !terms.contains(&word) {
                        terms.push(word);
                    }
                }
            }
        }
        terms
    }

    /// Vrai si la requête ne contient aucune clause.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.clauses.is_empty()
    }
}

impl Date {
    /// Jour validé (années 1 à 9999).
    #[must_use]
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return None,
        };
        ((1..=9999).contains(&year) && (1..=days).contains(&day)).then_some(Self {
            year,
            month,
            day,
        })
    }

    /// Lit `AAAA-MM-JJ`.
    fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('-');
        let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || year.len() != 4 || month.len() != 2 || day.len() != 2 {
            return None;
        }
        let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if !(digits(year) && digits(month) && digits(day)) {
            return None;
        }
        Self::new(year.parse().ok()?, month.parse().ok()?, day.parse().ok()?)
    }

    /// Instant (secondes Unix) du début de ce jour, pour un fuseau décalé
    /// de `utc_offset_seconds` par rapport à UTC.
    #[must_use]
    pub fn start_timestamp(self, utc_offset_seconds: i32) -> i64 {
        // Jours depuis le 1970-01-01 (algorithme « days from civil »).
        let month = i64::from(self.month);
        let year = i64::from(self.year) - i64::from(month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let shifted_month = (month + 9) % 12;
        let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(self.day) - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        let days = era * 146_097 + day_of_era - 719_468;
        days * 86_400 - i64::from(utc_offset_seconds)
    }
}

/// Morceau d'un texte surligné.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    /// Vrai si le morceau correspond à un terme cherché.
    pub hit: bool,
}

/// Texte en minuscules, sans diacritiques (pliage des lettres latines,
/// comme le tokenizer `unicode61 remove_diacritics 2`).
#[must_use]
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(base_letter)
        .collect()
}

/// Lettre latine minuscule sans son diacritique (lettres qui se
/// décomposent en lettre de base et signe diacritique seulement).
fn base_letter(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' => 'h',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

/// Mots pliés d'un texte (voir [`fold`]), dans l'ordre : les jetons
/// qu'indexe le tokenizer `unicode61 remove_diacritics 2`.
#[must_use]
pub fn tokens(text: &str) -> Vec<String> {
    words(text).map(fold).collect()
}

/// Mots d'un texte : suites de lettres et de chiffres, comme les jetons
/// du tokenizer `unicode61`.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
}

/// Découpe `text` en morceaux, ceux dont un mot commence par l'un des
/// `terms` (déjà pliés, voir [`Query::highlight_terms`]) étant marqués.
#[must_use]
pub fn highlight(text: &str, terms: &[String]) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut push = |piece: &str, hit: bool| match segments.last_mut() {
        Some(last) if last.hit == hit => last.text.push_str(piece),
        _ => segments.push(Segment {
            text: piece.to_owned(),
            hit,
        }),
    };
    let mut rest = text;
    while let Some(first) = rest.chars().next() {
        let alphanumeric = first.is_alphanumeric();
        let end = rest
            .find(|c: char| c.is_alphanumeric() != alphanumeric)
            .unwrap_or(rest.len());
        let (piece, after) = rest.split_at(end);
        let hit = alphanumeric && {
            let folded = fold(piece);
            terms.iter().any(|t| folded.starts_with(t.as_str()))
        };
        push(piece, hit);
        rest = after;
    }
    segments
}

#[cfg(test)]
mod tests;
