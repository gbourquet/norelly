//! Base de mesure pour le test end-to-end de performance (phase 9) : un
//! compte (serveurs sur un domaine `.invalid`, jamais joignables : aucune
//! synchronisation ne modifie la base pendant la mesure), une boîte de
//! réception de 50 000 messages synthétiques et des Archives de 20 000 ;
//! question du premier lancement déjà répondue.
//!
//! Usage : `cargo run -p norelly-sqlite --example seed_perf -- <fichier.sqlite3>`

use norelly_domain::mailbox::{MailboxName, MailboxRole, RemoteMailbox};
use norelly_domain::ports::{AccountRepository, MailStore, SettingsStore, SyncBatch};
use norelly_domain::settings::{Settings, UpdateCheck};
use norelly_sqlite::mail::SqliteMailStore;
use norelly_sqlite::preferences::SqliteSettingsStore;
use norelly_sqlite::{Database, SqliteAccountRepository};
use norelly_test_support::contracts::sample_account;
use norelly_test_support::synthetic::synthetic_messages;

fn mailbox(name: &str, role: MailboxRole) -> Result<RemoteMailbox, Box<dyn std::error::Error>> {
    Ok(RemoteMailbox {
        name: MailboxName::parse(name)?,
        delimiter: Some('/'),
        role,
        selectable: true,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("chemin de la base attendu")?;
    if let Some(dir) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let db = Database::open(std::path::Path::new(&path))?;
    let account = SqliteAccountRepository::new(db.clone())
        .insert(&sample_account("perf@norelly.invalid", false))
        .await?
        .id();
    // Utilisateur existant : la question du premier lancement a déjà eu
    // sa réponse (la mesure porte sur un démarrage ordinaire).
    SqliteSettingsStore::new(db.clone())
        .save_settings(&Settings {
            update_check: UpdateCheck::Disabled,
            ..Settings::default()
        })
        .await?;
    let store = SqliteMailStore::new(db);
    let boxes = store
        .replace_mailboxes(
            account,
            &[
                mailbox("INBOX", MailboxRole::Inbox)?,
                mailbox("Archives", MailboxRole::Archive)?,
            ],
        )
        .await?;
    for (stored, count, start, seed) in [
        (boxes.first(), 50_000, 1_600_000_000, 1),
        (boxes.get(1), 20_000, 1_500_000_000, 2),
    ] {
        let stored = stored.ok_or("boîte absente")?;
        for chunk in synthetic_messages(count, start, seed).chunks(5_000) {
            store
                .apply_sync(
                    stored.id,
                    &SyncBatch {
                        new: chunk.to_vec(),
                        ..SyncBatch::default()
                    },
                )
                .await?;
        }
    }
    println!("base de mesure prête : {path}");
    Ok(())
}
