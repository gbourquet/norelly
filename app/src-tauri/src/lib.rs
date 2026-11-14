//! Shell Tauri de Norelly : adaptateur entrant (commandes IPC, protocoles
//! personnalisés) et point d'assemblage des adaptateurs (ADR-008).

#[cfg(test)]
mod acl_tests;
pub mod actions;
#[cfg(test)]
mod command_tests;
pub mod commands;
pub mod compose;
pub mod contact_duplicates;
pub mod contact_export;
pub mod contact_import;
pub mod contact_lists;
pub mod contacts;
pub mod folders;
pub mod ipc;
pub mod logs;
pub mod mail;
pub mod protocols;
pub mod reading;
pub mod remote_images;
pub mod search;
pub mod settings;
pub mod state;
#[cfg(test)]
mod test_app;
pub mod updates;

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
            commands::oauth_provider_for,
            commands::oauth_sign_in_kept,
            commands::add_oauth_account,
            commands::reconnect_account,
            commands::trust_certificate,
            commands::allow_file_secrets,
            commands::secrets_backend,
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
            actions::mark_opened_seen,
            actions::undo_actions,
            actions::set_archive_folder,
            folders::create_folder,
            folders::rename_folder,
            folders::move_folder,
            folders::delete_folder,
            search::search_messages,
            settings::get_settings,
            settings::save_settings,
            settings::show_notification,
            compose::prepare_compose,
            compose::save_draft,
            compose::send_draft,
            compose::discard_draft,
            compose::schedule_draft,
            compose::unschedule_send,
            compose::list_drafts,
            compose::open_draft,
            compose::outbox,
            compose::retry_send,
            compose::cancel_send,
            compose::parse_recipient,
            compose::suggest_contacts,
            compose::pick_attachments,
            compose::attach_dropped,
            contacts::list_contacts,
            contacts::get_contact,
            contacts::create_contact,
            contacts::update_contact,
            contacts::delete_contacts,
            contact_import::preview_contact_import,
            contact_import::apply_contact_import,
            contact_export::export_contacts,
            contact_duplicates::find_contact_duplicates,
            contact_duplicates::merge_contacts,
            contact_duplicates::preview_contact_merge,
            contact_lists::list_contact_lists,
            contact_lists::get_contact_list,
            contact_lists::create_contact_list,
            contact_lists::update_contact_list,
            contact_lists::delete_contact_list,
            contact_lists::add_to_contact_list,
            contact_lists::expand_contact_list,
            contact_lists::suggest_contact_lists,
            updates::update_status,
            updates::install_update,
        ])
        .events(tauri_specta::collect_events![
            ipc::SyncEventDto,
            ipc::ComposeEventDto,
            ipc::UpdateEventDto
        ])
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
    let app = protocols::register(tauri::Builder::default())
        .plugin(protocols::navigation_guard())
        .plugin(tauri_plugin_dialog::init());
    // Installateur NSIS seulement (ADR-006) : aucune permission accordée à
    // l'interface, le module n'est appelé que depuis Rust (`updates`).
    #[cfg(feature = "updater")]
    let app = app.plugin(tauri_plugin_updater::Builder::new().build());
    app.manage(updates::Updates::default())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let data_dir = app.path().app_data_dir()?;
            logs::init(&data_dir);
            let cache_dir = app.path().app_cache_dir()?;
            let state = state::AppState::open(&data_dir, &cache_dir)?;
            app.manage(state);
            #[cfg(feature = "updater")]
            updates::spawn_checks(app.handle());
            // Fichiers déposés sur la fenêtre : les chemins viennent de
            // l'événement natif et restent ici ; l'interface ne reçoit
            // qu'un jeton (`attach_dropped`).
            if let Some(window) = app.get_webview_window("main") {
                let handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop {
                        paths, ..
                    }) = event
                    {
                        let files: Vec<_> = paths.iter().filter(|p| p.is_file()).cloned().collect();
                        if files.is_empty() {
                            return;
                        }
                        let count = u32::try_from(files.len()).unwrap_or(u32::MAX);
                        let token = handle
                            .state::<state::AppState>()
                            .mail
                            .remember_dropped(files);
                        let _ = tauri_specta::Event::emit(
                            &ipc::ComposeEventDto::FilesDropped { token, count },
                            &handle,
                        );
                    }
                });
            }
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
