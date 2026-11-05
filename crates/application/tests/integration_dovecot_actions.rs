//! Actions sur les messages de bout en bout contre Dovecot, avec MOVE et
//! UIDPLUS, sans MOVE, puis sans MOVE ni UIDPLUS (scénario commun à
//! Stalwart : `common/scenarios.rs`).

mod common;

use common::TestResult;
use common::scenarios;
use norelly_test_support::containers::ImapServerKind::Dovecot;

#[tokio::test]
async fn every_action_with_move_and_uidplus() -> TestResult {
    scenarios::actions(Dovecot, "actions-move", &[]).await
}

#[tokio::test]
async fn every_action_without_move() -> TestResult {
    scenarios::actions(Dovecot, "actions-sans-move", &["MOVE"]).await
}

#[tokio::test]
async fn every_action_without_move_nor_uidplus() -> TestResult {
    scenarios::actions(Dovecot, "actions-sans-uidplus", &["MOVE", "UIDPLUS"]).await
}
