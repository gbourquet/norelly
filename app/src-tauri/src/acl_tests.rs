//! Liste de contrôle d'accès de l'IPC (revue de la phase 3, reportée en
//! phase 10) : les commandes de l'application ne sont accordées qu'à la
//! fenêtre principale, depuis l'origine de l'application. Vérifié avec la
//! configuration réelle (`generate_context!` : capabilities et manifeste
//! applicatif de `build.rs`).

use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{WebviewUrl, WebviewWindowBuilder};

/// Commandes enregistrées dans `specta_builder` (source de vérité).
fn registered_commands() -> Vec<&'static str> {
    let src = include_str!("lib.rs");
    let start = src
        .find("collect_commands![")
        .unwrap_or_else(|| panic!("collect_commands! introuvable"));
    let block = &src[start + "collect_commands![".len()..];
    let end = block.find(']').unwrap_or_else(|| panic!("fin de liste"));
    block[..end]
        .split(',')
        .filter_map(|item| item.trim().rsplit("::").next())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Origine des pages de l'application : `tauri://localhost`, sauf sous
/// Windows où WebView2 sert le schéma personnalisé en
/// `http://tauri.localhost`.
const APP_ORIGIN: &str = if cfg!(windows) {
    "http://tauri.localhost"
} else {
    "tauri://localhost"
};

fn invoke(webview: &tauri::WebviewWindow<MockRuntime>, cmd: &str, url: &str) -> Result<(), String> {
    let request = InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: url.parse().unwrap_or_else(|e| panic!("{e}")),
        body: InvokeBody::default(),
        headers: tauri::http::HeaderMap::default(),
        invoke_key: INVOKE_KEY.to_string(),
    };
    get_ipc_response(webview, request)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[test]
fn app_commands_are_granted_to_the_main_window_only() {
    let app = mock_builder()
        .build(tauri::generate_context!(test = true))
        .unwrap_or_else(|e| panic!("{e}"));
    // Les fenêtres de la configuration ne sont créées qu'au lancement.
    let main = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .unwrap_or_else(|e| panic!("{e}"));
    let other = WebviewWindowBuilder::new(&app, "other", WebviewUrl::default())
        .build()
        .unwrap_or_else(|e| panic!("{e}"));
    let commands = registered_commands();
    assert!(commands.len() > 30, "{commands:?}");
    for cmd in commands {
        // Accordée : l'ACL laisse passer, le gestionnaire (vide ici) ne
        // connaît pas la commande.
        let from_main = invoke(&main, cmd, APP_ORIGIN);
        assert!(
            from_main.as_ref().is_err_and(|e| e.contains("not found")),
            "{cmd} depuis main : {from_main:?}"
        );
        for (webview, url) in [(&other, APP_ORIGIN), (&main, "https://example.com/")] {
            let refused = invoke(webview, cmd, url);
            assert!(
                refused.as_ref().is_err_and(|e| e.contains("not allowed")),
                "{cmd} depuis {} ({url}) : {refused:?}",
                webview.label()
            );
        }
    }
}
