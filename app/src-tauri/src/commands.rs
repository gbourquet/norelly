//! Commandes IPC. Chaque entrée de l'interface est revalidée ici par les
//! constructeurs du domaine ; aucune décision de sécurité n'est déléguée
//! au TypeScript.

use norelly_application::accounts::{
    AddAccount, AddAccountOutcome, AddAccountRequest, RemoveAccount, trust_certificate as trust,
};
use norelly_application::accounts::{
    AddOAuthAccount, AddOAuthAccountRequest, ReconnectError, reconnect_oauth_account,
};
use norelly_application::test_connection::TestConnectionOutcome;
use norelly_domain::account::{AccountId, DisplayName, ServerSettings};
use norelly_domain::address::EmailAddress;
use norelly_domain::credentials::{Secret, Username};
use norelly_domain::endpoint::{Host, Protocol, ServerEndpoint};
use norelly_domain::ports::AccountRepository;
use norelly_domain::security::{PlaintextConsent, PlaintextPolicy};
use tauri::State;
use tauri::ipc::Channel;

use crate::ipc::{
    AccountDto, AddAccountInput, AddAccountResultDto, DiscoveryDto, DiscoverySourceDto,
    ErrorCodeDto, FieldDto, IpcError, SecretsBackendDto, ServerDto,
};
use crate::ipc::{AddOAuthAccountInput, OAuthProviderDto};
use crate::state::AppState;

/// Comptes enregistrés.
#[tauri::command]
#[specta::specta]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<AccountDto>, IpcError> {
    let accounts = state.accounts.list().await.map_err(IpcError::storage)?;
    accounts.iter().map(AccountDto::try_from).collect()
}

/// Découvre les serveurs d'une adresse ; `on_progress` reçoit chaque
/// source essayée.
#[tauri::command]
#[specta::specta]
pub async fn discover_settings(
    state: State<'_, AppState>,
    address: String,
    on_progress: Channel<DiscoverySourceDto>,
) -> Result<DiscoveryDto, IpcError> {
    let address =
        EmailAddress::parse(&address).map_err(|_| IpcError::invalid(FieldDto::Address))?;
    let discovery = state
        .discover
        .execute(&address, |source| {
            // Interface fermée : la progression n'est plus affichée.
            let _ = on_progress.send(source.into());
        })
        .await;
    for found in [discovery.imap(), discovery.smtp()].into_iter().flatten() {
        if found.requires_confirmation {
            state.remember_unconfirmed(found.candidate.endpoint.host());
        }
    }
    Ok(DiscoveryDto::from(&discovery))
}

fn server_settings(protocol: Protocol, dto: &ServerDto) -> Result<ServerSettings, IpcError> {
    let host = Host::parse(dto.host.trim()).map_err(|_| IpcError::invalid(FieldDto::Host))?;
    let endpoint = ServerEndpoint::new(host, dto.port, dto.security.into())
        .map_err(|_| IpcError::invalid(FieldDto::Port))?;
    let username =
        Username::parse(dto.username.trim()).map_err(|_| IpcError::invalid(FieldDto::Username))?;
    Ok(ServerSettings {
        protocol,
        endpoint,
        username,
    })
}

fn add_request(input: AddAccountInput) -> Result<AddAccountRequest, IpcError> {
    let address =
        EmailAddress::parse(&input.address).map_err(|_| IpcError::invalid(FieldDto::Address))?;
    let display_name = match input.display_name.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(name) => {
            Some(DisplayName::parse(name).map_err(|_| IpcError::invalid(FieldDto::DisplayName))?)
        }
    };
    let servers = match (&input.imap, &input.smtp) {
        (Some(imap), Some(smtp)) => Some((
            server_settings(Protocol::Imap, imap)?,
            server_settings(Protocol::Smtp, smtp)?,
        )),
        (None, None) => None,
        _ => {
            return Err(IpcError::invalid(FieldDto::Servers));
        }
    };
    let mut plaintext = PlaintextPolicy::deny_all();
    for consent in &input.plaintext_consents {
        plaintext = plaintext.with_consent(PlaintextConsent {
            host: Host::parse(&consent.host).map_err(|_| IpcError::invalid(FieldDto::Host))?,
            port: consent.port,
        });
    }
    Ok(AddAccountRequest {
        address,
        display_name,
        password: Secret::new(input.password),
        servers,
        plaintext,
    })
}

/// Ajoute un compte : test des connexions, trousseau, persistance.
#[tauri::command]
#[specta::specta]
pub async fn add_account(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: AddAccountInput,
) -> Result<AddAccountResultDto, IpcError> {
    let confirmed = input.confirmed_hosts.clone();
    let request = add_request(input)?;
    if !state.secrets.usable().await {
        return Ok(AddAccountResultDto::SecretStorageConsentRequired {
            path: state.secrets_file().display().to_string(),
        });
    }
    if let Some((imap, smtp)) = &request.servers {
        let hosts = [imap.endpoint.host(), smtp.endpoint.host()];
        if let Some(host) = state.check_confirmations(&hosts, &confirmed) {
            return Ok(AddAccountResultDto::ConfirmationRequired {
                host: host.to_string(),
            });
        }
    }
    let outcome = AddAccount {
        test: &state.test,
        discover: &state.discover,
        secrets: &state.secrets,
        accounts: &state.accounts,
    }
    .execute(request)
    .await;
    if let AddAccountOutcome::ConnectionFailed {
        outcome: TestConnectionOutcome::Certificate(rejection),
        ..
    } = &outcome
        && rejection.can_be_excepted()
    {
        state.remember_rejection(rejection.clone());
    }
    if let AddAccountOutcome::Added(account) = &outcome
        && let Err(e) = state
            .mail
            .start(&app, account, &state.secrets, &state.exceptions)
            .await
    {
        tracing::warn!(error = %e, "synchronisation non démarrée");
    }
    Ok(AddAccountResultDto::from_outcome(&outcome))
}

/// Fournisseur OAuth d'une adresse (décidé par le domaine) : l'interface
/// propose alors « Se connecter avec … ».
#[tauri::command]
#[specta::specta]
pub fn oauth_provider_for(address: String) -> Option<OAuthProviderDto> {
    let address = EmailAddress::parse(&address).ok()?;
    norelly_domain::oauth::OAuthProvider::for_address(&address).map(Into::into)
}

/// Ajoute un compte par « Se connecter avec … » : navigateur du système,
/// test des connexions avec le jeton, persistance, jeton de
/// rafraîchissement dans le trousseau.
#[tauri::command]
#[specta::specta]
pub async fn add_oauth_account(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: AddOAuthAccountInput,
) -> Result<AddAccountResultDto, IpcError> {
    let address =
        EmailAddress::parse(&input.address).map_err(|_| IpcError::invalid(FieldDto::Address))?;
    let display_name = match input.display_name.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(name) => {
            Some(DisplayName::parse(name).map_err(|_| IpcError::invalid(FieldDto::DisplayName))?)
        }
    };
    let provider = norelly_domain::oauth::OAuthProvider::parse(&input.provider)
        .ok_or_else(|| IpcError::invalid(FieldDto::Address))?;
    if !state.secrets.usable().await {
        return Ok(AddAccountResultDto::SecretStorageConsentRequired {
            path: state.secrets_file().display().to_string(),
        });
    }
    let outcome = AddOAuthAccount {
        test: &state.test,
        broker: state.tokens.as_ref(),
        accounts: &state.accounts,
    }
    .execute(AddOAuthAccountRequest {
        address,
        display_name,
        provider,
    })
    .await;
    if let AddAccountOutcome::Added(account) = &outcome
        && let Err(e) = state
            .mail
            .start(&app, account, &state.secrets, &state.exceptions)
            .await
    {
        tracing::warn!(error = %e, "synchronisation non démarrée");
    }
    Ok(AddAccountResultDto::from_outcome(&outcome))
}

/// Se reconnecte chez le fournisseur OAuth d'un compte (« Reconnexion
/// nécessaire ») ; la synchronisation reprend avec le nouveau jeton.
#[tauri::command]
#[specta::specta]
pub async fn reconnect_account(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account: u32,
) -> Result<AddAccountResultDto, IpcError> {
    let id =
        AccountId::new(i64::from(account)).map_err(|_| IpcError::invalid(FieldDto::Account))?;
    match reconnect_oauth_account(state.tokens.as_ref(), &state.accounts, id).await {
        Ok(account) => {
            if let Err(e) = state
                .mail
                .start(&app, &account, &state.secrets, &state.exceptions)
                .await
            {
                tracing::warn!(error = %e, "synchronisation non relancée");
            }
            Ok(AddAccountResultDto::from_outcome(
                &AddAccountOutcome::Added(account),
            ))
        }
        Err(ReconnectError::OAuth(e)) => Ok(crate::ipc::oauth_failure(&e)),
        Err(ReconnectError::NotFound | ReconnectError::NotOAuth) => {
            Err(IpcError::new(ErrorCodeDto::NotFound))
        }
        Err(ReconnectError::SecretStore(e)) => Err(IpcError {
            detail: Some(e),
            ..IpcError::new(ErrorCodeDto::SecretStore)
        }),
        Err(ReconnectError::Storage(e)) => Err(IpcError::storage(e)),
    }
}

/// Fait confiance au certificat refusé `fingerprint` pour `host`. Seul un
/// refus effectivement présenté à l'utilisateur peut être accepté.
#[tauri::command]
#[specta::specta]
pub async fn trust_certificate(
    state: State<'_, AppState>,
    host: String,
    fingerprint: String,
) -> Result<(), IpcError> {
    let rejection = state
        .take_rejection(&host, &fingerprint)
        .ok_or_else(|| IpcError::not_found(FieldDto::Certificate))?;
    trust(&state.exceptions, &rejection)
        .await
        .map_err(IpcError::storage)
}

/// Accepte que les mots de passe soient enregistrés dans un fichier non
/// chiffré, lisible par l'utilisateur seul, faute de trousseau du système.
#[tauri::command]
#[specta::specta]
pub async fn allow_file_secrets(state: State<'_, AppState>) -> Result<(), IpcError> {
    state
        .secrets
        .consent_to_file()
        .map_err(|e| IpcError::new(ErrorCodeDto::SecretStore).with_detail(e))
}

/// Où les mots de passe seront conservés. Décidé ici, pas deviné par
/// l'interface : elle n'annonce le trousseau du système que s'il est
/// vraiment utilisé (sinon la garantie serait fausse avec le fichier).
#[tauri::command]
#[specta::specta]
pub async fn secrets_backend(state: State<'_, AppState>) -> Result<SecretsBackendDto, IpcError> {
    Ok(if state.secrets.stores_in_system_keyring().await {
        SecretsBackendDto::SystemKeyring
    } else {
        SecretsBackendDto::File
    })
}

/// Supprime un compte, ses mots de passe et ses données locales.
#[tauri::command]
#[specta::specta]
pub async fn remove_account(state: State<'_, AppState>, id: u32) -> Result<(), IpcError> {
    let id = AccountId::new(i64::from(id)).map_err(|_| IpcError::invalid(FieldDto::Account))?;
    // Synchronisation arrêtée avant l'effacement des données locales.
    state.mail.stop(id).await;
    RemoveAccount {
        secrets: &state.secrets,
        accounts: &state.accounts,
        local_data: &state.local_data,
    }
    .execute(id)
    .await
    .map_err(IpcError::storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{EndpointDto, SecurityDto};

    fn input() -> AddAccountInput {
        AddAccountInput {
            address: "alice@example.com".into(),
            display_name: Some("  ".into()),
            password: "secret".into(),
            imap: None,
            smtp: None,
            plaintext_consents: vec![],
            confirmed_hosts: vec![],
        }
    }

    fn server(host: &str) -> ServerDto {
        ServerDto {
            host: host.into(),
            port: 993,
            security: SecurityDto::Tls,
            username: "alice".into(),
        }
    }

    #[test]
    fn input_is_revalidated() {
        assert!(
            add_request(input()).is_ok_and(|r| r.display_name.is_none() && r.servers.is_none())
        );
        let bad_address = AddAccountInput {
            address: "pas une adresse".into(),
            ..input()
        };
        assert!(add_request(bad_address).is_err());
        let half = AddAccountInput {
            imap: Some(server("imap.example.com")),
            ..input()
        };
        assert!(add_request(half).is_err());
        let injected = AddAccountInput {
            imap: Some(server("imap.example.com\r\nA1 LOGIN")),
            smtp: Some(server("smtp.example.com")),
            ..input()
        };
        assert!(add_request(injected).is_err());
        let bad_consent = AddAccountInput {
            plaintext_consents: vec![EndpointDto {
                host: "a b".into(),
                port: 143,
            }],
            ..input()
        };
        assert!(add_request(bad_consent).is_err());
    }

    #[test]
    fn debug_hides_the_password() {
        let rendered = format!("{:?}", input());
        assert!(!rendered.contains("secret"), "{rendered}");
    }
}
