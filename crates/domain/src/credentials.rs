//! Identifiants de connexion. Le secret n'apparaît jamais dans `Debug`, n'a
//! pas de `Display` et n'est pas sérialisable : il ne peut être lu que par
//! [`Secret::expose`], appel facile à rechercher lors d'une relecture.

use std::fmt;

/// Longueur maximale acceptée pour un nom d'utilisateur.
const MAX_USERNAME_LEN: usize = 512;

/// Efface une zone mémoire au mieux de ce que permet du Rust sûr sans
/// dépendance : `black_box` empêche le compilateur de supprimer l'écriture.
/// Les copies faites par d'éventuelles réallocations ne sont pas couvertes ;
/// les adaptateurs utilisent `zeroize` pour leurs propres tampons.
fn wipe(bytes: &mut [u8]) {
    for b in bytes.iter_mut() {
        *b = 0;
    }
    std::hint::black_box(bytes);
}

/// Mot de passe ou jeton. Masqué dans `Debug`, effacé à la destruction.
#[derive(Clone, Default)]
pub struct Secret(String);

impl Secret {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Accès explicite à la valeur. À n'utiliser qu'au moment de l'envoyer
    /// au serveur ou au trousseau.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        wipe(&mut bytes);
    }
}

/// Octets sensibles (réponse SASL contenant le mot de passe…). Masqués dans
/// `Debug`, effacés à la destruction.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretBytes(Vec<u8>);

impl SecretBytes {
    #[must_use]
    pub fn new(value: Vec<u8>) -> Self {
        Self(value)
    }

    /// Accès explicite aux octets.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretBytes(*** {} octets)", self.0.len())
    }
}

impl Drop for SecretBytes {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

/// Erreur de validation des identifiants.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CredentialsError {
    #[error("le nom d'utilisateur est vide")]
    EmptyUsername,
    #[error("le nom d'utilisateur est trop long")]
    UsernameTooLong,
    #[error("le nom d'utilisateur contient un caractère de contrôle ou invisible")]
    UsernameControlCharacter,
    #[error("le mot de passe est vide")]
    EmptySecret,
    #[error("le mot de passe contient un caractère nul")]
    SecretNulCharacter,
}

/// Nom d'utilisateur : non vide, sans caractère de contrôle (CR, LF et NUL
/// compris, pour empêcher toute injection de commande ou de champ SASL).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Username(String);

impl Username {
    pub fn parse(raw: &str) -> Result<Self, CredentialsError> {
        if raw.is_empty() {
            return Err(CredentialsError::EmptyUsername);
        }
        if raw.len() > MAX_USERNAME_LEN {
            return Err(CredentialsError::UsernameTooLong);
        }
        if raw
            .chars()
            .any(|c| c.is_control() || crate::connection::is_format_char(c))
        {
            return Err(CredentialsError::UsernameControlCharacter);
        }
        Ok(Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Nom d'utilisateur et secret.
///
/// `Debug` masque le secret ; `Display` n'affiche que l'utilisateur.
#[derive(Clone)]
pub struct Credentials {
    username: Username,
    secret: Secret,
}

impl Credentials {
    /// Le secret doit être non vide et sans caractère nul (séparateur du
    /// mécanisme SASL PLAIN).
    pub fn new(username: Username, secret: Secret) -> Result<Self, CredentialsError> {
        if secret.is_empty() {
            return Err(CredentialsError::EmptySecret);
        }
        if secret.expose().contains('\0') {
            return Err(CredentialsError::SecretNulCharacter);
        }
        Ok(Self { username, secret })
    }

    #[must_use]
    pub fn username(&self) -> &Username {
        &self.username
    }

    #[must_use]
    pub fn secret(&self) -> &Secret {
        &self.secret
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username.as_str())
            .field("secret", &self.secret)
            .finish()
    }
}

impl fmt::Display for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (mot de passe masqué)", self.username)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "s3cr3t-P@ss";

    fn credentials() -> Credentials {
        let Ok(user) = Username::parse("alice@example.com") else {
            panic!("utilisateur valide refusé");
        };
        let Ok(c) = Credentials::new(user, Secret::new(PASSWORD.to_owned())) else {
            panic!("identifiants valides refusés");
        };
        c
    }

    #[test]
    fn debug_and_display_never_show_the_secret() {
        let c = credentials();
        for rendered in [
            format!("{c:?}"),
            format!("{c:#?}"),
            format!("{c}"),
            format!("{:?}", c.secret()),
            format!("{:?}", SecretBytes::new(PASSWORD.as_bytes().to_vec())),
        ] {
            assert!(!rendered.contains(PASSWORD), "secret visible : {rendered}");
        }
        assert!(format!("{c:?}").contains("alice@example.com"));
        assert!(format!("{c}").contains("alice@example.com"));
    }

    #[test]
    fn expose_gives_the_value() {
        assert_eq!(credentials().secret().expose(), PASSWORD);
    }

    #[test]
    fn username_rejects_control_characters() {
        assert_eq!(Username::parse(""), Err(CredentialsError::EmptyUsername));
        for raw in [
            "a\r\nb",
            "a\0b",
            "a\tb",
            "\u{7f}",
            "a\u{202E}b",
            "a\u{200B}b",
        ] {
            assert_eq!(
                Username::parse(raw),
                Err(CredentialsError::UsernameControlCharacter)
            );
        }
        assert_eq!(
            Username::parse(&"a".repeat(513)),
            Err(CredentialsError::UsernameTooLong)
        );
        assert!(Username::parse("prénom.nom@exemple.fr").is_ok());
    }

    #[test]
    fn secret_must_be_non_empty_and_without_nul() {
        let Ok(user) = Username::parse("bob") else {
            panic!("utilisateur valide refusé");
        };
        assert!(matches!(
            Credentials::new(user.clone(), Secret::new(String::new())),
            Err(CredentialsError::EmptySecret)
        ));
        assert!(matches!(
            Credentials::new(user.clone(), Secret::new("a\0b".to_owned())),
            Err(CredentialsError::SecretNulCharacter)
        ));
        // CR/LF sont admis dans un mot de passe : les adaptateurs les
        // transmettent en littéral ou en base64, jamais tels quels.
        assert!(Credentials::new(user, Secret::new("a\r\nb".to_owned())).is_ok());
    }

    #[test]
    fn wipe_zeroes_the_buffer() {
        let mut buf = PASSWORD.as_bytes().to_vec();
        wipe(&mut buf);
        assert!(buf.iter().all(|&b| b == 0));
    }
}
