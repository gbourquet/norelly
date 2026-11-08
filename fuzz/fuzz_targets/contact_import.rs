//! Fichier de contacts importé (vCard, CSV, LDIF) : lecture puis plan
//! d'import sans panique, bornes respectées.
#![no_main]

use libfuzzer_sys::fuzz_target;
use norelly_contacts_io::{ContactsIo, IcuNfc};
use norelly_domain::contact_import::{DuplicatePolicy, MAX_IMPORT_CARDS, plan_import};
use norelly_domain::ports::ContactCodec;

fuzz_target!(|bytes: &[u8]| {
    let Ok(decoded) = ContactsIo.decode(bytes) else {
        return;
    };
    assert!(decoded.cards.len() <= MAX_IMPORT_CARDS);
    for policy in [DuplicatePolicy::Skip, DuplicatePolicy::Complete] {
        let plan = plan_import(&[], &decoded.cards, policy, &IcuNfc);
        assert!(plan.create.len() + plan.rejected.len() + plan.duplicates <= decoded.cards.len());
    }
});
