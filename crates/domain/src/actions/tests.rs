use proptest::prelude::*;

use super::*;

fn mailbox_id(n: i64) -> MailboxId {
    MailboxId::new(n).unwrap_or_else(|| panic!("id"))
}

fn folder(id: i64, name: &str, role: MailboxRole) -> FolderRef {
    FolderRef {
        id: mailbox_id(id),
        name: MailboxName::parse(name).unwrap_or_else(|_| panic!("nom")),
        role,
        selectable: true,
    }
}

fn folders() -> Vec<FolderRef> {
    vec![
        folder(1, "INBOX", MailboxRole::Inbox),
        folder(2, "Corbeille", MailboxRole::Trash),
        folder(3, "Archives", MailboxRole::Archive),
        folder(4, "Projets", MailboxRole::Normal),
    ]
}

fn uid(n: u32) -> Uid {
    Uid::new(n).unwrap_or_else(|| panic!("uid"))
}

fn local(flags: Flags) -> LocalMessage {
    LocalMessage {
        id: MessageId::new(10).unwrap_or_else(|| panic!("message")),
        account: AccountId::new(1).unwrap_or_else(|_| panic!("compte")),
        mailbox: folder(1, "INBOX", MailboxRole::Inbox),
        uid: uid(42),
        flags,
        uid_validity: UidValidity::new(7),
        highest_modseq: ModSeq::new(99),
    }
}

fn op(kind: OpKind) -> PendingOp {
    let message = local(Flags::default());
    PendingOp {
        account: message.account,
        message: message.id,
        mailbox: message.mailbox.name,
        uid_validity: UidValidity::new(7).unwrap_or_else(|| panic!("validité")),
        uid: message.uid,
        kind,
        not_before: 100,
    }
}

fn seen_op(previous: bool, value: bool) -> PendingOp {
    op(OpKind::Flag {
        change: FlagChange {
            flag: ActionFlag::Seen,
            value,
        },
        previous,
        unchanged_since: None,
    })
}

#[test]
fn delete_moves_to_trash_then_expunges_from_trash() {
    let all = folders();
    let inbox = &all[0];
    let trash = &all[1];
    assert_eq!(
        resolve(MessageAction::Delete, inbox, &all, None),
        Ok(Effect::MoveTo(trash.clone()))
    );
    assert_eq!(
        resolve(MessageAction::Delete, trash, &all, None),
        Ok(Effect::Expunge)
    );
    let without_trash: Vec<_> = all
        .iter()
        .filter(|f| f.role != MailboxRole::Trash)
        .cloned()
        .collect();
    assert_eq!(
        resolve(MessageAction::Delete, inbox, &without_trash, None),
        Err(ActionError::NoTrash)
    );
}

#[test]
fn archive_uses_archive_role_then_configured_fallback() {
    let all = folders();
    let inbox = &all[0];
    assert_eq!(
        resolve(MessageAction::Archive, inbox, &all, None),
        Ok(Effect::MoveTo(all[2].clone()))
    );
    let without_archive: Vec<_> = all
        .iter()
        .filter(|f| f.role != MailboxRole::Archive)
        .cloned()
        .collect();
    assert_eq!(
        resolve(MessageAction::Archive, inbox, &without_archive, None),
        Err(ActionError::NoArchive)
    );
    assert_eq!(
        resolve(
            MessageAction::Archive,
            inbox,
            &without_archive,
            Some(mailbox_id(4))
        ),
        Ok(Effect::MoveTo(all[3].clone()))
    );
    // Déjà dans les archives.
    assert_eq!(
        resolve(MessageAction::Archive, &all[2], &all, None),
        Err(ActionError::SameFolder)
    );
}

#[test]
fn move_targets_are_checked() {
    let mut all = folders();
    let inbox = all[0].clone();
    assert_eq!(
        resolve(MessageAction::Move(mailbox_id(9)), &inbox, &all, None),
        Err(ActionError::UnknownFolder)
    );
    assert_eq!(
        resolve(MessageAction::Move(mailbox_id(1)), &inbox, &all, None),
        Err(ActionError::SameFolder)
    );
    if let Some(f) = all.get_mut(3) {
        f.selectable = false;
    }
    assert_eq!(
        resolve(MessageAction::Move(mailbox_id(4)), &inbox, &all, None),
        Err(ActionError::NotSelectable)
    );
}

#[test]
fn flag_ops_record_previous_value_and_skip_noops() {
    let effect = Effect::SetFlag(FlagChange {
        flag: ActionFlag::Seen,
        value: true,
    });
    let op = PendingOp::new(&effect, &local(Flags::default()), 1000);
    assert!(matches!(
        op.as_ref().map(|o| &o.kind),
        Some(OpKind::Flag {
            previous: false,
            unchanged_since: Some(_),
            ..
        })
    ));
    assert_eq!(op.map(|o| o.not_before), Some(1000 + UNDO_WINDOW_SECS));
    let mut seen = Flags::default();
    seen.seen = true;
    assert_eq!(PendingOp::new(&effect, &local(seen), 1000), None);
    // Boîte jamais synchronisée : pas d'opération.
    let mut unsynced = local(Flags::default());
    unsynced.uid_validity = None;
    assert_eq!(PendingOp::new(&effect, &unsynced, 1000), None);
}

#[test]
fn local_effects_are_reverted() {
    let flag = seen_op(false, true);
    assert_eq!(
        local_effect(&flag),
        LocalEffect::SetFlag {
            message: flag.message,
            flag: ActionFlag::Seen,
            value: true
        }
    );
    assert_eq!(
        revert_effect(&flag),
        LocalEffect::SetFlag {
            message: flag.message,
            flag: ActionFlag::Seen,
            value: false
        }
    );
    let moved = op(OpKind::Expunge);
    assert_eq!(
        local_effect(&moved),
        LocalEffect::Hide {
            message: moved.message
        }
    );
    assert_eq!(
        revert_effect(&moved),
        LocalEffect::Show {
            message: moved.message
        }
    );
}

#[test]
fn toggling_back_cancels_the_pending_op() {
    let queue = vec![seen_op(false, true)];
    assert_eq!(coalesce(&queue, &seen_op(true, false)), Coalesce::Cancel(0));
    assert_eq!(coalesce(&queue, &op(OpKind::Expunge)), Coalesce::Push);
    let flagged = op(OpKind::Flag {
        change: FlagChange {
            flag: ActionFlag::Flagged,
            value: true,
        },
        previous: false,
        unchanged_since: None,
    });
    assert_eq!(coalesce(&queue, &flagged), Coalesce::Push);
}

#[test]
fn strategies_follow_capabilities() {
    let caps = |t: &[&str]| ServerCapabilities::from_imap(t.iter().copied());
    assert_eq!(
        move_strategy(&caps(&["MOVE", "UIDPLUS"])),
        MoveStrategy::Move
    );
    assert_eq!(
        move_strategy(&caps(&["UIDPLUS"])),
        MoveStrategy::CopyUidExpunge
    );
    assert_eq!(move_strategy(&caps(&[])), MoveStrategy::CopyGuardedExpunge);
    assert_eq!(
        expunge_strategy(&caps(&["MOVE"])),
        MoveStrategy::CopyGuardedExpunge
    );
}

#[test]
fn guarded_expunge_refuses_foreign_deleted_messages() {
    let ours: BTreeSet<Uid> = [uid(1), uid(2)].into();
    assert!(expunge_is_safe(&ours, &[uid(1)].into()));
    assert!(expunge_is_safe(&ours, &BTreeSet::new()));
    assert!(!expunge_is_safe(&ours, &[uid(1), uid(3)].into()));
}

#[test]
fn conflicts_drop_the_op() {
    let view = |validity: u32, flags: Option<Flags>, target_exists: bool| ServerView {
        uid_validity: UidValidity::new(validity),
        flags,
        target_exists,
    };
    let moved = op(OpKind::Move {
        to: MailboxName::parse("Archives").unwrap_or_else(|_| panic!("nom")),
    });
    assert_eq!(
        decide(&moved, &view(8, Some(Flags::default()), true)),
        ReplayDecision::Drop(Conflict::UidValidityChanged)
    );
    assert_eq!(
        decide(&moved, &view(7, None, true)),
        ReplayDecision::Drop(Conflict::MessageGone)
    );
    assert_eq!(
        decide(&moved, &view(7, Some(Flags::default()), false)),
        ReplayDecision::Drop(Conflict::TargetMissing)
    );
    assert_eq!(
        decide(&moved, &view(7, Some(Flags::default()), true)),
        ReplayDecision::Apply
    );
    let mut seen = Flags::default();
    seen.seen = true;
    assert_eq!(
        decide(&seen_op(false, true), &view(7, Some(seen.clone()), false)),
        ReplayDecision::AlreadyDone
    );
    let change = FlagChange {
        flag: ActionFlag::Seen,
        value: true,
    };
    assert_eq!(after_modified(change, &seen), ReplayDecision::AlreadyDone);
    assert_eq!(
        after_modified(change, &Flags::default()),
        ReplayDecision::Apply
    );
}

fn flag() -> impl Strategy<Value = ActionFlag> {
    prop_oneof![Just(ActionFlag::Seen), Just(ActionFlag::Flagged)]
}

proptest! {
    /// Une suite quelconque de bascules sur un message, fusionnées dans la
    /// file et appliquées localement, donne le même état que les bascules
    /// appliquées directement ; la file ne garde au plus qu'une opération
    /// par drapeau, et seulement si l'état diffère de l'origine.
    #[test]
    fn coalesced_queue_matches_direct_application(
        steps in proptest::collection::vec((flag(), any::<bool>()), 0..40),
        start_seen in any::<bool>(),
        start_flagged in any::<bool>(),
    ) {
        let mut start = Flags::default();
        start.seen = start_seen;
        start.flagged = start_flagged;
        let mut current = local(start.clone());
        let mut queue: Vec<PendingOp> = Vec::new();
        for (f, value) in steps {
            let effect = Effect::SetFlag(FlagChange { flag: f, value });
            let Some(new) = PendingOp::new(&effect, &current, 0) else { continue };
            match coalesce(&queue, &new) {
                Coalesce::Push => queue.push(new),
                Coalesce::Cancel(i) => { queue.remove(i); }
            }
            f.set(&mut current.flags, value);
        }
        for f in [ActionFlag::Seen, ActionFlag::Flagged] {
            let ops: Vec<_> = queue.iter().filter(|o| matches!(&o.kind,
                OpKind::Flag { change, .. } if change.flag == f)).collect();
            let differs = f.get(&current.flags) != f.get(&start);
            prop_assert_eq!(ops.len(), usize::from(differs));
            // Annuler la file restaure exactement l'état d'origine.
            let mut reverted = current.flags.clone();
            for o in &ops {
                if let LocalEffect::SetFlag { flag, value, .. } = revert_effect(o) {
                    flag.set(&mut reverted, value);
                }
            }
            prop_assert_eq!(f.get(&reverted), f.get(&start));
        }
    }

    /// Le garde-fou n'autorise jamais un EXPUNGE qui emporterait un message
    /// marqué \Deleted par un autre client.
    #[test]
    fn guarded_expunge_never_takes_foreign_messages(
        ours in proptest::collection::btree_set(1u32..50, 0..10),
        deleted in proptest::collection::btree_set(1u32..50, 0..10),
    ) {
        let ours: BTreeSet<Uid> = ours.into_iter().filter_map(Uid::new).collect();
        let deleted: BTreeSet<Uid> = deleted.into_iter().filter_map(Uid::new).collect();
        if expunge_is_safe(&ours, &deleted) {
            prop_assert!(deleted.iter().all(|u| ours.contains(u)));
        }
    }
}

#[test]
fn conversation_targets_follow_the_action() {
    let member =
        |id: i64, mailbox: i64, role: MailboxRole, seen: bool, flagged: bool, date: i64| {
            let mut flags = Flags::default();
            flags.seen = seen;
            flags.flagged = flagged;
            Member {
                id: MessageId::new(id).unwrap_or_else(|| panic!("id")),
                mailbox: mailbox_id(mailbox),
                role,
                flags,
                date,
            }
        };
    let members = vec![
        member(1, 1, MailboxRole::Inbox, true, false, 10),
        member(2, 5, MailboxRole::Sent, true, false, 20),
        member(3, 1, MailboxRole::Inbox, false, false, 30),
        // Même message dans deux dossiers : deux exemplaires.
        member(4, 3, MailboxRole::Archive, false, true, 5),
    ];
    let id = |n: i64| MessageId::new(n).unwrap_or_else(|| panic!("id"));
    let inbox = ConversationScope::Mailbox(mailbox_id(1));
    assert_eq!(
        conversation_targets(MessageAction::MarkSeen(true), &members, &inbox),
        [id(3), id(4)],
        "toute la conversation"
    );
    assert_eq!(
        conversation_targets(MessageAction::Delete, &members, &inbox),
        [id(1), id(3)],
        "portée seulement"
    );
    assert_eq!(
        conversation_targets(
            MessageAction::Archive,
            &members,
            &ConversationScope::Unified(MailboxRole::Inbox)
        ),
        [id(1), id(3)]
    );
    assert_eq!(
        conversation_targets(MessageAction::MarkFlagged(true), &members, &inbox),
        [id(3)],
        "le plus récent de la portée"
    );
    assert!(
        conversation_targets(
            MessageAction::MarkFlagged(true),
            &members,
            &ConversationScope::Mailbox(mailbox_id(3))
        )
        .is_empty(),
        "déjà suivi"
    );
    assert_eq!(
        conversation_targets(MessageAction::MarkFlagged(false), &members, &inbox),
        [id(4)]
    );
}

fn op_id(n: i64) -> PendingOpId {
    PendingOpId::new(n).unwrap_or_else(|| panic!("opération"))
}

#[test]
fn plan_queue_coalesces_against_the_queue_read_once() {
    let queue = vec![(op_id(5), seen_op(false, true))];
    let changes = plan_queue(&queue, vec![seen_op(true, false), op(OpKind::Expunge)])
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        changes,
        vec![
            QueueChange::CancelOrPush {
                existing: op_id(5),
                op: seen_op(true, false),
            },
            QueueChange::Push(op(OpKind::Expunge)),
        ]
    );
}

#[test]
fn plan_queue_coalesces_planned_ops_between_themselves() {
    // Aller-retour dans le même lot : rien n'est enfilé.
    let changes = plan_queue(&[], vec![seen_op(false, true), seen_op(true, false)])
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(changes, Vec::new());
}

#[test]
fn plan_queue_refuses_beyond_the_per_account_cap() {
    let full: Vec<(PendingOpId, PendingOp)> = (1..=i64::try_from(MAX_PENDING_OPS).unwrap_or(0))
        .map(|n| (op_id(n), op(OpKind::Expunge)))
        .collect();
    assert_eq!(
        plan_queue(&full, vec![op(OpKind::Expunge)]),
        Err(ActionError::QueueFull)
    );
    // Une fusion ne fait pas grossir la file : acceptée même pleine.
    let mut almost = full;
    almost[0].1 = seen_op(false, true);
    assert!(plan_queue(&almost, vec![seen_op(true, false)]).is_ok());
    // Juste sous le plafond : accepté.
    almost.pop();
    assert!(plan_queue(&almost, vec![op(OpKind::Expunge)]).is_ok());
}

#[test]
fn expunges_beyond_the_expected_ones_are_reported() {
    assert_eq!(unexpected_expunges(1, 1), 0);
    assert_eq!(unexpected_expunges(1, 0), 0, "déjà supprimé : rien en trop");
    assert_eq!(unexpected_expunges(1, 3), 2);
    assert_eq!(unexpected_expunges(usize::MAX, u32::MAX), 0);
}
