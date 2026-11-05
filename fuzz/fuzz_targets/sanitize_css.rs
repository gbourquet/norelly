//! CSS d'un mail : analyse et réécriture sans panique.
#![no_main]

use libfuzzer_sys::fuzz_target;
use norelly_render::css::sanitize_css;

fuzz_target!(|css: &str| {
    let _ = sanitize_css(css, &|_| None);
});
