//! Adaptateur d'autodécouverte : client autoconfig HTTPS (fournisseur et
//! ISPDB de Mozilla), résolution SRV (RFC 6186) et sondage des ports.
//!
//! La stratégie (ordre des sources, préférence de sécurité) est dans le
//! domaine (`norelly_domain::discovery`) ; ce crate ne fait que les E/S.

pub mod autoconfig_xml;
pub mod http;
pub mod probe;
pub mod srv;

pub use http::HttpAutoconfigClient;
pub use probe::TcpPortProber;
pub use srv::HickorySrvResolver;

#[cfg(test)]
mod tests;
