//! Bench de la recherche plein texte (ADR-003, phase 8) sur 50 000
//! messages synthétiques avec leur texte (environ 80 mots chacun) :
//! première page de résultats, requête seule. Seuil exigé : 100 ms
//! (médiane) pour chaque requête type, vérifié avant les mesures
//! `criterion` ; `cargo bench -p norelly-sqlite -- --test` échoue s'il est
//! dépassé.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use norelly_domain::account::AccountId;
use norelly_domain::mailbox::{MailboxName, MailboxRole, RemoteMailbox};
use norelly_domain::ports::{
    AccountRepository, BlobId, MailStore, SearchIndex, SearchRequest, SearchScope, StoredBody,
    SyncBatch,
};
use norelly_domain::search::parse_query;
use norelly_sqlite::mail::SqliteMailStore;
use norelly_sqlite::{Database, SqliteAccountRepository};
use norelly_test_support::contracts::sample_account;
use norelly_test_support::mail_contracts::sha256_hex;
use norelly_test_support::synthetic::{synthetic_messages, synthetic_text};
use tokio::runtime::Runtime;

/// Nombre de messages.
const SIZE: u32 = 50_000;
/// Mots du texte de chaque message.
const TEXT_WORDS: usize = 80;
/// Taille d'une page d'écran.
const PAGE: u32 = 50;
/// Seuil exigé.
const THRESHOLD: Duration = Duration::from_millis(100);

/// Requêtes types : mot très fréquent (sujet et texte), mot rare, champ et
/// filtre, filtres seuls (tri par date), négation, phrase, préfixe court.
const QUERIES: &[&str] = &[
    "facture",
    "ref31337",
    "from:alice is:unread",
    "has:attachment is:unread",
    "projet -devis",
    "\"compte rendu\"",
    "bo",
    "bonjour merci document",
];

struct Fixture {
    rt: Runtime,
    store: SqliteMailStore,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture() -> Fixture {
    let rt = Runtime::new().unwrap_or_else(|e| panic!("{e}"));
    let dir = std::env::temp_dir().join(format!("norelly-bench-search-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let db = Database::open(&dir.join("bench.sqlite3")).unwrap_or_else(|e| panic!("{e}"));
    let store = SqliteMailStore::new(db.clone());
    rt.block_on(async {
        let account: AccountId = SqliteAccountRepository::new(db)
            .insert(&sample_account("bench@example.com", false))
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .id();
        let boxes = store
            .replace_mailboxes(
                account,
                &[RemoteMailbox {
                    name: MailboxName::parse("INBOX").unwrap_or_else(|e| panic!("{e}")),
                    delimiter: Some('/'),
                    role: MailboxRole::Inbox,
                    selectable: true,
                }],
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let inbox = boxes
            .first()
            .map(|b| b.id)
            .unwrap_or_else(|| panic!("boîte"));
        let started = Instant::now();
        let messages = synthetic_messages(SIZE, 1_600_000_000, 1);
        let uids: Vec<_> = messages.iter().map(|m| m.meta.uid).collect();
        for chunk in messages.chunks(5_000) {
            store
                .apply_sync(
                    inbox,
                    &SyncBatch {
                        new: chunk.to_vec(),
                        ..SyncBatch::default()
                    },
                )
                .await
                .unwrap_or_else(|e| panic!("{e}"));
        }
        let blob = BlobId::parse(&sha256_hex(b"corps")).unwrap_or_else(|| panic!("blob"));
        let body = StoredBody::Full(blob);
        for uid in uids {
            let text = synthetic_text(uid.get(), 1, TEXT_WORDS);
            store
                .set_body(inbox, uid, &body, Some(&text))
                .await
                .unwrap_or_else(|e| panic!("{e}"));
        }
        eprintln!(
            "boîte synthétique : {SIZE} messages et leur texte indexés en {:?}",
            started.elapsed()
        );
    });
    Fixture { rt, store, dir }
}

fn search(f: &Fixture, text: &str) -> usize {
    let request = SearchRequest {
        query: parse_query(text).unwrap_or_else(|e| panic!("{e}")),
        scope: SearchScope::All,
        utc_offset_seconds: 3600,
        offset: 0,
        limit: PAGE,
    };
    f.rt.block_on(f.store.search(request))
        .unwrap_or_else(|e| panic!("{e}"))
        .items
        .len()
}

/// Médiane de `runs` exécutions.
fn median(runs: usize, mut f: impl FnMut()) -> Duration {
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect();
    times.sort();
    times.get(runs / 2).copied().unwrap_or_default()
}

fn check_thresholds(f: &Fixture) {
    assert_eq!(search(f, "ref31337"), 1, "mot rare : un seul message");
    assert_eq!(search(f, "facture"), PAGE as usize);
    for query in QUERIES {
        assert!(search(f, query) > 0, "{query:?} ne trouve rien");
        let t = median(21, || {
            search(f, query);
        });
        eprintln!("recherche {query:?} : {t:?} (seuil {THRESHOLD:?})");
        assert!(
            t < THRESHOLD,
            "recherche {query:?} en {t:?} (> {THRESHOLD:?})"
        );
    }
}

fn bench(c: &mut Criterion) {
    let f = fixture();
    check_thresholds(&f);
    for query in QUERIES {
        c.bench_function(&format!("recherche {query} (50k messages)"), |b| {
            b.iter(|| search(&f, query));
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
