//! Requête de recherche saisie : analyse sans panique, bornes respectées.
#![no_main]

use libfuzzer_sys::fuzz_target;
use norelly_domain::search::parse_query;

fuzz_target!(|input: &str| {
    let _ = parse_query(input);
});
