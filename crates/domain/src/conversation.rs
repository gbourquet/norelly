//! Fils de conversation (algorithme JWZ) et boîte unifiée.
//!
//! Deux formes du même regroupement :
//! - [`thread`] : l'algorithme de Jamie Zawinski, en un seul passage sur
//!   un ensemble de messages (`References`, `In-Reply-To`, puis sujet en
//!   dernier recours). Il sert au recalcul complet d'un compte et à
//!   l'ordre d'affichage d'une conversation ;
//! - [`ConversationIndex`] et [`assign`] : le même regroupement, message
//!   par message, pour la mise à jour incrémentale du stockage. Deux
//!   messages reliés par leurs identifiants finissent dans la même
//!   conversation quel que soit leur ordre d'arrivée (fusion si besoin).
//!
//! Le regroupement par sujet est volontairement plus prudent que celui de
//! JWZ : il ne s'applique qu'à une réponse (préfixe `Re:`, `Fwd:`… ou
//! références sans correspondance) et seulement dans une fenêtre de
//! [`SUBJECT_WINDOW_SECS`], pour ne pas réunir deux « Bonjour » sans
//! rapport.
//!
//! Les identifiants viennent d'en-têtes hostiles : ils sont déjà assainis
//! et bornés par [`MessageHeaders::sanitize`] ; un message peut toutefois
//! se rattacher à n'importe quelle conversation dont il connaît un
//! identifiant (limite inhérente à JWZ, sans conséquence de sécurité : le
//! regroupement n'est qu'un affichage).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::mailbox::{MAX_REFERENCES, MailboxId, MailboxRole, MessageHeaders};

/// Fenêtre du regroupement par sujet : une réponse sans référence connue
/// ne rejoint une conversation de même sujet que si leurs dates sont à
/// moins de 31 jours d'écart.
pub const SUBJECT_WINDOW_SECS: i64 = 31 * 24 * 3600;

/// Nombre maximal de préfixes retirés d'un sujet (défense contre un sujet
/// fait de milliers de `Re:`).
const MAX_PREFIXES: usize = 32;
/// Longueur maximale d'une étiquette de liste (`[liste]`) retirée.
const MAX_TAG_CHARS: usize = 40;

/// Préfixes de réponse et de transfert usuels (comparés en minuscules).
const PREFIXES: &[&str] = &[
    "re", "fwd", "fw", "tr", "réf", "ref", "rép", "aw", "wg", "sv", "vs", "antw", "doorst", "rif",
    "r", "i", "enc", "res", "rv", "odp", "pd", "ynt", "vl", "fs", "vb",
];

/// Clé de sujet d'un message : sujet sans préfixes de réponse ou de
/// transfert ni étiquettes de liste, en minuscules, espaces réduites.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubjectKey {
    key: String,
    /// Vrai si au moins un préfixe de réponse ou de transfert a été
    /// retiré.
    reply: bool,
}

impl SubjectKey {
    #[must_use]
    pub fn of(subject: &str) -> Self {
        let mut rest = subject.trim();
        let mut reply = false;
        for _ in 0..MAX_PREFIXES {
            if let Some(after) = strip_tag(rest) {
                rest = after;
            } else if let Some(after) = strip_prefix(rest) {
                rest = after;
                reply = true;
            } else {
                break;
            }
        }
        let key = rest
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        Self { key, reply }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn is_reply(&self) -> bool {
        self.reply
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.key.is_empty()
    }
}

/// `[liste] reste` → `reste`.
fn strip_tag(s: &str) -> Option<&str> {
    let inner = s.strip_prefix('[')?;
    let end = inner.find(']')?;
    let tag = inner.get(..end)?;
    if tag.is_empty() || tag.chars().count() > MAX_TAG_CHARS {
        return None;
    }
    let rest = inner.get(end + 1..)?.trim_start();
    // Un sujet fait uniquement d'une étiquette reste tel quel.
    (!rest.is_empty()).then_some(rest)
}

/// `Re: reste`, `RE[2]: reste`, `Fwd : reste`, `Ｒｅ：` (deux-points pleine
/// chasse) → `reste`.
pub(crate) fn strip_prefix(s: &str) -> Option<&str> {
    let word_end = s
        .char_indices()
        .find(|(_, c)| !c.is_alphabetic())
        .map_or(s.len(), |(i, _)| i);
    let word = s.get(..word_end)?.to_lowercase();
    if !PREFIXES.contains(&word.as_str()) {
        return None;
    }
    let mut rest = s.get(word_end..)?.trim_start_matches(' ');
    // Compteur facultatif : [2] ou (2).
    for (open, close) in [('[', ']'), ('(', ')')] {
        if let Some(inner) = rest.strip_prefix(open)
            && let Some(end) = inner.find(close)
            && inner.get(..end).is_some_and(|n| {
                !n.is_empty() && n.len() <= 4 && n.bytes().all(|b| b.is_ascii_digit())
            })
        {
            rest = inner.get(end + 1..)?;
        }
    }
    let rest = rest.trim_start_matches(' ');
    let rest = rest.strip_prefix(':').or_else(|| rest.strip_prefix('：'))?;
    Some(rest.trim_start())
}

/// Identifiants qui relient un message à sa conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadKeys {
    /// `Message-ID` du message.
    pub own: Option<String>,
    /// Ancêtres annoncés : `References` (du plus ancien au plus récent),
    /// puis `In-Reply-To` s'il n'y figure pas ; sans doublon ni l'identifiant
    /// propre, au plus [`MAX_REFERENCES`].
    pub ancestors: Vec<String>,
    pub subject: SubjectKey,
}

impl ThreadKeys {
    #[must_use]
    pub fn of(headers: &MessageHeaders) -> Self {
        Self::from_parts(
            headers.message_id.as_deref(),
            &headers.references,
            &headers.in_reply_to,
            &headers.subject,
        )
    }

    #[must_use]
    pub fn from_parts(
        own: Option<&str>,
        references: &[String],
        in_reply_to: &[String],
        subject: &str,
    ) -> Self {
        let own = own.filter(|id| !id.is_empty()).map(str::to_owned);
        let mut seen = BTreeSet::new();
        let mut ancestors = Vec::new();
        // In-Reply-To : le premier identifiant seulement (les suivants sont
        // souvent des adresses ou du bruit, RFC 5322 3.6.4).
        for id in references.iter().chain(in_reply_to.iter().take(1)) {
            if ancestors.len() >= MAX_REFERENCES {
                break;
            }
            if id.is_empty() || own.as_deref() == Some(id.as_str()) {
                continue;
            }
            if seen.insert(id.as_str()) {
                ancestors.push(id.clone());
            }
        }
        Self {
            own,
            ancestors,
            subject: SubjectKey::of(subject),
        }
    }

    /// Tous les identifiants du message (propre et ancêtres).
    pub fn all_ids(&self) -> impl Iterator<Item = &str> {
        self.own
            .iter()
            .chain(self.ancestors.iter())
            .map(String::as_str)
    }

    /// Vrai si le message se présente comme une réponse : préfixe de
    /// réponse ou ancêtres annoncés. Seule une réponse peut rejoindre une
    /// conversation par son sujet.
    #[must_use]
    pub fn is_reply(&self) -> bool {
        self.subject.is_reply() || !self.ancestors.is_empty()
    }
}

/// Conversation candidate au regroupement par sujet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubjectCandidate<C> {
    pub conversation: C,
    /// Date du message le plus récent de la conversation.
    pub latest_date: i64,
}

/// Rattachement d'un nouveau message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assignment<C> {
    /// Nouvelle conversation.
    New,
    /// Conversation existante.
    Join(C),
    /// Le message relie plusieurs conversations : toutes sont fusionnées
    /// dans `into` (la plus ancienne, c'est-à-dire le plus petit
    /// identifiant).
    Merge { into: C, from: Vec<C> },
}

impl<C: Copy> Assignment<C> {
    /// Conversation qui reçoit le message, `None` pour une nouvelle.
    #[must_use]
    pub fn target(&self) -> Option<C> {
        match self {
            Self::New => None,
            Self::Join(c) | Self::Merge { into: c, .. } => Some(*c),
        }
    }
}

/// Vrai si le regroupement par sujet s'applique entre un message daté
/// `date` et une conversation dont le message le plus récent date de
/// `latest`.
#[must_use]
pub fn within_subject_window(date: i64, latest: i64) -> bool {
    date.abs_diff(latest) <= SUBJECT_WINDOW_SECS.unsigned_abs()
}

/// Rattache un message : `linked` contient les conversations qui
/// connaissent l'un de ses identifiants (propre ou ancêtre) ; à défaut,
/// `by_subject` est la conversation la plus récente de même clé de sujet,
/// prise en compte seulement pour une réponse datée dans la fenêtre.
#[must_use]
pub fn assign<C: Ord + Copy>(
    keys: &ThreadKeys,
    date: i64,
    linked: &BTreeSet<C>,
    by_subject: Option<SubjectCandidate<C>>,
) -> Assignment<C> {
    let mut linked = linked.iter().copied();
    match (linked.next(), linked.next()) {
        (Some(into), None) => Assignment::Join(into),
        (Some(into), Some(second)) => Assignment::Merge {
            into,
            from: std::iter::once(second).chain(linked).collect(),
        },
        (None, _) => match by_subject {
            Some(c)
                if keys.is_reply()
                    && !keys.subject.is_empty()
                    && within_subject_window(date, c.latest_date) =>
            {
                Assignment::Join(c.conversation)
            }
            _ => Assignment::New,
        },
    }
}

// ---------------------------------------------------------------------------
// Index incrémental en mémoire
// ---------------------------------------------------------------------------

/// Index incrémental des conversations d'un compte, en mémoire : même
/// logique que le stockage (identifiants connus par conversation, fusion,
/// sujet en dernier recours). Sert aux faux des tests et de référence aux
/// propriétés.
#[derive(Debug, Clone, Default)]
pub struct ConversationIndex<K> {
    next: u64,
    /// Identifiant de message → conversation.
    links: HashMap<String, u64>,
    /// Message → conversation.
    members: BTreeMap<K, u64>,
    /// Conversation → (clé de sujet, date la plus récente).
    subjects: BTreeMap<u64, (SubjectKey, i64)>,
}

impl<K: Ord + Clone> ConversationIndex<K> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next: 0,
            links: HashMap::new(),
            members: BTreeMap::new(),
            subjects: BTreeMap::new(),
        }
    }

    /// Ajoute un message ; renvoie sa conversation.
    pub fn insert(&mut self, key: K, keys: &ThreadKeys, date: i64) -> u64 {
        let linked: BTreeSet<u64> = keys
            .all_ids()
            .filter_map(|id| self.links.get(id).copied())
            .collect();
        let by_subject = self
            .subjects
            .iter()
            .filter(|(_, (s, _))| *s.as_str() == *keys.subject.as_str())
            .max_by_key(|(c, (_, latest))| (*latest, **c))
            .map(|(c, (_, latest))| SubjectCandidate {
                conversation: *c,
                latest_date: *latest,
            });
        let target = match assign(keys, date, &linked, by_subject) {
            Assignment::New => {
                self.next += 1;
                self.subjects
                    .insert(self.next, (keys.subject.clone(), date));
                self.next
            }
            Assignment::Join(c) => c,
            Assignment::Merge { into, from } => {
                for old in &from {
                    for v in self.links.values_mut().chain(self.members.values_mut()) {
                        if v == old {
                            *v = into;
                        }
                    }
                    if let Some((_, latest)) = self.subjects.remove(old)
                        && let Some(entry) = self.subjects.get_mut(&into)
                    {
                        entry.1 = entry.1.max(latest);
                    }
                }
                into
            }
        };
        for id in keys.all_ids() {
            self.links.insert(id.to_owned(), target);
        }
        if let Some(entry) = self.subjects.get_mut(&target) {
            entry.1 = entry.1.max(date);
        }
        self.members.insert(key, target);
        target
    }

    /// Conversation d'un message.
    #[must_use]
    pub fn conversation_of(&self, key: &K) -> Option<u64> {
        self.members.get(key).copied()
    }

    /// Partition des messages en conversations, chacune triée, l'ensemble
    /// trié : forme canonique pour comparer deux regroupements.
    #[must_use]
    pub fn partition(&self) -> Vec<Vec<K>> {
        let mut groups: BTreeMap<u64, Vec<K>> = BTreeMap::new();
        for (k, c) in &self.members {
            groups.entry(*c).or_default().push(k.clone());
        }
        let mut out: Vec<Vec<K>> = groups.into_values().collect();
        out.sort();
        out
    }
}

// ---------------------------------------------------------------------------
// Algorithme JWZ
// ---------------------------------------------------------------------------

/// Message à regrouper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMessage<K> {
    pub key: K,
    pub keys: ThreadKeys,
    pub date: i64,
}

/// Message placé dans un fil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadEntry<K> {
    pub key: K,
    /// Profondeur dans l'arbre (0 : racine).
    pub depth: usize,
    /// Message parent le plus proche présent (les ancêtres absents ne
    /// figurent pas).
    pub parent: Option<K>,
}

/// Fil : messages en parcours en profondeur, frères par date croissante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread<K> {
    pub entries: Vec<ThreadEntry<K>>,
    /// Date du message le plus récent.
    pub latest_date: i64,
}

#[derive(Debug, Default)]
struct Container {
    message: Option<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

struct Jwz<'a, K> {
    messages: &'a [ThreadMessage<K>],
    containers: Vec<Container>,
}

impl<K> Jwz<'_, K> {
    fn new_container(&mut self) -> usize {
        self.containers.push(Container::default());
        self.containers.len() - 1
    }

    /// Vrai si `ancestor` est `node` ou l'un de ses ascendants.
    fn is_ancestor(&self, ancestor: usize, node: usize) -> bool {
        let mut cur = Some(node);
        let mut steps = 0;
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            steps += 1;
            if steps > self.containers.len() {
                return true; // Boucle : par prudence, refuser le lien.
            }
            cur = self.containers.get(c).and_then(|c| c.parent);
        }
        false
    }

    fn unlink(&mut self, child: usize) {
        if let Some(parent) = self.containers.get(child).and_then(|c| c.parent) {
            if let Some(p) = self.containers.get_mut(parent) {
                p.children.retain(|&c| c != child);
            }
            if let Some(c) = self.containers.get_mut(child) {
                c.parent = None;
            }
        }
    }

    fn link(&mut self, parent: usize, child: usize) {
        if parent == child || self.is_ancestor(child, parent) {
            return;
        }
        self.unlink(child);
        if let Some(c) = self.containers.get_mut(child) {
            c.parent = Some(parent);
        }
        if let Some(p) = self.containers.get_mut(parent) {
            p.children.push(child);
        }
    }

    fn date_of(&self, container: usize) -> i64 {
        // Date la plus ancienne du sous-arbre (ordre des frères).
        let mut best = i64::MAX;
        let mut stack = vec![container];
        while let Some(c) = stack.pop() {
            if let Some(c) = self.containers.get(c) {
                if let Some(m) = c.message.and_then(|m| self.messages.get(m)) {
                    best = best.min(m.date);
                }
                stack.extend(&c.children);
            }
        }
        best
    }

    fn subject_of(&self, container: usize) -> Option<(&SubjectKey, bool)> {
        // Sujet du conteneur, ou de son premier enfant s'il est vide.
        let c = self.containers.get(container)?;
        let m = match c.message {
            Some(m) => m,
            None => c
                .children
                .iter()
                .filter_map(|&ch| self.containers.get(ch).and_then(|x| x.message))
                .min_by_key(|&m| self.messages.get(m).map_or(i64::MAX, |m| m.date))?,
        };
        let msg = self.messages.get(m)?;
        Some((&msg.keys.subject, msg.keys.is_reply()))
    }
}

/// Regroupe des messages en fils (algorithme JWZ, sujet en dernier
/// recours dans la fenêtre [`SUBJECT_WINDOW_SECS`]). Fils triés du plus
/// récent au plus ancien.
#[must_use]
pub fn thread<K: Clone>(messages: &[ThreadMessage<K>]) -> Vec<Thread<K>> {
    let mut jwz = Jwz {
        messages,
        containers: Vec::with_capacity(messages.len() * 2),
    };
    let mut ids: HashMap<&str, usize> = HashMap::new();

    // 1. Conteneurs et liens d'après les références.
    for (index, message) in messages.iter().enumerate() {
        let own = match message.keys.own.as_deref() {
            Some(id) => match ids.get(id).copied() {
                // Conteneur vide créé par une référence : il prend le message.
                Some(c) if jwz.containers.get(c).is_some_and(|c| c.message.is_none()) => c,
                // Identifiant en double : message traité à part.
                Some(_) => jwz.new_container(),
                None => {
                    let c = jwz.new_container();
                    ids.insert(id, c);
                    c
                }
            },
            None => jwz.new_container(),
        };
        if let Some(c) = jwz.containers.get_mut(own) {
            c.message = Some(index);
        }
        let mut previous: Option<usize> = None;
        for id in &message.keys.ancestors {
            let c = match ids.get(id.as_str()).copied() {
                Some(c) => c,
                None => {
                    let c = jwz.new_container();
                    ids.insert(id.as_str(), c);
                    c
                }
            };
            if let Some(p) = previous
                && jwz.containers.get(c).is_some_and(|x| x.parent.is_none())
            {
                jwz.link(p, c);
            }
            previous = Some(c);
        }
        // Le message lui-même : parent = dernière référence (ses propres
        // en-têtes font foi).
        match previous {
            Some(p) => jwz.link(p, own),
            None => jwz.unlink(own),
        }
    }

    // 2. Racines, conteneurs vides élagués.
    let roots: Vec<usize> = (0..jwz.containers.len())
        .filter(|&c| jwz.containers.get(c).is_some_and(|c| c.parent.is_none()))
        .collect();
    let mut roots = prune(&mut jwz, roots, true);

    // 3. Regroupement par sujet des racines (réponses seulement, fenêtre).
    roots.sort_by_key(|&r| jwz.date_of(r));
    let mut by_subject: HashMap<String, usize> = HashMap::new();
    let mut kept = Vec::new();
    for root in roots {
        let Some((subject, reply)) = jwz.subject_of(root) else {
            kept.push(root);
            continue;
        };
        let key = subject.as_str().to_owned();
        if key.is_empty() {
            kept.push(root);
            continue;
        }
        let date = jwz.date_of(root);
        match by_subject.get(&key).copied() {
            Some(existing) if reply && within_subject_window(date, latest_of(&jwz, existing)) => {
                attach(&mut jwz, existing, root);
            }
            _ => {
                by_subject.insert(key, root);
                kept.push(root);
            }
        }
    }

    // 4. Sortie : parcours en profondeur, frères par date.
    let mut threads: Vec<Thread<K>> = kept
        .into_iter()
        .filter_map(|root| flatten(&jwz, root))
        .collect();
    threads.sort_by_key(|t| std::cmp::Reverse(t.latest_date));
    threads
}

fn latest_of<K>(jwz: &Jwz<'_, K>, container: usize) -> i64 {
    let mut best = i64::MIN;
    let mut stack = vec![container];
    while let Some(c) = stack.pop() {
        if let Some(c) = jwz.containers.get(c) {
            if let Some(m) = c.message.and_then(|m| jwz.messages.get(m)) {
                best = best.max(m.date);
            }
            stack.extend(&c.children);
        }
    }
    best
}

/// Rattache la racine `root` (une réponse) au fil `existing`.
fn attach<K>(jwz: &mut Jwz<'_, K>, existing: usize, root: usize) {
    let has_message = jwz
        .containers
        .get(existing)
        .is_some_and(|c| c.message.is_some());
    if has_message {
        jwz.link(existing, root);
    } else {
        // Racine vide : la réponse devient une sœur.
        let children: Vec<usize> = jwz
            .containers
            .get(root)
            .filter(|c| c.message.is_none())
            .map(|c| c.children.clone())
            .unwrap_or_else(|| vec![root]);
        for child in children {
            jwz.link(existing, child);
        }
    }
}

/// Élague les conteneurs vides : un vide sans enfant disparaît ; un vide
/// avec enfants est remplacé par ses enfants, sauf à la racine s'il en a
/// plusieurs (il garde alors le fil uni).
fn prune<K>(jwz: &mut Jwz<'_, K>, level: Vec<usize>, at_root: bool) -> Vec<usize> {
    let mut out = Vec::new();
    let mut queue: std::collections::VecDeque<usize> = level.into();
    while let Some(c) = queue.pop_front() {
        let children = jwz
            .containers
            .get(c)
            .map(|x| x.children.clone())
            .unwrap_or_default();
        let children = prune(jwz, children, false);
        if let Some(x) = jwz.containers.get_mut(c) {
            x.children.clone_from(&children);
        }
        let empty = jwz.containers.get(c).is_some_and(|x| x.message.is_none());
        if !empty {
            out.push(c);
        } else if children.is_empty() {
            jwz.unlink(c);
        } else if !at_root || children.len() == 1 {
            let parent = jwz.containers.get(c).and_then(|x| x.parent);
            for &child in &children {
                if let Some(x) = jwz.containers.get_mut(child) {
                    x.parent = parent;
                }
            }
            if let Some(x) = jwz.containers.get_mut(c) {
                x.children.clear();
            }
            if let Some(p) = parent
                && let Some(x) = jwz.containers.get_mut(p)
            {
                x.children.retain(|&k| k != c);
            }
            out.extend(children);
        } else {
            out.push(c);
        }
    }
    out
}

fn flatten<K: Clone>(jwz: &Jwz<'_, K>, root: usize) -> Option<Thread<K>> {
    let mut entries = Vec::new();
    let mut latest = i64::MIN;
    // (conteneur, profondeur, parent présent le plus proche)
    let mut stack: Vec<(usize, usize, Option<usize>)> = vec![(root, 0, None)];
    let mut visited = BTreeSet::new();
    while let Some((c, depth, parent)) = stack.pop() {
        if !visited.insert(c) {
            continue;
        }
        let Some(container) = jwz.containers.get(c) else {
            continue;
        };
        let (next_depth, next_parent) = match container.message.and_then(|m| jwz.messages.get(m)) {
            Some(m) => {
                latest = latest.max(m.date);
                entries.push(ThreadEntry {
                    key: m.key.clone(),
                    depth,
                    parent: parent
                        .and_then(|p: usize| jwz.messages.get(p))
                        .map(|p| p.key.clone()),
                });
                (depth + 1, container.message)
            }
            None => (depth, parent),
        };
        let mut children = container.children.clone();
        // Pile : le plus ancien doit sortir en premier.
        children.sort_by_key(|&ch| std::cmp::Reverse(jwz.date_of(ch)));
        for ch in children {
            stack.push((ch, next_depth, next_parent));
        }
    }
    (!entries.is_empty()).then_some(Thread {
        entries,
        latest_date: latest,
    })
}

// ---------------------------------------------------------------------------
// Boîte unifiée
// ---------------------------------------------------------------------------

/// Erreur de portée de liste.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("ce rôle de dossier ne peut pas être unifié")]
pub struct ScopeError;

/// Portée d'une liste de conversations : un dossier, ou les dossiers d'un
/// même rôle de tous les comptes (boîte unifiée).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConversationScope {
    Mailbox(MailboxId),
    Unified(MailboxRole),
}

impl ConversationScope {
    /// Boîte unifiée d'un rôle. `\All` (doublons de tous les dossiers) et
    /// les dossiers ordinaires ne s'unifient pas.
    pub fn unified(role: MailboxRole) -> Result<Self, ScopeError> {
        match role {
            MailboxRole::Normal | MailboxRole::All => Err(ScopeError),
            _ => Ok(Self::Unified(role)),
        }
    }

    /// Boîte de réception unifiée.
    #[must_use]
    pub const fn unified_inbox() -> Self {
        Self::Unified(MailboxRole::Inbox)
    }
}

#[cfg(test)]
mod tests;
