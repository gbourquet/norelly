//! Paramètres de l'application (phase 9), revalidés par le domaine.
//!
//! - langue de l'interface : celle du système par défaut (ADR-011) ;
//! - recherche de mises à jour : demandée au premier lancement, « oui »
//!   présélectionné ([`UpdateCheck::PROPOSED`]), modifiable ensuite ;
//! - notifications des nouveaux messages, avec une option de
//!   confidentialité qui retire expéditeur et sujet ;
//! - rapatriement : seuil au-delà duquel les pièces jointes ne sont pas
//!   téléchargées pendant la synchronisation.

use crate::fetch_policy::{DEFAULT_ATTACHMENT_THRESHOLD, FetchPolicy};
use crate::notify::NewMail;

/// Seuil minimal de rapatriement des pièces jointes (64 Kio).
pub const MIN_ATTACHMENT_THRESHOLD: u32 = 64 * 1024;
/// Seuil maximal de rapatriement des pièces jointes (64 Mio).
pub const MAX_ATTACHMENT_THRESHOLD: u32 = 64 * 1024 * 1024;

/// Langue de l'interface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Language {
    /// Celle du système.
    #[default]
    System,
    French,
    English,
}

/// Recherche de mises à jour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UpdateCheck {
    /// Question pas encore posée (premier lancement).
    #[default]
    NotAsked,
    Enabled,
    Disabled,
}

impl UpdateCheck {
    /// Réponse présélectionnée au premier lancement.
    pub const PROPOSED: Self = Self::Enabled;
}

/// Paramètres de l'application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub language: Language,
    pub update_check: UpdateCheck,
    pub notifications: bool,
    pub notification_privacy: bool,
    pub attachment_threshold: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::System,
            update_check: UpdateCheck::NotAsked,
            notifications: true,
            notification_privacy: false,
            attachment_threshold: DEFAULT_ATTACHMENT_THRESHOLD,
        }
    }
}

/// Paramètres refusés.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    #[error("seuil de rapatriement hors bornes")]
    AttachmentThreshold,
}

impl Language {
    /// Code stocké et échangé (`system`, `fr`, `en`).
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::French => "fr",
            Self::English => "en",
        }
    }

    #[must_use]
    pub fn parse(code: &str) -> Option<Self> {
        [Self::System, Self::French, Self::English]
            .into_iter()
            .find(|l| l.code() == code)
    }
}

impl UpdateCheck {
    /// Code stocké et échangé (`not_asked`, `enabled`, `disabled`).
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotAsked => "not_asked",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }

    #[must_use]
    pub fn parse(code: &str) -> Option<Self> {
        [Self::NotAsked, Self::Enabled, Self::Disabled]
            .into_iter()
            .find(|u| u.code() == code)
    }
}

impl Settings {
    /// Vérifie les bornes.
    ///
    /// # Errors
    /// [`SettingsError::AttachmentThreshold`] hors de
    /// [`MIN_ATTACHMENT_THRESHOLD`]..=[`MAX_ATTACHMENT_THRESHOLD`].
    pub fn validate(&self) -> Result<(), SettingsError> {
        if (MIN_ATTACHMENT_THRESHOLD..=MAX_ATTACHMENT_THRESHOLD)
            .contains(&self.attachment_threshold)
        {
            Ok(())
        } else {
            Err(SettingsError::AttachmentThreshold)
        }
    }

    /// Politique de rapatriement correspondante.
    #[must_use]
    pub fn fetch_policy(&self) -> FetchPolicy {
        FetchPolicy {
            attachment_threshold: self.attachment_threshold,
        }
    }

    /// Notification à afficher pour des arrivées : aucune si les
    /// notifications sont désactivées, sans contenu avec l'option de
    /// confidentialité.
    #[must_use]
    pub fn notification(&self, mail: NewMail) -> Option<NewMail> {
        if !self.notifications {
            return None;
        }
        Some(if self.notification_privacy {
            mail.without_content()
        } else {
            mail
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::Arrival;

    fn mail() -> NewMail {
        NewMail {
            count: 2,
            latest: Some(Arrival {
                from: None,
                subject: "Bonjour".into(),
                date: 1,
            }),
        }
    }

    #[test]
    fn defaults_follow_the_system_and_ask_for_updates() {
        let s = Settings::default();
        assert_eq!(s.language, Language::System);
        assert_eq!(s.update_check, UpdateCheck::NotAsked);
        assert_eq!(
            UpdateCheck::PROPOSED,
            UpdateCheck::Enabled,
            "oui présélectionné"
        );
        assert!(s.notifications && !s.notification_privacy);
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn codes_round_trip() {
        for l in [Language::System, Language::French, Language::English] {
            assert_eq!(Language::parse(l.code()), Some(l));
        }
        for u in [
            UpdateCheck::NotAsked,
            UpdateCheck::Enabled,
            UpdateCheck::Disabled,
        ] {
            assert_eq!(UpdateCheck::parse(u.code()), Some(u));
        }
        assert_eq!(Language::parse("de"), None);
        assert_eq!(UpdateCheck::parse(""), None);
    }

    #[test]
    fn attachment_threshold_is_bounded_and_drives_the_fetch_policy() {
        let mut s = Settings {
            attachment_threshold: MIN_ATTACHMENT_THRESHOLD - 1,
            ..Settings::default()
        };
        assert_eq!(s.validate(), Err(SettingsError::AttachmentThreshold));
        s.attachment_threshold = MAX_ATTACHMENT_THRESHOLD + 1;
        assert_eq!(s.validate(), Err(SettingsError::AttachmentThreshold));
        s.attachment_threshold = 5 * 1024 * 1024;
        assert_eq!(s.validate(), Ok(()));
        assert_eq!(s.fetch_policy().attachment_threshold, 5 * 1024 * 1024);
    }

    #[test]
    fn notifications_follow_the_settings() {
        let mut s = Settings::default();
        assert_eq!(s.notification(mail()), Some(mail()));
        s.notification_privacy = true;
        assert_eq!(s.notification(mail()), Some(mail().without_content()));
        s.notifications = false;
        assert_eq!(s.notification(mail()), None);
    }
}
