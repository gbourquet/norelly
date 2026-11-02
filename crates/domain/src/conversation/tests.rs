//! Tests des fils de conversation : sujets, cas réels anonymisés (listes de
//! diffusion, Outlook, Gmail, Thunderbird), puis propriétés (`proptest`) :
//! l'index incrémental converge vers la partition de JWZ quel que soit
//! l'ordre d'arrivée, et aucune entrée ne provoque de panique.

use proptest::prelude::*;

use super::*;

const DAY: i64 = 24 * 3600;

fn ids(raw: &[&str]) -> Vec<String> {
    raw.iter().map(|s| (*s).to_owned()).collect()
}

fn msg(
    key: u32,
    own: Option<&str>,
    refs: &[&str],
    irt: &[&str],
    subject: &str,
    date: i64,
) -> ThreadMessage<u32> {
    ThreadMessage {
        key,
        keys: ThreadKeys::from_parts(own, &ids(refs), &ids(irt), subject),
        date,
    }
}

/// Partition canonique produite par JWZ.
fn jwz_partition(messages: &[ThreadMessage<u32>]) -> Vec<Vec<u32>> {
    let mut out: Vec<Vec<u32>> = thread(messages)
        .into_iter()
        .map(|t| {
            let mut keys: Vec<u32> = t.entries.into_iter().map(|e| e.key).collect();
            keys.sort_unstable();
            keys
        })
        .collect();
    out.sort();
    out
}

fn incremental_partition(messages: &[ThreadMessage<u32>]) -> Vec<Vec<u32>> {
    let mut index = ConversationIndex::new();
    for m in messages {
        index.insert(m.key, &m.keys, m.date);
    }
    index.partition()
}

// ---------------------------------------------------------------------------
// Sujets
// ---------------------------------------------------------------------------

#[test]
fn subject_prefixes_and_tags_are_removed() {
    for (raw, key, reply) in [
        ("Réunion jeudi", "réunion jeudi", false),
        ("Re: Réunion jeudi", "réunion jeudi", true),
        ("RE: RE: Re:  Réunion   jeudi", "réunion jeudi", true),
        ("Fwd: TR: réunion jeudi", "réunion jeudi", true),
        ("RE[2]: Devis", "devis", true),
        ("Re (3): Devis", "devis", true),
        ("AW: WG: Angebot", "angebot", true),
        ("SV: Tilbud", "tilbud", true),
        ("Réf : dossier 42", "dossier 42", true),
        ("Ｒｅ：Bonjour", "ｒｅ：bonjour", false),
        ("Re：Bonjour", "bonjour", true),
        ("[liste-dev] Re: [PATCH] correctif", "correctif", true),
        ("Re: [liste-dev] correctif", "correctif", true),
        (
            "[seulement une étiquette]",
            "[seulement une étiquette]",
            false,
        ),
        ("Regarde ça", "regarde ça", false),
        ("Rendez-vous", "rendez-vous", false),
        ("Re:", "", true),
        ("", "", false),
    ] {
        let k = SubjectKey::of(raw);
        assert_eq!((k.as_str(), k.is_reply()), (key, reply), "{raw:?}");
    }
}

#[test]
fn subject_with_thousands_of_prefixes_is_bounded() {
    let raw = "Re: ".repeat(10_000) + "fin";
    let k = SubjectKey::of(&raw);
    assert!(k.is_reply());
    assert!(k.as_str().starts_with("re:"));
}

#[test]
fn thread_keys_merge_references_and_in_reply_to() {
    let k = ThreadKeys::from_parts(
        Some("c@x"),
        &ids(&["a@x", "b@x", "a@x", "c@x", ""]),
        &ids(&["b@x", "autre@x"]),
        "Re: sujet",
    );
    assert_eq!(k.ancestors, ids(&["a@x", "b@x"]));
    let k = ThreadKeys::from_parts(Some("c@x"), &[], &ids(&["b@x", "autre@x"]), "sujet");
    assert_eq!(k.ancestors, ids(&["b@x"]));
    assert!(k.is_reply());
    let many: Vec<String> = (0..500).map(|i| format!("{i}@x")).collect();
    let k = ThreadKeys::from_parts(None, &many, &[], "");
    assert_eq!(k.ancestors.len(), MAX_REFERENCES);
}

// ---------------------------------------------------------------------------
// Cas réels anonymisés
// ---------------------------------------------------------------------------

/// Liste de diffusion : chaîne `References` complète, message racine
/// jamais reçu (abonnement postérieur).
#[test]
fn mailing_list_thread_with_missing_root() {
    let messages = vec![
        msg(
            1,
            Some("2@liste"),
            &["1@liste"],
            &["1@liste"],
            "Re: [dev] Fuite mémoire",
            100,
        ),
        msg(
            2,
            Some("3@liste"),
            &["1@liste", "2@liste"],
            &["2@liste"],
            "Re: [dev] Fuite mémoire",
            200,
        ),
        msg(
            3,
            Some("4@liste"),
            &["1@liste"],
            &["1@liste"],
            "Re: [dev] Fuite mémoire",
            300,
        ),
    ];
    let threads = thread(&messages);
    assert_eq!(threads.len(), 1);
    let entries = &threads[0].entries;
    // Racine absente : les deux réponses directes sont sœurs, la réponse à
    // la réponse est en dessous de la première.
    let order: Vec<(u32, usize, Option<u32>)> =
        entries.iter().map(|e| (e.key, e.depth, e.parent)).collect();
    assert_eq!(order, vec![(1, 0, None), (2, 1, Some(1)), (3, 0, None)]);
    assert_eq!(threads[0].latest_date, 300);
    assert_eq!(incremental_partition(&messages), jwz_partition(&messages));
}

/// Outlook : pas de `References`, seulement `In-Reply-To`, préfixes `RE:`
/// et `TR:`.
#[test]
fn outlook_in_reply_to_only() {
    let messages = vec![
        msg(1, Some("AM0PR01@outlook"), &[], &[], "Devis 2026", 0),
        msg(
            2,
            Some("AM0PR02@outlook"),
            &[],
            &["AM0PR01@outlook"],
            "RE: Devis 2026",
            DAY,
        ),
        msg(
            3,
            Some("AM0PR03@outlook"),
            &[],
            &["AM0PR02@outlook"],
            "TR: RE: Devis 2026",
            2 * DAY,
        ),
    ];
    let threads = thread(&messages);
    assert_eq!(threads.len(), 1);
    let depths: Vec<usize> = threads[0].entries.iter().map(|e| e.depth).collect();
    assert_eq!(depths, vec![0, 1, 2]);
    assert_eq!(incremental_partition(&messages), vec![vec![1, 2, 3]]);
}

/// Gmail : la réponse arrive avant l'original (synchronisations de deux
/// dossiers) ; l'original rejoint la conversation par son identifiant.
#[test]
fn reply_received_before_original() {
    let messages = vec![
        msg(
            1,
            Some("CAB2@mail.gmail.com"),
            &["CAB1@mail.gmail.com"],
            &["CAB1@mail.gmail.com"],
            "Re: Photos",
            50,
        ),
        msg(2, Some("CAB1@mail.gmail.com"), &[], &[], "Photos", 10),
    ];
    let threads = thread(&messages);
    assert_eq!(threads.len(), 1);
    let order: Vec<u32> = threads[0].entries.iter().map(|e| e.key).collect();
    assert_eq!(order, vec![2, 1]);
    assert_eq!(incremental_partition(&messages), vec![vec![1, 2]]);
}

/// Deux conversations séparées puis réunies par un message qui cite les
/// deux (réponse groupée) : fusion.
#[test]
fn message_citing_two_conversations_merges_them() {
    let messages = vec![
        msg(1, Some("a@x"), &[], &[], "Budget", 0),
        msg(2, Some("b@x"), &[], &[], "Planning", 10),
        msg(
            3,
            Some("c@x"),
            &["a@x", "b@x"],
            &["b@x"],
            "Re: Budget et planning",
            20,
        ),
    ];
    let mut index = ConversationIndex::new();
    let a = index.insert(1, &messages[0].keys, 0);
    let b = index.insert(2, &messages[1].keys, 10);
    assert_ne!(a, b);
    let c = index.insert(3, &messages[2].keys, 20);
    assert_eq!(c, a.min(b));
    assert_eq!(index.partition(), vec![vec![1, 2, 3]]);
    assert_eq!(jwz_partition(&messages), vec![vec![1, 2, 3]]);
}

/// Deux « Bonjour » sans rapport restent séparés (aucun n'est une
/// réponse) ; un « Re: Bonjour » sans référence rejoint le plus récent
/// dans la fenêtre, pas au-delà.
#[test]
fn subject_fallback_only_for_replies_within_window() {
    let messages = vec![
        msg(1, Some("1@a"), &[], &[], "Bonjour", 0),
        msg(2, Some("2@b"), &[], &[], "Bonjour", DAY),
        msg(3, Some("3@c"), &[], &[], "Re: Bonjour", 2 * DAY),
        msg(
            4,
            Some("4@d"),
            &[],
            &[],
            "Re: Bonjour",
            2 * DAY + SUBJECT_WINDOW_SECS + 1,
        ),
    ];
    assert_eq!(
        incremental_partition(&messages),
        vec![vec![1], vec![2, 3], vec![4]]
    );
    assert_eq!(jwz_partition(&messages), vec![vec![1], vec![2, 3], vec![4]]);
}

/// Thunderbird : chaîne `References` tronquée au milieu (RFC 5322
/// autorise à omettre des identifiants) : le fil reste uni.
#[test]
fn truncated_references_keep_the_thread() {
    let messages = vec![
        msg(1, Some("r@tb"), &[], &[], "Plan", 0),
        msg(2, Some("s@tb"), &["r@tb"], &["r@tb"], "Re: Plan", 1),
        msg(3, Some("t@tb"), &["r@tb", "s@tb"], &["s@tb"], "Re: Plan", 2),
        msg(4, Some("u@tb"), &["r@tb", "t@tb"], &["t@tb"], "Re: Plan", 3),
    ];
    let threads = thread(&messages);
    assert_eq!(threads.len(), 1);
    let parents: Vec<Option<u32>> = threads[0].entries.iter().map(|e| e.parent).collect();
    assert_eq!(parents, vec![None, Some(1), Some(2), Some(3)]);
}

/// Références en boucle (A cite B, B cite A) et identifiant en double :
/// pas de boucle infinie, chaque message apparaît une fois.
#[test]
fn reference_loops_and_duplicate_ids_are_harmless() {
    let messages = vec![
        msg(1, Some("a@x"), &["b@x"], &[], "x", 0),
        msg(2, Some("b@x"), &["a@x"], &[], "x", 1),
        msg(3, Some("a@x"), &[], &[], "doublon", 2),
        msg(4, None, &["a@x"], &[], "Re: x", 3),
    ];
    let threads = thread(&messages);
    let mut all: Vec<u32> = threads
        .iter()
        .flat_map(|t| t.entries.iter().map(|e| e.key))
        .collect();
    all.sort_unstable();
    assert_eq!(all, vec![1, 2, 3, 4]);
}

#[test]
fn threads_are_sorted_by_latest_message() {
    let messages = vec![
        msg(1, Some("a@x"), &[], &[], "ancien", 0),
        msg(2, Some("b@x"), &[], &[], "récent", 100),
        msg(3, Some("c@x"), &["a@x"], &[], "Re: ancien", 200),
    ];
    let threads = thread(&messages);
    let latest: Vec<i64> = threads.iter().map(|t| t.latest_date).collect();
    assert_eq!(latest, vec![200, 100]);
}

#[test]
fn assign_decides_join_merge_or_new() {
    let reply = ThreadKeys::from_parts(Some("x"), &[], &[], "Re: a");
    let plain = ThreadKeys::from_parts(Some("x"), &[], &[], "a");
    let none = BTreeSet::new();
    let cand = Some(SubjectCandidate {
        conversation: 7,
        latest_date: 0,
    });
    assert_eq!(assign(&plain, 0, &none, cand), Assignment::New);
    assert_eq!(assign(&reply, 0, &none, cand), Assignment::Join(7));
    assert_eq!(
        assign(&reply, SUBJECT_WINDOW_SECS + 1, &none, cand),
        Assignment::New
    );
    let one: BTreeSet<i32> = [3].into();
    assert_eq!(assign(&plain, 0, &one, cand), Assignment::Join(3));
    let many: BTreeSet<i32> = [9, 3, 5].into();
    assert_eq!(
        assign(&plain, 0, &many, cand),
        Assignment::Merge {
            into: 3,
            from: vec![5, 9]
        }
    );
    assert_eq!(Assignment::<i32>::New.target(), None);
}

#[test]
fn unified_scope_rejects_normal_and_all() {
    assert_eq!(
        ConversationScope::unified(MailboxRole::Inbox),
        Ok(ConversationScope::unified_inbox())
    );
    assert!(ConversationScope::unified(MailboxRole::Sent).is_ok());
    assert_eq!(
        ConversationScope::unified(MailboxRole::Normal),
        Err(ScopeError)
    );
    assert_eq!(
        ConversationScope::unified(MailboxRole::All),
        Err(ScopeError)
    );
}

// ---------------------------------------------------------------------------
// Propriétés
// ---------------------------------------------------------------------------

/// Forêt de messages cohérente : chaque message choisit un parent parmi
/// les précédents (ou aucun) ; ses références sont la chaîne de ses
/// ancêtres, éventuellement tronquée au début. Certains messages sont
/// « perdus » (jamais reçus) mais restent cités. Sujets tous distincts :
/// seul le chaînage par identifiants est en jeu.
fn forest() -> impl Strategy<Value = Vec<ThreadMessage<u32>>> {
    (1usize..40)
        .prop_flat_map(|n| {
            (
                proptest::collection::vec(proptest::option::of(0usize..64), n),
                proptest::collection::vec(0usize..4, n),
                proptest::collection::vec(proptest::bool::weighted(0.15), n),
                Just(n),
            )
        })
        .prop_map(|(parents, cuts, lost, n)| {
            let mut chains: Vec<Vec<String>> = Vec::with_capacity(n);
            let mut out = Vec::new();
            for i in 0..n {
                let parent = parents[i].filter(|_| i > 0).map(|p| p % i);
                let mut chain = parent.map(|p| chains[p].clone()).unwrap_or_default();
                if let Some(p) = parent {
                    chain.push(format!("{p}@forêt"));
                }
                chains.push(chain.clone());
                if lost[i] {
                    continue;
                }
                let cut = cuts[i].min(chain.len().saturating_sub(1));
                let refs: Vec<String> =
                    chain.get(cut..).map(<[String]>::to_vec).unwrap_or_default();
                let key = u32::try_from(i).unwrap_or(u32::MAX);
                out.push(ThreadMessage {
                    key,
                    keys: ThreadKeys::from_parts(
                        Some(&format!("{i}@forêt")),
                        &refs,
                        &[],
                        &format!("sujet {i}"),
                    ),
                    date: i64::from(key),
                });
            }
            out
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Convergence : l'index incrémental donne la partition de JWZ, quel
    /// que soit l'ordre d'arrivée des messages.
    #[test]
    fn incremental_matches_jwz_in_any_order(
        (messages, order) in forest().prop_flat_map(|m| {
            let n = m.len();
            (Just(m), Just((0..n).collect::<Vec<_>>()).prop_shuffle())
        })
    ) {
        let reference = jwz_partition(&messages);
        let shuffled: Vec<ThreadMessage<u32>> = order.iter().filter_map(|&i| messages.get(i).cloned()).collect();
        prop_assert_eq!(incremental_partition(&shuffled), reference.clone());
        // JWZ lui-même ne dépend pas de l'ordre d'entrée.
        prop_assert_eq!(jwz_partition(&shuffled), reference);
    }

    /// Idempotence : réinsérer les mêmes messages ne change rien.
    #[test]
    fn reinserting_is_idempotent(messages in forest()) {
        let mut index = ConversationIndex::new();
        for m in &messages {
            index.insert(m.key, &m.keys, m.date);
        }
        let first = index.partition();
        for m in &messages {
            index.insert(m.key, &m.keys, m.date);
        }
        prop_assert_eq!(index.partition(), first);
    }

    /// Entrées arbitraires (boucles, doublons, sujets hostiles) : pas de
    /// panique, chaque message apparaît exactement une fois, profondeurs
    /// cohérentes.
    #[test]
    fn arbitrary_input_never_panics(
        raw in proptest::collection::vec(
            (
                proptest::option::of(0u8..12),
                proptest::collection::vec(0u8..12, 0..6),
                proptest::collection::vec(0u8..12, 0..2),
                "(Re: |Fwd: |\\[l\\] )*[a-c]{0,2}",
                0i64..100,
            ),
            0..30,
        )
    ) {
        let messages: Vec<ThreadMessage<u32>> = raw
            .iter()
            .enumerate()
            .map(|(i, (own, refs, irt, subject, date))| ThreadMessage {
                key: u32::try_from(i).unwrap_or(u32::MAX),
                keys: ThreadKeys::from_parts(
                    own.map(|o| format!("{o}@x")).as_deref(),
                    &refs.iter().map(|r| format!("{r}@x")).collect::<Vec<_>>(),
                    &irt.iter().map(|r| format!("{r}@x")).collect::<Vec<_>>(),
                    subject,
                ),
                date: *date * DAY,
            })
            .collect();
        let threads = thread(&messages);
        let mut seen: Vec<u32> = threads.iter().flat_map(|t| t.entries.iter().map(|e| e.key)).collect();
        seen.sort_unstable();
        let expected: Vec<u32> = messages.iter().map(|m| m.key).collect();
        prop_assert_eq!(seen, expected);
        for t in &threads {
            for (i, e) in t.entries.iter().enumerate() {
                if e.depth > 0 {
                    prop_assert!(i > 0);
                    prop_assert!(e.parent.is_some());
                }
            }
        }
        let mut index = ConversationIndex::new();
        for m in &messages {
            index.insert(m.key, &m.keys, m.date);
        }
        prop_assert_eq!(index.partition().iter().map(Vec::len).sum::<usize>(), messages.len());
    }
}
