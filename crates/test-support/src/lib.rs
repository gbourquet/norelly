//! Outils de test de Norelly :
//! - [`containers`] : serveurs réels (Dovecot, Mailpit) via testcontainers ;
//! - [`certs`] : CA et certificats de test générés par `rcgen` ;
//! - [`fake_server`] : faux serveurs IMAP et SMTP scriptés ;
//! - [`contracts`] : suites de contrat des ports et faux en mémoire.
//!
//! Les tests qui utilisent les conteneurs vivent dans des fichiers
//! `tests/integration*.rs` : ils sont exclus du profil nextest par défaut et
//! lancés par `cargo nextest run --workspace --profile integration`.

pub mod certs;
pub mod containers;
pub mod contracts;
pub mod fake_server;
