//! Certificats de test générés avec `rcgen` : CA de test, certificats
//! émis par cette CA (valides, expirés, pour un autre nom) et certificats
//! autosignés.

use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, date_time_ymd,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// Erreur de génération de certificat.
#[derive(Debug, thiserror::Error)]
pub enum CertError {
    #[error("rcgen : {0}")]
    Rcgen(#[from] rcgen::Error),
    #[error("rustls : {0}")]
    Rustls(#[from] rustls::Error),
}

/// Fournisseur cryptographique utilisé partout dans les tests (ring).
#[must_use]
pub fn crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Certificat serveur et sa clé privée.
#[derive(Debug)]
pub struct TestCert {
    /// Chaîne présentée par le serveur (feuille seule).
    pub chain: Vec<CertificateDer<'static>>,
    key_der: Vec<u8>,
    /// Certificat feuille en PEM.
    pub cert_pem: String,
    /// Clé privée en PEM (PKCS#8).
    pub key_pem: String,
}

impl TestCert {
    /// Certificat autosigné pour `names`.
    pub fn self_signed(names: &[&str]) -> Result<Self, CertError> {
        let key = KeyPair::generate()?;
        let cert = server_params(names)?.self_signed(&key)?;
        Ok(Self::from_parts(cert.der().clone(), cert.pem(), &key))
    }

    fn from_parts(der: CertificateDer<'static>, cert_pem: String, key: &KeyPair) -> Self {
        Self {
            chain: vec![der],
            key_der: key.serialize_der(),
            cert_pem,
            key_pem: key.serialize_pem(),
        }
    }

    /// Certificat feuille DER.
    #[must_use]
    pub fn leaf(&self) -> &CertificateDer<'static> {
        // La chaîne est construite avec exactement un élément.
        &self.chain[0]
    }

    /// Clé privée DER.
    #[must_use]
    pub fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_der.clone()))
    }

    /// Configuration de serveur TLS (TLS 1.2 et 1.3) présentant ce
    /// certificat.
    pub fn server_config(&self) -> Result<Arc<rustls::ServerConfig>, CertError> {
        let config = rustls::ServerConfig::builder_with_provider(crypto_provider())
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(self.chain.clone(), self.key())?;
        Ok(Arc::new(config))
    }
}

fn server_params(names: &[&str]) -> Result<CertificateParams, CertError> {
    let mut params =
        CertificateParams::new(names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>())?;
    params
        .distinguished_name
        .push(DnType::CommonName, names.first().copied().unwrap_or("test"));
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.not_before = date_time_ymd(2020, 1, 1);
    params.not_after = date_time_ymd(2100, 1, 1);
    Ok(params)
}

/// Autorité de certification de test.
#[derive(Debug)]
pub struct TestCa {
    issuer: CertifiedIssuer<'static, KeyPair>,
}

impl TestCa {
    pub fn new() -> Result<Self, CertError> {
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(DnType::CommonName, "Norelly Test CA");
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        params.not_before = date_time_ymd(2020, 1, 1);
        params.not_after = date_time_ymd(2100, 1, 1);
        let issuer = CertifiedIssuer::self_signed(params, KeyPair::generate()?)?;
        Ok(Self { issuer })
    }

    /// Certificat racine DER, à ajouter aux racines de confiance du client.
    #[must_use]
    pub fn root(&self) -> CertificateDer<'static> {
        self.issuer.der().clone()
    }

    /// Certificat racine PEM.
    #[must_use]
    pub fn root_pem(&self) -> String {
        self.issuer.pem()
    }

    /// Certificat valide pour `names`, émis par cette CA.
    pub fn issue(&self, names: &[&str]) -> Result<TestCert, CertError> {
        self.issue_with(server_params(names)?)
    }

    /// Certificat expiré depuis 2001 pour `names`, émis par cette CA.
    pub fn issue_expired(&self, names: &[&str]) -> Result<TestCert, CertError> {
        let mut params = server_params(names)?;
        params.not_before = date_time_ymd(2000, 1, 1);
        params.not_after = date_time_ymd(2001, 1, 1);
        self.issue_with(params)
    }

    fn issue_with(&self, params: CertificateParams) -> Result<TestCert, CertError> {
        let key = KeyPair::generate()?;
        let cert = params.signed_by(&key, &self.issuer)?;
        Ok(TestCert::from_parts(cert.der().clone(), cert.pem(), &key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_all_kinds() -> Result<(), CertError> {
        let ca = TestCa::new()?;
        assert!(ca.root_pem().starts_with("-----BEGIN CERTIFICATE-----"));
        for cert in [
            ca.issue(&["localhost"])?,
            ca.issue_expired(&["localhost"])?,
            ca.issue(&["wrong.example"])?,
            TestCert::self_signed(&["localhost"])?,
        ] {
            assert!(cert.key_pem.contains("PRIVATE KEY"));
            assert_ne!(cert.leaf().as_ref(), ca.root().as_ref());
            cert.server_config()?;
        }
        Ok(())
    }
}
