//! Domaine de Norelly : règles métier pures, sans E/S, et ports (traits).
//!
//! - [`endpoint`] : hôte, port, sécurité du transport ;
//! - [`credentials`] : identifiants, secret masqué ;
//! - [`capabilities`] : capacités annoncées par un serveur ;
//! - [`auth`] : choix de la méthode et mécanismes SASL ;
//! - [`security`] : confiance TLS, consentement au clair, STARTTLS ;
//! - [`connection`] : erreurs de connexion structurées ;
//! - [`ports`] : traits implémentés par les adaptateurs.

pub mod auth;
pub mod capabilities;
pub mod connection;
pub mod credentials;
pub mod endpoint;
pub mod ports;
pub mod security;
