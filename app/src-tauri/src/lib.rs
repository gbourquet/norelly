//! Shell Tauri de Norelly : adaptateur entrant (commandes IPC, protocoles
//! personnalisés) et point d'assemblage des adaptateurs (ADR-008).

pub mod actions;
pub mod commands;
pub mod ipc;
pub mod mail;
pub mod protocols;
pub mod reading;
pub mod remote_images;
pub mod state;

use norelly_domain::ports::AccountRepository;
use tauri::Manager;

/// Chemin des bindings TypeScript générés, relatif à `app/src-tauri`.
pub const BINDINGS_PATH: &str = "../src/lib/ipc/bindings.ts";

/// Constructeur du contrat IPC : liste des commandes exposées.
#[must_use]
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![
            commands::list_accounts,
            commands::discover_settings,
            commands::add_account,
            commands::trust_certificate,
            commands::allow_file_secrets,
            commands::remove_account,
            reading::list_folders,
            reading::list_conversations,
            reading::conversation_messages,
            reading::open_message,
            reading::allow_remote_images,
            reading::reading_preferences,
            reading::set_prefer_plain_text,
            reading::sync_account,
            reading::open_external,
            reading::save_attachment,
            reading::open_attachment,
            actions::perform_action,
            actions::undo_actions,
            actions::set_archive_folder,
        ])
        .events(tauri_specta::collect_events![ipc::SyncEventDto])
}

/// Configuration de l'export TypeScript (en-tête stable).
#[must_use]
pub fn typescript() -> specta_typescript::Typescript {
    specta_typescript::Typescript::default()
        .header("// Fichier généré par tauri-specta (app/src-tauri) : ne pas modifier.\n// Régénérer : ./scripts/gen-bindings.sh\n")
}

/// Lance l'application.
///
/// # Panics
/// Si la boucle d'événements Tauri ne peut pas démarrer (erreur fatale au
/// lancement, rien à récupérer).
#[allow(clippy::expect_used)]
pub fn run() {
    let builder = specta_builder();
    protocols::register(tauri::Builder::default())
        .plugin(protocols::navigation_guard())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let data_dir = app.path().app_data_dir()?;
            let cache_dir = app.path().app_cache_dir()?;
            let state = state::AppState::open(&data_dir, &cache_dir)?;
            app.manage(state);
            // Synchronisation de chaque compte enregistré.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<state::AppState>();
                match state.accounts.list().await {
                    Ok(accounts) => {
                        for account in &accounts {
                            if let Err(e) = state
                                .mail
                                .start(&handle, account, &state.secrets, &state.exceptions)
                                .await
                            {
                                tracing::warn!(error = %e, "synchronisation non démarrée");
                            }
                        }
                    }
                    Err(e) => tracing::error!(error = %e, "lecture des comptes"),
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("échec du démarrage de Norelly");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Les bindings TypeScript commités correspondent au contrat Rust.
    /// Pour les régénérer : `./scripts/gen-bindings.sh`.
    #[test]
    fn bindings_are_up_to_date() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let committed = manifest.join(BINDINGS_PATH);
        let generated =
            std::env::temp_dir().join(format!("norelly-bindings-{}.ts", std::process::id()));
        if let Err(e) = specta_builder().export(typescript(), &generated) {
            panic!("export des bindings impossible : {e}");
        }
        let fresh = std::fs::read_to_string(&generated).unwrap_or_default();
        let _ = std::fs::remove_file(&generated);
        if std::env::var_os("NORELLY_UPDATE_BINDINGS").is_some() {
            if let Err(e) = std::fs::write(&committed, &fresh) {
                panic!("écriture de {} : {e}", committed.display());
            }
            return;
        }
        let current = std::fs::read_to_string(&committed).unwrap_or_default();
        assert!(
            current == fresh,
            "bindings TypeScript périmés ({}) : lancer ./scripts/gen-bindings.sh",
            committed.display()
        );
    }
}
