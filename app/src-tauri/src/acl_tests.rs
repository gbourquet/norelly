//! Liste de contrôle d'accès de l'IPC : les commandes de l'application ne sont accordées qu'à la
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

/// Commandes du noyau Tauri appelées par l'interface (`app/src/lib/ipc/api.ts`
/// et `bindings.ts`) : écoute et retrait des événements de l'application,
/// lecture et suivi du focus de la fenêtre native (`isFocused`,
/// `onFocusChanged`, qui écoute `tauri://focus` et `tauri://blur`), et
/// lecture des canaux (`Channel` de l'autodécouverte et de la connexion
/// OAuth).
const CORE_COMMANDS_USED: &[&str] = &[
    "plugin:__TAURI_CHANNEL__|fetch",
    "plugin:event|listen",
    "plugin:event|unlisten",
    "plugin:window|is_focused",
];

/// Toutes les commandes des plugins (noyau compris) connues de l'ACL, au
/// format de l'IPC (`plugin:<plugin>|<commande>`), d'après le manifeste que
/// `tauri-build` génère avant la compilation. Les commandes de
/// l'application (`__app-acl__`) sont vérifiées par le test précédent.
fn plugin_commands() -> Vec<String> {
    let manifests: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/gen/schemas/acl-manifests.json"
    )))
    .unwrap_or_else(|e| panic!("{e}"));
    let manifests = manifests
        .as_object()
        .unwrap_or_else(|| panic!("manifeste ACL : objet attendu"));
    let mut commands = std::collections::BTreeSet::new();
    for (plugin, manifest) in manifests {
        if plugin == "__app-acl__" {
            continue;
        }
        let name = plugin.strip_prefix("core:").unwrap_or(plugin);
        let permissions = manifest["permissions"]
            .as_object()
            .unwrap_or_else(|| panic!("{plugin} : permissions attendues"));
        for permission in permissions.values() {
            for cmd in permission["commands"]["allow"]
                .as_array()
                .unwrap_or_else(|| panic!("{plugin} : commandes attendues"))
            {
                let cmd = cmd.as_str().unwrap_or_else(|| panic!("{plugin} : {cmd}"));
                commands.insert(format!("plugin:{name}|{cmd}"));
            }
        }
    }
    commands.into_iter().collect()
}

/// Accordée par l'ACL : la commande s'exécute, ou échoue ensuite sur ses
/// arguments absents (ni « not allowed », ni commande inconnue).
fn passes_acl(result: &Result<(), String>) -> bool {
    result
        .as_ref()
        .err()
        .is_none_or(|e| !e.contains("not allowed") && !e.contains("not found"))
}

#[test]
fn main_window_is_granted_only_the_core_commands_the_front_uses() {
    let app = mock_builder()
        .build(tauri::generate_context!(test = true))
        .unwrap_or_else(|e| panic!("{e}"));
    let main = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .unwrap_or_else(|e| panic!("{e}"));
    let other = WebviewWindowBuilder::new(&app, "other", WebviewUrl::default())
        .build()
        .unwrap_or_else(|e| panic!("{e}"));
    let known = plugin_commands();
    // Garde-fou : le manifeste est bien lu (le noyau seul compte plus de
    // cent commandes).
    assert!(known.len() > 100, "{known:?}");
    let mut failures = Vec::new();
    for cmd in CORE_COMMANDS_USED {
        // Les canaux ne sont pas des commandes de l'ACL ; les autres doivent
        // exister sous ce nom (une faute de frappe serait refusée en silence).
        if !cmd.starts_with("plugin:__TAURI_CHANNEL__|") && !known.iter().any(|k| k == cmd) {
            failures.push(format!("{cmd} absente du manifeste ACL"));
        }
        let result = invoke(&main, cmd, APP_ORIGIN);
        if !passes_acl(&result) {
            failures.push(format!("{cmd} refusée : {result:?}"));
        }
    }
    // Les canaux échappent à l'ACL ; les permissions du noyau, elles, ne
    // valent que pour la fenêtre principale et l'origine de l'application.
    // Les schémas personnalisés (`mailview://`…) comptent comme locaux pour
    // l'ACL : leur isolation repose sur le cadre sans `allow-same-origin`,
    // la clé d'invocation et la CSP (e2e `reading-security`), pas ici.
    for cmd in CORE_COMMANDS_USED
        .iter()
        .filter(|c| c.starts_with("plugin:event|") || c.starts_with("plugin:window|"))
    {
        for (webview, url) in [(&other, APP_ORIGIN), (&main, "https://example.com/")] {
            let result = invoke(webview, cmd, url);
            if !result.as_ref().is_err_and(|e| e.contains("not allowed")) {
                failures.push(format!(
                    "{cmd} accordée à {} ({url}) : {result:?}",
                    webview.label()
                ));
            }
        }
    }
    // Toute autre commande de plugin est refusée, quelle que soit la
    // permission qui l'aurait accordée (capability ou `main-window.toml`).
    for cmd in known
        .iter()
        .filter(|k| !CORE_COMMANDS_USED.contains(&k.as_str()))
    {
        let result = invoke(&main, cmd, APP_ORIGIN);
        if !result.as_ref().is_err_and(|e| e.contains("not allowed")) {
            failures.push(format!("{cmd} accordée : {result:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
