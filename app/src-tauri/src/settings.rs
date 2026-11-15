//! Paramètres de l'application et notifications natives.
//!
//! - Les paramètres reçus de l'interface sont revalidés par le domaine
//!   (`Settings::validate`).
//! - Une arrivée de messages n'est relayée à l'interface que si les
//!   notifications sont activées, sans expéditeur ni sujet avec l'option
//!   de confidentialité : la décision est prise ici, en Rust.
//! - L'expéditeur annoncé est nommé d'après le carnet, comme dans la liste
//!   des messages.
//! - Le texte de la notification est composé par l'interface dans la
//!   langue de l'utilisateur (ADR-011), puis revalidé ici (contrôles et
//!   caractères de mise en forme retirés, longueurs bornées) avant d'être
//!   affiché par le système.

use norelly_application::contacts::NameSenders;
use norelly_domain::account::AccountId;
use norelly_domain::mailbox::header_text;
use norelly_domain::notify::NewMail;
use norelly_domain::ports::{ContactDirectory, SettingsStore};
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
/// L'expéditeur est nommé comme dans la liste des messages : d'après le
/// carnet (`directory`) quand son adresse y figure, sinon par le nom de
/// l'en-tête, écarté s'il imite une adresse. Un carnet illisible ne
/// retient pas la notification. Sans contenu (confidentialité), le carnet
/// n'est pas interrogé.
pub async fn new_mail_event<D: ContactDirectory>(
    account: AccountId,
    mail: NewMail,
    settings: &Settings,
    directory: &D,
) -> Option<SyncEventDto> {
    let mail = settings.notification(mail)?;
    let latest = mail.latest;
    let sender = latest.as_ref().and_then(|l| l.from.as_ref());
    let names = NameSenders { directory }.execute(sender).await;
    Some(SyncEventDto::NewMail {
        account: u32::try_from(account.get()).ok()?,
        count: mail.count,
        from: sender.map(|f| CorrespondentDto::named(&names, f)),
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
        mark_seen_delay_secs: s.mark_seen_delay_secs,
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
        mark_seen_delay_secs: dto.mark_seen_delay_secs,
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
    // Échappés après la troncature (aucune entité coupée) : le serveur de
    // notifications freedesktop interprète un balisage (liens, images)
    // dans le texte, qui vient d'un mail.
    (
        escape_markup(&header_text(title, MAX_NOTIFICATION_TITLE)),
        escape_markup(&header_text(body, MAX_NOTIFICATION_BODY)),
    )
}

/// Le serveur de notifications interprète-t-il un balisage ? Oui pour
/// `org.freedesktop.Notifications` (Linux et autres Unix) ; non pour
/// Windows (texte brut) et macOS.
const NOTIFICATION_MARKUP: bool = cfg!(all(unix, not(target_os = "macos")));

/// `&`, `<` et `>` en entités XML, là où le balisage est interprété.
fn escape_markup(text: &str) -> String {
    if !NOTIFICATION_MARKUP {
        return text.to_owned();
    }
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use norelly_domain::mailbox::Correspondent;
    use norelly_domain::notify::Arrival;
    use norelly_domain::ports::{ContactBook, StoreError};
    use norelly_test_support::contacts_contracts::{InMemoryContactBook, sample_contact};

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

    #[tokio::test]
    async fn arrivals_follow_the_notification_settings() {
        let book = InMemoryContactBook::default();
        let mut settings = Settings::default();
        assert!(matches!(
            new_mail_event(account(), mail(), &settings, &book).await,
            Some(SyncEventDto::NewMail { account: 3, count: 2, from: Some(CorrespondentDto { ref address, .. }), subject: Some(ref s) })
                if address == "bob@example.com" && s == "Devis"
        ));
        settings.notification_privacy = true;
        assert!(matches!(
            new_mail_event(account(), mail(), &settings, &book).await,
            Some(SyncEventDto::NewMail {
                count: 2,
                from: None,
                subject: None,
                ..
            })
        ));
        settings.notifications = false;
        assert!(
            new_mail_event(account(), mail(), &settings, &book)
                .await
                .is_none()
        );
    }

    /// L'expéditeur est nommé comme dans la liste des messages : d'après
    /// le carnet quand son adresse y figure.
    #[tokio::test]
    async fn the_sender_is_named_from_the_address_book() {
        let book = InMemoryContactBook::default();
        let bob = book
            .create(&sample_contact("Robert Durand", &["BOB@example.com"]))
            .await
            .unwrap_or_else(|e| panic!("fiche : {e}"));
        let event = new_mail_event(account(), mail(), &Settings::default(), &book).await;
        let Some(SyncEventDto::NewMail {
            from: Some(from), ..
        }) = event
        else {
            panic!("notification attendue : {event:?}");
        };
        assert_eq!(from.name, "Robert Durand");
        assert_eq!(from.address, "bob@example.com");
        assert_eq!(
            from.contact,
            u32::try_from(bob.id().get()).ok(),
            "fiche du carnet"
        );
    }

    /// Hors du carnet, le nom de l'en-tête reste, écarté s'il imite une
    /// adresse.
    #[tokio::test]
    async fn outside_the_book_the_filtered_header_name_stays() {
        let book = InMemoryContactBook::default();
        let spoofed = NewMail {
            count: 1,
            latest: Some(Arrival {
                from: Some(Correspondent::new(
                    "banque@example.org",
                    "pirate@example.net",
                )),
                subject: "Urgent".into(),
                date: 1,
            }),
        };
        let event = new_mail_event(account(), spoofed, &Settings::default(), &book).await;
        let Some(SyncEventDto::NewMail {
            from: Some(from), ..
        }) = event
        else {
            panic!("notification attendue : {event:?}");
        };
        assert_eq!(from.name, "");
        assert_eq!(from.address, "pirate@example.net");
        assert_eq!(from.contact, None);
    }

    /// Carnet dont toute lecture échoue.
    struct BrokenDirectory;

    impl ContactDirectory for BrokenDirectory {
        async fn page(
            &self,
            _: &norelly_domain::contacts::ContactFilter,
            _: Option<norelly_domain::ports::ContactCursor>,
            _: norelly_domain::ports::PageSize,
        ) -> Result<norelly_domain::ports::ContactPage, StoreError> {
            Err(StoreError("carnet illisible".into()))
        }

        async fn get(
            &self,
            _: norelly_domain::contacts::ContactId,
        ) -> Result<Option<norelly_domain::contacts::Contact>, StoreError> {
            Err(StoreError("carnet illisible".into()))
        }

        async fn by_addresses(
            &self,
            _: &[norelly_domain::contacts::AddressKey],
        ) -> Result<Vec<norelly_domain::contacts::Contact>, StoreError> {
            Err(StoreError("carnet illisible".into()))
        }

        async fn contacts(
            &self,
            _: &norelly_domain::contact_export::ContactSelection,
            _: usize,
        ) -> Result<Vec<norelly_domain::contacts::Contact>, StoreError> {
            Err(StoreError("carnet illisible".into()))
        }
    }

    /// Un carnet illisible n'empêche pas la notification : le nom de
    /// l'en-tête est gardé.
    #[tokio::test]
    async fn an_unreadable_book_keeps_the_notification() {
        let event = new_mail_event(account(), mail(), &Settings::default(), &BrokenDirectory).await;
        let Some(SyncEventDto::NewMail {
            from: Some(from), ..
        }) = event
        else {
            panic!("notification attendue : {event:?}");
        };
        assert_eq!(from.name, "Bob");
        assert_eq!(from.contact, None);
    }

    /// Le serveur de notifications
    /// freedesktop interprète un balisage (liens, images) ; le texte reçu
    /// d'un mail est échappé.
    #[test]
    #[cfg(all(unix, not(target_os = "macos")))]
    fn notification_markup_is_escaped() {
        let (title, body) = notification_text(
            "<b>Bob</b> & co",
            r#"<a href="file:///x.desktop">Colis</a> <img src="https://pisteur/x"> &amp;"#,
        );
        assert_eq!(title, "&lt;b&gt;Bob&lt;/b&gt; &amp; co");
        assert!(!body.contains('<') && !body.contains('>'), "{body}");
        assert!(body.starts_with("&lt;a href="), "{body}");
        assert!(body.ends_with("&amp;amp;"), "{body}");
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
