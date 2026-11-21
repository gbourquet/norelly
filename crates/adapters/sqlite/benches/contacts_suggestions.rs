//! Bench des lectures du carnet faites par les suggestions de
//! destinataires (`SuggestRecipients`) sur un carnet synthétique de
//! 50 000 fiches : les favoris qui passent le filtre, puis tous, au plus
//! 20 chacun, puis leurs fiches complètes. Seuil exigé sur la lecture des
//! fiches trouvées (en une fois) : 10 ms (médiane) pour chaque saisie
//! type, vérifié avant les mesures `criterion` ;
//! `cargo bench -p norelly-sqlite -- --test` échoue s'il est dépassé. La
//! suggestion entière (listes filtrées comprises) est mesurée et affichée :
//! le filtrage des listes parcourt tout le carnet quand peu de fiches
//! correspondent.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use norelly_domain::contacts::{
    Contact, ContactDraft, ContactFilter, ContactId, ContactInput, EmailInput, PhoneInput,
};
use norelly_domain::ports::{ContactBook, ContactDirectory, MAX_CONTACT_SUGGESTIONS, PageSize};
use norelly_sqlite::Database;
use norelly_sqlite::contacts::SqliteContactBook;
use norelly_test_support::synthetic::Rng;
use tokio::runtime::Runtime;

/// Nombre de fiches.
const SIZE: usize = 50_000;
/// Une fiche sur `FAVORITE_EVERY` est un favori.
const FAVORITE_EVERY: usize = 20;
/// Seuil exigé pour la lecture des fiches trouvées.
const THRESHOLD: Duration = Duration::from_millis(10);

/// Saisies types : préfixe très fréquent (une lettre, deux lettres), nom
/// fréquent, organisation, nom rare (une seule fiche), domaine d'adresse,
/// texte absent (parcours de tout le carnet).
const INPUTS: &[&str] = &[
    "a",
    "ma",
    "martin",
    "acme",
    "zéphyrin",
    "example.org",
    "introuvable",
];

const GIVEN: &[&str] = &[
    "Alice",
    "Bruno",
    "Chloé",
    "David",
    "Émilie",
    "François",
    "Gaëlle",
    "Hugo",
    "Inès",
    "Julien",
    "Karim",
    "Léa",
    "Manon",
    "Nicolas",
    "Océane",
    "Paul",
    "Quentin",
    "Rose",
    "Sophie",
    "Thomas",
];
const FAMILY: &[&str] = &[
    "Martin", "Petit", "Durand", "Leroy", "Moreau", "Simon", "Laurent", "Michel", "Garcia", "Roux",
    "Bernard", "Thomas", "Robert", "Richard", "Dubois", "Lefebvre", "Mercier", "Blanc", "Guerin",
    "Fournier",
];
const ORGANIZATIONS: &[&str] = &["ACME", "Initech", "Globex", "", "", ""];
const DOMAINS: &[&str] = &["example.com", "example.org", "example.net"];

/// Fiche synthétique `n` : prénom, nom, organisation, une ou deux
/// adresses uniques, un téléphone, une note ; la fiche 31 337 porte seule
/// le prénom « Zéphyrin ».
fn synthetic_contact(rng: &mut Rng, n: usize) -> ContactDraft {
    let pick = |rng: &mut Rng, list: &[&str]| {
        let i = usize::try_from(rng.below(list.len() as u64)).unwrap_or(0);
        list[i].to_owned()
    };
    let given = if n == 31_337 {
        "Zéphyrin".to_owned()
    } else {
        pick(rng, GIVEN)
    };
    let family = pick(rng, FAMILY);
    let local = format!("contact{n}");
    let mut emails = vec![EmailInput {
        address: format!("{local}@{}", pick(rng, DOMAINS)),
        label: "travail".into(),
        preferred: true,
    }];
    if rng.below(3) == 0 {
        emails.push(EmailInput {
            address: format!("{local}.perso@{}", pick(rng, DOMAINS)),
            label: "perso".into(),
            preferred: false,
        });
    }
    ContactDraft::new(ContactInput {
        given_name: given,
        family_name: family,
        organization: pick(rng, ORGANIZATIONS),
        notes: format!("Fiche synthétique numéro {n}."),
        emails,
        phones: vec![PhoneInput {
            number: format!(
                "+33 6 {:02} {:02} {:02} {:02}",
                n % 100,
                n / 100 % 100,
                n % 97,
                n % 89
            ),
            label: "mobile".into(),
        }],
        favorite: n.is_multiple_of(FAVORITE_EVERY),
        ..ContactInput::default()
    })
    .unwrap_or_else(|e| panic!("fiche synthétique {n} refusée : {e}"))
}

struct Fixture {
    rt: Runtime,
    book: SqliteContactBook,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture() -> Fixture {
    let rt = Runtime::new().unwrap_or_else(|e| panic!("{e}"));
    // Dossier temporaire de la cible de compilation (propre à
    // l'utilisateur), pas le dossier temporaire partagé ; créé neuf, il
    // n'est jamais un dossier préparé par un autre.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "norelly-bench-contacts-suggestions-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap_or_else(|e| panic!("{e}"));
    let db = Database::open(
        &dir.join("bench.sqlite3"),
        norelly_test_support::contact_import_contracts::FakeNfc,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let book = SqliteContactBook::new(db);
    rt.block_on(async {
        let started = Instant::now();
        let mut rng = Rng::new(17);
        let drafts: Vec<ContactDraft> =
            (1..=SIZE).map(|n| synthetic_contact(&mut rng, n)).collect();
        for chunk in drafts.chunks(5_000) {
            book.import(chunk, &[], &[], &[])
                .await
                .unwrap_or_else(|e| panic!("{e}"));
        }
        eprintln!(
            "carnet synthétique : {SIZE} fiches importées en {:?}",
            started.elapsed()
        );
    });
    Fixture { rt, book, dir }
}

/// Identifiants proposés pour la saisie `text`, comme
/// `SuggestRecipients` : favoris puis tous, au plus
/// [`MAX_CONTACT_SUGGESTIONS`] chacun, chacun une fois. Reproduit les
/// lectures de `SuggestRecipients::contacts`
/// (`crates/application/src/contacts.rs`, hors de portée de cet
/// adaptateur) : à modifier avec elles.
fn found_ids(f: &Fixture, text: &str) -> Vec<ContactId> {
    f.rt.block_on(async {
        let limit = PageSize::new(u32::try_from(MAX_CONTACT_SUGGESTIONS).unwrap_or(u32::MAX));
        let mut ids = Vec::new();
        for favorites in [true, false] {
            let filter = ContactFilter::new(text, favorites);
            let page = f
                .book
                .page(&filter, None, limit)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            for item in page.items {
                if !ids.contains(&item.id) {
                    ids.push(item.id);
                }
            }
        }
        ids
    })
}

/// Fiches lues par les suggestions pour la saisie `text` : identifiants
/// trouvés ([`found_ids`]), puis leurs fiches complètes lues en une fois.
fn suggest(f: &Fixture, text: &str) -> Vec<Contact> {
    read_contacts(f, &found_ids(f, text))
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
    assert_eq!(
        suggest(f, "zéphyrin").len(),
        1,
        "nom rare : une seule fiche"
    );
    assert!(suggest(f, "introuvable").is_empty());
    // Saisie fréquente : favoris d'abord, puis assez de fiches pour remplir
    // les deux listes.
    let frequent = suggest(f, "ma");
    assert!(
        frequent.len() > MAX_CONTACT_SUGGESTIONS,
        "{} fiches",
        frequent.len()
    );
    assert!(frequent[0].draft().favorite(), "favoris d'abord");
    // Toutes les saisies mesurées avant de conclure.
    let times: Vec<(&str, Duration)> = INPUTS
        .iter()
        .map(|text| {
            let ids = found_ids(f, text);
            let read = median(21, || {
                read_contacts(f, &ids);
            });
            let total = median(21, || {
                suggest(f, text);
            });
            eprintln!(
                "suggestions {text:?} : {} fiches lues en {read:?} (seuil {THRESHOLD:?}), \
                 listes filtrées comprises {total:?}",
                ids.len()
            );
            (*text, read)
        })
        .collect();
    for (text, t) in times {
        assert!(
            t < THRESHOLD,
            "fiches des suggestions {text:?} lues en {t:?} (> {THRESHOLD:?})"
        );
    }
}

/// Fiches complètes de `ids`, lues en une fois.
fn read_contacts(f: &Fixture, ids: &[ContactId]) -> Vec<Contact> {
    f.rt.block_on(f.book.get_many(ids))
        .unwrap_or_else(|e| panic!("{e}"))
}

fn bench(c: &mut Criterion) {
    let f = fixture();
    check_thresholds(&f);
    for text in INPUTS {
        c.bench_function(&format!("suggestions {text} (50k contacts)"), |b| {
            b.iter(|| suggest(&f, text));
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
