//! Faux serveur IMAP en mémoire, fidèle à CONDSTORE et QRESYNC : boîtes,
//! messages, MODSEQ par message, journal des suppressions (VANISHED),
//! UIDVALIDITY, IDLE. Sert aux tests des cas d'usage et à la suite de
//! contrat de la session IMAP (exécutée aussi contre Dovecot).
//!
//! Pannes simulées : serveur injoignable ([`FakeImap::set_offline`]) et
//! coupure de la session après un nombre d'opérations
//! ([`FakeImap::cut_after`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use norelly_domain::actions::{FlagChange, StoreOutcome};
use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::compose::draft::RemoteCopy;
use norelly_domain::connection::ConnectionError;
use norelly_domain::fetch_policy::BodyFetch;
use norelly_domain::fetch_policy::PartPath;
use norelly_domain::mailbox::{
    Flags, ListedMailbox, MailboxName, MessageHeaders, MessageMeta, ModSeq, RawHeaders, Uid,
    UidValidity,
};
use norelly_domain::ports::{FetchedBody, IdleOutcome, RemoteMailboxSession};
use norelly_domain::sync::{
    FlagUpdate, SelectRequest, SelectedMailbox, ServerQuery, SyncExtensions, UidRanges,
};
use tokio::sync::Notify;

#[derive(Debug, Clone)]
struct Message {
    raw: Vec<u8>,
    flags: Flags,
    modseq: u64,
    internal_date: i64,
}

#[derive(Debug, Clone)]
struct Mailbox {
    attributes: Vec<String>,
    validity: u32,
    uid_next: u32,
    modseq: u64,
    messages: BTreeMap<u32, Message>,
    expunged: Vec<(u32, u64)>,
    /// Incrémenté à chaque changement (IDLE).
    version: u64,
}

impl Mailbox {
    fn new(validity: u32, attributes: Vec<String>) -> Self {
        Self {
            attributes,
            validity,
            uid_next: 1,
            modseq: 1,
            messages: BTreeMap::new(),
            expunged: Vec::new(),
            version: 0,
        }
    }
}

#[derive(Debug)]
struct State {
    /// Boîtes, dans l'ordre de création.
    mailboxes: Vec<(String, Mailbox)>,
    next_validity: u32,
    condstore: bool,
    qresync: bool,
    idle: bool,
    /// MOVE et UIDPLUS annoncés (masquables pour tester les replis).
    move_ext: bool,
    uidplus: bool,
    offline: bool,
    /// Opérations restantes avant coupure des sessions ouvertes.
    cut_after: Option<usize>,
    /// Nombre d'opérations exécutées (toutes sessions).
    operations: usize,
    /// Durée de chaque opération (serveur lent).
    delay: Duration,
    /// Course simulée : un autre client marque ce message `\Deleted`
    /// juste avant le prochain `EXPUNGE` (une seule fois).
    race_deleted: Option<(String, u32)>,
    /// Taille maximale d'un message déposé (APPEND) ; au-delà : refus
    /// `NO [OVERQUOTA]`.
    append_limit: Option<usize>,
}

impl State {
    fn mailbox(&mut self, name: &str) -> Option<&mut Mailbox> {
        self.mailboxes
            .iter_mut()
            .find(|(n, _)| n == name)
            .map(|(_, m)| m)
    }
}

/// Faux serveur IMAP partagé (clonable).
#[derive(Debug, Clone)]
pub struct FakeImap {
    state: Arc<Mutex<State>>,
    changed: Arc<Notify>,
}

impl Default for FakeImap {
    fn default() -> Self {
        Self::new()
    }
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn uid(n: u32) -> Option<Uid> {
    Uid::new(n)
}

impl FakeImap {
    /// Serveur avec une boîte `INBOX` vide, CONDSTORE, QRESYNC et IDLE.
    #[must_use]
    pub fn new() -> Self {
        let state = State {
            mailboxes: vec![("INBOX".to_owned(), Mailbox::new(1000, vec![]))],
            next_validity: 1001,
            condstore: true,
            qresync: true,
            idle: true,
            move_ext: true,
            uidplus: true,
            offline: false,
            cut_after: None,
            operations: 0,
            delay: Duration::ZERO,
            race_deleted: None,
            append_limit: None,
        };
        Self {
            state: Arc::new(Mutex::new(state)),
            changed: Arc::new(Notify::new()),
        }
    }

    /// Extensions annoncées.
    pub fn set_extensions(&self, condstore: bool, qresync: bool, idle: bool) {
        let mut s = lock(&self.state);
        s.condstore = condstore;
        s.qresync = qresync && condstore;
        s.idle = idle;
    }

    /// Annonce (ou masque) MOVE et UIDPLUS ; masquées, les commandes
    /// correspondantes sont refusées comme par un serveur qui ne les
    /// connaît pas.
    pub fn set_move_extensions(&self, move_ext: bool, uidplus: bool) {
        let mut s = lock(&self.state);
        s.move_ext = move_ext;
        s.uidplus = uidplus;
    }

    /// Refuse les dépôts (APPEND) de plus de `limit` octets, comme un
    /// quota dépassé.
    pub fn set_append_limit(&self, limit: Option<usize>) {
        lock(&self.state).append_limit = limit;
    }

    /// Serveur injoignable : les connexions échouent.
    pub fn set_offline(&self, offline: bool) {
        lock(&self.state).offline = offline;
    }

    /// Les sessions sont coupées après `n` opérations supplémentaires
    /// (`None` : jamais).
    pub fn cut_after(&self, n: Option<usize>) {
        lock(&self.state).cut_after = n;
    }

    /// Chaque opération de session dure `delay` (serveur lent).
    pub fn set_delay(&self, delay: Duration) {
        lock(&self.state).delay = delay;
    }

    /// Nombre total d'opérations de session exécutées.
    #[must_use]
    pub fn operations(&self) -> usize {
        lock(&self.state).operations
    }

    pub(crate) fn is_offline(&self) -> bool {
        lock(&self.state).offline
    }

    fn touch(&self, name: &str) {
        if let Some(m) = lock(&self.state).mailbox(name) {
            m.version += 1;
        }
        self.changed.notify_waiters();
    }

    /// Crée une boîte (sans effet si elle existe).
    pub fn create_mailbox(&self, name: &str, attributes: &[&str]) {
        {
            let mut s = lock(&self.state);
            if s.mailbox(name).is_none() {
                let validity = s.next_validity;
                s.next_validity += 1;
                let attrs = attributes.iter().map(|a| (*a).to_owned()).collect();
                s.mailboxes
                    .push((name.to_owned(), Mailbox::new(validity, attrs)));
            }
        }
        self.changed.notify_waiters();
    }

    /// Remplace les attributs d'une boîte (rôle déplacé vers une autre).
    pub fn set_attributes(&self, name: &str, attributes: &[&str]) {
        if let Some(m) = lock(&self.state).mailbox(name) {
            m.attributes = attributes.iter().map(|a| (*a).to_owned()).collect();
        }
        self.changed.notify_waiters();
    }

    /// Supprime une boîte et ses messages.
    pub fn delete_mailbox(&self, name: &str) {
        lock(&self.state).mailboxes.retain(|(n, _)| n != name);
        self.changed.notify_waiters();
    }

    /// Supprime puis recrée une boîte avec les mêmes messages : nouvelle
    /// UIDVALIDITY, UID renumérotés à partir de 1.
    pub fn recreate_mailbox(&self, name: &str) {
        {
            let mut s = lock(&self.state);
            let validity = s.next_validity;
            s.next_validity += 1;
            if let Some(m) = s.mailbox(name) {
                let messages: Vec<Message> =
                    std::mem::take(&mut m.messages).into_values().collect();
                let version = m.version + 1;
                *m = Mailbox::new(validity, std::mem::take(&mut m.attributes));
                m.version = version;
                for mut msg in messages {
                    m.modseq += 1;
                    msg.modseq = m.modseq;
                    m.messages.insert(m.uid_next, msg);
                    m.uid_next += 1;
                }
            }
        }
        self.changed.notify_waiters();
    }

    /// Ajoute un message ; renvoie son UID.
    pub fn append(&self, name: &str, raw: &[u8], flags: Flags, internal_date: i64) -> Option<Uid> {
        let uid_value = {
            let mut s = lock(&self.state);
            let m = s.mailbox(name)?;
            m.modseq += 1;
            let u = m.uid_next;
            m.messages.insert(
                u,
                Message {
                    raw: raw.to_vec(),
                    flags,
                    modseq: m.modseq,
                    internal_date,
                },
            );
            m.uid_next += 1;
            u
        };
        self.touch(name);
        uid(uid_value)
    }

    /// Remplace les drapeaux d'un message.
    pub fn set_flags(&self, name: &str, target: Uid, flags: Flags) {
        {
            let mut s = lock(&self.state);
            if let Some(m) = s.mailbox(name) {
                m.modseq += 1;
                let modseq = m.modseq;
                if let Some(msg) = m.messages.get_mut(&target.get()) {
                    msg.flags = flags;
                    msg.modseq = modseq;
                }
            }
        }
        self.touch(name);
    }

    /// Simule un autre client qui marque `target` `\Deleted` entre le
    /// `UID SEARCH DELETED` et l'`EXPUNGE` suivants (une seule fois).
    pub fn race_deleted_before_expunge(&self, name: &str, target: Uid) {
        lock(&self.state).race_deleted = Some((name.to_owned(), target.get()));
    }

    /// Supprime définitivement un message.
    pub fn expunge(&self, name: &str, target: Uid) {
        {
            let mut s = lock(&self.state);
            if let Some(m) = s.mailbox(name)
                && m.messages.remove(&target.get()).is_some()
            {
                m.modseq += 1;
                m.expunged.push((target.get(), m.modseq));
            }
        }
        self.touch(name);
    }

    /// UID et drapeaux d'une boîte (vérifications des tests).
    #[must_use]
    pub fn flags_of(&self, name: &str) -> BTreeMap<Uid, Flags> {
        let mut s = lock(&self.state);
        s.mailbox(name)
            .map(|m| {
                m.messages
                    .iter()
                    .filter_map(|(u, msg)| Some((uid(*u)?, msg.flags.clone())))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// UIDVALIDITY d'une boîte.
    #[must_use]
    pub fn uid_validity(&self, name: &str) -> Option<UidValidity> {
        lock(&self.state)
            .mailbox(name)
            .and_then(|m| UidValidity::new(m.validity))
    }

    /// Capacités annoncées.
    #[must_use]
    pub fn capabilities(&self) -> ServerCapabilities {
        let s = lock(&self.state);
        let mut tokens = vec!["IMAP4rev1", "ENABLE", "SPECIAL-USE"];
        if s.move_ext {
            tokens.push("MOVE");
        }
        if s.uidplus {
            tokens.push("UIDPLUS");
        }
        if s.condstore {
            tokens.push("CONDSTORE");
        }
        if s.qresync {
            tokens.push("QRESYNC");
        }
        if s.idle {
            tokens.push("IDLE");
        }
        ServerCapabilities::from_imap(tokens)
    }

    /// Ouvre une session (sans authentification).
    pub fn session(&self) -> Result<FakeImapSession, ConnectionError> {
        if self.is_offline() {
            return Err(ConnectionError::Network("serveur injoignable".into()));
        }
        Ok(FakeImapSession {
            server: self.clone(),
            capabilities: self.capabilities(),
            selected: None,
            extensions: SyncExtensions::default(),
            broken: false,
        })
    }
}

/// Session sur le faux serveur.
#[derive(Debug)]
pub struct FakeImapSession {
    server: FakeImap,
    capabilities: ServerCapabilities,
    /// Boîte sélectionnée et version vue au moment de la sélection.
    selected: Option<(String, u64)>,
    extensions: SyncExtensions,
    broken: bool,
}

/// En-têtes d'un message brut, décodés par `mail-parser` comme dans
/// l'adaptateur réel.
fn headers_of(raw: &[u8]) -> MessageHeaders {
    use mail_parser::{Address, HeaderValue, MessageParser};
    let Some(m) = MessageParser::new().parse_headers(raw) else {
        return MessageHeaders::default();
    };
    let people = |a: Option<&Address<'_>>| -> Vec<(String, String)> {
        a.map(|a| {
            a.iter()
                .map(|x| {
                    (
                        x.name().unwrap_or_default().to_owned(),
                        x.address().unwrap_or_default().to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
    };
    let ids = |v: &HeaderValue<'_>| -> Vec<String> {
        match v {
            HeaderValue::Text(t) => vec![t.to_string()],
            HeaderValue::TextList(l) => l.iter().map(ToString::to_string).collect(),
            _ => vec![],
        }
    };
    let (from, to, cc) = (people(m.from()), people(m.to()), people(m.cc()));
    let (irt, refs) = (ids(m.in_reply_to()), ids(m.references()));
    let pairs = |v: &[(String, String)]| -> Vec<(String, String)> { v.to_vec() };
    let (from, to, cc) = (pairs(&from), pairs(&to), pairs(&cc));
    MessageHeaders::sanitize(&RawHeaders {
        subject: m.subject().unwrap_or_default(),
        from: from.iter().map(|(n, a)| (n.as_str(), a.as_str())).collect(),
        to: to.iter().map(|(n, a)| (n.as_str(), a.as_str())).collect(),
        cc: cc.iter().map(|(n, a)| (n.as_str(), a.as_str())).collect(),
        date: m.date().map(mail_parser::DateTime::to_timestamp),
        message_id: m.message_id(),
        in_reply_to: irt.iter().map(String::as_str).collect(),
        references: refs.iter().map(String::as_str).collect(),
    })
}

impl FakeImapSession {
    /// Attend la durée d'une opération (serveur lent).
    async fn throttle(&self) {
        let delay = lock(&self.server.state).delay;
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }

    /// Compte une opération ; échoue si la session est coupée.
    fn step(&mut self) -> Result<MutexGuard<'_, State>, ConnectionError> {
        if self.broken {
            return Err(ConnectionError::Closed);
        }
        let mut s = lock(&self.server.state);
        if s.offline {
            self.broken = true;
            return Err(ConnectionError::Closed);
        }
        if let Some(n) = s.cut_after {
            if n == 0 {
                self.broken = true;
                return Err(ConnectionError::Closed);
            }
            s.cut_after = Some(n - 1);
        }
        s.operations += 1;
        Ok(s)
    }

    fn selected_name(&self) -> Result<String, ConnectionError> {
        self.selected
            .as_ref()
            .map(|(n, _)| n.clone())
            .ok_or_else(|| ConnectionError::Protocol("aucune boîte sélectionnée".into()))
    }

    fn update(&self, u: u32, msg: &Message) -> Option<FlagUpdate> {
        Some(FlagUpdate {
            uid: uid(u)?,
            flags: msg.flags.clone(),
            modseq: if self.extensions.condstore {
                ModSeq::new(msg.modseq)
            } else {
                None
            },
        })
    }
}

impl FakeImapSession {
    /// Copie (ou déplace) un message vers `to` ; nouvel UID dans `to`.
    async fn transfer(
        &mut self,
        target: Uid,
        to: &MailboxName,
        keep_source: bool,
    ) -> Result<(), ConnectionError> {
        let name = self.selected_name()?;
        self.throttle().await;
        {
            let mut s = self.step()?;
            if s.mailbox(to.as_str()).is_none() {
                return Err(ConnectionError::Rejected(
                    "[TRYCREATE] boîte inconnue".into(),
                ));
            }
            let source = s.mailbox(&name).ok_or(ConnectionError::Closed)?;
            let Some(mut msg) = source.messages.get(&target.get()).cloned() else {
                return Ok(());
            };
            if !keep_source {
                source.messages.remove(&target.get());
                source.modseq += 1;
                let modseq = source.modseq;
                source.expunged.push((target.get(), modseq));
            }
            let dest = s.mailbox(to.as_str()).ok_or(ConnectionError::Closed)?;
            dest.modseq += 1;
            msg.modseq = dest.modseq;
            msg.flags.deleted = false;
            dest.messages.insert(dest.uid_next, msg);
            dest.uid_next += 1;
        }
        self.server.touch(&name);
        self.server.touch(to.as_str());
        Ok(())
    }

    /// Supprime définitivement les messages qui satisfont `which`.
    async fn expunge_where(
        &mut self,
        which: impl Fn(u32, &Message) -> bool,
    ) -> Result<u32, ConnectionError> {
        let name = self.selected_name()?;
        self.throttle().await;
        let count;
        {
            let mut s = self.step()?;
            let m = s.mailbox(&name).ok_or(ConnectionError::Closed)?;
            let gone: Vec<u32> = m
                .messages
                .iter()
                .filter(|(u, msg)| which(**u, msg))
                .map(|(u, _)| *u)
                .collect();
            count = u32::try_from(gone.len()).unwrap_or(u32::MAX);
            for u in gone {
                m.messages.remove(&u);
                m.modseq += 1;
                let modseq = m.modseq;
                m.expunged.push((u, modseq));
            }
        }
        self.server.touch(&name);
        Ok(count)
    }
}

impl RemoteMailboxSession for FakeImapSession {
    fn capabilities(&self) -> &ServerCapabilities {
        &self.capabilities
    }

    async fn enable_sync_extensions(&mut self) -> Result<SyncExtensions, ConnectionError> {
        self.throttle().await;
        let s = self.step()?;
        let ext = SyncExtensions {
            condstore: s.condstore,
            qresync: s.qresync,
        };
        drop(s);
        self.extensions = ext;
        Ok(ext)
    }

    async fn list_mailboxes(&mut self) -> Result<Vec<ListedMailbox>, ConnectionError> {
        self.throttle().await;
        let s = self.step()?;
        s.mailboxes
            .iter()
            .map(|(name, m)| {
                Ok(ListedMailbox {
                    name: MailboxName::parse(name)
                        .map_err(|e| ConnectionError::Protocol(e.to_string()))?,
                    delimiter: Some('/'),
                    attributes: m.attributes.clone(),
                })
            })
            .collect()
    }

    async fn select(
        &mut self,
        mailbox: &MailboxName,
        request: &SelectRequest,
    ) -> Result<SelectedMailbox, ConnectionError> {
        let ext = self.extensions;
        self.throttle().await;
        let mut s = self.step()?;
        let Some(m) = s.mailbox(mailbox.as_str()).cloned() else {
            return Err(ConnectionError::Rejected("boîte inconnue".into()));
        };
        drop(s);
        let condstore = ext.condstore && (request.condstore || request.qresync.is_some());
        let mut selected = SelectedMailbox {
            uid_validity: UidValidity::new(m.validity),
            uid_next: uid(m.uid_next),
            highest_modseq: if condstore {
                ModSeq::new(m.modseq)
            } else {
                None
            },
            exists: u32::try_from(m.messages.len()).unwrap_or(u32::MAX),
            changed: vec![],
            vanished: UidRanges::default(),
        };
        if let Some(q) = &request.qresync
            && ext.qresync
            && q.uid_validity.get() == m.validity
        {
            let since = q.modseq.get();
            selected.changed = m
                .messages
                .iter()
                .filter(|(_, msg)| msg.modseq > since)
                .filter_map(|(u, msg)| self.update(*u, msg))
                .collect();
            // Sans liste d'UID connus (comme Norelly l'envoie) : toutes
            // les suppressions depuis `since`.
            selected.vanished = UidRanges::from_uids(
                m.expunged
                    .iter()
                    .filter(|(_, ms)| *ms > since)
                    .filter_map(|(u, _)| uid(*u)),
            );
        }
        self.selected = Some((mailbox.as_str().to_owned(), m.version));
        Ok(selected)
    }

    async fn fetch_flags(
        &mut self,
        query: ServerQuery,
    ) -> Result<Vec<FlagUpdate>, ConnectionError> {
        let name = self.selected_name()?;
        self.throttle().await;
        let mut s = self.step()?;
        let m = s.mailbox(&name).cloned().ok_or(ConnectionError::Closed)?;
        drop(s);
        let since = match query {
            ServerQuery::AllFlags => 0,
            ServerQuery::ChangedFlags(m) => m.get(),
        };
        Ok(m.messages
            .iter()
            .filter(|(_, msg)| msg.modseq > since)
            .filter_map(|(u, msg)| self.update(*u, msg))
            .collect())
    }

    async fn search_all_uids(&mut self) -> Result<BTreeSet<Uid>, ConnectionError> {
        let name = self.selected_name()?;
        self.throttle().await;
        let mut s = self.step()?;
        let m = s.mailbox(&name).ok_or(ConnectionError::Closed)?;
        Ok(m.messages.keys().filter_map(|u| uid(*u)).collect())
    }

    async fn fetch_meta(&mut self, uids: &[Uid]) -> Result<Vec<MessageMeta>, ConnectionError> {
        let name = self.selected_name()?;
        let condstore = self.extensions.condstore;
        self.throttle().await;
        let mut s = self.step()?;
        let m = s.mailbox(&name).ok_or(ConnectionError::Closed)?;
        let uids: BTreeSet<Uid> = uids.iter().copied().collect();
        Ok(uids
            .iter()
            .filter_map(|u| {
                let msg = m.messages.get(&u.get())?;
                Some(MessageMeta {
                    uid: *u,
                    flags: msg.flags.clone(),
                    modseq: if condstore {
                        ModSeq::new(msg.modseq)
                    } else {
                        None
                    },
                    internal_date: msg.internal_date,
                    size: u32::try_from(msg.raw.len()).unwrap_or(u32::MAX),
                    headers: headers_of(&msg.raw),
                    structure: None,
                })
            })
            .collect())
    }

    async fn fetch_body(
        &mut self,
        target: Uid,
        fetch: &BodyFetch,
    ) -> Result<Option<FetchedBody>, ConnectionError> {
        let name = self.selected_name()?;
        self.throttle().await;
        let mut s = self.step()?;
        let m = s.mailbox(&name).ok_or(ConnectionError::Closed)?;
        let Some(msg) = m.messages.get(&target.get()) else {
            return Ok(None);
        };
        Ok(Some(match fetch {
            BodyFetch::Full => FetchedBody::Full(msg.raw.clone()),
            BodyFetch::Partial { parts, .. } => {
                let end = msg
                    .raw
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map_or(msg.raw.len(), |p| p + 4);
                FetchedBody::Partial {
                    header: msg.raw.get(..end).unwrap_or_default().to_vec(),
                    parts: parts
                        .iter()
                        .filter_map(|p| Some((p.clone(), part_entity(&msg.raw, p)?)))
                        .collect(),
                }
            }
        }))
    }

    async fn select_writable(
        &mut self,
        mailbox: &MailboxName,
    ) -> Result<SelectedMailbox, ConnectionError> {
        let request = SelectRequest {
            condstore: self.extensions.condstore,
            qresync: None,
        };
        self.select(mailbox, &request).await
    }

    async fn message_flags(&mut self, target: Uid) -> Result<Option<Flags>, ConnectionError> {
        let name = self.selected_name()?;
        self.throttle().await;
        let mut s = self.step()?;
        let m = s.mailbox(&name).ok_or(ConnectionError::Closed)?;
        Ok(m.messages.get(&target.get()).map(|msg| msg.flags.clone()))
    }

    async fn store_flag(
        &mut self,
        target: Uid,
        change: FlagChange,
        unchanged_since: Option<ModSeq>,
    ) -> Result<StoreOutcome, ConnectionError> {
        let name = self.selected_name()?;
        let condstore = self.extensions.condstore;
        self.throttle().await;
        let outcome = {
            let mut s = self.step()?;
            let m = s.mailbox(&name).ok_or(ConnectionError::Closed)?;
            let next = m.modseq + 1;
            let Some(msg) = m.messages.get_mut(&target.get()) else {
                return Ok(StoreOutcome::Stored);
            };
            if let Some(since) = unchanged_since.filter(|_| condstore)
                && msg.modseq > since.get()
            {
                return Ok(StoreOutcome::Modified);
            }
            if change.flag.get(&msg.flags) != change.value {
                change.flag.set(&mut msg.flags, change.value);
                msg.modseq = next;
                m.modseq = next;
            }
            StoreOutcome::Stored
        };
        self.server.touch(&name);
        Ok(outcome)
    }

    async fn mark_deleted(&mut self, target: Uid) -> Result<(), ConnectionError> {
        let name = self.selected_name()?;
        self.throttle().await;
        {
            let mut s = self.step()?;
            let m = s.mailbox(&name).ok_or(ConnectionError::Closed)?;
            let next = m.modseq + 1;
            if let Some(msg) = m.messages.get_mut(&target.get()) {
                msg.flags.deleted = true;
                msg.modseq = next;
                m.modseq = next;
            }
        }
        self.server.touch(&name);
        Ok(())
    }

    async fn move_to(&mut self, target: Uid, to: &MailboxName) -> Result<(), ConnectionError> {
        if !lock(&self.server.state).move_ext {
            return Err(ConnectionError::Protocol("commande inconnue : MOVE".into()));
        }
        self.transfer(target, to, false).await
    }

    async fn copy_to(&mut self, target: Uid, to: &MailboxName) -> Result<(), ConnectionError> {
        self.transfer(target, to, true).await
    }

    async fn expunge_uid(&mut self, target: Uid) -> Result<(), ConnectionError> {
        if !lock(&self.server.state).uidplus {
            return Err(ConnectionError::Protocol(
                "commande inconnue : UID EXPUNGE".into(),
            ));
        }
        self.expunge_where(|u, msg| u == target.get() && msg.flags.deleted)
            .await
            .map(|_| ())
    }

    async fn append(
        &mut self,
        mailbox: &MailboxName,
        raw: &[u8],
        draft: bool,
    ) -> Result<Option<RemoteCopy>, ConnectionError> {
        self.throttle().await;
        let (uidplus, exists, limit) = {
            let mut s = self.step()?;
            let uidplus = s.uidplus;
            let limit = s.append_limit;
            (uidplus, s.mailbox(mailbox.as_str()).is_some(), limit)
        };
        if limit.is_some_and(|l| raw.len() > l) {
            return Err(ConnectionError::Rejected(
                "NO [OVERQUOTA] quota dépassé".into(),
            ));
        }
        if !exists {
            return Err(ConnectionError::Rejected(
                "NO [TRYCREATE] boîte inexistante".into(),
            ));
        }
        let mut flags = Flags::from_tokens(["\\Seen"]);
        flags.draft = draft;
        let assigned = self.server.append(mailbox.as_str(), raw, flags, 0);
        let validity = self.server.uid_validity(mailbox.as_str());
        Ok(assigned
            .zip(validity)
            .filter(|_| uidplus)
            .map(|(uid, validity)| RemoteCopy {
                mailbox: mailbox.clone(),
                validity,
                uid,
            }))
    }

    async fn expunge_all(&mut self) -> Result<u32, ConnectionError> {
        {
            let mut s = lock(&self.server.state);
            if let Some((name, target)) = s.race_deleted.take()
                && let Some(msg) = s.mailbox(&name).and_then(|m| m.messages.get_mut(&target))
            {
                msg.flags.deleted = true;
            }
        }
        self.expunge_where(|_, msg| msg.flags.deleted).await
    }

    async fn deleted_uids(&mut self) -> Result<BTreeSet<Uid>, ConnectionError> {
        let name = self.selected_name()?;
        self.throttle().await;
        let mut s = self.step()?;
        let m = s.mailbox(&name).ok_or(ConnectionError::Closed)?;
        Ok(m.messages
            .iter()
            .filter(|(_, msg)| msg.flags.deleted)
            .filter_map(|(u, _)| uid(*u))
            .collect())
    }

    async fn idle(&mut self, max: Duration) -> Result<IdleOutcome, ConnectionError> {
        let name = self.selected_name()?;
        let deadline = tokio::time::Instant::now() + max;
        let changed = Arc::clone(&self.server.changed);
        loop {
            // Inscription avant la lecture de la version : aucun
            // changement ne peut être manqué entre les deux.
            let notified = changed.notified();
            let (supports_idle, version) = {
                self.throttle().await;
                let mut s = self.step()?;
                let version = s.mailbox(&name).map_or(u64::MAX, |m| m.version);
                (s.idle, version)
            };
            let seen = self.selected.as_ref().map_or(0, |(_, v)| *v);
            if !supports_idle {
                tokio::time::sleep_until(deadline).await;
                return Ok(IdleOutcome::Changed);
            }
            if version != seen {
                if let Some((_, v)) = self.selected.as_mut() {
                    *v = version;
                }
                return Ok(IdleOutcome::Changed);
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return Ok(IdleOutcome::Timeout);
            }
        }
    }

    async fn logout(self) -> Result<(), ConnectionError> {
        Ok(())
    }
}

/// Entité MIME (en-têtes de partie et contenu brut) d'une partie, comme
/// la renvoie l'adaptateur IMAP réel (`BODY[n.MIME]` + `BODY[n]`).
fn part_entity(raw: &[u8], path: &PartPath) -> Option<Vec<u8>> {
    let message = mail_parser::MessageParser::default().parse(raw)?;
    let mut id = 0u32;
    for (depth, n) in path.components().iter().enumerate() {
        let part = message.part(id)?;
        match part.sub_parts() {
            Some(children) => id = *children.get(usize::try_from(*n).ok()?.checked_sub(1)?)?,
            // Message non multipart : sa partie 1 est le corps.
            None if depth == 0 && *n == 1 && path.components().len() == 1 => {}
            None => return None,
        }
    }
    let part = message.part(id)?;
    let start = usize::try_from(part.raw_header_offset()).ok()?;
    let end = usize::try_from(part.raw_end_offset()).ok()?;
    raw.get(start..end).map(<[u8]>::to_vec)
}
