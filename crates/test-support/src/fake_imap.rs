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

use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::connection::ConnectionError;
use norelly_domain::fetch_policy::BodyFetch;
use norelly_domain::mailbox::{
    Flags, ListedMailbox, MailboxName, MessageHeaders, MessageMeta, ModSeq, RawHeaders, Uid,
    UidValidity,
};
use norelly_domain::ports::{FetchedBody, IdleOutcome, RemoteMailboxSession};
use norelly_domain::sync::{
    FlagUpdate, SelectRequest, SelectedMailbox, ServerQuery, SyncExtensions,
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
    offline: bool,
    /// Opérations restantes avant coupure des sessions ouvertes.
    cut_after: Option<usize>,
    /// Nombre d'opérations exécutées (toutes sessions).
    operations: usize,
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
            offline: false,
            cut_after: None,
            operations: 0,
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

    /// Serveur injoignable : les connexions échouent.
    pub fn set_offline(&self, offline: bool) {
        lock(&self.state).offline = offline;
    }

    /// Les sessions sont coupées après `n` opérations supplémentaires
    /// (`None` : jamais).
    pub fn cut_after(&self, n: Option<usize>) {
        lock(&self.state).cut_after = n;
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
        let mut tokens = vec!["IMAP4rev1", "ENABLE", "UIDPLUS", "MOVE", "SPECIAL-USE"];
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

/// En-têtes minimaux d'un message brut (`Subject`, `From`, `Message-ID`).
fn headers_of(raw: &[u8]) -> MessageHeaders {
    let text = String::from_utf8_lossy(raw);
    let header = text.split("\r\n\r\n").next().unwrap_or_default();
    let field = |name: &str| {
        header.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_owned())
        })
    };
    let subject = field("Subject").unwrap_or_default();
    let from = field("From").unwrap_or_default();
    let message_id = field("Message-ID");
    MessageHeaders::sanitize(&RawHeaders {
        subject: &subject,
        from: vec![("", from.as_str())],
        message_id: message_id.as_deref(),
        ..RawHeaders::default()
    })
}

impl FakeImapSession {
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

impl RemoteMailboxSession for FakeImapSession {
    fn capabilities(&self) -> &ServerCapabilities {
        &self.capabilities
    }

    async fn enable_sync_extensions(&mut self) -> Result<SyncExtensions, ConnectionError> {
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
        let mut s = self.step()?;
        let Some(m) = s.mailbox(mailbox.as_str()).cloned() else {
            return Err(ConnectionError::Protocol("boîte inconnue".into()));
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
            vanished: BTreeSet::new(),
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
            let known: BTreeSet<u32> = q.known_uids.iter().map(|u| u.get()).collect();
            selected.vanished = m
                .expunged
                .iter()
                .filter(|(u, ms)| *ms > since && known.contains(u))
                .filter_map(|(u, _)| uid(*u))
                .collect();
        }
        self.selected = Some((mailbox.as_str().to_owned(), m.version));
        Ok(selected)
    }

    async fn fetch_flags(
        &mut self,
        query: ServerQuery,
    ) -> Result<Vec<FlagUpdate>, ConnectionError> {
        let name = self.selected_name()?;
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
        let mut s = self.step()?;
        let m = s.mailbox(&name).ok_or(ConnectionError::Closed)?;
        Ok(m.messages.keys().filter_map(|u| uid(*u)).collect())
    }

    async fn fetch_meta(&mut self, uids: &[Uid]) -> Result<Vec<MessageMeta>, ConnectionError> {
        let name = self.selected_name()?;
        let condstore = self.extensions.condstore;
        let mut s = self.step()?;
        let m = s.mailbox(&name).ok_or(ConnectionError::Closed)?;
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
        let mut s = self.step()?;
        let m = s.mailbox(&name).ok_or(ConnectionError::Closed)?;
        let Some(msg) = m.messages.get(&target.get()) else {
            return Ok(None);
        };
        Ok(Some(match fetch {
            BodyFetch::Full => FetchedBody::Full(msg.raw.clone()),
            BodyFetch::Partial { .. } => {
                let end = msg
                    .raw
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map_or(msg.raw.len(), |p| p + 4);
                FetchedBody::Partial {
                    header: msg.raw.get(..end).unwrap_or_default().to_vec(),
                    parts: vec![],
                }
            }
        }))
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
