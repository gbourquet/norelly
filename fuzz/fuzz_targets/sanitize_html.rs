//! HTML d'un mail : l'assainissement ne panique pas et ne laisse passer
//! aucun contenu actif (`<script>`, `<iframe>`, `<base>`, `<meta>`,
//! `<form>`, …, attribut `on…`, adresse `javascript:` ou `data:`) ni
//! adresse d'image ou de lien inattendue (aucune `remote-img://` quand
//! les images distantes sont bloquées), selon l'oracle
//! `norelly_render::oracle`, qui analyse la sortie comme le tokeniseur
//! HTML du navigateur.
#![no_main]

use std::collections::HashMap;

use libfuzzer_sys::fuzz_target;
use norelly_domain::reading::RemoteContent;
use norelly_render::Protocols;
use norelly_render::oracle::{find_active_content, find_unexpected_address};
use norelly_render::sanitize::{SanitizeOptions, sanitize_html};

fuzz_target!(|html: &str| {
    for (remote, sources) in [
        (RemoteContent::Blocked, &["mailpart://localhost/1/"][..]),
        (
            RemoteContent::Allowed,
            &["mailpart://localhost/1/", "remote-img://localhost/1/"][..],
        ),
    ] {
        let options = SanitizeOptions {
            message: 1,
            remote,
            content_ids: HashMap::new(),
            protocols: Protocols::custom_schemes(),
        };
        let out = sanitize_html(html, &options);
        if remote == RemoteContent::Blocked {
            assert!(out.remote_images.is_empty(), "{:?}", out.remote_images);
        }
        let finding =
            find_active_content(&out.html).or_else(|| find_unexpected_address(&out.html, sources));
        if let Some(finding) = finding {
            panic!("{finding} : {}", out.html);
        }
    }
});
