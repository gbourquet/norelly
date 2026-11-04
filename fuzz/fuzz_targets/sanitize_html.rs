//! HTML d'un mail : l'assainissement ne panique pas et ne laisse passer
//! aucune balise `<script` (le texte affiché, lui, est échappé).
#![no_main]

use std::collections::HashMap;

use libfuzzer_sys::fuzz_target;
use norelly_domain::reading::RemoteContent;
use norelly_render::Protocols;
use norelly_render::sanitize::{SanitizeOptions, sanitize_html};

fuzz_target!(|html: &str| {
    for remote in [RemoteContent::Blocked, RemoteContent::Allowed] {
        let options = SanitizeOptions {
            message: 1,
            remote,
            content_ids: HashMap::new(),
            protocols: Protocols::custom_schemes(),
        };
        let out = sanitize_html(html, &options).html.to_ascii_lowercase();
        assert!(!out.contains("<script"), "{out}");
    }
});
