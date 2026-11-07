//! Commandes IPC appelées sans webview (application factice) : entrées
//! revalidées, codes d'erreur, effets sur le stockage réel.

use crate::actions::{perform_action, set_archive_folder, undo_actions};
use crate::compose::{
    cancel_send, discard_draft, outbox, prepare_compose, retry_send, save_draft, send_draft,
    suggest_contacts,
};
use crate::ipc::{
    ActionDto, ComposeKindDto, ErrorCodeDto, FieldDto, IpcError, OutboxStateDto, RecipientDto,
    ScopeDto,
};
use crate::reading::{list_conversations, list_folders};
use crate::test_app::test_app;

fn code<T: std::fmt::Debug>(r: Result<T, IpcError>) -> (ErrorCodeDto, Option<FieldDto>) {
    match r {
        Err(e) => (e.code, e.field),
        Ok(v) => panic!("erreur attendue : {v:?}"),
    }
}

fn id32(raw: i64) -> u32 {
    u32::try_from(raw).unwrap_or_else(|_| panic!("identifiant"))
}

#[tokio::test]
async fn folders_conversations_and_actions_round_trip() {
    let t = test_app().await;
    let account = id32(t.account.get());
    let folders = list_folders(t.state(), account)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(folders.len(), 3);
    let inbox = folders
        .iter()
        .find(|f| f.role == "inbox")
        .map(|f| f.id)
        .unwrap_or_else(|| panic!("INBOX"));
    let scope = ScopeDto::Mailbox { id: inbox };
    let page = list_conversations(t.state(), scope.clone(), None, 50)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(page.items.len(), 2);
    let first = page.items[0].id;

    let result = perform_action(t.state(), vec![first], scope.clone(), ActionDto::Delete)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(result.messages, 1);
    let after = list_conversations(t.state(), scope.clone(), None, 50)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(after.items.len(), 1, "masquée tout de suite");
    let undone = undo_actions(t.state(), result.ops)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(undone, 1);

    // Entrées refusées.
    assert_eq!(
        code(perform_action(t.state(), vec![0], scope.clone(), ActionDto::Delete).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Conversation))
    );
    assert_eq!(
        code(
            perform_action(
                t.state(),
                vec![first; 1001],
                scope.clone(),
                ActionDto::Archive
            )
            .await
        )
        .0,
        ErrorCodeDto::TooManyMessages
    );
    assert_eq!(
        code(undo_actions(t.state(), vec![0]).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Message))
    );
    // Pas de dossier d'archives ni de repli : refus explicite.
    assert_eq!(
        code(perform_action(t.state(), vec![first], scope, ActionDto::Archive).await).0,
        ErrorCodeDto::NoArchive
    );
    // Repli : seulement un dossier du compte.
    assert_eq!(
        code(set_archive_folder(t.state(), account, Some(999_999)).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Mailbox))
    );
    set_archive_folder(t.state(), account, Some(inbox))
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        code(set_archive_folder(t.state(), 0, None).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Account))
    );
}

#[tokio::test]
async fn drafts_are_saved_sent_listed_and_cancelled() {
    let t = test_app().await;
    let account = id32(t.account.get());
    let mut draft = prepare_compose(t.state(), account, ComposeKindDto::New, None, String::new())
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!((draft.account, draft.id), (account, None));
    // Réponse sans message : refusée.
    assert_eq!(
        code(
            prepare_compose(
                t.state(),
                account,
                ComposeKindDto::Reply,
                None,
                String::new()
            )
            .await
        ),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Message))
    );

    draft.subject = "Bonjour".into();
    draft.html = "<p>Salut</p>".into();
    let saved = save_draft(t.state(), draft.clone())
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(saved.revision, 1);
    draft.id = Some(saved.id);
    // Sans destinataire : refus du domaine.
    assert_eq!(
        code(send_draft(t.state(), draft.clone()).await).0,
        ErrorCodeDto::NoRecipient
    );
    draft.to = vec![RecipientDto {
        name: "Bob".into(),
        address: "bob@example.com".into(),
    }];
    let queued = send_draft(t.state(), draft.clone())
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    let items = outbox(t.state(), account)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, queued);
    assert!(matches!(items[0].state, OutboxStateDto::Queued { .. }));
    // Pas en échec : rien à relancer ; annulation possible.
    assert_eq!(
        code(retry_send(t.state(), account, queued).await).0,
        ErrorCodeDto::OutboxBusy
    );
    cancel_send(t.state(), account, queued)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(
        outbox(t.state(), account)
            .await
            .unwrap_or_else(|e| panic!("{e:?}"))
            .is_empty()
    );
    assert_eq!(
        code(cancel_send(t.state(), account, queued).await),
        (ErrorCodeDto::NotFound, Some(FieldDto::Outbox))
    );

    // Brouillon abandonné.
    let other = save_draft(
        t.state(),
        crate::ipc::DraftDto {
            id: None,
            ..draft.clone()
        },
    )
    .await
    .unwrap_or_else(|e| panic!("{e:?}"));
    discard_draft(t.state(), other.id)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        code(discard_draft(t.state(), 0).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Draft))
    );
}

#[tokio::test]
async fn contacts_come_from_received_messages() {
    let t = test_app().await;
    let account = id32(t.account.get());
    let found = suggest_contacts(t.state(), account, "e1@".into())
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        found.iter().map(|r| r.address.as_str()).collect::<Vec<_>>(),
        ["e1@example.com"]
    );
    assert_eq!(
        code(suggest_contacts(t.state(), 0, "e".into()).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Account))
    );
    assert!(!t.messages.is_empty());
}

#[tokio::test]
async fn accounts_are_listed_and_removed() {
    use crate::commands::{list_accounts, remove_account, trust_certificate};
    let t = test_app().await;
    let accounts = list_accounts(t.state())
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].address, "alice@example.com");
    // Certificat jamais présenté à l'utilisateur : pas de confiance possible.
    assert_eq!(
        code(trust_certificate(t.state(), "imap.example.com".into(), "AA:BB".into()).await),
        (ErrorCodeDto::NotFound, Some(FieldDto::Certificate))
    );
    assert_eq!(
        code(remove_account(t.state(), 0).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Account))
    );
    remove_account(t.state(), id32(t.account.get()))
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(
        list_accounts(t.state())
            .await
            .unwrap_or_else(|e| panic!("{e:?}"))
            .is_empty()
    );
}

#[tokio::test]
async fn reading_commands_revalidate_and_answer() {
    use crate::reading::{
        allow_remote_images, conversation_messages, open_attachment, open_external, open_message,
        reading_preferences, set_prefer_plain_text, sync_account,
    };
    let t = test_app().await;
    let account = id32(t.account.get());
    let message = id32(t.messages[0].get());

    // Message ouvert (corps absent : vue sans contenu, pas d'erreur).
    let view = open_message(t.state(), message)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(view.id, message);
    assert_eq!(
        code(open_message(t.state(), 0).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Message))
    );
    assert_eq!(
        code(open_message(t.state(), 999_999).await).0,
        ErrorCodeDto::NotFound
    );

    // Conversation du message.
    let folders = list_folders(t.state(), account)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    let inbox = folders
        .iter()
        .find(|f| f.role == "inbox")
        .map(|f| f.id)
        .unwrap_or_else(|| panic!("INBOX"));
    let page = list_conversations(t.state(), ScopeDto::Mailbox { id: inbox }, None, 10)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    let messages = conversation_messages(t.state(), page.items[0].id)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(messages.len(), 1);
    assert_eq!(
        code(conversation_messages(t.state(), 0).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Conversation))
    );

    // Préférences de lecture.
    set_prefer_plain_text(t.state(), true)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(
        reading_preferences(t.state())
            .await
            .unwrap_or_else(|e| panic!("{e:?}"))
            .prefer_plain_text
    );
    allow_remote_images(t.state(), message)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));

    // Liens : seuls http, https et mailto (aucune ouverture tentée ici).
    for bad in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/html,x",
    ] {
        assert_eq!(
            code(open_external(bad.into()).await).0,
            ErrorCodeDto::LinkRefused,
            "{bad}"
        );
    }
    // Pièce jointe introuvable, partie invalide.
    assert_eq!(
        code(open_attachment(t.state(), message, "pas-une-partie".into()).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Part))
    );
    assert_eq!(
        code(open_attachment(t.state(), message, "1.9".into()).await).0,
        ErrorCodeDto::NotFound
    );
    // Compte inconnu ou invalide.
    assert_eq!(
        code(sync_account(t.state(), 0).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Account))
    );
}

fn plain(segments: &[crate::ipc::SegmentDto]) -> String {
    segments.iter().map(|s| s.text.as_str()).collect()
}

fn hits(segments: &[crate::ipc::SegmentDto]) -> Vec<&str> {
    segments
        .iter()
        .filter(|s| s.hit)
        .map(|s| s.text.as_str())
        .collect()
}

#[tokio::test]
async fn search_finds_messages_with_highlighted_segments() {
    use crate::ipc::SearchScopeDto;
    use crate::search::search_messages;
    let t = test_app().await;
    let account = id32(t.account.get());
    let page = search_messages(t.state(), "sujet".into(), SearchScopeDto::All, 60, 0, 50)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.next, None);
    let subjects: Vec<String> = page.items.iter().map(|h| plain(&h.subject)).collect();
    assert_eq!(
        subjects,
        ["Sujet 2", "Sujet 1"],
        "à pertinence égale : le plus récent"
    );
    let first = &page.items[0];
    assert_eq!(hits(&first.subject), ["Sujet"]);
    assert_eq!(first.account, account);
    assert!(first.conversation.is_some());
    assert!(first.unread && !first.flagged);
    assert_eq!(
        first.from.as_ref().map(|f| f.address.as_str()),
        Some("e2@example.com")
    );

    let one = search_messages(t.state(), "from:e1".into(), SearchScopeDto::All, 0, 0, 50)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        one.items
            .iter()
            .map(|h| plain(&h.subject))
            .collect::<Vec<_>>(),
        ["Sujet 1"]
    );
    assert!(
        hits(&one.items[0].subject).is_empty(),
        "terme de l'expéditeur : sujet non surligné"
    );

    let scoped = search_messages(
        t.state(),
        "sujet".into(),
        SearchScopeDto::Account { id: account },
        0,
        1,
        1,
    )
    .await
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(scoped.items.len(), 1);
    assert_eq!(scoped.next, None, "dernière page");

    let empty = search_messages(t.state(), "  ".into(), SearchScopeDto::All, 0, 0, 50)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(empty.items.is_empty());
}

#[tokio::test]
async fn search_inputs_are_revalidated() {
    use crate::ipc::SearchScopeDto;
    use crate::search::search_messages;
    let t = test_app().await;
    let long = "a".repeat(norelly_domain::search::MAX_QUERY_CHARS + 1);
    assert_eq!(
        code(search_messages(t.state(), long, SearchScopeDto::All, 0, 0, 50).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Query))
    );
    let many = vec!["a"; norelly_domain::search::MAX_CLAUSES + 1].join(" ");
    assert_eq!(
        code(search_messages(t.state(), many, SearchScopeDto::All, 0, 0, 50).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Query))
    );
    assert_eq!(
        code(search_messages(t.state(), "a".into(), SearchScopeDto::All, 15 * 60, 0, 50).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Query))
    );
    assert_eq!(
        code(
            search_messages(
                t.state(),
                "a".into(),
                SearchScopeDto::Account { id: 0 },
                0,
                0,
                50
            )
            .await
        ),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Account))
    );
    assert_eq!(
        code(
            search_messages(
                t.state(),
                "a".into(),
                SearchScopeDto::Mailbox { id: 0 },
                0,
                0,
                50
            )
            .await
        ),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Mailbox))
    );
}

#[tokio::test]
async fn settings_round_trip_and_are_revalidated() {
    use crate::ipc::{LanguageDto, SettingsDto, UpdateCheckDto};
    use crate::settings::{get_settings, save_settings};
    let t = test_app().await;
    let first = get_settings(t.state())
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(first.language, LanguageDto::System);
    assert_eq!(
        first.update_check,
        UpdateCheckDto::NotAsked,
        "question pas encore posée"
    );
    assert!(first.notifications && !first.notification_privacy);
    let chosen = SettingsDto {
        language: LanguageDto::En,
        update_check: UpdateCheckDto::Enabled,
        notifications: true,
        notification_privacy: true,
        attachment_threshold: 4 * 1024 * 1024,
    };
    save_settings(t.state(), chosen)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        get_settings(t.state())
            .await
            .unwrap_or_else(|e| panic!("{e:?}")),
        chosen
    );
    let invalid = SettingsDto {
        attachment_threshold: 1,
        ..chosen
    };
    assert_eq!(
        code(save_settings(t.state(), invalid).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Settings))
    );
}

#[tokio::test]
async fn drafts_are_listed_reopened_scheduled_and_taken_back() {
    use crate::compose::{list_drafts, open_draft, schedule_draft, unschedule_send};
    let t = test_app().await;
    let account = id32(t.account.get());
    let mut draft = prepare_compose(t.state(), account, ComposeKindDto::New, None, String::new())
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    draft.subject = "Programmé".into();
    draft.html = "<p>Plus tard</p>".into();
    draft.to = vec![RecipientDto {
        name: "Bob".into(),
        address: "bob@example.com".into(),
    }];
    let saved = save_draft(t.state(), draft.clone())
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    draft.id = Some(saved.id);
    let listed = list_drafts(t.state(), account)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"))
        .local;
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].id, listed[0].subject.as_str()),
        (saved.id, "Programmé")
    );
    assert_eq!(listed[0].to, ["bob@example.com"]);
    let reopened = open_draft(t.state(), saved.id)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(reopened.subject, "Programmé");
    assert_eq!(reopened.id, Some(saved.id));
    assert_eq!(
        code(open_draft(t.state(), 9999).await),
        (ErrorCodeDto::NotFound, Some(FieldDto::Draft))
    );

    let in_an_hour =
        u32::try_from(norelly_application::account_actor::unix_now() + 3600).unwrap_or(u32::MAX);
    let queued = schedule_draft(t.state(), draft.clone(), in_an_hour)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(
        list_drafts(t.state(), account)
            .await
            .unwrap_or_else(|e| panic!("{e:?}"))
            .local
            .is_empty()
    );
    let items = outbox(t.state(), account)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(matches!(
        items.iter().find(|i| i.id == queued).map(|i| &i.state),
        Some(OutboxStateDto::Queued { not_before }) if *not_before == in_an_hour
    ));
    let back = unschedule_send(t.state(), account, queued)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(back.subject, "Programmé");
    assert!(back.id.is_some());
    assert!(
        outbox(t.state(), account)
            .await
            .unwrap_or_else(|e| panic!("{e:?}"))
            .is_empty()
    );
    assert_eq!(
        code(unschedule_send(t.state(), account, queued).await).0,
        ErrorCodeDto::NotFound,
        "déjà repris : plus dans la file"
    );
    let far = u32::MAX;
    assert_eq!(
        code(schedule_draft(t.state(), back, far).await).0,
        ErrorCodeDto::ScheduleTooFar
    );
}

#[tokio::test]
async fn drafts_from_another_client_are_listed_and_resumed() {
    use crate::compose::list_drafts;
    use norelly_domain::mailbox::{MailboxName, MailboxRole, RemoteMailbox};
    use norelly_domain::ports::{BlobStore, MailStore, StoredBody, SyncBatch};
    let t = test_app().await;
    let account = id32(t.account.get());
    let store = &t.state().mail.store;
    let mailbox = |name: &str, role| RemoteMailbox {
        name: MailboxName::parse(name).unwrap_or_else(|_| panic!("{name}")),
        delimiter: Some('/'),
        role,
        selectable: true,
    };
    let boxes = store
        .replace_mailboxes(
            t.account,
            &[
                mailbox("INBOX", MailboxRole::Inbox),
                mailbox("Corbeille", MailboxRole::Trash),
                mailbox("Envoyés", MailboxRole::Sent),
                mailbox("Brouillons", MailboxRole::Drafts),
            ],
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let drafts_box = boxes
        .iter()
        .find(|m| m.mailbox.role == MailboxRole::Drafts)
        .map(|m| m.id)
        .unwrap_or_else(|| panic!("Brouillons"));
    let mut foreign = norelly_test_support::mail_contracts::sample_message(1, 5_000, true);
    foreign.meta.headers.subject = "Pas fini".into();
    let raw = b"To: bob@example.com\r\nSubject: Pas fini\r\n\r\nTexte en cours\r\n".to_vec();
    foreign.body = StoredBody::Full(
        t.state()
            .mail
            .blobs
            .put(t.account, raw)
            .await
            .unwrap_or_else(|e| panic!("{e}")),
    );
    store
        .apply_sync(
            drafts_box,
            &SyncBatch {
                new: vec![foreign],
                ..SyncBatch::default()
            },
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let drafts = list_drafts(t.state(), account)
        .await
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(drafts.remote.len(), 1);
    assert_eq!(drafts.remote[0].subject, "Pas fini");
    let resumed = prepare_compose(
        t.state(),
        account,
        ComposeKindDto::Resume,
        Some(drafts.remote[0].message),
        String::new(),
    )
    .await
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(resumed.subject, "Pas fini");
    assert_eq!(
        resumed.to.first().map(|r| r.address.as_str()),
        Some("bob@example.com")
    );
    assert!(resumed.html.contains("Texte en cours"));
}

/// « Reprendre » ne vise qu'un message du
/// dossier « Brouillons » du compte, vérifié en Rust avant tout
/// téléchargement.
#[tokio::test]
async fn only_drafts_folder_messages_can_be_resumed() {
    let t = test_app().await;
    let account = id32(t.account.get());
    let inbox_message = id32(t.messages[0].get());
    assert_eq!(
        code(
            prepare_compose(
                t.state(),
                account,
                ComposeKindDto::Resume,
                Some(inbox_message),
                String::new()
            )
            .await
        ),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Message))
    );
}

#[tokio::test]
async fn folder_creation_revalidates_its_inputs() {
    use crate::folders::create_folder;
    use norelly_domain::ports::{AccountRepository, MailStore};

    let t = test_app().await;
    let account = id32(t.account.get());
    assert_eq!(
        code(create_folder(t.state(), 0, "Factures".into(), None).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Account))
    );
    assert_eq!(
        code(create_folder(t.state(), account, "Factures".into(), Some(0)).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Mailbox))
    );
    // Parent inconnu : refusé avant tout envoi.
    assert_eq!(
        code(create_folder(t.state(), account, "Factures".into(), Some(999_999)).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Mailbox))
    );
    // Parent qui existe, mais dans un autre compte : refusé aussi.
    let other = t
        .state()
        .accounts
        .insert(&norelly_test_support::contracts::sample_account(
            "bob@example.com",
            false,
        ))
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .id();
    let others = t
        .state()
        .mail
        .store
        .replace_mailboxes(
            other,
            &[norelly_domain::mailbox::RemoteMailbox {
                name: norelly_domain::mailbox::MailboxName::inbox(),
                delimiter: Some('/'),
                role: norelly_domain::mailbox::MailboxRole::Inbox,
                selectable: true,
            }],
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let foreign = others
        .first()
        .map(|m| id32(m.id.get()))
        .unwrap_or_else(|| panic!("dossier de bob"));
    assert_eq!(
        code(create_folder(t.state(), account, "Factures".into(), Some(foreign)).await),
        (ErrorCodeDto::InvalidInput, Some(FieldDto::Mailbox))
    );
    // Nom démesuré : refusé sans passer par le serveur.
    assert_eq!(
        code(create_folder(t.state(), account, "x".repeat(10_000), None).await).0,
        ErrorCodeDto::FolderNameTooLong
    );
    // Compte sans connexion en cours : échec explicite.
    assert_eq!(
        code(create_folder(t.state(), account, "Factures".into(), None).await).0,
        ErrorCodeDto::FolderCreateFailed
    );
}

#[tokio::test]
async fn folder_rename_and_delete_revalidate_their_inputs() {
    use crate::folders::{delete_folder, move_folder, rename_folder};
    use norelly_domain::mailbox::{MailboxName, MailboxRole, RemoteMailbox};
    use norelly_domain::ports::{AccountRepository, MailStore};

    let t = test_app().await;
    let account = id32(t.account.get());
    let mailbox = |name: &str, role| RemoteMailbox {
        name: MailboxName::parse(name).unwrap_or_else(|e| panic!("{e}")),
        delimiter: Some('/'),
        role,
        selectable: true,
    };
    let mine = t
        .state()
        .mail
        .store
        .replace_mailboxes(
            t.account,
            &[
                mailbox("INBOX", MailboxRole::Inbox),
                mailbox("Projets", MailboxRole::Normal),
            ],
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let projets = mine
        .iter()
        .find(|m| m.mailbox.name.as_str() == "Projets")
        .map(|m| id32(m.id.get()))
        .unwrap_or_else(|| panic!("Projets"));
    let other = t
        .state()
        .accounts
        .insert(&norelly_test_support::contracts::sample_account(
            "bob@example.com",
            false,
        ))
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .id();
    let foreign = t
        .state()
        .mail
        .store
        .replace_mailboxes(other, &[mailbox("Projets", MailboxRole::Normal)])
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .first()
        .map(|m| id32(m.id.get()))
        .unwrap_or_else(|| panic!("dossier de bob"));

    for (account, mailbox, field) in [
        (0, projets, FieldDto::Account),
        (account, 0, FieldDto::Mailbox),
        // Inconnu, ou dossier d'un autre compte : refusé avant tout envoi.
        (account, 999_999, FieldDto::Mailbox),
        (account, foreign, FieldDto::Mailbox),
    ] {
        assert_eq!(
            code(rename_folder(t.state(), account, mailbox, "Chantiers".into()).await),
            (ErrorCodeDto::InvalidInput, Some(field)),
            "renommer {account}/{mailbox}"
        );
        assert_eq!(
            code(delete_folder(t.state(), account, mailbox).await),
            (ErrorCodeDto::InvalidInput, Some(field)),
            "supprimer {account}/{mailbox}"
        );
        assert_eq!(
            code(move_folder(t.state(), account, mailbox, None).await),
            (ErrorCodeDto::InvalidInput, Some(field)),
            "déplacer {account}/{mailbox}"
        );
    }
    // Destination nulle, inconnue ou d'un autre compte : refusée avant tout
    // envoi.
    for parent in [0, 999_999, foreign] {
        assert_eq!(
            code(move_folder(t.state(), account, projets, Some(parent)).await),
            (ErrorCodeDto::InvalidInput, Some(FieldDto::Mailbox)),
            "déplacer sous {parent}"
        );
    }
    // Nom démesuré : refusé sans passer par le serveur.
    assert_eq!(
        code(rename_folder(t.state(), account, projets, "x".repeat(10_000)).await).0,
        ErrorCodeDto::FolderNameTooLong
    );
    // Compte sans connexion en cours : échec explicite, propre à l'opération.
    assert_eq!(
        code(rename_folder(t.state(), account, projets, "Chantiers".into()).await).0,
        ErrorCodeDto::FolderRenameFailed
    );
    assert_eq!(
        code(delete_folder(t.state(), account, projets).await).0,
        ErrorCodeDto::FolderDeleteFailed
    );
    assert_eq!(
        code(move_folder(t.state(), account, projets, None).await).0,
        ErrorCodeDto::FolderMoveFailed
    );
}
