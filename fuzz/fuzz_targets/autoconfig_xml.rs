//! Fichier autoconfig reçu du réseau : analyse sans panique.
#![no_main]

use libfuzzer_sys::fuzz_target;
use norelly_discovery::autoconfig_xml::parse;
use norelly_domain::endpoint::Host;

fuzz_target!(|xml: &[u8]| {
    if let Ok(domain) = Host::parse("example.com") {
        let _ = parse(xml, &domain);
    }
});
