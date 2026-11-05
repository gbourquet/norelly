use norelly_domain::capabilities::ServerCapabilities;
use norelly_domain::compose::draft::{
    DraftAttachment, MAX_ATTACHMENTS_BYTES, REMOTE_SAVE_INTERVAL_SECS,
};
use norelly_domain::compose::outbox::{OutboxEntry, OutboxState, RETRY_BASE_SECS};
use norelly_domain::credentials::{Credentials, Secret, Username};
use norelly_domain::endpoint::{Host, Security, ServerEndpoint};
use norelly_domain::fetch_policy::FetchPolicy;
use norelly_domain::mailbox::Flags;
use norelly_domain::ports::{
    BlobId, MailStore, MessageListQuery, MessageLocator, SendError, StoredMailbox,
};
use norelly_domain::security::PlaintextPolicy;
use norelly_render::compose::MimeComposer;
use norelly_test_support::compose_contracts::InMemoryComposeStore;
use norelly_test_support::fake_imap::FakeImap;
use norelly_test_support::fakes::{FakeSession, FakeSmtp};
use norelly_test_support::mail_contracts::{InMemoryBlobStore, InMemoryMailStore};

use super::*;
use crate::sync::SyncAccount;

const NOW: i64 = 1_800_000_000;

fn account() -> AccountId {
    AccountId::new(1).unwrap_or_else(|_| panic!("compte"))
}

fn address(raw: &str) -> EmailAddress {
    EmailAddress::parse(raw).unwrap_or_else(|_| panic!("{raw}"))
}

fn me() -> Recipient {
    Recipient::new("Moi", address("moi@example.com"))
}

fn request() -> ConnectRequest {
    ConnectRequest {
        endpoint: ServerEndpoint::new(
            Host::parse("smtp.example.com").unwrap_or_else(|_| panic!("hôte")),
            465,
            Security::Tls,
        )
        .unwrap_or_else(|_| panic!("serveur")),
        credentials: Credentials::new(
            Username::parse("moi").unwrap_or_else(|_| panic!("utilisateur")),
            Secret::new("x".into()),
        )
        .unwrap_or_else(|_| panic!("identifiants")),
        certificate_exceptions: vec![],
        plaintext: PlaintextPolicy::deny_all(),
    }
}

/// Message reçu, avec `Reply-To`.
const RECEIVED: &[u8] = b"From: Alice <alice@example.com>\r\n\
Reply-To: Liste <liste@example.org>\r\n\
To: moi@example.com, bob@example.com\r\n\
Cc: carole@example.com\r\n\
Subject: AW: Re: Reunion\r\n\
Message-ID: <m2@example.com>\r\n\
References: <m1@example.com>\r\n\
Date: Mon, 2 Nov 2026 10:00:00 +0000\r\n\
Content-Type: text/plain; charset=utf-8\r\n\r\n\
Bonjour,\r\n\r\nA demain.\r\n";

/// Téléchargeur qui ne doit jamais être appelé (corps complet local).
struct NoFetch;

impl norelly_domain::ports::RemoteBodyFetcher for NoFetch {
    async fn fetch(
        &self,
        _: AccountId,
        _: &norelly_domain::mailbox::MailboxName,
        _: norelly_domain::mailbox::Uid,
        _: &norelly_domain::fetch_policy::BodyFetch,
    ) -> Result<
        Option<norelly_domain::ports::FetchedBody>,
        norelly_domain::connection::ConnectionError,
    > {
        panic!("téléchargement inattendu")
    }
}

struct World {
    imap: FakeImap,
    mail: InMemoryMailStore,
    blobs: InMemoryBlobStore,
    compose: InMemoryComposeStore,
    smtp: FakeSmtp,
    boxes: Vec<StoredMailbox>,
}

/// INBOX (un message reçu), Brouillons (`\Drafts`), Envoyés (`\Sent`),
/// synchronisés.
async fn world(uidplus: bool) -> World {
    let imap = FakeImap::new();
    imap.set_move_extensions(true, uidplus);
    imap.create_mailbox("Brouillons", &["\\Drafts"]);
    imap.create_mailbox("Envoyés", &["\\Sent"]);
    imap.append("INBOX", RECEIVED, Flags::default(), 100);
    let mut w = World {
        imap,
        mail: InMemoryMailStore::default(),
        blobs: InMemoryBlobStore::default(),
        compose: InMemoryComposeStore::default(),
        smtp: FakeSmtp::default(),
        boxes: Vec::new(),
    };
    w.sync().await;
    w
}

impl World {
    async fn sync(&mut self) {
        let mut session = self.imap.session().unwrap_or_else(|e| panic!("{e}"));
        let ext = session
            .enable_sync_extensions()
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        self.boxes = SyncAccount {
            account: account(),
            store: &self.mail,
            blobs: &self.blobs,
            policy: FetchPolicy::default(),
            text: std::sync::Arc::new(norelly_render::search_text::MimeTextExtractor),
        }
        .sync_all(&mut session, ext, |_, _| {})
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    }

    async fn received(&self) -> MessageId {
        let inbox = self
            .boxes
            .iter()
            .find(|m| m.mailbox.name.is_inbox())
            .map(|m| m.id)
            .unwrap_or_else(|| panic!("INBOX"));
        self.mail
            .page(inbox, None, 10)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .items
            .first()
            .map(|i| i.id)
            .unwrap_or_else(|| panic!("message reçu"))
    }

    fn session(&self, caps: &[&str]) -> FakeSession {
        FakeSession::new(
            ServerCapabilities::from_imap(caps.iter().copied()),
            &self.imap,
            None,
        )
        .unwrap_or_else(|e| panic!("{e}"))
    }

    fn queue(&self) -> QueueMessage<'_, InMemoryComposeStore, InMemoryBlobStore, MimeComposer> {
        QueueMessage {
            store: &self.compose,
            blobs: &self.blobs,
            composer: &MimeComposer,
        }
    }

    fn deliver(&self) -> DeliverOutbox<'_, InMemoryComposeStore, FakeSmtp> {
        DeliverOutbox {
            account: account(),
            outbox: &self.compose,
            sender: &self.smtp,
        }
    }

    fn sync_drafts(
        &self,
    ) -> SyncDrafts<'_, InMemoryComposeStore, InMemoryMailStore, InMemoryBlobStore, MimeComposer>
    {
        SyncDrafts {
            account: account(),
            drafts: &self.compose,
            folders: &self.mail,
            blobs: &self.blobs,
            composer: &MimeComposer,
        }
    }

    async fn entries(&self) -> Vec<OutboxEntry> {
        self.compose
            .outbox(account())
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .iter()
            .map(|i| i.entry)
            .collect()
    }
}

fn draft_to(to: &str) -> Draft {
    let mut d = Draft::empty(account());
    d.to = vec![Recipient::new("", address(to))];
    d.bcc = vec![Recipient::new("Caché", address("cache@example.com"))];
    d.subject = "Bonjour".into();
    d.html = Some("<p>Salut</p>".into());
    d
}

// --- Brouillons ---

#[tokio::test]
async fn save_draft_validates_then_increments_the_revision() {
    let w = world(true).await;
    let save = SaveDraft { drafts: &w.compose };
    let mut d = draft_to("bob@example.com");
    d.subject = "Salut\r\nBcc: pirate@example.com".into();
    assert_eq!(
        save.execute(None, &d, NOW).await,
        Err(ComposeFailure::Invalid(ComposeError::InvalidSubject))
    );
    let mut d = Draft::empty(account());
    d.subject = "incomplet".into();
    let first = save
        .execute(None, &d, NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(first.sync.revision, 1);
    let second = save
        .execute(Some(first.id), &d, NOW + 1)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((second.id, second.sync.revision), (first.id, 2));
}

// --- Mise en file ---

#[tokio::test]
async fn queue_message_composes_and_retires_the_draft() {
    let w = world(true).await;
    let blob = w
        .blobs
        .put(account(), b"%PDF contenu".to_vec())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut d = draft_to("bob@example.com");
    d.attachments = vec![DraftAttachment {
        blob,
        file_name: "rapport.pdf".into(),
        content_type: "application/pdf".into(),
        size: 12,
    }];
    let saved = SaveDraft { drafts: &w.compose }
        .execute(None, &d, NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let id = w
        .queue()
        .execute(Some(saved.id), &d, &me(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        w.compose
            .draft(saved.id)
            .await
            .unwrap_or_else(|e| panic!("{e}")),
        None,
        "brouillon retiré"
    );
    let items = w
        .compose
        .outbox(account())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let item = items.first().unwrap_or_else(|| panic!("message en file"));
    assert_eq!(item.id, id);
    assert_eq!(item.subject, "Bonjour");
    assert_eq!(item.message.from, "moi@example.com");
    assert_eq!(
        item.message.recipients,
        ["bob@example.com", "cache@example.com"]
    );
    let raw = String::from_utf8_lossy(&item.message.raw);
    assert!(raw.contains("rapport.pdf") && !raw.contains("cache@example.com"));
    assert!(String::from_utf8_lossy(&item.sent_copy).contains("cache@example.com"));
}

/// Phase 9 : envoi différé, en file jusqu'à l'échéance, repris pour être
/// modifié tant qu'il n'est pas parti.
#[tokio::test]
async fn scheduled_messages_wait_and_can_be_taken_back() {
    use norelly_domain::compose::outbox::{MAX_SCHEDULE_SECS, OutboxEntry};
    let w = world(true).await;
    let d = draft_to("bob@example.com");
    let at = NOW + 7200;
    let id = w
        .queue()
        .schedule(None, &d, &me(), NOW, at)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let items = w
        .compose
        .outbox(account())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        items.first().map(|i| i.entry),
        Some(OutboxEntry::queued(at))
    );
    let report = w
        .deliver()
        .execute(&request(), NOW + 60)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(report.sent.is_empty(), "pas avant l'échéance");
    assert_eq!(report.next_attempt, Some(at));
    let back = w
        .compose
        .unschedule(id, NOW + 120)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("brouillon rendu"));
    assert_eq!(back.draft, d);
    assert_eq!(
        w.queue()
            .schedule(None, &d, &me(), NOW, NOW + MAX_SCHEDULE_SECS + 1)
            .await,
        Err(ComposeFailure::Invalid(ComposeError::ScheduleTooFar))
    );
}

#[tokio::test]
async fn queue_message_refuses_invalid_drafts() {
    let w = world(true).await;
    let mut d = draft_to("bob@example.com");
    d.to.clear();
    d.bcc.clear();
    assert_eq!(
        w.queue().execute(None, &d, &me(), NOW).await,
        Err(ComposeFailure::Invalid(ComposeError::NoRecipient))
    );
    // Pièce jointe absente du stockage.
    let mut d = draft_to("bob@example.com");
    d.attachments = vec![DraftAttachment {
        blob: BlobId::parse(&"cd".repeat(32)).unwrap_or_else(|| panic!("blob")),
        file_name: "x".into(),
        content_type: "text/plain".into(),
        size: 1,
    }];
    assert_eq!(
        w.queue().execute(None, &d, &me(), NOW).await,
        Err(ComposeFailure::NotFound)
    );
    // Brouillon d'un autre compte que l'expéditeur : refusé par le stockage
    // (le brouillon est relu par identifiant).
    assert!(
        w.compose
            .outbox(account())
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .is_empty()
    );
}

#[tokio::test]
async fn queue_message_checks_the_real_attachment_sizes() {
    let w = world(true).await;
    let size = usize::try_from(MAX_ATTACHMENTS_BYTES).unwrap_or(usize::MAX) + 1;
    let blob = w
        .blobs
        .put(account(), vec![b'x'; size])
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut d = draft_to("bob@example.com");
    // Taille déclarée par l'interface : mensongère.
    d.attachments = vec![DraftAttachment {
        blob,
        file_name: "gros.bin".into(),
        content_type: "application/octet-stream".into(),
        size: 1,
    }];
    assert_eq!(
        w.queue().execute(None, &d, &me(), NOW).await,
        Err(ComposeFailure::Invalid(ComposeError::AttachmentsTooLarge))
    );
}

// --- Réponses ---

#[tokio::test]
async fn prepare_reply_fills_recipients_subject_threading_and_quote() {
    let w = world(true).await;
    let message = w.received().await;
    let prepare = PrepareReply {
        locator: &w.mail,
        blobs: &w.blobs,
        composer: &MimeComposer,
        fetcher: &NoFetch,
    };
    let own = [address("moi@example.com")];
    let reply = prepare
        .execute(message, ReplyKind::ReplyAll, &own, "Alice a écrit :")
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(reply.account, account());
    let to: Vec<String> = reply.to.iter().map(|r| r.address.to_string()).collect();
    assert_eq!(to, ["liste@example.org", "bob@example.com"]);
    let cc: Vec<String> = reply.cc.iter().map(|r| r.address.to_string()).collect();
    assert_eq!(cc, ["carole@example.com"]);
    assert_eq!(reply.subject, "Re: Reunion");
    assert_eq!(
        reply.threading.in_reply_to.as_deref(),
        Some("m2@example.com")
    );
    assert_eq!(
        reply.threading.references,
        ["m1@example.com", "m2@example.com"]
    );
    assert!(
        reply
            .text
            .contains("Alice a écrit :\n> Bonjour,\n>\n> A demain."),
        "{:?}",
        reply.text
    );
    let html = reply.html.clone().unwrap_or_default();
    assert!(
        html.contains("<blockquote>") && html.contains("A demain."),
        "{html}"
    );
    assert_eq!(reply.check_storable(), Ok(()));

    let forward = prepare
        .execute(message, ReplyKind::Forward, &own, "Message transféré")
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(forward.to.is_empty() && forward.cc.is_empty());
    assert_eq!(forward.subject, "Fwd: Reunion");
    assert_eq!(forward.threading, Default::default());

    let ghost = MessageId::new(999_999).unwrap_or_else(|| panic!("id"));
    assert_eq!(
        prepare.execute(ghost, ReplyKind::Reply, &own, "").await,
        Err(ComposeFailure::NotFound)
    );
}

// --- Envoi ---

#[tokio::test]
async fn delivery_sends_due_messages_and_retries_temporary_failures() {
    let w = world(true).await;
    let first = w
        .queue()
        .execute(None, &draft_to("a@example.com"), &me(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let second = w
        .queue()
        .execute(None, &draft_to("b@example.com"), &me(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    // Pas encore dû.
    let early = w
        .deliver()
        .execute(&request(), NOW - 1)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        early,
        DeliveryReport {
            next_attempt: Some(NOW),
            ..DeliveryReport::default()
        }
    );
    w.smtp
        .reply(Err(SendError::Deferred("451 plus tard".into())));
    let report = w
        .deliver()
        .execute(&request(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.sent, [second]);
    assert_eq!(report.failed, [(first, SendFailure::Temporary)]);
    assert_eq!(report.next_attempt, Some(NOW + RETRY_BASE_SECS));
    assert_eq!(w.smtp.accepted().len(), 1);
    let entries = w.entries().await;
    assert_eq!(
        entries,
        [
            OutboxEntry {
                state: OutboxState::Queued {
                    not_before: NOW + RETRY_BASE_SECS
                },
                attempts: 1
            },
            OutboxEntry {
                state: OutboxState::Sent,
                attempts: 1
            },
        ]
    );
}

#[tokio::test]
async fn delivery_stops_at_the_first_unreachable_server_without_counting() {
    let w = world(true).await;
    for to in ["a@example.com", "b@example.com"] {
        w.queue()
            .execute(None, &draft_to(to), &me(), NOW)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    }
    w.smtp
        .reply(Err(SendError::Connection(ConnectionError::Network(
            "hors ligne".into(),
        ))));
    let report = w
        .deliver()
        .execute(&request(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.failed.len(), 1, "le second n'est pas tenté");
    assert!(w.smtp.accepted().is_empty());
    let entries = w.entries().await;
    assert_eq!(entries[0].attempts, 0, "hors ligne : non compté");
    assert_eq!(entries[1], OutboxEntry::queued(NOW));
}

#[tokio::test]
async fn permanent_refusal_fails_the_message() {
    let w = world(true).await;
    let id = w
        .queue()
        .execute(None, &draft_to("a@example.com"), &me(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.smtp.reply(Err(SendError::Rejected("550 inconnu".into())));
    let report = w
        .deliver()
        .execute(&request(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.failed, [(id, SendFailure::Rejected)]);
    assert_eq!(report.next_attempt, None);
    assert_eq!(
        w.entries().await.first().map(|e| e.state),
        Some(OutboxState::Failed(SendFailure::Rejected))
    );
}

#[tokio::test]
async fn interrupted_sends_are_recovered_as_uncertain() {
    let w = world(true).await;
    let id = w
        .queue()
        .execute(None, &draft_to("a@example.com"), &me(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let queued = OutboxEntry::queued(NOW);
    let sending = queued.start().unwrap_or_else(|e| panic!("{e}"));
    assert!(
        w.compose
            .transition(id, queued, sending)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    );
    let recovered = RecoverOutbox {
        account: account(),
        outbox: &w.compose,
    }
    .execute()
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(recovered, 1);
    assert_eq!(
        w.entries().await.first().map(|e| e.state),
        Some(OutboxState::Failed(SendFailure::Uncertain))
    );
    let report = w
        .deliver()
        .execute(&request(), NOW + 10_000)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        report.sent.is_empty() && w.smtp.accepted().is_empty(),
        "jamais renvoyé"
    );
}

// --- Copies dans Envoyés ---

#[tokio::test]
async fn sent_messages_are_filed_in_sent_then_removed() {
    let w = world(true).await;
    w.queue()
        .execute(None, &draft_to("a@example.com"), &me(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.deliver()
        .execute(&request(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut session = w.session(&["IMAP4rev1", "UIDPLUS"]);
    let filed = FileSentCopies {
        account: account(),
        outbox: &w.compose,
        folders: &w.mail,
    }
    .execute(&mut session)
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(filed, 1);
    let sent = w.imap.flags_of("Envoyés");
    assert_eq!(sent.len(), 1);
    assert!(sent.values().all(|f| f.seen && !f.draft));
    assert!(w.entries().await.is_empty(), "retiré de la file");
}

#[tokio::test]
async fn gmail_files_sent_messages_itself() {
    let w = world(true).await;
    w.queue()
        .execute(None, &draft_to("a@example.com"), &me(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.deliver()
        .execute(&request(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut session = w.session(&["IMAP4rev1", "UIDPLUS", "X-GM-EXT-1"]);
    FileSentCopies {
        account: account(),
        outbox: &w.compose,
        folders: &w.mail,
    }
    .execute(&mut session)
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(w.imap.flags_of("Envoyés").is_empty(), "pas de doublon");
    assert!(w.entries().await.is_empty());
}

/// Message reçu avec un devis PDF en pièce jointe.
const WITH_ATTACHMENT: &[u8] = b"From: Alice <alice@example.com>\r\nTo: moi@example.com\r\n\
Subject: Devis\r\nMessage-ID: <devis@example.com>\r\nDate: Mon, 2 Nov 2026 11:00:00 +0000\r\n\
Content-Type: multipart/mixed; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nVoici le devis.\r\n\
--b\r\nContent-Type: application/pdf; name=devis.pdf\r\n\
Content-Disposition: attachment; filename=devis.pdf\r\n\
Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQ=\r\n--b--\r\n";

/// Téléchargeur du message entier (corps partiel localement).
struct FullFetch(Vec<u8>, std::sync::atomic::AtomicUsize);

impl norelly_domain::ports::RemoteBodyFetcher for FullFetch {
    async fn fetch(
        &self,
        _: AccountId,
        _: &norelly_domain::mailbox::MailboxName,
        _: norelly_domain::mailbox::Uid,
        fetch: &norelly_domain::fetch_policy::BodyFetch,
    ) -> Result<
        Option<norelly_domain::ports::FetchedBody>,
        norelly_domain::connection::ConnectionError,
    > {
        assert_eq!(fetch, &norelly_domain::fetch_policy::BodyFetch::Full);
        self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(Some(norelly_domain::ports::FetchedBody::Full(
            self.0.clone(),
        )))
    }
}

/// Phase 9 : le transfert reprend les pièces jointes du message d'origine.
#[tokio::test]
async fn forward_carries_the_original_attachments() {
    let mut w = world(true).await;
    w.imap
        .append("INBOX", WITH_ATTACHMENT, Flags::default(), 200);
    w.sync().await;
    let message = w.received().await;
    let own = [address("moi@example.com")];
    let draft = PrepareReply {
        locator: &w.mail,
        blobs: &w.blobs,
        composer: &MimeComposer,
        fetcher: &NoFetch,
    }
    .execute(message, ReplyKind::Forward, &own, "Message transféré")
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(draft.subject, "Fwd: Devis");
    let names: Vec<&str> = draft
        .attachments
        .iter()
        .map(|a| a.file_name.as_str())
        .collect();
    assert_eq!(names, ["devis.pdf"]);
    let a = &draft.attachments[0];
    assert_eq!((a.content_type.as_str(), a.size), ("application/pdf", 8));
    let bytes = w
        .blobs
        .get(account(), &a.blob)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(bytes.as_deref(), Some(&b"%PDF-1.4"[..]));
    // Répondre ne reprend pas les pièces jointes.
    let reply = PrepareReply {
        locator: &w.mail,
        blobs: &w.blobs,
        composer: &MimeComposer,
        fetcher: &NoFetch,
    }
    .execute(message, ReplyKind::Reply, &own, "a écrit")
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(reply.attachments.is_empty());
}

/// Corps partiel localement (pièce jointe différée) : le message entier
/// est téléchargé pour le transfert.
#[tokio::test]
async fn forward_downloads_deferred_attachments() {
    let mut w = world(true).await;
    w.imap
        .append("INBOX", WITH_ATTACHMENT, Flags::default(), 200);
    w.sync().await;
    let message = w.received().await;
    // Corps remplacé par un corps partiel (en-têtes seulement).
    let location = w
        .mail
        .locate(message)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("message"));
    let header = w
        .blobs
        .put(account(), b"Subject: Devis\r\n\r\n".to_vec())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.mail
        .set_body(
            location.mailbox,
            location.uid,
            &StoredBody::Partial {
                header,
                parts: vec![],
                deferred: vec![
                    norelly_domain::fetch_policy::PartPath::parse("2")
                        .unwrap_or_else(|| panic!("chemin")),
                ],
            },
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let fetcher = FullFetch(
        WITH_ATTACHMENT.to_vec(),
        std::sync::atomic::AtomicUsize::new(0),
    );
    let draft = PrepareReply {
        locator: &w.mail,
        blobs: &w.blobs,
        composer: &MimeComposer,
        fetcher: &fetcher,
    }
    .execute(
        message,
        ReplyKind::Forward,
        &[address("moi@example.com")],
        "Transféré",
    )
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(fetcher.1.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(draft.attachments.len(), 1);
}

/// Phase 9 : un brouillon écrit par un autre client (dossier
/// « Brouillons ») se reprend tel quel.
#[tokio::test]
async fn a_draft_from_another_client_is_resumed_as_is() {
    let mut w = world(true).await;
    let foreign: &[u8] = b"From: moi@example.com\r\nTo: Bob <bob@example.com>, pas-une-adresse\r\n\
Subject: Pas fini\r\nIn-Reply-To: <parent@example.com>\r\nReferences: <parent@example.com>\r\n\
Content-Type: multipart/mixed; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nTexte du brouillon <b>brut</b>\r\n\
--b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=plan.pdf\r\n\
Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQ=\r\n--b--\r\n";
    w.imap.append(
        "Brouillons",
        foreign,
        Flags::from_tokens(["\\Draft", "\\Seen"]),
        300,
    );
    w.sync().await;
    let drafts_box = w
        .boxes
        .iter()
        .find(|m| m.mailbox.name.as_str() == "Brouillons")
        .map(|m| m.id)
        .unwrap_or_else(|| panic!("Brouillons"));
    let message = w
        .mail
        .page(drafts_box, None, 10)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .items
        .first()
        .map(|i| i.id)
        .unwrap_or_else(|| panic!("brouillon distant"));
    let draft = PrepareReply {
        locator: &w.mail,
        blobs: &w.blobs,
        composer: &MimeComposer,
        fetcher: &NoFetch,
    }
    .resume(message)
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(draft.subject, "Pas fini");
    assert_eq!(
        draft
            .to
            .iter()
            .map(|r| r.address.to_string())
            .collect::<Vec<_>>(),
        ["bob@example.com"]
    );
    let html = draft.html.unwrap_or_default();
    assert!(
        html.contains("Texte du brouillon &lt;b&gt;brut&lt;/b&gt;"),
        "{html}"
    );
    assert!(!html.contains("blockquote"), "pas de citation");
    assert_eq!(
        draft.threading.in_reply_to.as_deref(),
        Some("parent@example.com")
    );
    assert_eq!(draft.attachments.len(), 1);
}

/// Revue de la phase 9 (BASSE) : un message trop gros est refusé avant
/// l'analyse de ses pièces jointes, même complet localement.
#[tokio::test]
async fn oversized_messages_are_refused_before_parsing() {
    let w = world(true).await;
    let prepare = PrepareReply {
        locator: &w.mail,
        blobs: &w.blobs,
        composer: &MimeComposer,
        fetcher: &NoFetch,
    };
    let size = usize::try_from(2 * MAX_ATTACHMENTS_BYTES).unwrap_or(usize::MAX) + 1;
    let started = std::time::Instant::now();
    assert_eq!(
        prepare
            .forwarded_attachments(account(), &vec![b'x'; size])
            .await,
        Err(ComposeFailure::Invalid(ComposeError::AttachmentsTooLarge))
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

// --- Brouillons sur le serveur ---

#[tokio::test]
async fn drafts_are_copied_replaced_then_removed_on_the_server() {
    let w = world(true).await;
    let caps = ["IMAP4rev1", "UIDPLUS"];
    let save = SaveDraft { drafts: &w.compose };
    let d = draft_to("bob@example.com");
    let saved = save
        .execute(None, &d, NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    let mut session = w.session(&caps);
    let report = w
        .sync_drafts()
        .execute(&mut session, &me(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.saved, 1);
    let on_server = w.imap.flags_of("Brouillons");
    assert_eq!(on_server.len(), 1);
    assert!(on_server.values().all(|f| f.draft && f.seen));
    let first_uid = on_server
        .keys()
        .next()
        .copied()
        .unwrap_or_else(|| panic!("uid"));

    // Modifié, trop tôt : rien ; après l'intervalle : remplacé.
    save.execute(Some(saved.id), &d, NOW + 1)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let report = w
        .sync_drafts()
        .execute(&mut session, &me(), NOW + 1)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.saved, 0);
    let later = NOW + REMOTE_SAVE_INTERVAL_SECS;
    let report = w
        .sync_drafts()
        .execute(&mut session, &me(), later)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((report.saved, report.removed), (1, 1));
    let on_server = w.imap.flags_of("Brouillons");
    assert_eq!(on_server.len(), 1, "ancienne copie supprimée");
    assert!(!on_server.contains_key(&first_uid));

    // Brouillon supprimé localement : sa copie disparaît du serveur.
    w.compose
        .delete_draft(saved.id)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let report = w
        .sync_drafts()
        .execute(&mut session, &me(), later + 1)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.removed, 1);
    assert!(w.imap.flags_of("Brouillons").is_empty());
    assert!(
        w.compose
            .remote_trash(account())
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .is_empty()
    );
}

#[tokio::test]
async fn drafts_stay_local_without_uidplus() {
    let w = world(false).await;
    SaveDraft { drafts: &w.compose }
        .execute(None, &draft_to("bob@example.com"), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut session = w.session(&["IMAP4rev1"]);
    let report = w
        .sync_drafts()
        .execute(&mut session, &me(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report, DraftSyncReport::default());
    assert!(w.imap.flags_of("Brouillons").is_empty());
}

#[tokio::test]
async fn stale_or_foreign_copies_are_never_deleted() {
    let w = world(true).await;
    let caps = ["IMAP4rev1", "UIDPLUS"];
    let save = SaveDraft { drafts: &w.compose };
    let saved = save
        .execute(None, &draft_to("bob@example.com"), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut session = w.session(&caps);
    w.sync_drafts()
        .execute(&mut session, &me(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let copy_uid = w
        .imap
        .flags_of("Brouillons")
        .keys()
        .next()
        .copied()
        .unwrap_or_else(|| panic!("copie"));
    // Un autre client retire \Draft de ce message : ce n'est plus notre
    // brouillon, il ne doit pas être supprimé.
    let mut seen = Flags::default();
    seen.seen = true;
    w.imap.set_flags("Brouillons", copy_uid, seen);
    w.compose
        .delete_draft(saved.id)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let report = w
        .sync_drafts()
        .execute(&mut session, &me(), NOW + 1)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.removed, 1, "oubliée");
    assert_eq!(w.imap.flags_of("Brouillons").len(), 1, "message conservé");

    // Boîte recréée (UIDVALIDITY changée) : l'UID désigne un autre message.
    let saved = save
        .execute(None, &draft_to("bob@example.com"), NOW + 2)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.sync_drafts()
        .execute(&mut session, &me(), NOW + 2)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.imap.recreate_mailbox("Brouillons");
    w.imap
        .append("Brouillons", RECEIVED, Flags::from_tokens(["\\Draft"]), 0);
    let before = w.imap.flags_of("Brouillons").len();
    w.compose
        .delete_draft(saved.id)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let report = w
        .sync_drafts()
        .execute(&mut session, &me(), NOW + 3)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.removed, 1, "oubliée");
    assert_eq!(
        w.imap.flags_of("Brouillons").len(),
        before,
        "rien supprimé dans la boîte recréée"
    );
}

/// Revue de la phase 7 (BASSE) : une ancienne copie est supprimée dans le
/// dossier où elle a été déposée, même si le rôle `\Drafts` est passé à un
/// autre dossier entre-temps.
#[tokio::test]
async fn old_copies_are_removed_from_their_own_mailbox() {
    let mut w = world(true).await;
    let caps = ["IMAP4rev1", "UIDPLUS"];
    let saved = SaveDraft { drafts: &w.compose }
        .execute(None, &draft_to("bob@example.com"), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut session = w.session(&caps);
    w.sync_drafts()
        .execute(&mut session, &me(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(w.imap.flags_of("Brouillons").len(), 1);
    w.imap.set_attributes("Brouillons", &[]);
    w.imap.create_mailbox("Drafts", &["\\Drafts"]);
    w.sync().await;
    w.compose
        .delete_draft(saved.id)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut session = w.session(&caps);
    let report = w
        .sync_drafts()
        .execute(&mut session, &me(), NOW + 1)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.removed, 1);
    assert!(
        w.imap.flags_of("Brouillons").is_empty(),
        "copie supprimée dans son propre dossier"
    );
}

/// Revue de la phase 7 (BASSE) : un brouillon refusé par le serveur
/// (quota) ne bloque ni les autres brouillons ni la corbeille, et n'est
/// retenté qu'après une longue attente.
#[tokio::test]
async fn a_refused_draft_does_not_block_the_others() {
    use norelly_domain::compose::draft::REMOTE_REFUSED_RETRY_SECS;
    let w = world(true).await;
    let caps = ["IMAP4rev1", "UIDPLUS"];
    let save = SaveDraft { drafts: &w.compose };
    // Une ancienne copie à supprimer (brouillon supprimé localement).
    let gone = save
        .execute(None, &draft_to("old@example.com"), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut session = w.session(&caps);
    w.sync_drafts()
        .execute(&mut session, &me(), NOW)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.compose
        .delete_draft(gone.id)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    // Un gros brouillon (refusé) puis un petit.
    let mut big = draft_to("bob@example.com");
    big.html = Some(format!("<p>{}</p>", "mot ".repeat(50_000)));
    save.execute(None, &big, NOW + 1)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    save.execute(None, &draft_to("carole@example.com"), NOW + 1)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.imap.set_append_limit(Some(50_000));
    let report = w
        .sync_drafts()
        .execute(&mut session, &me(), NOW + 1)
        .await
        .unwrap_or_else(|e| panic!("refus propagé : {e}"));
    assert_eq!((report.saved, report.removed), (1, 1));
    // Quota levé : pas de nouvel essai avant la longue attente.
    w.imap.set_append_limit(None);
    let soon = NOW + 1 + REMOTE_SAVE_INTERVAL_SECS;
    let report = w
        .sync_drafts()
        .execute(&mut session, &me(), soon)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.saved, 0);
    let later = NOW + 1 + REMOTE_REFUSED_RETRY_SECS;
    let report = w
        .sync_drafts()
        .execute(&mut session, &me(), later)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.saved, 1);
}

/// Revue de la phase 7 (BASSE) : mot de passe ou certificat refusé, même
/// cause pour tous les messages : la tournée s'arrête (pas de rafale
/// d'échecs d'authentification).
#[tokio::test]
async fn delivery_stops_on_authentication_or_security_failure() {
    use norelly_domain::auth::AuthError;
    for (error, failure) in [
        (
            ConnectionError::Authentication(AuthError::Rejected {
                message: "535".into(),
            }),
            SendFailure::Authentication,
        ),
        (
            ConnectionError::Tls("certificat".into()),
            SendFailure::Security,
        ),
    ] {
        let w = world(true).await;
        for to in ["a@example.com", "b@example.com"] {
            w.queue()
                .execute(None, &draft_to(to), &me(), NOW)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
        }
        w.smtp.reply(Err(SendError::Connection(error)));
        let report = w
            .deliver()
            .execute(&request(), NOW)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            report.failed.len(),
            1,
            "{failure:?} : le second n'est pas tenté"
        );
        assert_eq!(report.failed[0].1, failure);
        assert_eq!(w.entries().await[1], OutboxEntry::queued(NOW));
    }
}

/// Composeur qui note le fil où l'analyse des pièces jointes s'exécute.
#[derive(Debug, Clone, Default)]
struct ThreadRecorder(std::sync::Arc<std::sync::Mutex<Option<std::thread::ThreadId>>>);

impl MessageComposer for ThreadRecorder {
    fn new_message_id(&self, from: &EmailAddress) -> Result<String, ComposeBuildError> {
        MimeComposer.new_message_id(from)
    }
    fn compose(&self, composition: &Composition) -> Result<ComposedMessage, ComposeBuildError> {
        MimeComposer.compose(composition)
    }
    fn reply_source(&self, raw: &[u8]) -> Option<norelly_domain::compose::ReplySource> {
        MimeComposer.reply_source(raw)
    }
    fn forwarded_attachments(&self, raw: &[u8]) -> Result<Vec<ComposedAttachment>, ComposeError> {
        if let Ok(mut seen) = self.0.lock() {
            *seen = Some(std::thread::current().id());
        }
        MimeComposer.forwarded_attachments(raw)
    }
    fn text_html(&self, text: &str) -> String {
        MimeComposer.text_html(text)
    }
    fn quote_html(&self, text: &str, attribution: &str) -> String {
        MimeComposer.quote_html(text, attribution)
    }
}

/// Revue de la phase 9 (BASSE, report) : l'analyse des pièces jointes
/// transférées ne bloque pas le fil asynchrone.
#[tokio::test]
async fn forwarded_attachments_are_parsed_off_the_async_thread() {
    let w = world(true).await;
    let recorder = ThreadRecorder::default();
    let prepare = PrepareReply {
        locator: &w.mail,
        blobs: &w.blobs,
        composer: &recorder,
        fetcher: &NoFetch,
    };
    let found = prepare
        .forwarded_attachments(account(), WITH_ATTACHMENT)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(found.len(), 1);
    let seen = recorder.0.lock().ok().and_then(|seen| *seen);
    assert!(seen.is_some());
    assert_ne!(seen, Some(std::thread::current().id()));
}
