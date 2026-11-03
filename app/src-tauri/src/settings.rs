//! Paramètres de l'application et notifications natives (phase 9).
//!
//! - Les paramètres reçus de l'interface sont revalidés par le domaine
//!   (`Settings::validate`).
//! - Une arrivée de messages n'est relayée à l'interface que si les
//!   notifications sont activées, sans expéditeur ni sujet avec l'option
//!   de confidentialité : la décision est prise ici, en Rust.
//! - Le texte de la notification est composé par l'interface dans la
//!   langue de l'utilisateur (ADR-011), puis revalidé ici (contrôles et
//!   caractères de mise en forme retirés, longueurs bornées) avant d'être
//!   affiché par le système.

use norelly_domain::account::AccountId;
use norelly_domain::mailbox::header_text;
use norelly_domain::notify::NewMail;
use norelly_domain::ports::SettingsStore;
use norelly_domain::settings::{Language, Settings, UpdateCheck};

use tauri::State;

use crate::ipc::{
    CorrespondentDto, FieldDto, IpcError, LanguageDto, SettingsDto, SyncEventDto, UpdateCheckDto,
};
use crate::state::AppState;

/// Longueur maximale du titre d'une notification, en caractères.
pub const MAX_NOTIFICATION_TITLE: usize = 120;
/// Longueur maximale du texte d'une notification, en caractères.
pub const MAX_NOTIFICATION_BODY: usize = 300;

/// Événement à relayer pour des arrivées, selon les paramètres.
#[must_use]
pub fn new_mail_event(
    account: AccountId,
    mail: NewMail,
    settings: &Settings,
) -> Option<SyncEventDto> {
    let mail = settings.notification(mail)?;
    let latest = mail.latest;
    Some(SyncEventDto::NewMail {
        account: u32::try_from(account.get()).ok()?,
        count: mail.count,
        from: latest
            .as_ref()
            .and_then(|l| l.from.as_ref())
            .map(CorrespondentDto::from),
        subject: latest.map(|l| l.subject),
    })
}

/// Paramètres de l'application.
#[tauri::command]
#[specta::specta]
pub async fn get_settings(state: State<'_, AppState>) -> Result<SettingsDto, IpcError> {
    let s = state
        .mail
        .settings
        .settings()
        .await
        .map_err(IpcError::storage)?;
    Ok(settings_dto(&s))
}

/// Enregistre les paramètres (revalidés par le domaine).
#[tauri::command]
#[specta::specta]
pub async fn save_settings(
    state: State<'_, AppState>,
    settings: SettingsDto,
) -> Result<(), IpcError> {
    let settings = settings_of(settings);
    settings
        .validate()
        .map_err(|_| IpcError::invalid(FieldDto::Settings))?;
    state
        .mail
        .settings
        .save_settings(&settings)
        .await
        .map_err(IpcError::storage)
}

/// DTO des paramètres.
#[must_use]
pub fn settings_dto(s: &Settings) -> SettingsDto {
    SettingsDto {
        language: match s.language {
            Language::System => LanguageDto::System,
            Language::French => LanguageDto::Fr,
            Language::English => LanguageDto::En,
        },
        update_check: match s.update_check {
            UpdateCheck::NotAsked => UpdateCheckDto::NotAsked,
            UpdateCheck::Enabled => UpdateCheckDto::Enabled,
            UpdateCheck::Disabled => UpdateCheckDto::Disabled,
        },
        notifications: s.notifications,
        notification_privacy: s.notification_privacy,
        attachment_threshold: s.attachment_threshold,
    }
}

fn settings_of(dto: SettingsDto) -> Settings {
    Settings {
        language: match dto.language {
            LanguageDto::System => Language::System,
            LanguageDto::Fr => Language::French,
            LanguageDto::En => Language::English,
        },
        update_check: match dto.update_check {
            UpdateCheckDto::NotAsked => UpdateCheck::NotAsked,
            UpdateCheckDto::Enabled => UpdateCheck::Enabled,
            UpdateCheckDto::Disabled => UpdateCheck::Disabled,
        },
        notifications: dto.notifications,
        notification_privacy: dto.notification_privacy,
        attachment_threshold: dto.attachment_threshold,
    }
}

/// Affiche une notification du système. Le texte, composé par
/// l'interface dans la langue de l'utilisateur, est revalidé ; l'appel au
/// système se fait hors du fil asynchrone. Une notification impossible
/// (pas de service de notification) est seulement journalisée.
#[tauri::command]
#[specta::specta]
pub async fn show_notification(title: String, body: String) -> Result<(), IpcError> {
    let (title, body) = notification_text(&title, &body);
    // Titre vide (texte entièrement retiré par la revalidation) : rien à
    // afficher.
    if title.is_empty() {
        return Ok(());
    }
    let shown = tauri::async_runtime::spawn_blocking(move || {
        notify_rust::Notification::new()
            .appname("Norelly")
            .summary(&title)
            .body(&body)
            .show()
            .map(|_| ())
    })
    .await;
    match shown {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!(error = %e, "notification impossible"),
        Err(e) => tracing::warn!(error = %e, "notification interrompue"),
    }
    Ok(())
}

/// Titre et texte d'une notification, revalidés.
#[must_use]
pub fn notification_text(title: &str, body: &str) -> (String, String) {
    (
        header_text(title, MAX_NOTIFICATION_TITLE),
        header_text(body, MAX_NOTIFICATION_BODY),
    )
}

#[cfg(test)]
mod tests {
    use norelly_domain::mailbox::Correspondent;
    use norelly_domain::notify::Arrival;

    use super::*;

    fn account() -> AccountId {
        AccountId::new(3).unwrap_or_else(|_| panic!("compte"))
    }

    fn mail() -> NewMail {
        NewMail {
            count: 2,
            latest: Some(Arrival {
                from: Some(Correspondent::new("Bob", "bob@example.com")),
                subject: "Devis".into(),
                date: 1,
            }),
        }
    }

    #[test]
    fn arrivals_follow_the_notification_settings() {
        let mut settings = Settings::default();
        assert!(matches!(
            new_mail_event(account(), mail(), &settings),
            Some(SyncEventDto::NewMail { account: 3, count: 2, from: Some(CorrespondentDto { ref address, .. }), subject: Some(ref s) })
                if address == "bob@example.com" && s == "Devis"
        ));
        settings.notification_privacy = true;
        assert!(matches!(
            new_mail_event(account(), mail(), &settings),
            Some(SyncEventDto::NewMail {
                count: 2,
                from: None,
                subject: None,
                ..
            })
        ));
        settings.notifications = false;
        assert!(new_mail_event(account(), mail(), &settings).is_none());
    }

    #[test]
    fn notification_text_is_cleaned_and_bounded() {
        let (title, body) = notification_text(
            "Bob\r\nX-Injecté: 1\u{202E}",
            &"é".repeat(MAX_NOTIFICATION_BODY + 50),
        );
        assert_eq!(title, "Bob X-Injecté: 1");
        assert_eq!(body.chars().count(), MAX_NOTIFICATION_BODY);
        let (title, _) = notification_text(&"t".repeat(500), "");
        assert_eq!(title.chars().count(), MAX_NOTIFICATION_TITLE);
    }
}
