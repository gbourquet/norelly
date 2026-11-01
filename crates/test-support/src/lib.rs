//! Outils de test de Norelly : conteneurs (Dovecot, Mailpit), faux serveurs
//! scriptés (phase 2), fixtures et générateurs.
//!
//! Les tests qui utilisent les conteneurs vivent dans des fichiers
//! `tests/integration*.rs` : ils sont exclus du profil nextest par défaut et
//! lancés par `cargo nextest run --workspace --profile integration`.

pub mod containers;
