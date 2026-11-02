//! Tests du rapprochement : cas ciblés, puis propriétés (`proptest`)
//! contre un serveur simulé qui applique fidèlement CONDSTORE et QRESYNC.

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;

use super::*;
use crate::mailbox::Keyword;

fn uid(n: u32) -> Uid {
    Uid::new(n).unwrap_or_else(|| panic!("uid {n}"))
}

fn validity(n: u32) -> UidValidity {
    UidValidity::new(n).unwrap_or_else(|| panic!("uidvalidity {n}"))
}

fn modseq(n: u64) -> ModSeq {
    ModSeq::new(n).unwrap_or_else(|| panic!("modseq {n}"))
}

fn flags(bits: u8) -> Flags {
    let mut f = Flags::from_parts(bits & 0x1f, Vec::<String>::new());
    if bits & 0x20 != 0
        && let Some(k) = Keyword::parse("$Forwarded")
    {
        f.insert_keyword(k);
    }
    f
}

// ---------------------------------------------------------------------------
// Serveur simulé
// ---------------------------------------------------------------------------

/// Boîte côté serveur, avec MODSEQ par message et journal des
/// suppressions (pour VANISHED).
#[derive(Debug, Clone)]
struct Server {
    validity: u32,
    uid_next: u32,
    modseq: u64,
    messages: BTreeMap<Uid, (Flags, u64)>,
    expunged: Vec<(Uid, u64)>,
    condstore: bool,
}

#[derive(Debug, Clone)]
enum Op {
    Append(u8),
    SetFlags(usize, u8),
    Expunge(usize),
    /// Boîte supprimée puis recréée : nouvelle UIDVALIDITY, UID repartant
    /// de 1, mêmes messages.
    Recreate,
}

impl Server {
    fn new(condstore: bool) -> Self {
        Self {
            validity: 100,
            uid_next: 1,
            modseq: 1,
            messages: BTreeMap::new(),
            expunged: Vec::new(),
            condstore,
        }
    }

    fn nth(&self, i: usize) -> Option<Uid> {
        let n = self.messages.len();
        (n > 0)
            .then(|| self.messages.keys().nth(i % n).copied())
            .flatten()
    }

    fn apply(&mut self, op: &Op) {
        match op {
            Op::Append(bits) => {
                self.modseq += 1;
                self.messages
                    .insert(uid(self.uid_next), (flags(*bits), self.modseq));
                self.uid_next += 1;
            }
            Op::SetFlags(i, bits) => {
                if let Some(u) = self.nth(*i) {
                    self.modseq += 1;
                    self.messages.insert(u, (flags(*bits), self.modseq));
                }
            }
            Op::Expunge(i) => {
                if let Some(u) = self.nth(*i) {
                    self.modseq += 1;
                    self.messages.remove(&u);
                    self.expunged.push((u, self.modseq));
                }
            }
            Op::Recreate => {
                self.validity += 1;
                let old: Vec<Flags> = self.messages.values().map(|(f, _)| f.clone()).collect();
                self.messages.clear();
                self.expunged.clear();
                self.uid_next = 1;
                for f in old {
                    self.modseq += 1;
                    self.messages.insert(uid(self.uid_next), (f, self.modseq));
                    self.uid_next += 1;
                }
            }
        }
    }

    fn flags_view(&self) -> BTreeMap<Uid, Flags> {
        self.messages
            .iter()
            .map(|(u, (f, _))| (*u, f.clone()))
            .collect()
    }

    fn update(&self, u: Uid) -> Option<FlagUpdate> {
        let (f, m) = self.messages.get(&u)?;
        Some(FlagUpdate {
            uid: u,
            flags: f.clone(),
            modseq: if self.condstore {
                ModSeq::new(*m)
            } else {
                None
            },
        })
    }

    fn select(&self, request: &SelectRequest) -> SelectedMailbox {
        let mut selected = SelectedMailbox {
            uid_validity: UidValidity::new(self.validity),
            uid_next: Uid::new(self.uid_next),
            highest_modseq: if self.condstore {
                ModSeq::new(self.modseq)
            } else {
                None
            },
            exists: u32::try_from(self.messages.len()).unwrap_or(u32::MAX),
            changed: vec![],
            vanished: BTreeSet::new(),
        };
        if let Some(q) = &request.qresync
            && self.condstore
            && q.uid_validity.get() == self.validity
        {
            let since = q.modseq.get();
            selected.changed = self
                .messages
                .iter()
                .filter(|(_, (_, m))| *m > since)
                .filter_map(|(u, _)| self.update(*u))
                .collect();
            let known: BTreeSet<Uid> = q.known_uids.iter().copied().collect();
            selected.vanished = self
                .expunged
                .iter()
                .filter(|(u, m)| *m > since && known.contains(u))
                .map(|(u, _)| *u)
                .collect();
        }
        selected
    }

    fn report(
        &self,
        local: &LocalMailbox,
        selected: &SelectedMailbox,
        plan: SyncPlan,
    ) -> ServerReport {
        let mut report = ServerReport::default();
        match plan.query() {
            None => {}
            Some(ServerQuery::AllFlags) => {
                report.flags = self
                    .messages
                    .keys()
                    .filter_map(|u| self.update(*u))
                    .collect();
            }
            Some(ServerQuery::ChangedFlags(since)) => {
                report.flags = self
                    .messages
                    .iter()
                    .filter(|(_, (_, m))| *m > since.get())
                    .filter_map(|(u, _)| self.update(*u))
                    .collect();
                if needs_uid_list(local, selected, &report.flags) {
                    report.all_uids = Some(self.messages.keys().copied().collect());
                }
            }
        }
        report
    }
}

fn extensions(mode: u8, server: &Server) -> SyncExtensions {
    let condstore = server.condstore && mode >= 1;
    SyncExtensions {
        condstore,
        qresync: condstore && mode >= 2,
    }
}

/// Synchronisation complète, comme le cas d'usage.
fn sync(local: &mut LocalMailbox, server: &Server, ext: SyncExtensions) -> SyncOutcome {
    let request = select_request(local, ext);
    let selected = server.select(&request);
    let plan = plan(local, &request, &selected).unwrap_or_else(|e| panic!("{e}"));
    let report = server.report(local, &selected, plan);
    let outcome = reconcile(local, &selected, plan, &report).unwrap_or_else(|e| panic!("{e}"));
    apply(local, &outcome);
    outcome
}

/// Synchronisation interrompue après l'enregistrement de `kept` nouveaux
/// messages (lot intermédiaire), comme le ferait une coupure réseau.
fn interrupted_sync(local: &mut LocalMailbox, server: &Server, ext: SyncExtensions, kept: usize) {
    let request = select_request(local, ext);
    let selected = server.select(&request);
    let plan = plan(local, &request, &selected).unwrap_or_else(|e| panic!("{e}"));
    let report = server.report(local, &selected, plan);
    let mut outcome = reconcile(local, &selected, plan, &report).unwrap_or_else(|e| panic!("{e}"));
    if outcome.new.len() > kept {
        outcome.new.truncate(kept);
        outcome.state = outcome.interim_state();
    }
    apply(local, &outcome);
}

// ---------------------------------------------------------------------------
// Cas ciblés
// ---------------------------------------------------------------------------

fn server_with(n: u32, condstore: bool) -> Server {
    let mut s = Server::new(condstore);
    for i in 0..n {
        s.apply(&Op::Append(u8::try_from(i % 4).unwrap_or(0)));
    }
    s
}

const NONE: SyncExtensions = SyncExtensions {
    condstore: false,
    qresync: false,
};
const CONDSTORE: SyncExtensions = SyncExtensions {
    condstore: true,
    qresync: false,
};
const QRESYNC: SyncExtensions = SyncExtensions {
    condstore: true,
    qresync: true,
};

#[test]
fn initial_sync_is_full_and_fetches_everything() {
    let server = server_with(5, true);
    let mut local = LocalMailbox::default();
    let request = select_request(&local, QRESYNC);
    assert!(request.qresync.is_none(), "pas de QRESYNC sans état");
    assert!(request.condstore);
    let selected = server.select(&request);
    assert_eq!(
        plan(&local, &request, &selected),
        Ok(SyncPlan::Full {
            discard_local: false
        })
    );
    let outcome = sync(&mut local, &server, QRESYNC);
    assert_eq!(outcome.new.len(), 5);
    assert_eq!(
        outcome.new.iter().map(|n| n.uid.get()).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5]
    );
    assert_eq!(local.messages, server.flags_view());
    assert_eq!(local.state.uid_validity, Some(validity(100)));
    assert_eq!(local.state.uid_next, Some(uid(6)));
    assert_eq!(local.state.highest_modseq, Some(modseq(6)));
}

#[test]
fn qresync_select_uses_local_state() {
    let server = server_with(3, true);
    let mut local = LocalMailbox::default();
    sync(&mut local, &server, QRESYNC);
    let request = select_request(&local, QRESYNC);
    assert_eq!(
        request.qresync,
        Some(QresyncParams {
            uid_validity: validity(100),
            modseq: modseq(4),
            known_uids: vec![uid(1), uid(2), uid(3)],
        })
    );
    assert!(!request.condstore, "QRESYNC implique CONDSTORE");
}

#[test]
fn qresync_applies_vanished_and_changes_without_extra_query() {
    let mut server = server_with(4, true);
    let mut local = LocalMailbox::default();
    sync(&mut local, &server, QRESYNC);
    server.apply(&Op::Expunge(1)); // UID 2
    server.apply(&Op::SetFlags(0, 0x05)); // UID 1
    server.apply(&Op::Append(0)); // UID 5
    let request = select_request(&local, QRESYNC);
    let selected = server.select(&request);
    let p = plan(&local, &request, &selected);
    assert_eq!(p, Ok(SyncPlan::Qresync));
    assert_eq!(SyncPlan::Qresync.query(), None);
    let outcome = sync(&mut local, &server, QRESYNC);
    assert_eq!(outcome.removed, BTreeSet::from([uid(2)]));
    assert_eq!(
        outcome.flag_changes.keys().copied().collect::<Vec<_>>(),
        [uid(1)]
    );
    assert_eq!(
        outcome.new.iter().map(|n| n.uid).collect::<Vec<_>>(),
        [uid(5)]
    );
    assert_eq!(local.messages, server.flags_view());
}

#[test]
fn condstore_without_expunge_skips_uid_list() {
    let mut server = server_with(4, true);
    let mut local = LocalMailbox::default();
    sync(&mut local, &server, CONDSTORE);
    server.apply(&Op::SetFlags(2, 0x01));
    server.apply(&Op::Append(0));
    let request = select_request(&local, CONDSTORE);
    let selected = server.select(&request);
    assert_eq!(
        plan(&local, &request, &selected),
        Ok(SyncPlan::Condstore {
            changed_since: modseq(5)
        })
    );
    let report = server.report(
        &local,
        &selected,
        SyncPlan::Condstore {
            changed_since: modseq(5),
        },
    );
    assert_eq!(report.flags.len(), 2);
    assert!(report.all_uids.is_none(), "aucune suppression possible");
    sync(&mut local, &server, CONDSTORE);
    assert_eq!(local.messages, server.flags_view());
}

#[test]
fn condstore_with_expunge_asks_for_uid_list() {
    let mut server = server_with(4, true);
    let mut local = LocalMailbox::default();
    sync(&mut local, &server, CONDSTORE);
    server.apply(&Op::Expunge(0));
    server.apply(&Op::Append(0));
    let request = select_request(&local, CONDSTORE);
    let selected = server.select(&request);
    let changed = server.report(
        &local,
        &selected,
        SyncPlan::Condstore {
            changed_since: modseq(5),
        },
    );
    assert!(
        changed.all_uids.is_some(),
        "EXISTS inchangé mais un message a disparu"
    );
    let outcome = sync(&mut local, &server, CONDSTORE);
    assert_eq!(outcome.removed, BTreeSet::from([uid(1)]));
    assert_eq!(local.messages, server.flags_view());
}

#[test]
fn unchanged_mailbox_is_up_to_date_with_condstore() {
    let server = server_with(3, true);
    let mut local = LocalMailbox::default();
    sync(&mut local, &server, CONDSTORE);
    let request = select_request(&local, CONDSTORE);
    let selected = server.select(&request);
    assert_eq!(plan(&local, &request, &selected), Ok(SyncPlan::UpToDate));
    assert!(sync(&mut local, &server, CONDSTORE).is_empty());
}

#[test]
fn without_extensions_compares_all_uids() {
    let mut server = server_with(4, false);
    let mut local = LocalMailbox::default();
    sync(&mut local, &server, NONE);
    assert_eq!(local.state.highest_modseq, None);
    server.apply(&Op::Expunge(3));
    server.apply(&Op::SetFlags(0, 0x04));
    let request = select_request(&local, NONE);
    let selected = server.select(&request);
    assert_eq!(plan(&local, &request, &selected), Ok(SyncPlan::UidCompare));
    let outcome = sync(&mut local, &server, NONE);
    assert_eq!(outcome.removed, BTreeSet::from([uid(4)]));
    assert_eq!(outcome.flag_changes.len(), 1);
    assert_eq!(local.messages, server.flags_view());
}

#[test]
fn uidvalidity_change_discards_local_messages() {
    let mut server = server_with(3, true);
    let mut local = LocalMailbox::default();
    sync(&mut local, &server, QRESYNC);
    server.apply(&Op::Expunge(0));
    server.apply(&Op::Recreate);
    let request = select_request(&local, QRESYNC);
    let selected = server.select(&request);
    assert_eq!(
        plan(&local, &request, &selected),
        Ok(SyncPlan::Full {
            discard_local: true
        })
    );
    let outcome = sync(&mut local, &server, QRESYNC);
    assert!(outcome.reset);
    assert!(outcome.removed.is_empty(), "tout est déjà retiré par reset");
    assert_eq!(outcome.new.len(), 2);
    assert_eq!(local.messages, server.flags_view());
    assert_eq!(local.state.uid_validity, Some(validity(101)));
}

#[test]
fn modseq_going_backwards_falls_back_to_uid_compare() {
    let server = server_with(3, true);
    let mut local = LocalMailbox::default();
    sync(&mut local, &server, QRESYNC);
    local.state.highest_modseq = Some(modseq(1000));
    let request = select_request(&local, QRESYNC);
    let selected = server.select(&request);
    assert_eq!(plan(&local, &request, &selected), Ok(SyncPlan::UidCompare));
}

#[test]
fn missing_uidvalidity_is_an_error() {
    let local = LocalMailbox::default();
    let selected = SelectedMailbox::default();
    assert_eq!(
        plan(&local, &SelectRequest::default(), &selected),
        Err(SyncError::MissingUidValidity)
    );
    assert_eq!(
        reconcile(
            &local,
            &selected,
            SyncPlan::UidCompare,
            &ServerReport::default()
        ),
        Err(SyncError::MissingUidValidity)
    );
}

#[test]
fn uids_beyond_uidnext_are_ignored() {
    let local = LocalMailbox::default();
    let selected = SelectedMailbox {
        uid_validity: Some(validity(1)),
        uid_next: Some(uid(3)),
        exists: 2,
        ..SelectedMailbox::default()
    };
    let report = ServerReport {
        flags: [1, 2, 99]
            .into_iter()
            .map(|n| FlagUpdate {
                uid: uid(n),
                flags: Flags::default(),
                modseq: None,
            })
            .collect(),
        all_uids: None,
    };
    let outcome = reconcile(
        &local,
        &selected,
        SyncPlan::Full {
            discard_local: false,
        },
        &report,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        outcome.new.iter().map(|n| n.uid.get()).collect::<Vec<_>>(),
        [1, 2]
    );
}

#[test]
fn duplicate_fetch_responses_keep_the_last_one() {
    let local = LocalMailbox::default();
    let selected = SelectedMailbox {
        uid_validity: Some(validity(1)),
        uid_next: Some(uid(2)),
        exists: 1,
        ..SelectedMailbox::default()
    };
    let report = ServerReport {
        flags: [0u8, 1]
            .into_iter()
            .map(|b| FlagUpdate {
                uid: uid(1),
                flags: flags(b),
                modseq: None,
            })
            .collect(),
        all_uids: None,
    };
    let outcome = reconcile(&local, &selected, SyncPlan::UidCompare, &report)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(outcome.new.len(), 1);
    assert_eq!(outcome.new.first().map(|n| n.flags.seen), Some(true));
}

#[test]
fn interrupted_initial_sync_resumes_by_uid_compare() {
    let server = server_with(10, true);
    let mut local = LocalMailbox::default();
    interrupted_sync(&mut local, &server, QRESYNC, 4);
    assert_eq!(local.messages.len(), 4);
    assert_eq!(local.state.highest_modseq, None, "état intermédiaire");
    let request = select_request(&local, QRESYNC);
    assert!(request.qresync.is_none());
    let selected = server.select(&request);
    assert_eq!(plan(&local, &request, &selected), Ok(SyncPlan::UidCompare));
    sync(&mut local, &server, QRESYNC);
    assert_eq!(local.messages, server.flags_view());
}

#[test]
fn backoff_grows_exponentially_and_is_capped() {
    let mut b = Backoff::new(1_000, 10_000);
    let delays: Vec<u64> = (0..6).map(|_| b.next_delay_ms(0.0)).collect();
    assert_eq!(delays, [1_000, 2_000, 4_000, 8_000, 10_000, 10_000]);
    assert_eq!(b.attempts(), 6);
    b.reset();
    assert_eq!(b.next_delay_ms(1.0), 500, "gigue : au plus la moitié");
    assert_eq!(b.next_delay_ms(f64::NAN), 2_000, "gigue invalide ignorée");
    let mut huge = Backoff::new(u64::MAX / 2, u64::MAX);
    for _ in 0..100 {
        assert!(huge.next_delay_ms(0.5) > 0);
    }
}

// ---------------------------------------------------------------------------
// Propriétés
// ---------------------------------------------------------------------------

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => any::<u8>().prop_map(|b| Op::Append(b & 0x3f)),
        3 => (any::<usize>(), any::<u8>()).prop_map(|(i, b)| Op::SetFlags(i, b & 0x3f)),
        3 => any::<usize>().prop_map(Op::Expunge),
        1 => Just(Op::Recreate),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Convergence et idempotence, quelles que soient les extensions et
    /// la suite des changements côté serveur.
    #[test]
    fn sync_converges_and_is_idempotent(
        condstore in any::<bool>(),
        mode in 0u8..3,
        initial in prop::collection::vec(op(), 0..20),
        rounds in prop::collection::vec(prop::collection::vec(op(), 0..10), 1..5),
    ) {
        let mut server = Server::new(condstore);
        for o in &initial {
            server.apply(o);
        }
        let ext = extensions(mode, &server);
        let mut local = LocalMailbox::default();
        sync(&mut local, &server, ext);
        prop_assert_eq!(&local.messages, &server.flags_view());
        for round in &rounds {
            for o in round {
                server.apply(o);
            }
            sync(&mut local, &server, ext);
            prop_assert_eq!(&local.messages, &server.flags_view());
            prop_assert_eq!(local.state.uid_validity.map(UidValidity::get), Some(server.validity));
            // Idempotence : rien ne change sans changement côté serveur.
            let again = sync(&mut local, &server, ext);
            prop_assert!(again.is_empty(), "deuxième synchronisation non vide : {:?}", again);
            prop_assert_eq!(&local.messages, &server.flags_view());
        }
    }

    /// Appliquer deux fois le même résultat donne le même état.
    #[test]
    fn applying_an_outcome_twice_is_harmless(
        condstore in any::<bool>(),
        mode in 0u8..3,
        initial in prop::collection::vec(op(), 0..20),
        changes in prop::collection::vec(op(), 0..10),
    ) {
        let mut server = Server::new(condstore);
        for o in &initial {
            server.apply(o);
        }
        let ext = extensions(mode, &server);
        let mut local = LocalMailbox::default();
        sync(&mut local, &server, ext);
        for o in &changes {
            server.apply(o);
        }
        let request = select_request(&local, ext);
        let selected = server.select(&request);
        let p = plan(&local, &request, &selected).unwrap_or_else(|e| panic!("{e}"));
        let report = server.report(&local, &selected, p);
        let outcome = reconcile(&local, &selected, p, &report).unwrap_or_else(|e| panic!("{e}"));
        let mut once = local.clone();
        apply(&mut once, &outcome);
        let mut twice = once.clone();
        apply(&mut twice, &outcome);
        prop_assert_eq!(once, twice);
    }

    /// Une synchronisation interrompue à n'importe quel lot, puis reprise,
    /// converge.
    #[test]
    fn interrupted_sync_converges_after_resume(
        condstore in any::<bool>(),
        mode in 0u8..3,
        initial in prop::collection::vec(op(), 0..30),
        changes in prop::collection::vec(op(), 0..10),
        kept in 0usize..10,
    ) {
        let mut server = Server::new(condstore);
        for o in &initial {
            server.apply(o);
        }
        let ext = extensions(mode, &server);
        let mut local = LocalMailbox::default();
        interrupted_sync(&mut local, &server, ext, kept);
        for o in &changes {
            server.apply(o);
        }
        interrupted_sync(&mut local, &server, ext, kept);
        sync(&mut local, &server, ext);
        prop_assert_eq!(&local.messages, &server.flags_view());
    }
}
