//! Cas d'usage `DiscoverSettings` : applique la stratégie d'autodécouverte
//! du domaine (fournisseur, ISPDB, SRV, sondage) et signale la progression.
//!
//! L'échec d'une source n'est jamais fatal : on passe à la suivante. Le
//! résultat peut être partiel (IMAP sans SMTP…) ; l'interface propose
//! alors la saisie manuelle, pré-remplie.

use norelly_domain::address::EmailAddress;
use norelly_domain::discovery::{
    DiscoveredServers, Discovery, DiscoverySource, SrvService, autoconfig_requests,
    probe_candidates, srv_candidates,
};
use norelly_domain::endpoint::Protocol;
use norelly_domain::ports::{AutoconfigClient, PortProber, SrvResolver};

/// Cas d'usage : découvrir les serveurs d'une adresse.
#[derive(Debug)]
pub struct DiscoverSettings<A, R, P> {
    autoconfig: A,
    srv: R,
    prober: P,
}

impl<A, R, P> DiscoverSettings<A, R, P>
where
    A: AutoconfigClient,
    R: SrvResolver,
    P: PortProber,
{
    pub fn new(autoconfig: A, srv: R, prober: P) -> Self {
        Self {
            autoconfig,
            srv,
            prober,
        }
    }

    /// Essaie les sources dans l'ordre du domaine ; `progress` est appelé
    /// au début de chaque source réellement essayée.
    pub async fn execute(
        &self,
        address: &EmailAddress,
        mut progress: impl FnMut(DiscoverySource) + Send,
    ) -> Discovery {
        let mut discovery = Discovery::new(address.clone());
        for source in DiscoverySource::AUTOMATIC {
            if discovery.is_complete() {
                break;
            }
            progress(source);
            match source {
                DiscoverySource::ProviderAutoconfig | DiscoverySource::MozillaIspdb => {
                    self.autoconfig_source(source, &mut discovery).await;
                }
                DiscoverySource::DnsSrv => self.srv_source(&mut discovery).await,
                DiscoverySource::Probe => self.probe_source(&mut discovery).await,
                DiscoverySource::Manual => {}
            }
        }
        discovery
    }

    /// Premier fichier autoconfig trouvé pour la source.
    async fn autoconfig_source(&self, source: DiscoverySource, discovery: &mut Discovery) {
        for request in autoconfig_requests(source, discovery.address()) {
            match self.autoconfig.fetch(&request).await {
                Ok(Some(servers)) => {
                    discovery.record(source, &servers);
                    return;
                }
                Ok(None) => {}
                Err(e) => tracing_free_note(source, &e),
            }
        }
    }

    async fn srv_source(&self, discovery: &mut Discovery) {
        let mut servers = DiscoveredServers::default();
        let domain = discovery.address().domain().clone();
        for service in SrvService::ALL {
            if is_found(discovery, service.protocol()) {
                continue;
            }
            match self.srv.lookup(service, &domain).await {
                Ok(records) => {
                    for candidate in srv_candidates(service, &records) {
                        servers.push(candidate);
                    }
                }
                Err(e) => tracing_free_note(DiscoverySource::DnsSrv, &e),
            }
        }
        discovery.record(DiscoverySource::DnsSrv, &servers);
    }

    async fn probe_source(&self, discovery: &mut Discovery) {
        let candidates: Vec<_> = probe_candidates(discovery.address())
            .into_iter()
            .filter(|c| !is_found(discovery, c.protocol))
            .collect();
        let mut servers = DiscoveredServers::default();
        for candidate in self.prober.probe(candidates).await {
            servers.push(candidate);
        }
        discovery.record(DiscoverySource::Probe, &servers);
    }
}

fn is_found(discovery: &Discovery, protocol: Protocol) -> bool {
    match protocol {
        Protocol::Imap => discovery.imap().is_some(),
        Protocol::Smtp => discovery.smtp().is_some(),
    }
}

/// L'application ne journalise pas (pas de dépendance `tracing`) : les
/// échecs de source sont attendus et sans conséquence. Point d'accroche
/// unique si un journal devient nécessaire.
fn tracing_free_note(_source: DiscoverySource, _error: &dyn std::fmt::Display) {}

#[cfg(test)]
mod tests {
    use norelly_domain::discovery::{ServerCandidate, SrvRecord, UsernameTemplate};
    use norelly_domain::endpoint::{Host, Security, ServerEndpoint};
    use norelly_domain::ports::DiscoveryError;
    use norelly_test_support::fakes::{FakeAutoconfig, FakeProber, FakeSrvResolver};

    use super::*;

    fn address() -> EmailAddress {
        EmailAddress::parse("alice@example.com").unwrap_or_else(|_| panic!("adresse"))
    }

    fn endpoint(host: &str, port: u16, security: Security) -> ServerEndpoint {
        ServerEndpoint::new(
            Host::parse(host).unwrap_or_else(|_| panic!("hôte")),
            port,
            security,
        )
        .unwrap_or_else(|_| panic!("port"))
    }

    fn servers(imap: Option<ServerEndpoint>, smtp: Option<ServerEndpoint>) -> DiscoveredServers {
        let mut out = DiscoveredServers::default();
        for (protocol, e) in [(Protocol::Imap, imap), (Protocol::Smtp, smtp)] {
            if let Some(endpoint) = e {
                out.push(ServerCandidate {
                    protocol,
                    endpoint,
                    username: UsernameTemplate::email_address(),
                });
            }
        }
        out
    }

    const PROVIDER: &str =
        "https://autoconfig.example.com/mail/config-v1.1.xml?emailaddress=alice%40example.com";
    const WELL_KNOWN: &str = "https://example.com/.well-known/autoconfig/mail/config-v1.1.xml";
    const ISPDB: &str = "https://autoconfig.thunderbird.net/v1.1/example.com";

    struct Fakes {
        autoconfig: FakeAutoconfig,
        srv: FakeSrvResolver,
        prober: FakeProber,
    }

    fn fakes() -> Fakes {
        Fakes {
            autoconfig: FakeAutoconfig::default(),
            srv: FakeSrvResolver::default(),
            prober: FakeProber::default(),
        }
    }

    async fn run(f: &Fakes) -> (Discovery, Vec<DiscoverySource>) {
        let use_case = DiscoverSettings::new(f.autoconfig.clone(), f.srv.clone(), f.prober.clone());
        let mut steps = Vec::new();
        let d = use_case.execute(&address(), |s| steps.push(s)).await;
        (d, steps)
    }

    #[tokio::test]
    async fn provider_autoconfig_wins_and_stops_the_search() {
        let f = fakes();
        f.autoconfig.set(
            PROVIDER,
            Ok(Some(servers(
                Some(endpoint("imap.example.com", 993, Security::Tls)),
                Some(endpoint("smtp.example.com", 465, Security::Tls)),
            ))),
        );
        let (d, steps) = run(&f).await;
        assert!(d.is_complete());
        assert_eq!(steps, [DiscoverySource::ProviderAutoconfig]);
        assert_eq!(f.autoconfig.requested(), [PROVIDER]);
        assert!(f.srv.requested().is_empty());
        assert!(f.prober.probed().is_empty());
    }

    #[tokio::test]
    async fn failures_fall_through_to_the_next_source() {
        let f = fakes();
        f.autoconfig
            .set(PROVIDER, Err(DiscoveryError::Network("refusé".into())));
        f.autoconfig
            .set(WELL_KNOWN, Err(DiscoveryError::Invalid("DTD".into())));
        f.autoconfig.set(
            ISPDB,
            Ok(Some(servers(
                Some(endpoint("imap.example.com", 143, Security::StartTls)),
                None,
            ))),
        );
        f.srv.set(
            SrvService::Submissions,
            "example.com",
            Ok(vec![SrvRecord {
                priority: 0,
                weight: 0,
                port: 465,
                target: Host::parse("smtp.example.com").ok(),
            }]),
        );
        let (d, steps) = run(&f).await;
        assert_eq!(
            steps,
            [
                DiscoverySource::ProviderAutoconfig,
                DiscoverySource::MozillaIspdb,
                DiscoverySource::DnsSrv
            ]
        );
        assert_eq!(f.autoconfig.requested(), [PROVIDER, WELL_KNOWN, ISPDB]);
        assert_eq!(
            d.imap().map(|s| s.source),
            Some(DiscoverySource::MozillaIspdb)
        );
        assert_eq!(d.smtp().map(|s| s.source), Some(DiscoverySource::DnsSrv));
        // IMAP déjà trouvé : seuls les services SMTP sont interrogés.
        assert_eq!(
            f.srv.requested(),
            [
                "_submissions._tcp.example.com",
                "_submission._tcp.example.com"
            ]
        );
        assert!(f.prober.probed().is_empty());
    }

    #[tokio::test]
    async fn srv_prefers_implicit_tls_service() {
        let f = fakes();
        let rec = |host: &str, port| SrvRecord {
            priority: 0,
            weight: 0,
            port,
            target: Host::parse(host).ok(),
        };
        f.srv.set(
            SrvService::Imaps,
            "example.com",
            Ok(vec![rec("imap.example.com", 993)]),
        );
        f.srv.set(
            SrvService::Imap,
            "example.com",
            Ok(vec![rec("imap.example.com", 143)]),
        );
        f.srv.set(
            SrvService::Submission,
            "example.com",
            Ok(vec![rec("mx.other.net", 587)]),
        );
        let (d, _) = run(&f).await;
        assert_eq!(d.imap().map(|s| s.candidate.endpoint.port()), Some(993));
        let smtp = d.smtp().unwrap_or_else(|| panic!("smtp"));
        assert_eq!(smtp.candidate.endpoint.security(), Security::StartTls);
        assert!(smtp.requires_confirmation, "hôte hors domaine via DNS");
    }

    #[tokio::test]
    async fn probe_is_last_and_only_for_missing_protocols() {
        let f = fakes();
        f.autoconfig.set(
            ISPDB,
            Ok(Some(servers(
                None,
                Some(endpoint("smtp.example.com", 465, Security::Tls)),
            ))),
        );
        f.prober
            .reachable(endpoint("mail.example.com", 143, Security::StartTls));
        f.prober
            .reachable(endpoint("example.com", 993, Security::Tls));
        let (d, steps) = run(&f).await;
        assert_eq!(steps.last(), Some(&DiscoverySource::Probe));
        assert!(
            f.prober
                .probed()
                .iter()
                .all(|c| c.protocol == Protocol::Imap)
        );
        // TLS implicite préféré à STARTTLS.
        assert_eq!(
            d.imap().map(|s| s.candidate.endpoint.to_string()),
            Some("example.com:993 (TLS)".to_owned())
        );
        assert_eq!(d.imap().map(|s| s.source), Some(DiscoverySource::Probe));
    }

    #[tokio::test]
    async fn nothing_found_gives_an_empty_discovery() {
        let f = fakes();
        let (d, steps) = run(&f).await;
        assert_eq!(steps, DiscoverySource::AUTOMATIC);
        assert!(d.imap().is_none() && d.smtp().is_none());
    }
}
