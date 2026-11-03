//! Application factice pour tester les commandes IPC sans webview
//! (`tauri::test`, MockRuntime) : état réel sur un dossier temporaire, un
//! compte avec INBOX, Corbeille et Envoyés, deux messages synchronisés.

use std::sync::atomic::{AtomicUsize, Ordering};

use norelly_domain::account::AccountId;
use norelly_domain::mailbox::MailboxRole;
use norelly_domain::ports::{AccountRepository, MailStore, MessageId, MessageListQuery, SyncBatch};
use norelly_domain::sync::MailboxSyncState;
use norelly_test_support::contracts::sample_account;
use norelly_test_support::mail_contracts::sample_message;
use tauri::Manager;
use tauri::test::{MockRuntime, mock_app};

use crate::state::AppState;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Application factice et son compte.
pub struct TestApp {
    pub app: tauri::App<MockRuntime>,
    pub account: AccountId,
    pub messages: Vec<MessageId>,
    dir: std::path::PathBuf,
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl TestApp {
    pub fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }
}

fn mailbox(name: &str, role: MailboxRole) -> norelly_domain::mailbox::RemoteMailbox {
    norelly_domain::mailbox::RemoteMailbox {
        name: norelly_domain::mailbox::MailboxName::parse(name)
            .unwrap_or_else(|_| panic!("{name}")),
        delimiter: Some('/'),
        role,
        selectable: true,
    }
}

pub async fn test_app() -> TestApp {
    let dir = std::env::temp_dir().join(format!(
        "norelly-app-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let state = AppState::open(&dir, &dir.join("cache")).unwrap_or_else(|e| panic!("{e}"));
    let account = state
        .accounts
        .insert(&sample_account("alice@example.com", false))
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .id();
    let store = &state.mail.store;
    let boxes = store
        .replace_mailboxes(
            account,
            &[
                mailbox("INBOX", MailboxRole::Inbox),
                mailbox("Corbeille", MailboxRole::Trash),
                mailbox("Envoyés", MailboxRole::Sent),
            ],
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let inbox = boxes
        .iter()
        .find(|m| m.mailbox.name.is_inbox())
        .map(|m| m.id)
        .unwrap_or_else(|| panic!("INBOX"));
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                new: vec![
                    sample_message(1, 1_000, false),
                    sample_message(2, 2_000, false),
                ],
                state: MailboxSyncState {
                    uid_validity: norelly_domain::mailbox::UidValidity::new(7),
                    uid_next: norelly_domain::mailbox::Uid::new(3),
                    ..MailboxSyncState::default()
                },
                ..SyncBatch::default()
            },
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let messages = store
        .page(inbox, None, 10)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .items
        .iter()
        .map(|i| i.id)
        .collect();
    let app = mock_app();
    app.manage(state);
    TestApp {
        app,
        account,
        messages,
        dir,
    }
}
