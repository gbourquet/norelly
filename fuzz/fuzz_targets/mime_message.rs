//! Message reçu quelconque : lecture du corps, relecture pour répondre ou
//! transférer. Aucune panique ; bornes des pièces jointes respectées.
#![no_main]

use libfuzzer_sys::fuzz_target;
use norelly_domain::compose::draft::{MAX_ATTACHMENTS, MAX_ATTACHMENTS_BYTES};
use norelly_domain::ports::MessageComposer;
use norelly_render::compose::MimeComposer;
use norelly_render::{Source, parse_body};

fuzz_target!(|raw: &[u8]| {
    let _ = parse_body(&Source::Full(raw), None);
    let _ = MimeComposer.reply_source(raw);
    if let Ok(found) = MimeComposer.forwarded_attachments(raw) {
        assert!(found.len() <= MAX_ATTACHMENTS);
        let total: usize = found.iter().map(|a| a.bytes.len()).sum();
        assert!(total as u64 <= MAX_ATTACHMENTS_BYTES);
        for a in &found {
            assert!(!a.file_name.contains(['/', '\\', '\0', '\r', '\n']));
        }
    }
});
