//! Vérifie la signature minisign d'un installateur avec la clé publique
//! embarquée dans l'application (`tauri.updater.conf.json`), comme le fera
//! le module de mise à jour : une clé du secret de publication qui ne
//! correspond pas à celle du dépôt arrête la publication au lieu de
//! produire des mises à jour que tous les postes refuseraient.
//!
//! `cargo run --locked -p norelly-verify-update-signature -- <tauri.updater.conf.json> <installateur> <installateur.sig>`

use std::process::ExitCode;

/// Clé publique de la configuration : base64 d'un fichier de clé minisign.
fn embedded_key(config: &str) -> Result<minisign_verify::PublicKey, String> {
    let config: serde_json::Value = serde_json::from_str(config).map_err(|e| e.to_string())?;
    let pubkey = config["plugins"]["updater"]["pubkey"]
        .as_str()
        .ok_or("clé publique absente de la configuration")?;
    let text = decode_base64(pubkey)?;
    let key = minisign_verify::PublicKey::decode(&text).map_err(|e| e.to_string())?;
    // Le commentaire (non signé) cite l'identifiant de la clé : il doit être
    // celui des octets de la clé, pour qu'une rotation se relise sans
    // ambiguïté.
    let mut lines = text.lines();
    let named = lines
        .next()
        .and_then(|l| l.strip_prefix("untrusted comment: minisign public key: "))
        .ok_or("commentaire de la clé publique absent")?;
    let actual = lines
        .next()
        .and_then(|l| {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(l.trim())
                .ok()
        })
        .and_then(|bytes| bytes.get(2..10).map(<[u8]>::to_vec))
        .map(|id| {
            id.iter()
                .rev()
                .map(|b| format!("{b:02X}"))
                .collect::<String>()
        })
        .ok_or("identifiant de la clé publique illisible")?;
    if named.trim() != actual {
        return Err(format!(
            "le commentaire cite la clé {named}, la clé est {actual}"
        ));
    }
    Ok(key)
}

/// Texte encodé en base64 (format des clés et signatures de Tauri).
fn decode_base64(encoded: &str) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// Signature `.sig` de `tauri signer sign` (base64 d'un fichier de
/// signature minisign) vérifiée sur `data` ; seul le format préhaché
/// actuel est accepté.
fn verify(key: &minisign_verify::PublicKey, data: &[u8], sig: &str) -> Result<(), String> {
    let sig =
        minisign_verify::Signature::decode(&decode_base64(sig)?).map_err(|e| e.to_string())?;
    key.verify(data, &sig, false).map_err(|e| e.to_string())
}

fn run(args: &[String]) -> Result<(), String> {
    let [config, installer, sig] = args else {
        return Err("usage : verify-update-signature <config> <installateur> <signature>".into());
    };
    let read = |path: &str| std::fs::read(path).map_err(|e| format!("{path} : {e}"));
    let config = String::from_utf8(read(config)?).map_err(|e| e.to_string())?;
    let sig = String::from_utf8(read(sig)?).map_err(|e| e.to_string())?;
    verify(&embedded_key(&config)?, &read(installer)?, &sig)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => {
            println!("signature valide pour la clé publique embarquée");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("signature refusée : {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{FIXTURES}/{name}")).unwrap_or_default()
    }

    fn text(name: &str) -> String {
        String::from_utf8(fixture(name)).unwrap_or_default()
    }

    fn config(pubkey: &str) -> String {
        format!(
            r#"{{"plugins":{{"updater":{{"pubkey":"{}"}}}}}}"#,
            pubkey.trim()
        )
    }

    #[test]
    fn a_signature_of_the_embedded_key_is_accepted() {
        let key = embedded_key(&config(&text("key-a.pub")));
        assert!(key.is_ok(), "{key:?}");
        let key = key.unwrap_or_else(|_| unreachable!());
        assert_eq!(
            verify(&key, &fixture("installer.bin"), &text("installer.bin.sig")),
            Ok(())
        );
    }

    #[test]
    fn a_signature_of_another_key_or_other_data_is_refused() {
        let Ok(other) = embedded_key(&config(&text("key-b.pub"))) else {
            panic!("clé b illisible");
        };
        assert!(
            verify(
                &other,
                &fixture("installer.bin"),
                &text("installer.bin.sig")
            )
            .is_err()
        );
        let Ok(key) = embedded_key(&config(&text("key-a.pub"))) else {
            panic!("clé a illisible");
        };
        let mut altered = fixture("installer.bin");
        altered.push(b'!');
        assert!(verify(&key, &altered, &text("installer.bin.sig")).is_err());
        assert!(verify(&key, &fixture("installer.bin"), "pas une signature").is_err());
    }

    #[test]
    fn a_malformed_embedded_key_is_refused() {
        for bad in ["", "pas du base64 !", "dGV4dGUgcXVlbGNvbnF1ZQ=="] {
            assert!(embedded_key(&config(bad)).is_err(), "{bad}");
        }
        assert!(embedded_key("{}").is_err());
    }

    #[test]
    fn a_key_whose_comment_names_another_key_is_refused() {
        use base64::Engine as _;
        let engine = base64::engine::general_purpose::STANDARD;
        let Ok(text) =
            String::from_utf8(engine.decode(text("key-a.pub").trim()).unwrap_or_default())
        else {
            panic!("clé a illisible");
        };
        assert!(embedded_key(&config(&engine.encode(&text))).is_ok());
        let Some((_, id)) = text.lines().next().and_then(|l| l.rsplit_once(": ")) else {
            panic!("commentaire absent");
        };
        let renamed = text.replacen(id, "0123456789ABCDEF", 1);
        assert!(embedded_key(&config(&engine.encode(renamed))).is_err());
    }

    /// La clé publique livrée avec l'application se lit : une rotation mal
    /// collée échoue ici, pas chez l'utilisateur.
    #[test]
    fn the_shipped_key_is_a_valid_minisign_key() {
        let shipped = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../app/src-tauri/tauri.updater.conf.json"
        ))
        .unwrap_or_default();
        assert!(embedded_key(&shipped).is_ok());
    }
}
