//! Résolution SRV (RFC 6186) avec `hickory-resolver`, configuration DNS du
//! système.

use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::{RData, RecordType};
use norelly_domain::discovery::{SrvRecord, SrvService};
use norelly_domain::endpoint::Host;
use norelly_domain::ports::{DiscoveryError, SrvResolver};

/// Nombre maximal d'enregistrements retenus par service.
const MAX_RECORDS: usize = 16;

/// Nom DNS interrogé : `_imaps._tcp.example.com.` (absolu, pour ne pas
/// appliquer les domaines de recherche locaux).
#[must_use]
pub fn srv_name(service: SrvService, domain: &Host) -> String {
    format!("{}.{}.", service.label(), domain.as_str())
}

/// Résolveur SRV du système.
#[derive(Clone)]
pub struct HickorySrvResolver {
    resolver: TokioResolver,
}

impl std::fmt::Debug for HickorySrvResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HickorySrvResolver")
    }
}

impl HickorySrvResolver {
    /// Résolveur configuré comme le système (`/etc/resolv.conf`, registre
    /// Windows).
    pub fn from_system() -> Result<Self, DiscoveryError> {
        let resolver = TokioResolver::builder_tokio()
            .map_err(|e| DiscoveryError::Network(e.to_string()))?
            .build()
            .map_err(|e| DiscoveryError::Network(e.to_string()))?;
        Ok(Self { resolver })
    }
}

impl SrvResolver for HickorySrvResolver {
    async fn lookup(
        &self,
        service: SrvService,
        domain: &Host,
    ) -> Result<Vec<SrvRecord>, DiscoveryError> {
        let lookup = match self
            .resolver
            .lookup(srv_name(service, domain), RecordType::SRV)
            .await
        {
            Ok(lookup) => lookup,
            Err(e) if e.is_no_records_found() || e.is_nx_domain() => return Ok(Vec::new()),
            Err(e) => return Err(DiscoveryError::Network(e.to_string())),
        };
        Ok(lookup
            .answers()
            .iter()
            .filter_map(|record| match &record.data {
                RData::SRV(srv) => Some(srv_record(
                    srv.priority,
                    srv.weight,
                    srv.port,
                    &srv.target.to_utf8(),
                )),
                _ => None,
            })
            .flatten()
            .take(MAX_RECORDS)
            .collect())
    }
}

/// Enregistrement à partir des champs DNS. La cible « . » signifie
/// « service absent » ; une cible invalide est ignorée (`None`).
fn srv_record(priority: u16, weight: u16, port: u16, target: &str) -> Option<SrvRecord> {
    let target = if target == "." || target.is_empty() {
        None
    } else {
        Some(Host::parse(target).ok()?)
    };
    Some(SrvRecord {
        priority,
        weight,
        port,
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_absolute() {
        let Ok(d) = Host::parse("example.com") else {
            panic!("domaine");
        };
        assert_eq!(srv_name(SrvService::Imaps, &d), "_imaps._tcp.example.com.");
        assert_eq!(
            srv_name(SrvService::Submission, &d),
            "_submission._tcp.example.com."
        );
    }

    #[test]
    fn records_from_dns_fields() {
        assert_eq!(
            srv_record(0, 1, 993, "IMAP.example.com."),
            Some(SrvRecord {
                priority: 0,
                weight: 1,
                port: 993,
                target: Host::parse("imap.example.com").ok(),
            })
        );
        assert_eq!(srv_record(0, 0, 0, ".").map(|r| r.target), Some(None));
        assert_eq!(srv_record(0, 0, 993, "bad host\r\n"), None);
    }
}
