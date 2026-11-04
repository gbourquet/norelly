//! Synchronisation de bout en bout contre Dovecot (scénarios communs à
//! Stalwart : `common/scenarios.rs`).

mod common;

use common::TestResult;
use common::scenarios;
use norelly_test_support::containers::ImapServerKind::Dovecot;

#[tokio::test]
async fn initial_then_incremental_sync() -> TestResult {
    scenarios::initial_then_incremental_sync(Dovecot).await
}

#[tokio::test]
async fn uidvalidity_change_resyncs_the_mailbox() -> TestResult {
    scenarios::uidvalidity_change_resyncs_the_mailbox(Dovecot).await
}

#[tokio::test]
async fn idle_receives_appended_message_within_five_seconds() -> TestResult {
    scenarios::idle_receives_appended_message_within_five_seconds(Dovecot).await
}
