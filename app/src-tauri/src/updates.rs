//! Mises à jour de l'application (ADR-006, phase 10).
//!
//! - Seul l'installateur NSIS se met à jour lui-même : le module
//!   `tauri-plugin-updater` n'est compilé qu'avec la fonctionnalité
//!   `updater` (activée par `release.yml` pour Windows). Ailleurs, les
//!   commandes répondent « non pris en charge » et rien n'est contacté.
//! - Aucune recherche sans un « oui » explicite (`update_check_due`) ; la
//!   requête vise une adresse fixe (manifeste `latest.json` des versions
//!   GitHub), sans aucun identifiant.
//! - Le paquet téléchargé est vérifié par sa signature minisign (clé
//!   publique de la configuration) avant toute installation ; une version
//!   antérieure ou égale est ignorée par le module.

use tauri::State;

use crate::ipc::{ErrorCodeDto, FieldDto, IpcError, UpdateStatusDto};

/// Ce binaire se met-il à jour lui-même ?
pub const SUPPORTED: bool = cfg!(feature = "updater");

/// Longueur maximale d'un numéro de version affiché.
const MAX_VERSION_LEN: usize = 64;

/// Version annoncée par le manifeste distant, revalidée avant d'être
/// montrée : chiffres, lettres, `.`, `-` et `+`, longueur bornée.
#[must_use]
pub fn available_version(raw: &str) -> Option<String> {
    let valid = !raw.is_empty()
        && raw.len() <= MAX_VERSION_LEN
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    valid.then(|| raw.to_owned())
}

/// Mise à jour en attente d'installation.
#[derive(Default)]
pub struct Updates {
    available: std::sync::Mutex<Option<String>>,
    #[cfg(feature = "updater")]
    pending: tokio::sync::Mutex<Option<tauri_plugin_updater::Update>>,
}

impl std::fmt::Debug for Updates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Updates")
            .field("available", &self.available())
            .finish_non_exhaustive()
    }
}

impl Updates {
    fn available(&self) -> Option<String> {
        self.available.lock().ok().and_then(|v| v.clone())
    }
}

/// État de la recherche de mises à jour.
#[tauri::command]
#[specta::specta]
pub fn update_status(updates: State<'_, Updates>) -> UpdateStatusDto {
    UpdateStatusDto {
        supported: SUPPORTED,
        available: updates.available(),
    }
}

/// Télécharge et installe la mise à jour en attente (l'installateur ferme
/// l'application).
#[tauri::command]
#[specta::specta]
pub async fn install_update(updates: State<'_, Updates>) -> Result<(), IpcError> {
    #[cfg(feature = "updater")]
    {
        let Some(update) = updates.pending.lock().await.take() else {
            return Err(IpcError::not_found(FieldDto::Update));
        };
        update
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|e| IpcError {
                detail: Some(e.to_string()),
                ..IpcError::new(ErrorCodeDto::UpdateFailed)
            })
    }
    #[cfg(not(feature = "updater"))]
    {
        let _ = (&updates, ErrorCodeDto::UpdateFailed);
        Err(IpcError::not_found(FieldDto::Update))
    }
}

/// Intervalle entre deux examens de l'échéance (la recherche elle-même est
/// quotidienne, `update_check_due`).
#[cfg(feature = "updater")]
const POLL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Lance la recherche périodique : au lancement si l'utilisateur a dit
/// « oui », puis une fois par jour ; un « oui » donné en cours d'exécution
/// est pris en compte au prochain examen. Paramètres illisibles : aucune
/// recherche (échec fermé).
#[cfg(feature = "updater")]
pub fn spawn_checks(app: &tauri::AppHandle) {
    use norelly_domain::ports::SettingsStore;
    use norelly_domain::settings::{UpdateCheck, update_check_due};
    use tauri::Manager;
    use tauri_plugin_updater::UpdaterExt;

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last = None;
        loop {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let check = handle
                .state::<crate::state::AppState>()
                .mail
                .settings
                .settings()
                .await
                .map_or(UpdateCheck::NotAsked, |s| s.update_check);
            if update_check_due(check, last, now) {
                last = Some(now);
                let checked = match handle.updater() {
                    Ok(updater) => updater.check().await,
                    Err(e) => Err(e),
                };
                match checked {
                    Ok(Some(update)) => found(&handle, update).await,
                    Ok(None) => {}
                    Err(e) => tracing::warn!(error = %e, "recherche de mise à jour"),
                }
            }
            tokio::time::sleep(POLL).await;
        }
    });
}

/// Mise à jour trouvée : gardée pour `install_update`, annoncée à
/// l'interface.
#[cfg(feature = "updater")]
async fn found(handle: &tauri::AppHandle, update: tauri_plugin_updater::Update) {
    use tauri::Manager;

    let Some(version) = available_version(&update.version) else {
        tracing::warn!("version annoncée refusée");
        return;
    };
    let updates = handle.state::<Updates>();
    if let Ok(mut available) = updates.available.lock() {
        *available = Some(version.clone());
    }
    *updates.pending.lock().await = Some(update);
    let _ = tauri_specta::Event::emit(&crate::ipc::UpdateEventDto::Available { version }, handle);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Manager;
    use tauri::test::mock_app;

    #[test]
    fn announced_versions_are_revalidated() {
        assert_eq!(available_version("1.2.3"), Some("1.2.3".into()));
        assert_eq!(
            available_version("1.2.3-beta.1+build.5"),
            Some("1.2.3-beta.1+build.5".into())
        );
        for refused in [
            "",
            "1.2.3 <a href=x>",
            "1.2\n3",
            &"9".repeat(65),
            "1.2/../3",
        ] {
            assert_eq!(available_version(refused), None, "{refused:?}");
        }
    }

    /// Sans la fonctionnalité `updater` (Flathub, paquets, winget) : rien
    /// n'est proposé ; avec, rien à installer tant que rien n'est trouvé.
    #[tokio::test]
    async fn nothing_is_installed_without_a_verified_update() {
        let app = mock_app();
        app.manage(Updates::default());
        assert_eq!(
            update_status(app.state::<Updates>()),
            UpdateStatusDto {
                supported: cfg!(feature = "updater"),
                available: None
            }
        );
        let refused = install_update(app.state::<Updates>()).await;
        assert!(
            refused.as_ref().is_err_and(
                |e| e.code == ErrorCodeDto::NotFound && e.field == Some(FieldDto::Update)
            ),
            "{refused:?}"
        );
    }
}
