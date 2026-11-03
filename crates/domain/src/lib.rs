//! Domaine de Norelly : règles métier pures, sans E/S, et ports (traits).
//!
//! - [`address`] : adresse électronique validée ;
//! - [`account`] : compte (adresse, nom, serveurs) ;
//! - [`discovery`] : stratégie d'autodécouverte des serveurs ;
//! - [`endpoint`] : hôte, port, sécurité du transport ;
//! - [`credentials`] : identifiants, secret masqué ;
//! - [`capabilities`] : capacités annoncées par un serveur ;
//! - [`auth`] : choix de la méthode et mécanismes SASL ;
//! - [`security`] : confiance TLS, consentement au clair, STARTTLS ;
//! - [`connection`] : erreurs de connexion structurées ;
//! - [`mailbox`] : boîtes, UID, drapeaux, métadonnées de message ;
//! - [`sync`] : rapprochement de synchronisation, reconnexion ;
//! - [`conversation`] : fils de conversation (JWZ), boîte unifiée ;
//! - [`fetch_policy`] : politique de rapatriement des corps ;
//! - [`reading`] : liens externes, images distantes, pièces jointes ;
//! - [`actions`] : actions sur les messages, file d'opérations, rejeu ;
//! - [`ports`] : traits implémentés par les adaptateurs.

pub mod account;
pub mod actions;
pub mod address;
pub mod auth;
pub mod capabilities;
pub mod connection;
pub mod compose;
pub mod conversation;
pub mod credentials;
pub mod discovery;
pub mod endpoint;
pub mod fetch_policy;
pub mod mailbox;
pub mod ports;
pub mod reading;
pub mod security;
pub mod sync;
