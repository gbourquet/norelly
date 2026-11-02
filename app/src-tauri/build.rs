//! Script de build du shell Tauri.
//!
//! Sous Windows (MSVC), tauri-build n'embarque le manifeste applicatif que
//! dans les exécutables (`rustc-link-arg-bins`) : les binaires de tests liés
//! à Tauri échouent alors au chargement (0xc0000139). Le manifeste de
//! tauri-build est donc désactivé et le même manifeste
//! (`windows-app-manifest.xml`) est passé à l'éditeur de liens pour toutes
//! les cibles.

fn main() {
    let msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let attributes = if msvc {
        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
    } else {
        tauri_build::Attributes::new()
    };
    tauri_build::try_build(attributes).unwrap_or_else(|e| panic!("tauri-build : {e:#}"));
}
