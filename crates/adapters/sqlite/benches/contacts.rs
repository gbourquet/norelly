//! Bancs du carnet d'adresses sur un carnet plein ([`MAX_CONTACTS`]
//! fiches synthétiques) : première page et page suivante de la liste,
//! liste filtrée, suggestions de destinataires (lectures du carnet faites
//! par la saisie d'un destinataire) et import d'un fichier de taille
//! maximale ([`MAX_IMPORT_CARDS`] fiches) qui remplit le carnet. Seuils
//! vérifiés avant les mesures `criterion` ; `cargo bench -p norelly-sqlite
//! -- --test` échoue s'ils sont dépassés.
//!
//! `NORELLY_BENCH_CONTACT_SIZES=25000,50000,100000` mesure à la place ces
//! tailles de carnet, sans seuil (au plus [`MAX_CONTACTS`]) : les mesures
//! qui ont guidé le choix du plafond.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use norelly_domain::contact_import::MAX_IMPORT_CARDS;
use norelly_domain::contacts::{ContactDraft, ContactFilter, MAX_CONTACTS};
use norelly_domain::ports::{
    ContactBook, ContactCursor, ContactDirectory, MAX_CONTACT_SUGGESTIONS, PageSize,
};
use norelly_sqlite::Database;
use norelly_sqlite::contacts::SqliteContactBook;
use norelly_test_support::synthetic::synthetic_contacts;
use tokio::runtime::Runtime;

/// Taille d'une page d'écran.
const PAGE: u32 = 50;
/// Seuil des lectures d'écran (liste, suggestions), médiane.
const READ_THRESHOLD: Duration = Duration::from_millis(50);
/// Seuil de l'import d'un fichier de taille maximale dans un carnet qui
/// en devient plein.
const IMPORT_THRESHOLD: Duration = Duration::from_secs(20);
/// Fiches écrites par import pendant le remplissage.
const FILL_CHUNK: usize = 5_000;
/// Texte courant (beaucoup de fiches le contiennent).
const COMMON: &str = "mar";
/// Texte qu'aucune fiche ne contient : la recherche parcourt tout.
const ABSENT: &str = "zqx";

struct Fixture {
    rt: Runtime,
    book: SqliteContactBook,
    /// Durée de l'import de [`MAX_IMPORT_CARDS`] fiches qui a rempli le
    /// carnet.
    import: Duration,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn import(book: &SqliteContactBook, drafts: &[ContactDraft]) {
    book.import(drafts, &[], &[], &[])
        .await
        .unwrap_or_else(|e| panic!("{e}"));
}

/// Carnet de `size` fiches : `size - MAX_IMPORT_CARDS` écrites par lots,
/// puis un import de [`MAX_IMPORT_CARDS`] fiches, chronométré.
fn fixture(size: usize) -> Fixture {
    let rt = Runtime::new().unwrap_or_else(|e| panic!("{e}"));
    let dir = std::env::temp_dir().join(format!(
        "norelly-bench-contacts-{}-{size}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let db = Database::open(
        &dir.join("bench.sqlite3"),
        norelly_test_support::contact_import_contracts::FakeNfc,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let book = SqliteContactBook::new(db);
    let base = size.saturating_sub(MAX_IMPORT_CARDS);
    let started = Instant::now();
    let existing = synthetic_contacts(base, 1);
    rt.block_on(async {
        for chunk in existing.chunks(FILL_CHUNK) {
            import(&book, chunk).await;
        }
    });
    let filled = started.elapsed();
    let file = synthetic_contacts(size - base, 2);
    let started = Instant::now();
    rt.block_on(import(&book, &file));
    let import = started.elapsed();
    eprintln!(
        "carnet de {size} fiches : {base} écrites en {filled:?}, puis {} importées en {import:?}",
        file.len()
    );
    Fixture {
        rt,
        book,
        import,
        dir,
    }
}

fn page(f: &Fixture, filter: &ContactFilter, cursor: Option<ContactCursor>) -> usize {
    f.rt.block_on(f.book.page(filter, cursor, PageSize::new(PAGE)))
        .unwrap_or_else(|e| panic!("{e}"))
        .items
        .len()
}

/// Lectures du carnet faites par la suggestion de destinataires pour
/// `text` : favoris puis tous, une page chacun, puis chaque fiche.
fn suggestions(f: &Fixture, text: &str) -> usize {
    let limit = PageSize::new(u32::try_from(MAX_CONTACT_SUGGESTIONS).unwrap_or(u32::MAX));
    f.rt.block_on(async {
        let mut ids = Vec::new();
        for filter in [
            ContactFilter::new(text, true),
            ContactFilter::new(text, false),
        ] {
            let items = f
                .book
                .page(&filter, None, limit)
                .await
                .unwrap_or_else(|e| panic!("{e}"))
                .items;
            for item in items {
                if !ids.contains(&item.id) {
                    ids.push(item.id);
                }
            }
        }
        let mut found = 0;
        for id in ids {
            if f.book
                .get(id)
                .await
                .unwrap_or_else(|e| panic!("{e}"))
                .is_some()
            {
                found += 1;
            }
        }
        found
    })
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

/// Lectures d'écran mesurées : (nom, médiane).
fn reads(f: &Fixture) -> Vec<(&'static str, Duration)> {
    let all = ContactFilter::default();
    let second =
        f.rt.block_on(f.book.page(&all, None, PageSize::new(PAGE)))
            .unwrap_or_else(|e| panic!("{e}"))
            .next;
    let common = ContactFilter::new(COMMON, false);
    let absent = ContactFilter::new(ABSENT, false);
    vec![
        (
            "première page",
            median(31, || {
                page(f, &all, None);
            }),
        ),
        (
            "page suivante",
            median(31, || {
                page(f, &all, second.clone());
            }),
        ),
        (
            "liste filtrée (texte courant)",
            median(31, || {
                page(f, &common, None);
            }),
        ),
        (
            "liste filtrée (texte absent)",
            median(31, || {
                page(f, &absent, None);
            }),
        ),
        (
            "suggestions (texte courant)",
            median(31, || {
                suggestions(f, COMMON);
            }),
        ),
        (
            "suggestions (texte absent)",
            median(31, || {
                suggestions(f, ABSENT);
            }),
        ),
    ]
}

fn check_thresholds(f: &Fixture) {
    let all = ContactFilter::default();
    assert_eq!(page(f, &all, None), PAGE as usize);
    assert_eq!(page(f, &ContactFilter::new(ABSENT, false), None), 0);
    assert_eq!(suggestions(f, COMMON), MAX_CONTACT_SUGGESTIONS);
    for (what, t) in reads(f) {
        eprintln!("{what} : {t:?} (seuil {READ_THRESHOLD:?})");
        assert!(t < READ_THRESHOLD, "{what} en {t:?} (> {READ_THRESHOLD:?})");
    }
    eprintln!(
        "import de {MAX_IMPORT_CARDS} fiches : {:?} (seuil {IMPORT_THRESHOLD:?})",
        f.import
    );
    assert!(
        f.import < IMPORT_THRESHOLD,
        "import en {:?} (> {IMPORT_THRESHOLD:?})",
        f.import
    );
}

/// Mesures à d'autres tailles de carnet, sans seuil ni banc `criterion`
/// (`true` si elles ont été demandées). Une taille au-delà de
/// [`MAX_CONTACTS`] est refusée par le carnet : pour la mesurer, relever
/// temporairement le plafond.
fn explore() -> bool {
    let Ok(sizes) = std::env::var("NORELLY_BENCH_CONTACT_SIZES") else {
        return false;
    };
    for size in sizes
        .split(',')
        .filter_map(|s| s.trim().parse::<usize>().ok())
    {
        if size > MAX_CONTACTS {
            eprintln!("[{size}] au-delà du plafond ({MAX_CONTACTS}) : ignoré");
            continue;
        }
        let f = fixture(size.max(MAX_IMPORT_CARDS));
        for (what, t) in reads(&f) {
            eprintln!("[{size}] {what} : {t:?}");
        }
        eprintln!(
            "[{size}] import de {MAX_IMPORT_CARDS} fiches : {:?}",
            f.import
        );
    }
    true
}

fn bench(c: &mut Criterion) {
    if explore() {
        return;
    }
    let f = fixture(MAX_CONTACTS);
    check_thresholds(&f);
    let all = ContactFilter::default();
    c.bench_function("carnet plein : première page", |b| {
        b.iter(|| page(&f, &all, None));
    });
    let absent = ContactFilter::new(ABSENT, false);
    c.bench_function("carnet plein : liste filtrée (texte absent)", |b| {
        b.iter(|| page(&f, &absent, None));
    });
    c.bench_function("carnet plein : suggestions (texte courant)", |b| {
        b.iter(|| suggestions(&f, COMMON));
    });
    c.bench_function("carnet plein : suggestions (texte absent)", |b| {
        b.iter(|| suggestions(&f, ABSENT));
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
