//! Empreinte SHA-256 de l'agent (`src/agent.js`), calculée à la
//! compilation : la CSP du rendu n'autorise que ce script
//! (`script-src 'sha256-…'`, ADR-004).

use sha2::{Digest, Sha256};

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let index = ((n >> (18 - 6 * i)) & 0x3f) as usize;
                out.push(char::from(BASE64[index]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn main() {
    println!("cargo::rerun-if-changed=src/agent.js");
    let agent = match std::fs::read("src/agent.js") {
        Ok(a) => a,
        Err(e) => panic!("lecture de src/agent.js : {e}"),
    };
    let hash = base64(&Sha256::digest(&agent));
    let out = match std::env::var("OUT_DIR") {
        Ok(o) => std::path::PathBuf::from(o),
        Err(e) => panic!("OUT_DIR : {e}"),
    };
    let code = format!(
        "/// Empreinte SHA-256 (base64) de l'agent.\npub const AGENT_SHA256: &str = \"{hash}\";\n"
    );
    if let Err(e) = std::fs::write(out.join("agent_hash.rs"), code) {
        panic!("écriture de agent_hash.rs : {e}");
    }
}
