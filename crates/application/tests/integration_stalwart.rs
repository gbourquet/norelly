//! Second serveur IMAP (ADR-007) : les scénarios de Dovecot rejoués contre
//! Stalwart. Job nocturne seulement (`--profile stalwart`, nightly.yml) :
//! exclu du profil `integration` de `./scripts/check.sh full`.

mod common;

use common::TestResult;
use common::scenarios;
use norelly_test_support::containers::ImapServerKind::Stalwart;

#[tokio::test]
async fn initial_then_incremental_sync() -> TestResult {
    scenarios::initial_then_incremental_sync(Stalwart).await
}

#[tokio::test]
async fn uidvalidity_change_resyncs_the_mailbox() -> TestResult {
    scenarios::uidvalidity_change_resyncs_the_mailbox(Stalwart).await
}

#[tokio::test]
async fn idle_receives_appended_message_within_five_seconds() -> TestResult {
    scenarios::idle_receives_appended_message_within_five_seconds(Stalwart).await
}

#[tokio::test]
async fn every_action_with_move_and_uidplus() -> TestResult {
    scenarios::actions(Stalwart, "stalwart-actions-move", &[]).await
}

#[tokio::test]
async fn every_action_without_move_nor_uidplus() -> TestResult {
    scenarios::actions(
        Stalwart,
        "stalwart-actions-sans-uidplus",
        &["MOVE", "UIDPLUS"],
    )
    .await
}

#[tokio::test]
async fn every_action_without_qresync() -> TestResult {
    scenarios::actions(Stalwart, "stalwart-actions-sans-qresync", &["QRESYNC"]).await
}
