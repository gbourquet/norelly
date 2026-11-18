//! Adaptateur WebDAV (ADR-014) : client CardDAV
//! ([`norelly_domain::ports::CardDavClient`]) sur
//! `reqwest` et `norelly-tls`.
//!
//! - HTTPS seulement, vérification des certificats de la plateforme,
//!   **aucune exception** (ADR-002) ; jamais de repli en clair ;
//! - identifiants en HTTP Basic, envoyés à la seule origine du carnet :
//!   toute adresse citée par le serveur (ressource, redirection,
//!   principal) est revalidée par le domaine
//!   ([`norelly_domain::carddav::DavUrl::join`]), et les redirections ne
//!   sont jamais suivies automatiquement ;
//! - réponses bornées (taille, nombre, profondeur XML), DTD refusée ;
//! - écritures conditionnelles (`If-Match`, `If-None-Match: *`) : une
//!   fiche changée sur le serveur entre-temps n'est jamais écrasée.
//!
//! Les règles (adresses, découverte, conflits) sont dans le domaine
//! (`norelly_domain::carddav`) ; ce crate ne fait que les E/S.

pub mod client;
pub mod xml;

pub use client::WebDavClient;
