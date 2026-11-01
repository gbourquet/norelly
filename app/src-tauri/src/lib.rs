//! Shell Tauri de Norelly : adaptateur entrant (commandes IPC, protocoles
//! personnalisés) et point d'assemblage des adaptateurs (ADR-008).

/// Lance l'application.
///
/// # Panics
/// Si la boucle d'événements Tauri ne peut pas démarrer (erreur fatale au
/// lancement, rien à récupérer).
#[allow(clippy::expect_used)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("échec du démarrage de Norelly");
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles() {}
}
