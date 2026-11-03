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

impl HickorySrvResolver {
    /// Résolveur qui n'interroge que `server` (UDP) : serveur DNS de test,
    /// jamais un vrai DNS.
    pub fn with_nameserver(server: std::net::SocketAddr) -> Result<Self, DiscoveryError> {
        use hickory_resolver::config::{ConnectionConfig, NameServerConfig, ResolverConfig};
        use hickory_resolver::net::runtime::TokioRuntimeProvider;
        let mut udp = ConnectionConfig::udp();
        udp.port = server.port();
        let config = ResolverConfig::from_name_servers(vec![NameServerConfig::new(
            server.ip(),
            true,
            vec![udp],
        )]);
        let resolver = TokioResolver::builder_with_config(config, TokioRuntimeProvider::default())
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

    use hickory_resolver::proto::op::{Message, OpCode, ResponseCode};
    use hickory_resolver::proto::rr::rdata::SRV;
    use hickory_resolver::proto::rr::{Name, Record};

    /// Serveur DNS de test (UDP, 127.0.0.1) : SRV pour `_imaps._tcp.example.com.`
    /// (dont un « service absent »), 20 SRV pour
    /// `_imap._tcp.example.com.`, SERVFAIL pour `panne.example.`, NXDOMAIN
    /// sinon.
    async fn fake_dns() -> std::net::SocketAddr {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let addr = socket.local_addr().unwrap_or_else(|e| panic!("{e}"));
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            loop {
                let Ok((n, peer)) = socket.recv_from(&mut buf).await else {
                    return;
                };
                let Ok(query) = Message::from_vec(&buf[..n]) else {
                    continue;
                };
                let mut reply = Message::response(query.metadata.id, OpCode::Query);
                reply.metadata.recursion_desired = true;
                reply.metadata.recursion_available = true;
                let name = query.queries.first().map(|q| q.name().to_ascii());
                for q in &query.queries {
                    reply.add_query(q.clone());
                }
                let srv = |prio, port, target: &str| {
                    Record::from_rdata(
                        Name::from_ascii(name.as_deref().unwrap_or("."))
                            .unwrap_or_else(|e| panic!("{e}")),
                        60,
                        hickory_resolver::proto::rr::RData::SRV(SRV::new(
                            prio,
                            1,
                            port,
                            Name::from_ascii(target).unwrap_or_else(|e| panic!("{e}")),
                        )),
                    )
                };
                match name.as_deref() {
                    Some("_imaps._tcp.example.com.") => {
                        reply.add_answer(srv(0, 993, "imap.example.com."));
                        reply.add_answer(srv(10, 0, "."));
                    }
                    Some("_imap._tcp.example.com.") => {
                        for i in 0..20 {
                            reply.add_answer(srv(i, 143, &format!("imap{i}.example.com.")));
                        }
                    }
                    Some(n) if n.ends_with("panne.example.") => {
                        reply.metadata.response_code = ResponseCode::ServFail;
                    }
                    _ => reply.metadata.response_code = ResponseCode::NXDomain,
                }
                if let Ok(bytes) = reply.to_vec() {
                    let _ = socket.send_to(&bytes, peer).await;
                }
            }
        });
        addr
    }

    fn host(raw: &str) -> Host {
        Host::parse(raw).unwrap_or_else(|_| panic!("{raw}"))
    }

    #[tokio::test]
    async fn srv_records_are_read_from_the_test_server() {
        let resolver =
            HickorySrvResolver::with_nameserver(fake_dns().await).unwrap_or_else(|e| panic!("{e}"));
        let records = resolver
            .lookup(SrvService::Imaps, &host("example.com"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(records.len(), 2, "{records:?}");
        assert!(
            records
                .iter()
                .any(|r| r.target.as_ref().map(Host::as_str) == Some("imap.example.com"))
        );
        assert!(
            records.iter().any(|r| r.target.is_none()),
            "« . » : service absent"
        );
        let many = resolver
            .lookup(SrvService::Imap, &host("example.com"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(many.len(), MAX_RECORDS);
    }

    #[tokio::test]
    async fn missing_names_are_empty_and_failures_are_errors() {
        let resolver =
            HickorySrvResolver::with_nameserver(fake_dns().await).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            resolver
                .lookup(SrvService::Submission, &host("example.com"))
                .await,
            Ok(Vec::new())
        );
        assert!(matches!(
            resolver
                .lookup(SrvService::Imaps, &host("panne.example"))
                .await,
            Err(DiscoveryError::Network(_))
        ));
    }
}
