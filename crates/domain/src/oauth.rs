//! Connexion OAuth 2.0 (« Se connecter avec Microsoft ») : configuration
//! des fournisseurs, demande d'autorisation (code + PKCE, RFC 7636), retour
//! du navigateur, réponse SASL XOAUTH2 et règles d'expiration des jetons.
//!
//! Aucune E/S : l'aléa (`state`, vérificateur PKCE), le haché SHA-256 du
//! vérificateur et l'heure sont fournis par l'appelant.

use std::fmt;

use crate::address::EmailAddress;
use crate::credentials::{Secret, SecretBytes};

/// Marge avant l'expiration d'un jeton d'accès : un jeton qui expire dans
/// moins de temps est rafraîchi avant d'être présenté au serveur.
pub const ACCESS_TOKEN_MARGIN_SECS: u64 = 120;

/// Fournisseur OAuth connu de Norelly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OAuthProvider {
    /// Comptes Microsoft personnels (Outlook.com, Hotmail, Live).
    Microsoft,
}

/// Configuration d'un fournisseur : points d'accès, identifiant client
/// (public : une application de bureau ne garde pas de secret), scopes et
/// serveurs de messagerie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthConfig {
    pub authorize_url: &'static str,
    pub token_url: &'static str,
    pub client_id: &'static str,
    pub scopes: &'static [&'static str],
    /// Scopes OpenID demandés à la seule connexion interactive : l'identité
    /// du compte choisi dans le navigateur revient dans l'`id_token`.
    pub identity_scopes: &'static [&'static str],
    pub imap_host: &'static str,
    pub smtp_host: &'static str,
}

const MICROSOFT: OAuthConfig = OAuthConfig {
    authorize_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize",
    token_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
    client_id: "10a28ed0-5f58-4684-bc78-f4a2c6c1a0eb",
    scopes: &[
        "https://outlook.office.com/IMAP.AccessAsUser.All",
        "https://outlook.office.com/SMTP.Send",
        "offline_access",
    ],
    identity_scopes: &["openid", "email", "profile"],
    imap_host: "outlook.office365.com",
    smtp_host: "smtp-mail.outlook.com",
};

/// Domaines des comptes Microsoft personnels.
const MICROSOFT_DOMAINS: [&str; 12] = [
    "outlook.com",
    "outlook.fr",
    "outlook.be",
    "hotmail.com",
    "hotmail.fr",
    "hotmail.be",
    "hotmail.co.uk",
    "live.com",
    "live.fr",
    "live.be",
    "msn.com",
    "passport.com",
];

impl OAuthProvider {
    /// Nom stocké (base locale) et transmis à l'interface.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Microsoft => "microsoft",
        }
    }

    /// Inverse de [`Self::as_str`].
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "microsoft" => Some(Self::Microsoft),
            _ => None,
        }
    }

    #[must_use]
    pub fn config(self) -> &'static OAuthConfig {
        match self {
            Self::Microsoft => &MICROSOFT,
        }
    }

    /// Fournisseur OAuth d'une adresse, d'après son domaine.
    #[must_use]
    pub fn for_address(address: &EmailAddress) -> Option<Self> {
        let domain = address.domain().as_str().to_ascii_lowercase();
        MICROSOFT_DOMAINS
            .contains(&domain.as_str())
            .then_some(Self::Microsoft)
    }
}

impl fmt::Display for OAuthProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Paramètres d'une demande d'autorisation. `code_challenge` est le
/// SHA-256 du vérificateur PKCE, encodé en base64url sans remplissage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationRequest<'a> {
    pub redirect_uri: &'a str,
    pub state: &'a str,
    pub code_challenge: &'a str,
    pub login_hint: &'a EmailAddress,
}

/// URL à ouvrir dans le navigateur du système.
#[must_use]
pub fn authorization_url(config: &OAuthConfig, request: &AuthorizationRequest<'_>) -> String {
    let params = [
        ("response_type", "code"),
        ("client_id", config.client_id),
        ("redirect_uri", request.redirect_uri),
        ("scope", &authorization_scopes(config)),
        ("state", request.state),
        ("code_challenge", request.code_challenge),
        ("code_challenge_method", "S256"),
        ("login_hint", &request.login_hint.to_string()),
        ("prompt", "select_account"),
    ];
    format!("{}?{}", config.authorize_url, encode_pairs(&params))
}

/// Scopes de la connexion interactive : messagerie, puis identité.
fn authorization_scopes(config: &OAuthConfig) -> String {
    config
        .scopes
        .iter()
        .chain(config.identity_scopes)
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

fn encode_pairs(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={}", form_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Corps (formulaire) de l'échange du code contre les jetons.
#[must_use]
pub fn code_exchange_form(
    config: &OAuthConfig,
    code: &Secret,
    redirect_uri: &str,
    code_verifier: &Secret,
) -> Secret {
    Secret::new(encode_pairs(&[
        ("grant_type", "authorization_code"),
        ("client_id", config.client_id),
        ("code", code.expose()),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier.expose()),
    ]))
}

/// Corps (formulaire) du rafraîchissement d'un jeton d'accès.
#[must_use]
pub fn refresh_form(config: &OAuthConfig, refresh_token: &Secret) -> Secret {
    Secret::new(encode_pairs(&[
        ("grant_type", "refresh_token"),
        ("client_id", config.client_id),
        ("refresh_token", refresh_token.expose()),
        ("scope", &config.scopes.join(" ")),
    ]))
}

/// Encodage `application/x-www-form-urlencoded` d'une valeur.
fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(b));
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Décodage d'une valeur de requête (`%XX`, `+` pour l'espace). `None` si
/// une séquence est invalide ou si le résultat n'est pas de l'UTF-8.
fn form_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = value.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// Refus du retour du navigateur.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallbackError {
    /// `state` absent ou différent : retour forgé ou d'une autre demande.
    #[error("retour d'autorisation d'une autre demande")]
    StateMismatch,
    /// L'utilisateur a refusé, ou le fournisseur signale une erreur.
    #[error("autorisation refusée : {0}")]
    Denied(String),
    #[error("retour d'autorisation sans code")]
    MissingCode,
    #[error("retour d'autorisation illisible")]
    Malformed,
}

/// Code d'autorisation extrait de la cible de la requête reçue sur la
/// redirection locale (`/?code=…&state=…`). Le `state` est vérifié avant
/// tout le reste ; une erreur du fournisseur n'est crue qu'avec le bon
/// `state`.
pub fn parse_callback(target: &str, expected_state: &str) -> Result<Secret, CallbackError> {
    let query = target.strip_prefix("/?").ok_or(CallbackError::Malformed)?;
    let (mut code, mut state, mut error) = (None, None, None);
    for pair in query.split('&') {
        let (key, raw) = pair.split_once('=').unwrap_or((pair, ""));
        let value = form_decode(raw).ok_or(CallbackError::Malformed)?;
        let slot = match key {
            "code" => &mut code,
            "state" => &mut state,
            "error" => &mut error,
            _ => continue,
        };
        if slot.replace(value).is_some() {
            return Err(CallbackError::Malformed);
        }
    }
    match state {
        Some(s) if !expected_state.is_empty() && s == expected_state => {}
        _ => return Err(CallbackError::StateMismatch),
    }
    if let Some(error) = error {
        return Err(CallbackError::Denied(error));
    }
    code.filter(|c| !c.is_empty())
        .map(Secret::new)
        .ok_or(CallbackError::MissingCode)
}

/// Réponse initiale SASL XOAUTH2 :
/// `user=<utilisateur>\x01auth=Bearer <jeton>\x01\x01`.
#[must_use]
pub fn xoauth2_initial_response(username: &str, access_token: &Secret) -> SecretBytes {
    let mut out = Vec::with_capacity(username.len() + access_token.expose().len() + 20);
    out.extend_from_slice(b"user=");
    out.extend_from_slice(username.as_bytes());
    out.extend_from_slice(b"\x01auth=Bearer ");
    out.extend_from_slice(access_token.expose().as_bytes());
    out.extend_from_slice(b"\x01\x01");
    SecretBytes::new(out)
}

/// Un jeton de `provider` ne part que vers ses serveurs IMAP et SMTP, en
/// TLS ou STARTTLS : un serveur tiers pourrait le rejouer chez le
/// fournisseur, et le clair l'exposerait sur le réseau.
pub fn token_destination_allowed(
    provider: OAuthProvider,
    endpoint: &crate::endpoint::ServerEndpoint,
) -> Result<(), crate::auth::AuthError> {
    let config = provider.config();
    let host = endpoint.host().as_str();
    let known = [config.imap_host, config.smtp_host]
        .iter()
        .any(|h| h.eq_ignore_ascii_case(host));
    if known && endpoint.security() != crate::endpoint::Security::Plain {
        Ok(())
    } else {
        Err(crate::auth::AuthError::TokenDestinationRefused {
            host: host.to_owned(),
        })
    }
}

/// Origine du jeton d'accès d'un compte OAuth : fournisseur et clé du
/// jeton de rafraîchissement dans le trousseau.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OAuthSource {
    pub provider: OAuthProvider,
    pub refresh_key: crate::ports::SecretKey,
}

/// Jeton d'accès et son instant d'expiration (secondes, horloge choisie
/// par l'appelant).
#[derive(Debug, Clone)]
pub struct AccessToken {
    pub token: Secret,
    pub expires_at: u64,
}

impl AccessToken {
    /// Vrai si le jeton peut encore être présenté à `now`, marge comprise.
    #[must_use]
    pub fn is_fresh(&self, now: u64) -> bool {
        is_fresh(self.expires_at, now)
    }
}

/// Vrai si un jeton qui expire à `expires_at` peut encore être présenté à
/// `now`, marge comprise.
fn is_fresh(expires_at: u64, now: u64) -> bool {
    now.saturating_add(ACCESS_TOKEN_MARGIN_SECS) < expires_at
}

/// Durée pendant laquelle une connexion chez le fournisseur reste
/// réutilisable après un ajout de compte échoué (serveur muet, réseau
/// instable) : « Réessayer » ne repasse pas par le navigateur.
pub const PENDING_AUTHORIZATION_MAX_AGE_SECS: u64 = 600;

/// Connexion chez le fournisseur obtenue pour un ajout de compte qui n'a
/// pas abouti. Les jetons eux-mêmes restent chez l'appelant, en mémoire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAuthorization {
    pub provider: OAuthProvider,
    pub address: EmailAddress,
    /// Instant d'obtention (secondes, horloge choisie par l'appelant).
    pub obtained_at: u64,
    /// Durée de validité du jeton d'accès, en secondes.
    pub expires_in: u64,
}

impl PendingAuthorization {
    /// Vrai si la connexion peut resservir à `now` pour ajouter `address`
    /// chez `provider` : même fournisseur et même adresse, obtenue depuis
    /// moins de [`PENDING_AUTHORIZATION_MAX_AGE_SECS`], jeton d'accès encore
    /// valable (marge comprise).
    #[must_use]
    pub fn usable_for(&self, provider: OAuthProvider, address: &EmailAddress, now: u64) -> bool {
        self.provider == provider
            && self.address == *address
            // Horloge revenue en arrière : âge inconnu.
            && now >= self.obtained_at
            && now - self.obtained_at < PENDING_AUTHORIZATION_MAX_AGE_SECS
            && is_fresh(self.obtained_at.saturating_add(self.expires_in), now)
    }
}

/// Identité du compte connecté, lue dans l'`id_token` reçu du point
/// d'accès des jetons.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OAuthIdentity {
    pub preferred_username: Option<String>,
    pub email: Option<String>,
}

/// Refuse les jetons d'un autre compte que celui de `address` : erreur de
/// compte dans la page du fournisseur, ou code d'autorisation d'un autre
/// compte injecté dans la redirection locale. Le compte est le bon si son
/// identifiant de connexion (`preferred_username`) ou son adresse
/// (`email`) est l'adresse, sans tenir compte de la casse. Sans identité,
/// rien ne peut être vérifié : refus.
pub fn verify_identity(
    address: &EmailAddress,
    identity: Option<&OAuthIdentity>,
) -> Result<(), TokenError> {
    let identity =
        identity.ok_or_else(|| TokenError::Protocol("identité du compte absente".to_owned()))?;
    let expected = address.to_string();
    let matches = [&identity.preferred_username, &identity.email]
        .into_iter()
        .flatten()
        .any(|claim| claim.trim().eq_ignore_ascii_case(&expected));
    if matches {
        Ok(())
    } else {
        Err(TokenError::WrongAccount)
    }
}

/// Refuse une connexion interactive sans jeton de rafraîchissement : sans
/// lui, le compte ne pourrait plus se connecter une fois le jeton d'accès
/// expiré.
pub fn require_offline_access(refresh_token: Option<&Secret>) -> Result<(), TokenError> {
    refresh_token
        .map(|_| ())
        .ok_or(TokenError::OfflineAccessRefused)
}

/// Échec d'obtention ou de rafraîchissement d'un jeton. Ne contient jamais
/// de jeton.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    /// Le jeton de rafraîchissement n'est plus accepté (révoqué, expiré,
    /// mot de passe changé) : l'utilisateur doit se reconnecter.
    #[error("reconnexion nécessaire")]
    ReauthRequired,
    /// Le compte choisi dans le navigateur n'est pas celui de l'adresse.
    #[error("compte connecté différent de l'adresse")]
    WrongAccount,
    /// Le fournisseur n'a pas délivré de jeton de rafraîchissement (accès
    /// hors ligne refusé dans sa page) : le compte cesserait de fonctionner
    /// à l'expiration du jeton d'accès.
    #[error("accès hors ligne refusé")]
    OfflineAccessRefused,
    /// L'utilisateur a refusé ou abandonné la connexion.
    #[error("connexion annulée : {0}")]
    Cancelled(String),
    #[error("erreur réseau : {0}")]
    Network(String),
    #[error("réponse du fournisseur invalide : {0}")]
    Protocol(String),
}

/// Classe une réponse d'erreur du point d'accès des jetons (RFC 6749,
/// section 5.2) : `invalid_grant` impose une reconnexion.
#[must_use]
pub fn classify_token_error(status: u16, error_code: Option<&str>) -> TokenError {
    match error_code {
        Some("invalid_grant") => TokenError::ReauthRequired,
        Some(code) => TokenError::Protocol(code.to_owned()),
        None => TokenError::Network(format!("HTTP {status}")),
    }
}

/// Vrai si le serveur IMAP de Microsoft a accepté le jeton sans pouvoir
/// rattacher la session à la boîte (« User is authenticated but not
/// connected ») : défaillance intermittente du côté de Microsoft, ou accès
/// IMAP désactivé sur la boîte.
#[must_use]
pub fn is_mailbox_not_connected(server_message: &str) -> bool {
    server_message
        .to_ascii_lowercase()
        .contains("authenticated but not connected")
}

/// Vrai si un refus SMTP indique que l'envoi authentifié par OAuth est
/// désactivé sur la boîte (Outlook.com : `535 5.7.139`), alors que l'IMAP
/// fonctionne.
#[must_use]
pub fn is_smtp_oauth_disabled(server_message: &str) -> bool {
    server_message.contains("5.7.139")
}

#[cfg(test)]
mod tests;
