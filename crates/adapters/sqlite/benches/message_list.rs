//! Bench de la liste des messages (ADR-007) sur une boîte synthétique de
//! 50 000 messages : première page et changement de dossier, requête
//! seule. Seuil exigé : 50 ms (médiane), vérifié avant les mesures
//! `criterion` ; `cargo bench -p norelly-sqlite -- --test` échoue s'il
//! est dépassé.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use norelly_domain::account::AccountId;
use norelly_domain::mailbox::{MailboxId, MailboxName, MailboxRole, RemoteMailbox};
use norelly_domain::ports::{
    AccountRepository, MailStore, MessageListQuery, StoredMessage, SyncBatch,
};
use norelly_sqlite::mail::SqliteMailStore;
use norelly_sqlite::{Database, SqliteAccountRepository};
use norelly_test_support::contracts::sample_account;
use norelly_test_support::synthetic::synthetic_messages;
use tokio::runtime::Runtime;

/// Taille de la boîte principale.
const INBOX_SIZE: u32 = 50_000;
/// Taille du second dossier.
const OTHER_SIZE: u32 = 20_000;
/// Taille d'une page d'écran.
const PAGE: u32 = 50;
/// Seuil exigé.
const THRESHOLD: Duration = Duration::from_millis(50);

struct Fixture {
    rt: Runtime,
    store: SqliteMailStore,
    inbox: MailboxId,
    other: MailboxId,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn mailbox(name: &str, role: MailboxRole) -> RemoteMailbox {
    RemoteMailbox {
        name: MailboxName::parse(name).unwrap_or_else(|e| panic!("{e}")),
        delimiter: Some('/'),
        role,
        selectable: true,
    }
}

async fn fill(store: &SqliteMailStore, mailbox: MailboxId, messages: Vec<StoredMessage>) {
    for chunk in messages.chunks(5_000) {
        store
            .apply_sync(
                mailbox,
                &SyncBatch {
                    new: chunk.to_vec(),
                    ..SyncBatch::default()
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    }
}

fn fixture() -> Fixture {
    let rt = Runtime::new().unwrap_or_else(|e| panic!("{e}"));
    let dir = std::env::temp_dir().join(format!("norelly-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let db = Database::open(&dir.join("bench.sqlite3")).unwrap_or_else(|e| panic!("{e}"));
    let store = SqliteMailStore::new(db.clone());
    let (inbox, other) = rt.block_on(async {
        let account: AccountId = SqliteAccountRepository::new(db)
            .insert(&sample_account("bench@example.com", false))
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .id();
        let boxes = store
            .replace_mailboxes(
                account,
                &[
                    mailbox("INBOX", MailboxRole::Inbox),
                    mailbox("Archives", MailboxRole::Archive),
                ],
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let (Some(inbox), Some(other)) = (boxes.first(), boxes.get(1)) else {
            panic!("boîtes absentes");
        };
        let started = Instant::now();
        fill(
            &store,
            inbox.id,
            synthetic_messages(INBOX_SIZE, 1_600_000_000, 1),
        )
        .await;
        fill(
            &store,
            other.id,
            synthetic_messages(OTHER_SIZE, 1_500_000_000, 2),
        )
        .await;
        eprintln!(
            "boîte synthétique : {} messages insérés en {:?}",
            INBOX_SIZE + OTHER_SIZE,
            started.elapsed()
        );
        (inbox.id, other.id)
    });
    Fixture {
        rt,
        store,
        inbox,
        other,
        dir,
    }
}

fn page(f: &Fixture, mailbox: MailboxId) -> usize {
    f.rt.block_on(f.store.page(mailbox, None, PAGE))
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
    assert_eq!(page(f, f.inbox), PAGE as usize);
    let first = median(31, || {
        page(f, f.inbox);
    });
    let mut flip = false;
    let switch = median(31, || {
        flip = !flip;
        page(f, if flip { f.other } else { f.inbox });
    });
    eprintln!(
        "première page : {first:?} ; changement de dossier : {switch:?} (seuil {THRESHOLD:?})"
    );
    assert!(
        first < THRESHOLD,
        "première page en {first:?} (> {THRESHOLD:?})"
    );
    assert!(
        switch < THRESHOLD,
        "changement de dossier en {switch:?} (> {THRESHOLD:?})"
    );
}

fn bench(c: &mut Criterion) {
    let f = fixture();
    check_thresholds(&f);
    c.bench_function("première page (50k messages)", |b| {
        b.iter(|| page(&f, f.inbox));
    });
    let mut flip = false;
    c.bench_function("changement de dossier", |b| {
        b.iter(|| {
            flip = !flip;
            page(&f, if flip { f.other } else { f.inbox })
        });
    });
    c.bench_function("page suivante (curseur)", |b| {
        let first =
            f.rt.block_on(f.store.page(f.inbox, None, PAGE))
                .unwrap_or_else(|e| panic!("{e}"));
        b.iter(|| {
            f.rt.block_on(f.store.page(f.inbox, first.next, PAGE))
                .unwrap_or_else(|e| panic!("{e}"))
        });
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
