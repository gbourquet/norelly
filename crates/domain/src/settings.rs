//! Paramètres de l'application, revalidés par le domaine.
//!
//! - langue de l'interface : celle du système par défaut (ADR-011) ;
//! - recherche de mises à jour : demandée au premier lancement, « oui »
//!   présélectionné ([`UpdateCheck::PROPOSED`]), modifiable ensuite ;
//! - notifications des nouveaux messages, avec une option de
//!   confidentialité qui retire expéditeur et sujet ;
//! - rapatriement : seuil au-delà duquel les pièces jointes ne sont pas
//!   téléchargées pendant la synchronisation ;
//! - lecture : délai au bout duquel une conversation non lue restée
//!   ouverte est marquée comme lue.

use crate::fetch_policy::{DEFAULT_ATTACHMENT_THRESHOLD, FetchPolicy};
use crate::notify::NewMail;

/// Seuil minimal de rapatriement des pièces jointes (64 Kio).
pub const MIN_ATTACHMENT_THRESHOLD: u32 = 64 * 1024;
/// Seuil maximal de rapatriement des pièces jointes (64 Mio).
pub const MAX_ATTACHMENT_THRESHOLD: u32 = 64 * 1024 * 1024;

/// Délai par défaut du marquage comme lu à l'ouverture (secondes).
pub const DEFAULT_MARK_SEEN_DELAY_SECS: u32 = 5;
/// Délai maximal du marquage comme lu à l'ouverture (secondes).
pub const MAX_MARK_SEEN_DELAY_SECS: u32 = 60;

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

    /// Une recherche de mises à jour est permise : seulement après un
    /// « oui » explicite (jamais avant que la question soit répondue).
    #[must_use]
    pub const fn allows_check(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

/// Intervalle entre deux recherches de mises à jour pendant que
/// l'application tourne (une au lancement, puis une par jour).
pub const UPDATE_CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Une recherche de mises à jour est due : permise, et jamais faite pendant
/// cette exécution (`last` : secondes Unix de la dernière) ou faite il y a
/// au moins [`UPDATE_CHECK_INTERVAL_SECS`]. Une horloge revenue en arrière
/// ne déclenche rien.
#[must_use]
pub const fn update_check_due(check: UpdateCheck, last: Option<u64>, now: u64) -> bool {
    if !check.allows_check() {
        return false;
    }
    match last {
        None => true,
        Some(last) => now >= last && now - last >= UPDATE_CHECK_INTERVAL_SECS,
    }
}

/// Paramètres de l'application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub language: Language,
    pub update_check: UpdateCheck,
    pub notifications: bool,
    pub notification_privacy: bool,
    pub attachment_threshold: u32,
    /// Délai au bout duquel une conversation non lue restée ouverte est
    /// marquée comme lue (secondes ; 0 : à l'ouverture).
    pub mark_seen_delay_secs: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::System,
            update_check: UpdateCheck::NotAsked,
            notifications: true,
            notification_privacy: false,
            attachment_threshold: DEFAULT_ATTACHMENT_THRESHOLD,
            mark_seen_delay_secs: DEFAULT_MARK_SEEN_DELAY_SECS,
        }
    }
}

/// Paramètres refusés.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    #[error("seuil de rapatriement hors bornes")]
    AttachmentThreshold,
    #[error("délai du marquage comme lu hors bornes")]
    MarkSeenDelay,
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
    /// [`MIN_ATTACHMENT_THRESHOLD`]..=[`MAX_ATTACHMENT_THRESHOLD`] ;
    /// [`SettingsError::MarkSeenDelay`] au-delà de
    /// [`MAX_MARK_SEEN_DELAY_SECS`].
    pub fn validate(&self) -> Result<(), SettingsError> {
        if !self.threshold_ok() {
            return Err(SettingsError::AttachmentThreshold);
        }
        if !self.mark_seen_delay_ok() {
            return Err(SettingsError::MarkSeenDelay);
        }
        Ok(())
    }

    fn threshold_ok(&self) -> bool {
        (MIN_ATTACHMENT_THRESHOLD..=MAX_ATTACHMENT_THRESHOLD).contains(&self.attachment_threshold)
    }

    const fn mark_seen_delay_ok(&self) -> bool {
        self.mark_seen_delay_secs <= MAX_MARK_SEEN_DELAY_SECS
    }

    /// Chaque réglage hors bornes remplacé par sa valeur par défaut, les
    /// autres gardés (paramètres relus d'une base modifiée à la main).
    #[must_use]
    pub fn sanitized(self) -> Self {
        let defaults = Self::default();
        Self {
            attachment_threshold: if self.threshold_ok() {
                self.attachment_threshold
            } else {
                defaults.attachment_threshold
            },
            mark_seen_delay_secs: if self.mark_seen_delay_ok() {
                self.mark_seen_delay_secs
            } else {
                defaults.mark_seen_delay_secs
            },
            ..self
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
    fn updates_are_checked_only_after_an_explicit_yes() {
        assert!(UpdateCheck::Enabled.allows_check());
        assert!(!UpdateCheck::NotAsked.allows_check(), "pas de consentement");
        assert!(!UpdateCheck::Disabled.allows_check());
    }

    #[test]
    fn update_checks_are_due_at_start_then_daily() {
        let day = UPDATE_CHECK_INTERVAL_SECS;
        assert!(update_check_due(UpdateCheck::Enabled, None, 1_000));
        assert!(!update_check_due(
            UpdateCheck::Enabled,
            Some(1_000),
            1_000 + day - 1
        ));
        assert!(update_check_due(
            UpdateCheck::Enabled,
            Some(1_000),
            1_000 + day
        ));
        assert!(
            !update_check_due(UpdateCheck::Enabled, Some(5_000), 1_000),
            "horloge en arrière"
        );
        assert!(!update_check_due(UpdateCheck::NotAsked, None, 1_000));
        assert!(!update_check_due(UpdateCheck::Disabled, Some(0), day * 10));
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
    fn mark_seen_delay_defaults_to_five_seconds_and_is_bounded() {
        let mut s = Settings::default();
        assert_eq!(s.mark_seen_delay_secs, DEFAULT_MARK_SEEN_DELAY_SECS);
        assert_eq!(DEFAULT_MARK_SEEN_DELAY_SECS, 5);
        s.mark_seen_delay_secs = 0;
        assert_eq!(s.validate(), Ok(()), "immédiat");
        s.mark_seen_delay_secs = MAX_MARK_SEEN_DELAY_SECS;
        assert_eq!(s.validate(), Ok(()));
        s.mark_seen_delay_secs = MAX_MARK_SEEN_DELAY_SECS + 1;
        assert_eq!(s.validate(), Err(SettingsError::MarkSeenDelay));
    }

    #[test]
    fn sanitized_resets_each_out_of_bounds_setting_to_its_default() {
        let chosen = Settings {
            notifications: false,
            attachment_threshold: 8 * 1024 * 1024,
            mark_seen_delay_secs: 12,
            ..Settings::default()
        };
        assert_eq!(chosen.sanitized(), chosen, "valide : inchangé");
        let all_wrong = Settings {
            attachment_threshold: 1,
            mark_seen_delay_secs: MAX_MARK_SEEN_DELAY_SECS + 1,
            ..chosen
        };
        let fixed = all_wrong.sanitized();
        assert_eq!(fixed.validate(), Ok(()));
        assert_eq!(
            fixed,
            Settings {
                attachment_threshold: DEFAULT_ATTACHMENT_THRESHOLD,
                mark_seen_delay_secs: DEFAULT_MARK_SEEN_DELAY_SECS,
                ..chosen
            },
            "les autres réglages sont gardés"
        );
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
