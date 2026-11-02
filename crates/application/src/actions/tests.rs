use norelly_domain::fetch_policy::FetchPolicy;
use norelly_domain::mailbox::Flags;
use norelly_domain::ports::{MessageListQuery, RemoteMailboxSession, StoredMailbox};
use norelly_domain::sync::SyncExtensions;
use norelly_test_support::fake_imap::{FakeImap, FakeImapSession};
use norelly_test_support::mail_contracts::{InMemoryBlobStore, InMemoryMailStore};
use norelly_test_support::session_contract::contract_message;

use super::*;
use crate::sync::SyncAccount;

/// Instant de l'action ; le rejeu a lieu après la fenêtre d'annulation.
const NOW: i64 = 1_000;
const LATER: i64 = NOW + 60;

fn account() -> AccountId {
    AccountId::new(1).unwrap_or_else(|_| panic!("compte"))
}

fn uid(n: u32) -> Uid {
    Uid::new(n).unwrap_or_else(|| panic!("uid"))
}

struct World {
    imap: FakeImap,
    store: InMemoryMailStore,
    blobs: InMemoryBlobStore,
    session: FakeImapSession,
    ext: SyncExtensions,
    boxes: Vec<StoredMailbox>,
}

/// INBOX (3 messages non lus), Corbeille (`\Trash`), Archives
/// (`\Archive`), synchronisés.
async fn world(move_ext: bool, uidplus: bool) -> World {
    let imap = FakeImap::new();
    imap.set_move_extensions(move_ext, uidplus);
    imap.create_mailbox("Corbeille", &["\\Trash"]);
    imap.create_mailbox("Archives", &["\\Archive"]);
    for n in 1..=3 {
        imap.append(
            "INBOX",
            &contract_message(n),
            Flags::default(),
            100 + i64::from(n),
        );
    }
    let mut session = imap.session().unwrap_or_else(|e| panic!("{e}"));
    let ext = session
        .enable_sync_extensions()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut w = World {
        imap,
        store: InMemoryMailStore::default(),
        blobs: InMemoryBlobStore::default(),
        session,
        ext,
        boxes: Vec::new(),
    };
    w.sync().await;
    w
}

impl World {
    async fn sync(&mut self) {
        let use_case = SyncAccount {
            account: account(),
            store: &self.store,
            blobs: &self.blobs,
            policy: FetchPolicy::default(),
        };
        self.boxes = use_case
            .sync_all(&mut self.session, self.ext, |_| {})
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        self.store
            .reapply(account())
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        self.store
            .purge_placeholders(account(), None)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    }

    fn box_id(&self, name: &str) -> MailboxId {
        self.boxes
            .iter()
            .find(|m| m.mailbox.name.as_str() == name)
            .map(|m| m.id)
            .unwrap_or_else(|| panic!("boîte {name}"))
    }

    /// Messages visibles d'une boîte : (identifiant, UID, drapeaux).
    async fn visible(&self, name: &str) -> Vec<(MessageId, u32, Flags)> {
        let mut items: Vec<_> = self
            .store
            .page(self.box_id(name), None, 100)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .items
            .into_iter()
            .map(|i| (i.id, i.uid.get(), i.flags))
            .collect();
        items.sort_by_key(|i| i.1);
        items
    }

    async fn id_of(&self, name: &str, n: u32) -> MessageId {
        self.visible(name)
            .await
            .into_iter()
            .find(|i| i.1 == n)
            .map(|i| i.0)
            .unwrap_or_else(|| panic!("message {n} de {name}"))
    }

    async fn act(&self, ids: &[MessageId], action: MessageAction) -> ActionReport {
        PerformAction { store: &self.store }
            .execute(ids, action, NOW)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    }

    async fn replay(&mut self, now: i64) -> ReplayReport {
        ReplayActions {
            account: account(),
            store: &self.store,
        }
        .execute(&mut self.session, now)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
    }

    async fn pending(&self) -> usize {
        self.store
            .pending(account())
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .len()
    }

    fn server_uids(&self, name: &str) -> Vec<u32> {
        self.imap.flags_of(name).keys().map(|u| u.get()).collect()
    }
}

#[tokio::test]
async fn mark_read_is_optimistic_then_replayed_after_the_undo_window() {
    let mut w = world(true, true).await;
    let m1 = w.id_of("INBOX", 1).await;
    let report = w.act(&[m1], MessageAction::MarkSeen(true)).await;
    assert_eq!(report.recorded.len(), 1);
    assert!(w.visible("INBOX").await[0].2.seen, "lu tout de suite");
    // Pendant la fenêtre d'annulation, rien n'est envoyé.
    let early = w.replay(NOW + 1).await;
    assert_eq!(early.applied, 0);
    assert!(!w.imap.flags_of("INBOX")[&uid(1)].seen);
    let report = w.replay(LATER).await;
    assert_eq!(report.applied, 1);
    assert!(w.imap.flags_of("INBOX")[&uid(1)].seen, "rejoué");
    assert_eq!(w.pending().await, 0);
    // Une synchronisation confirme l'état.
    w.sync().await;
    assert!(w.visible("INBOX").await[0].2.seen);
}

#[tokio::test]
async fn moved_message_shows_in_the_destination_right_away() {
    let mut w = world(true, true).await;
    let m1 = w.id_of("INBOX", 1).await;
    let report = w.act(&[m1], MessageAction::Archive).await;
    // Tout de suite : parti de la boîte de réception, arrivé dans les
    // archives (copie provisoire), sans rien envoyer au serveur.
    assert_eq!(w.visible("INBOX").await.len(), 2);
    assert_eq!(
        w.visible("Archives").await.len(),
        1,
        "visible dans la destination"
    );
    assert!(w.server_uids("Archives").is_empty());
    // Une synchronisation avant le rejeu ne la fait pas disparaître.
    w.sync().await;
    assert_eq!(w.visible("Archives").await.len(), 1);
    // Annulation : de retour à l'origine, retirée de la destination.
    UndoActions { store: &w.store }
        .execute(&report.recorded)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(w.visible("INBOX").await.len(), 3);
    assert!(w.visible("Archives").await.is_empty());
    // De nouveau, puis rejeu et synchronisation : le vrai message remplace
    // la copie, sans doublon.
    w.act(&[m1], MessageAction::Archive).await;
    w.replay(LATER).await;
    assert_eq!(
        w.visible("Archives").await.len(),
        1,
        "pas de trou après le rejeu"
    );
    w.sync().await;
    let archived = w.visible("Archives").await;
    assert_eq!(archived.len(), 1, "le vrai message seul : {archived:?}");
    assert_eq!(
        archived[0].1,
        w.server_uids("Archives")[0],
        "UID du serveur"
    );
}

#[tokio::test]
async fn sync_before_replay_keeps_the_optimistic_state() {
    let mut w = world(true, true).await;
    let m1 = w.id_of("INBOX", 1).await;
    w.act(&[m1], MessageAction::MarkFlagged(true)).await;
    let m2 = w.id_of("INBOX", 2).await;
    w.act(&[m2], MessageAction::Archive).await;
    w.sync().await;
    let visible = w.visible("INBOX").await;
    assert_eq!(visible.iter().map(|v| v.1).collect::<Vec<_>>(), [1, 3]);
    assert!(
        visible[0].2.flagged,
        "drapeau réappliqué après la synchronisation"
    );
}

#[tokio::test]
async fn delete_moves_to_trash_with_every_strategy() {
    for (move_ext, uidplus) in [(true, true), (false, true), (false, false)] {
        let mut w = world(move_ext, uidplus).await;
        let ids = [w.id_of("INBOX", 1).await, w.id_of("INBOX", 2).await];
        w.act(&ids, MessageAction::Delete).await;
        assert_eq!(w.visible("INBOX").await.len(), 1, "masqués tout de suite");
        let report = w.replay(LATER).await;
        assert_eq!(report.applied, 2, "MOVE={move_ext} UIDPLUS={uidplus}");
        assert!(report.touched.iter().any(|m| m.as_str() == "Corbeille"));
        assert_eq!(
            w.server_uids("INBOX"),
            [3],
            "MOVE={move_ext} UIDPLUS={uidplus}"
        );
        assert_eq!(w.server_uids("Corbeille").len(), 2);
        w.sync().await;
        assert_eq!(w.visible("INBOX").await.len(), 1);
        assert_eq!(w.visible("Corbeille").await.len(), 2);
    }
}

#[tokio::test]
async fn guarded_expunge_spares_messages_deleted_by_another_client() {
    let mut w = world(false, false).await;
    // Un autre client a marqué le message 3 \Deleted sans EXPUNGE.
    let mut deleted = Flags::default();
    deleted.deleted = true;
    w.imap.set_flags("INBOX", uid(3), deleted);
    w.sync().await;
    let m1 = w.id_of("INBOX", 1).await;
    w.act(&[m1], MessageAction::Delete).await;
    let report = w.replay(LATER).await;
    assert_eq!(report.applied, 1);
    // Copié dans la Corbeille, marqué \Deleted, mais pas d'EXPUNGE : le
    // message 3 de l'autre client est épargné.
    assert_eq!(w.server_uids("INBOX"), [1, 2, 3]);
    assert!(w.imap.flags_of("INBOX")[&uid(1)].deleted);
    assert_eq!(w.server_uids("Corbeille").len(), 1);
    w.sync().await;
    assert_eq!(
        w.visible("INBOX")
            .await
            .iter()
            .map(|v| v.1)
            .collect::<Vec<_>>(),
        [2],
        "messages \\Deleted masqués"
    );
}

#[tokio::test]
async fn delete_from_trash_is_permanent() {
    let mut w = world(true, true).await;
    let m1 = w.id_of("INBOX", 1).await;
    w.act(&[m1], MessageAction::Delete).await;
    w.replay(LATER).await;
    w.sync().await;
    let trashed = w.visible("Corbeille").await[0].0;
    w.act(&[trashed], MessageAction::Delete).await;
    assert!(w.visible("Corbeille").await.is_empty());
    let report = w.replay(LATER).await;
    assert_eq!(report.applied, 1);
    assert!(
        w.server_uids("Corbeille").is_empty(),
        "supprimé définitivement"
    );
}

#[tokio::test]
async fn undo_before_replay_restores_everything() {
    let mut w = world(true, true).await;
    let m1 = w.id_of("INBOX", 1).await;
    let m2 = w.id_of("INBOX", 2).await;
    let seen = w.act(&[m1], MessageAction::MarkSeen(true)).await;
    let moved = w.act(&[m2], MessageAction::Delete).await;
    let undone = UndoActions { store: &w.store }
        .execute(&[seen.recorded, moved.recorded].concat())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(undone, 2);
    let visible = w.visible("INBOX").await;
    assert_eq!(visible.len(), 3, "de nouveau visible");
    assert!(!visible[0].2.seen, "drapeau rétabli");
    assert_eq!(w.replay(LATER).await.applied, 0);
    assert_eq!(w.server_uids("INBOX"), [1, 2, 3]);
    // Déjà rejouée : plus annulable.
    let again = w.act(&[m1], MessageAction::MarkSeen(true)).await;
    w.replay(LATER).await;
    let undone = UndoActions { store: &w.store }
        .execute(&again.recorded)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(undone, 0);
}

#[tokio::test]
async fn toggling_back_before_replay_sends_nothing() {
    let mut w = world(true, true).await;
    let m1 = w.id_of("INBOX", 1).await;
    w.act(&[m1], MessageAction::MarkFlagged(true)).await;
    let back = w.act(&[m1], MessageAction::MarkFlagged(false)).await;
    assert!(back.recorded.is_empty());
    assert_eq!(w.pending().await, 0);
    assert!(!w.visible("INBOX").await[0].2.flagged);
    let operations = w.imap.operations();
    assert_eq!(w.replay(LATER).await, ReplayReport::default());
    assert_eq!(w.imap.operations(), operations, "aucune commande envoyée");
}

#[tokio::test]
async fn impossible_actions_apply_nothing() {
    let w = world(true, true).await;
    w.imap.delete_mailbox("Archives");
    let m1 = w.id_of("INBOX", 1).await;
    // Les dossiers locaux ne changent qu'à la synchronisation : on retire
    // l'archive localement en resynchronisant.
    let mut w = w;
    w.sync().await;
    let failure = PerformAction { store: &w.store }
        .execute(&[m1], MessageAction::Archive, NOW)
        .await;
    assert_eq!(failure, Err(ActionFailure::Invalid(ActionError::NoArchive)));
    assert_eq!(w.pending().await, 0);
    // Repli configuré : l'archivage vise ce dossier.
    let trash = w.box_id("Corbeille");
    w.store
        .set_archive_fallback(account(), Some(trash))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let report = w.act(&[m1], MessageAction::Archive).await;
    assert_eq!(report.recorded.len(), 1);
}

#[tokio::test]
async fn conflicts_drop_the_op_and_restore_the_local_state() {
    // Message supprimé par un autre client.
    let mut w = world(true, true).await;
    let m1 = w.id_of("INBOX", 1).await;
    w.act(&[m1], MessageAction::Archive).await;
    w.imap.expunge("INBOX", uid(1));
    let report = w.replay(LATER).await;
    assert_eq!(report.dropped.len(), 1);
    assert_eq!(report.dropped[0].1, Some(Conflict::MessageGone));
    w.sync().await;
    assert_eq!(w.visible("INBOX").await.len(), 2);

    // Dossier de destination supprimé.
    let mut w = world(true, true).await;
    let m2 = w.id_of("INBOX", 2).await;
    w.act(&[m2], MessageAction::Archive).await;
    w.imap.delete_mailbox("Archives");
    let report = w.replay(LATER).await;
    assert_eq!(report.dropped[0].1, Some(Conflict::TargetMissing));
    assert_eq!(
        w.visible("INBOX").await.len(),
        3,
        "message de nouveau visible"
    );
    assert_eq!(w.server_uids("INBOX"), [1, 2, 3]);

    // UIDVALIDITY changée : l'UID ne désigne plus le même message.
    let mut w = world(true, true).await;
    let m3 = w.id_of("INBOX", 3).await;
    w.act(&[m3], MessageAction::MarkSeen(true)).await;
    w.imap.recreate_mailbox("INBOX");
    let report = w.replay(LATER).await;
    assert_eq!(report.dropped[0].1, Some(Conflict::UidValidityChanged));
    assert!(w.imap.flags_of("INBOX").values().all(|f| !f.seen));
}

#[tokio::test]
async fn concurrent_flag_change_is_detected_and_reapplied() {
    let mut w = world(true, true).await;
    let m1 = w.id_of("INBOX", 1).await;
    w.act(&[m1], MessageAction::MarkSeen(true)).await;
    // Changement concurrent sur le serveur (MODSEQ augmenté) : le STORE
    // conditionnel est refusé (MODIFIED), puis réappliqué sur ce drapeau
    // seulement.
    let mut flagged = Flags::default();
    flagged.flagged = true;
    w.imap.set_flags("INBOX", uid(1), flagged);
    let report = w.replay(LATER).await;
    assert_eq!(report.applied, 1);
    let server = &w.imap.flags_of("INBOX")[&uid(1)];
    assert!(
        server.seen && server.flagged,
        "changement concurrent conservé"
    );
}

#[tokio::test]
async fn an_op_refused_forever_is_dropped_after_too_many_attempts() {
    let mut w = world(true, true).await;
    let m1 = w.id_of("INBOX", 1).await;
    w.act(&[m1], MessageAction::Archive).await;
    // Le serveur retire MOVE alors que la session l'annonce encore : la
    // commande échoue à chaque essai (comme une réponse BAD).
    w.imap.set_move_extensions(false, true);
    for attempt in 1..=MAX_REPLAY_ATTEMPTS {
        let failure = ReplayActions {
            account: account(),
            store: &w.store,
        }
        .execute(&mut w.session, LATER)
        .await;
        assert!(failure.is_err(), "essai {attempt} : {failure:?}");
        // Nouvelle session qui annonce encore MOVE (capacités lues à
        // l'ouverture), puis refus de la commande.
        w.imap.set_move_extensions(true, true);
        w.session = w.imap.session().unwrap_or_else(|e| panic!("{e}"));
        w.imap.set_move_extensions(false, true);
    }
    let report = w.replay(LATER).await;
    assert_eq!(report.dropped.len(), 1);
    assert_eq!(report.dropped[0].1, Some(Conflict::TooManyAttempts));
    assert_eq!(w.pending().await, 0, "la file ne bloque plus le compte");
    assert_eq!(w.visible("INBOX").await.len(), 3, "état local rétabli");
}

#[tokio::test]
async fn undo_during_replay_is_refused() {
    let w = world(true, true).await;
    let m1 = w.id_of("INBOX", 1).await;
    let report = w.act(&[m1], MessageAction::Delete).await;
    // Le rejeu a réservé l'opération (commandes en cours d'envoi).
    w.store
        .claim(report.recorded[0])
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let undone = UndoActions { store: &w.store }
        .execute(&report.recorded)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        undone, 0,
        "trop tard : ne pas annoncer une annulation fictive"
    );
    assert_eq!(w.visible("INBOX").await.len(), 2, "toujours masqué");
    // Une bascule inverse ne fusionne pas avec une opération réservée :
    // elle est enregistrée à la suite.
    let m2 = w.id_of("INBOX", 2).await;
    let flag = w.act(&[m2], MessageAction::MarkFlagged(true)).await;
    w.store
        .claim(flag.recorded[0])
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let back = w.act(&[m2], MessageAction::MarkFlagged(false)).await;
    assert_eq!(back.recorded.len(), 1);
    assert_eq!(w.pending().await, 3);
}

#[tokio::test]
async fn connection_loss_keeps_the_queue_for_the_next_session() {
    let mut w = world(true, true).await;
    let m1 = w.id_of("INBOX", 1).await;
    w.act(&[m1], MessageAction::Delete).await;
    w.imap.set_offline(true);
    let failure = ReplayActions {
        account: account(),
        store: &w.store,
    }
    .execute(&mut w.session, LATER)
    .await;
    assert!(matches!(failure, Err(ReplayFailure::Connection(_))));
    assert_eq!(w.pending().await, 1, "opération conservée");
    assert_eq!(w.visible("INBOX").await.len(), 2, "toujours masqué");
    // Reconnexion : nouvelle session, rejeu.
    w.imap.set_offline(false);
    w.session = w.imap.session().unwrap_or_else(|e| panic!("{e}"));
    w.ext = w
        .session
        .enable_sync_extensions()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(w.replay(LATER).await.applied, 1);
    assert_eq!(w.server_uids("INBOX"), [2, 3]);
}
