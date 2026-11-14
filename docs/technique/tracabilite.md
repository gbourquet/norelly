# Traçabilité : des fonctionnalités au code

[← Documentation technique](README.md)

La [documentation fonctionnelle](../fonctionnel/README.md) décrit ce que
fait Norelly, sans rien dire de sa construction. Cette page fait le lien
inverse : pour chaque fiche, les modules qui portent ses règles, les cas
d'usage, les clés des catalogues de messages (`app/messages/{fr,en}.json`,
ADR-011) et les identifiants des commandes du registre
(`registerAppCommands`, `app/src/lib/app.svelte.ts`).

En cas de désaccord entre une fiche et le code, **le domaine fait foi**
(`crates/domain`) : la fiche est à corriger, ou l'écart est un défaut à
signaler. Une règle modifiée met à jour sa fiche **et** cette page.

## Comptes : ajout, configuration, retrait

Fiche : [comptes.md](../fonctionnel/comptes.md).

### Règles et code

| Règle | Où |
|---|---|
| Adresse validée strictement : `dot-atom` RFC 5322, UTF-8 admis dans la partie locale (RFC 6531), domaine validé comme nom d'hôte ; guillemets et IP littérales refusés | `crates/domain/src/address.rs` |
| Ordre d'autodécouverte : autoconfig du fournisseur (HTTPS) → ISPDB de Mozilla (HTTPS, seul le domaine transmis) → SRV RFC 6186 → sondage des ports usuels (`imap.`, `smtp.`, `mail.`, domaine) en TLS implicite ou STARTTLS → saisie manuelle ; arrêt dès que IMAP et SMTP sont trouvés | `crates/domain/src/discovery.rs` |
| Préférence de sécurité : TLS implicite > STARTTLS > clair | `discovery.rs` |
| Jamais de HTTP en clair pour l'autoconfiguration ; serveur SRV hors du domaine de l'adresse : confirmation obligatoire | `discovery.rs` |
| Certificat : valide pour la plateforme, ou exception mémorisée (hôte + empreinte SHA-256) ; sinon rejet avec un `CertificateIssue` détaillé ; une empreinte différente de l'exception est signalée comme changement | `crates/domain/src/security.rs` (`TlsTrustPolicy`) |
| Un certificat autosigné reste une « autorité inconnue » même marqué autorité (`CA:TRUE`, `openssl req -x509` par défaut), que webpki refuse comme `CaUsedAsEndEntity` | `crates/adapters/tls/src/lib.rs` (`certificate_issue`) |
| Clair refusé sans consentement explicite par serveur ; STARTTLS obligatoire s'il est annoncé ou demandé (aucun repli silencieux) ; capacités relues après TLS | `security.rs` (`PlaintextPolicy`), `capabilities.rs` |
| Authentification : `AUTHENTICATE PLAIN` ou `LOGIN` selon les capacités, `LOGINDISABLED` respecté ; avec un jeton OAuth, `XOAUTH2` seulement (jamais de repli PLAIN ou LOGIN), et un mot de passe n'est jamais envoyé comme jeton | `crates/domain/src/auth.rs` |
| Comptes Microsoft personnels et Gmail (`gmail.com`) reconnus au domaine de l'adresse (égalité exacte ; un domaine Google Workspace garde le mot de passe) ; configuration Google : points d'accès `accounts.google.com` et `oauth2.googleapis.com`, scope `https://mail.google.com/`, `openid email`, identifiant et `client_secret` du client de bureau fournis à la compilation (`option_env!`), le secret dans l'échange et le rafraîchissement ; fournisseur proposé seulement s'il est utilisable (`OAuthConfig::is_usable`, `offered_provider`, `OAuthProvider::offered_for`), recoupé par la commande `add_oauth_account` (`oauth_request_provider`) ; valeurs factices aux e2e (`app/e2e/run.sh`), secrets exigés au build de version (`.github/workflows/release.yml`) ; paramètres d'autorisation propres au fournisseur (`authorize_params` : `access_type=offline`, `prompt=select_account consent`), rafraîchissement sans scope (`refresh_with_scopes`), serveurs `imap.gmail.com` et `smtp.gmail.com` ; configuration du fournisseur (`consumers`, scopes `outlook.office.com`, scopes d'identité `openid email profile` à la seule connexion interactive, identifiant client public) ; identité de l'`id_token` comparée à l'adresse du compte (`verify_identity` : `preferred_username` ou `email`, casse ignorée ; absente : refus) ; URL d'autorisation PKCE S256 + `state` ; retour du navigateur : `state` vérifié d'abord, doublons refusés ; réponse `user=…\x01auth=Bearer …\x01\x01` ; jeton au format `b64token` (RFC 6750), 16 Kio au plus | `crates/domain/src/oauth.rs`, `credentials.rs` |
| Un jeton OAuth ne part que vers les serveurs IMAP et SMTP de son fournisseur, en TLS ou STARTTLS, vérifié par les adaptateurs avant toute authentification | `oauth.rs` (`token_destination_allowed`), `ports.rs` (`ConnectRequest::authorize_credentials`), `crates/adapters/imap/src/lib.rs`, `crates/adapters/smtp/src/lib.rs` |
| Jeton d'accès en mémoire jusqu'à son expiration (horloge murale, marge de 120 s), un verrou par compte, rafraîchissement et enregistrement de la rotation bornés ensemble à 30 s (`REFRESH_TIMEOUT`, une seule échéance) ; jeton de rafraîchissement dans le trousseau, remplacé à chaque rotation ; rotation refusée par le trousseau : nouveau jeton gardé en mémoire (`Slot::unsaved_refresh`), présenté aux rafraîchissements suivants et réenregistré à chacun, conservé par `forget` ; `invalid_grant` ou jeton absent : « reconnexion nécessaire » ; trousseau illisible : `AuthError::SecretStoreUnavailable`, sans appel au fournisseur ni nouvel essai, code `secret_store` (`connection_failure_of`), `SendFailure::Temporary` dans la file d'envoi ; fournisseur injoignable ou trop lent (`TokenError::Network`) : `AuthError::ProviderUnavailable` (`auth_error_of`, domaine), sans nouvel essai ni oubli du jeton, code `oauth`, `SendFailure::Temporary` ; autre refus du fournisseur (`invalid_client`, réponse invalide) : refus définitif ; reconnexion (`store_grant`) : verrou du compte pris avant l'écriture du trousseau, elle-même bornée à 30 s | `crates/application/src/oauth.rs` (`TokenBroker`), `crates/domain/src/oauth.rs` (`auth_error_of`), `crates/domain/src/auth.rs`, `crates/domain/src/compose/outbox.rs`, `app/src-tauri/src/ipc.rs` |
| Suppression d'un compte : jetons OAuth gardés en mémoire oubliés d'abord (`TokenCache::discard`, qui attend un rafraîchissement en cours et laisse un emplacement marqué), puis secrets du trousseau effacés, chaque effacement borné à `SECRET_STORE_TIMEOUT` (échec : compte gardé, suppression relançable, jetons en mémoire perdus, oubli levé par `TokenCache::restore` pour permettre la reconnexion) ; un compte oublié n'obtient plus de jeton et n'écrit plus dans le trousseau, même au terme d'une reconnexion commencée avant ; ajout OAuth dont l'enregistrement du jeton échoue (y compris délai dépassé) : jetons abandonnés (`TokenBroker::abandon` : oubli puis effacement borné, qui l'emporte sur l'écriture abandonnée) avant le retrait du compte | `crates/application/src/accounts.rs` (`RemoveAccount`), `crates/application/src/oauth.rs`, `app/src-tauri/src/commands.rs` |
| Écritures du trousseau (système ou fichier) numérotées à l'appel : une écriture abandonnée par l'appelant, qui aboutit plus tard dans sa tâche bloquante, n'écrase pas une écriture ou un effacement plus récent de la même clé ; un verrou par clé, une écriture bloquée n'arrête que sa clé | `crates/adapters/keyring/src/order.rs` (`WriteOrder`), `lib.rs`, `file.rs` |
| Avant chaque connexion d'un compte OAuth (synchronisation, envoi), jeton renouvelé ; après un refus du serveur, jeton oublié et une seule nouvelle tentative | `crates/application/src/oauth.rs` (`FreshTokens`), `app/src-tauri/src/state.rs` |
| « Se connecter avec Microsoft » : serveurs du fournisseur (IMAP 993 TLS, SMTP 587 STARTTLS), tests IMAP et SMTP avec le jeton, compte enregistré avec son fournisseur, puis jeton de rafraîchissement ; sans jeton de rafraîchissement (accès hors ligne refusé, `require_offline_access`), refus à l'ajout comme à la reconnexion : `TokenError::OfflineAccessRefused`, code `oauth_offline_access`, messages `add_fail_oauth_offline_access` et `failure_oauth_offline_access` ; `535 5.7.139` : conseil `MicrosoftSmtpDisabled` ; « User is authenticated but not connected » (défaillance intermittente de Microsoft) : test IMAP réessayé (`OAUTH_IMAP_ATTEMPTS`, 1 s d'écart), puis conseil `MicrosoftImapUnavailable` | `crates/application/src/accounts.rs` (`AddOAuthAccount`, `reconnect_oauth_account`), `crates/domain/src/oauth.rs` (`is_smtp_oauth_disabled`, `require_offline_access`), `app/src-tauri/src/ipc.rs` (`oauth_failure`), `app/src/lib/add-account.svelte.ts`, `app/src/lib/errors.ts` |
| Ajout (`AddOAuthAccount`, y compris connexion réutilisée) et reconnexion (`reconnect_oauth_account`) : identité vérifiée avant tout test, stockage ou remplacement du jeton ; autre compte : `TokenError::WrongAccount`, code `oauth_wrong_account`, messages `add_fail_oauth_wrong_account` et `failure_oauth_wrong_account` | `crates/application/src/accounts.rs`, `app/src-tauri/src/ipc.rs` (`oauth_failure`), `app/src/lib/add-account.svelte.ts`, `app/src/lib/errors.ts` |
| Navigateur du système, redirection `http://localhost:<port>/` écoutée sur `127.0.0.1` ; retour au mauvais `state` ou illisible ignoré (l'attente continue, 5 min au plus) ; code échangé avec le vérificateur PKCE (vérifié par le faux point d'accès des tests) ; `id_token` décodé (base64url, JSON) en `OAuthIdentity`, illisible : pas d'identité ; erreurs du fournisseur classées sans reprendre le corps | `crates/adapters/oauth/src/lib.rs` |
| Test de connexion (ajout de compte, réglages des serveurs) : échec passager (`ConnectionError::is_transient` : réseau, délai, connexion coupée) réessayé sur une nouvelle connexion, `CONNECTION_ATTEMPTS` essais à `CONNECTION_RETRY_DELAY` d'écart ; refus, certificat ou protection jamais réessayés ; poignée de main TLS bornée à 10 s (`DEFAULT_HANDSHAKE_TIMEOUT`) | `crates/domain/src/connection.rs`, `crates/application/src/test_connection.rs`, `crates/adapters/tls/src/lib.rs` |
| Ajout d'un compte OAuth (serveurs imposés par le fournisseur) : réception vérifiée et test d'envoi en échec `send_check_can_wait` (serveur joint puis muet ou connexion coupée, y compris réinitialisée en SMTP) : compte ajouté (`AddAccountOutcome::AddedSendUnchecked`, `send_unchecked` dans le DTO), synchronisation démarrée, message `notice_send_unchecked` ; serveur introuvable, refus ou protection : ajout refusé ; compte par mot de passe : envoi toujours vérifié (port ou chiffrement faux) | `crates/domain/src/connection.rs`, `crates/application/src/accounts.rs`, `crates/adapters/smtp/src/lib.rs` (`connection_cut`), `app/src-tauri/src/ipc.rs`, `app/src-tauri/src/commands.rs`, `app/src/lib/add-account.svelte.ts`, `App.svelte` |
| Serveur joint puis muet ou qui coupe la connexion (`ConnectionError::is_unresponsive`) : `FailureKindDto::Unresponsive`, « le serveur ne répond pas », distinct de « serveur injoignable » (nom ou port en cause) | `crates/domain/src/connection.rs`, `app/src-tauri/src/ipc.rs` (`network_kind`), `app/src/lib/add-account.svelte.ts`, `app/src/lib/errors.ts` |
| Ajout OAuth échoué après la connexion chez le fournisseur (`ConnectionFailed`) : jetons gardés en mémoire par `TokenBroker` (`keep_pending`, un seul à la fois), réutilisés une fois pour le même fournisseur et la même adresse, moins de `PENDING_AUTHORIZATION_MAX_AGE_SECS` après leur obtention et jeton d'accès encore valable (`PendingAuthorization::usable_for`) ; l'écran ne parle pas du navigateur quand `oauth_sign_in_kept` le dit | `crates/domain/src/oauth.rs`, `crates/application/src/oauth.rs`, `crates/application/src/accounts.rs` (`AddOAuthAccount`), `app/src-tauri/src/commands.rs`, `app/src/lib/add-account.svelte.ts`, `AddAccountDialog.svelte` |
| Connexion Google annulée à l'ajout d'un compte (refus dans la page, délai dépassé ; plafond d'utilisateurs de l'application non validée possible) : `add_account_sign_in_error` en fait `TokenError::GoogleSignInRefused` (pas à la reconnexion), code `oauth_google_refused`, messages `add_fail_oauth_google_refused` et `failure_oauth_google_refused` ; l'interface bascule sur le mot de passe d'application (`passwordInstead`) et en montre les étapes | `crates/domain/src/oauth.rs`, `crates/application/src/accounts.rs` (`AddOAuthAccount`), `app/src-tauri/src/ipc.rs` (`oauth_failure`), `app/src/lib/add-account.svelte.ts`, `AddAccountDialog.svelte` |
| Ajout d'un compte OAuth : fournisseur demandé recoupé avec celui de l'adresse (`OAuthProvider::serves`, domaine) avant toute connexion ; écart (réponse d'interface périmée) : `AddAccountOutcome::Invalid`, rien n'est contacté | `crates/domain/src/oauth.rs`, `crates/application/src/accounts.rs` (`AddOAuthAccount`) |
| Identité Google (`check_identity_claims`, appelée par `verify_sign_in` à l'ajout et à la reconnexion avec la configuration du gestionnaire de jetons) : `iss` de Google, `aud` = identifiant client (jamais vide), `email_verified` vrai (sinon `WrongAccount`) ; revendications lues dans l'`id_token` (`aud` chaîne ou liste, `email_verified` booléen ou chaîne) ; scopes accordés lus (`OAuthGrant::granted_scopes`) et scope de messagerie exigé chez Google (`require_mail_access`, `TokenError::MailAccessRefused`, code `oauth_mail_access`) | `crates/domain/src/oauth.rs`, `crates/domain/src/ports.rs`, `crates/adapters/oauth/src/lib.rs`, `crates/application/src/accounts.rs`, `app/src-tauri/src/ipc.rs`, `app/src/lib/add-account.svelte.ts`, `app/src/lib/errors.ts` |
| Fournisseur absent de la version compilée : `TokenBroker` (configuration injectable `with_configs`, par défaut celle compilée ; `is_usable`) refuse avant le trousseau et le fournisseur (`AuthError::ProviderNotConfigured`, refus définitif : `SendFailure::Authentication`, code `oauth_not_configured` ; synchronisation arrêtée après l'événement `Offline`, sans nouvel essai : `ConnectionError::retry_is_useless`, `SyncFailure::retry_is_useless`, `account_actor.rs`) ; configuration passée au port `OAuthClient` (`authorize(config, …)`, `refresh(config, …)`), seule source des contrôles et des requêtes ; ajout et reconnexion refusés sans navigateur (`TokenError::ProviderNotConfigured`) ; l'interface redemande le fournisseur avant d'ouvrir le navigateur | `crates/application/src/oauth.rs`, `crates/application/src/accounts.rs`, `crates/application/src/account_actor.rs`, `crates/domain/src/auth.rs`, `crates/domain/src/connection.rs`, `crates/domain/src/ports.rs`, `crates/domain/src/compose/outbox.rs`, `crates/adapters/oauth/src/lib.rs`, `app/src-tauri/src/ipc.rs`, `app/src/lib/add-account.svelte.ts` |
| Fournisseur `google` accepté par la base (migration 14, contrainte `CHECK` remplacée sans reconstruire `accounts`) ; suite de contrat du dépôt de comptes jouée avec un compte Google | `crates/adapters/sqlite/migrations/0014_oauth_google.sql`, `crates/test-support/src/contracts.rs` |
| L'interface propose « Se connecter avec Microsoft » ou « Se connecter avec Google » quand Rust reconnaît l'adresse (`oauth_provider_for`) ; libellés par fournisseur ; Google : explication de l'avertissement « application non validée » avant le navigateur et pendant l'attente, bouton « Utiliser un mot de passe d'application » (`usePassword`) et retour « Se connecter avec Google plutôt » (`useProviderSignIn`, mot de passe effacé) ; Microsoft : jamais de mot de passe (`canUsePassword`) ; « Se reconnecter » visible sur la ligne du compte quand la synchronisation signale `reauth_required`, et dans le menu « ⋯ » des seuls comptes OAuth | `app/src/lib/add-account.svelte.ts`, `AddAccountDialog.svelte`, `app/src/lib/mail.svelte.ts`, `Sidebar.svelte` |
| Compte Google (domaine `gmail.com` ou `googlemail.com`, ou serveur `imap.gmail.com` ou `smtp.gmail.com`, égalité exacte) dont les identifiants sont refusés (`AuthError::Rejected` seulement) : conseil `GoogleAppPassword`, joint par `AddAccount` à l'échec, projeté en `FailureKindDto::GoogleAppPassword` ; l'interface revient à l'écran de l'essai, mot de passe vidé, et montre les trois étapes avec un bouton vers `myaccount.google.com/apppasswords` (`open_external`, revalidé en Rust) | `crates/domain/src/discovery.rs` (`authentication_advice`), `crates/application/src/accounts.rs`, `app/src-tauri/src/ipc.rs`, `app/src/lib/add-account.svelte.ts`, `AddAccountDialog.svelte` |
| Le secret n'est jamais dans `Debug`, n'a pas de `Display`, n'est pas sérialisable | `crates/domain/src/credentials.rs` |
| Le mot de passe n'appartient pas au compte : il vit dans le trousseau, sous des clés propres au compte | `crates/domain/src/account.rs` |
| Fichier de secrets non chiffré (droits 0600) seulement si aucun trousseau ne répond, après consentement (dérogation ADR-002) | `crates/adapters/keyring/src/file.rs` |
| L'écran d'ajout n'annonce « trousseau du système » que sur la réponse de `secrets_backend` (sonde du trousseau) : avec le fichier de repli, ou sans réponse, aucune garantie n'est affichée | `app/src-tauri/src/state.rs` (`AppSecretStore::stores_in_system_keyring`), `app/src-tauri/src/commands.rs` (`secrets_backend`), `app/src/lib/add-account.svelte.ts`, `AddAccountDialog.svelte` |

Cas d'usage : `DiscoverSettings`, `TestConnection`, `AddAccount`,
`AddOAuthAccount`, `reconnect_oauth_account`, `TokenBroker`,
`RemoveAccount` (`crates/application/src/accounts.rs`, `oauth.rs`,
`discover_settings.rs`, `test_connection.rs`).

### Clés des messages

`add_discovering`, `add_testing`, `add_msg_incomplete`, `add_msg_dns_outside`, `add_cert_title`, `add_issue_*`, `add_cert_changed`, `add_plain_warning`, `add_secrets_warning_*`, `add_fail_authentication`, `add_fail_google_app_password`, `add_app_password_*`, `add_fail_network`, `add_fail_unresponsive`, `failure_unresponsive`, `notice_send_unchecked`, `add_fail_security`, `add_msg_already_exists`, `add_oauth_*` (dont `add_oauth_testing_microsoft`, `add_oauth_hint_google`), `add_use_app_password`, `add_use_google_sign_in`, `add_fail_oauth_google_refused`, `failure_oauth_google_refused`, `add_fail_microsoft_smtp_disabled`, `add_fail_reauth_required`, `add_fail_oauth_cancelled`, `add_fail_oauth_wrong_account`, `failure_oauth_wrong_account`, `add_fail_oauth`, `sidebar_reconnect`, `sidebar_reauth_required`, `notice_reconnected`, `remove_explanation_*`.

### Commandes

`account.add`, `account.reconnect`, `account.remove`.

## Synchronisation

Fiche : [synchronisation.md](../fonctionnel/synchronisation.md).

### Règles et code

| Règle | Où |
|---|---|
| Rôle d'un dossier : attribut SPECIAL-USE (`\Sent`, `\Drafts`, `\Trash`, `\Junk`, `\Archive`, `\All`), sinon heuristique de nom | `crates/domain/src/mailbox.rs` |
| Stratégie par dossier : complète (première fois ou UIDVALIDITY changée) ; QRESYNC (`VANISHED`) ; CONDSTORE (drapeaux depuis `HIGHESTMODSEQ`) ; sans extension (comparaison des UID et drapeaux) ; rien si `HIGHESTMODSEQ` et `UIDNEXT` n'ont pas bougé | `crates/domain/src/sync.rs` (`plan`) |
| Propriétés : convergence (local = serveur), idempotence, application double sans effet (tests `proptest`) | `sync.rs`, `sync/tests.rs` |
| Rapatriement : en-têtes et texte pour tous ; pièces jointes sous le seuil ; structure MIME bornée (profondeur 16, 256 parties) | `crates/domain/src/fetch_policy.rs` |
| Seuil de rapatriement borné entre 64 Kio et 64 Mio | `crates/domain/src/settings.rs` |
| IDLE renouvelé toutes les 25 minutes (avant la limite des 29 minutes de la RFC 2177) | `sync.rs` (`IDLE_RENEWAL_SECS`) |
| Reconnexion : attente exponentielle de 1 s à 5 min, avec gigue, remise à zéro après un succès | `sync.rs` (`Backoff`) |
| Tout texte venu du serveur est assaini (contrôles retirés, longueur bornée) ; noms de boîte ≤ 1024 caractères, 10 000 boîtes au plus | `mailbox.rs` |
| Ordre de synchronisation : boîte de réception d'abord, dossiers virtuels (`\All`, `\Flagged`) en dernier, les autres dans l'ordre du serveur (tri stable) | `crates/domain/src/sync.rs` (`sync_order`), `crates/application/src/sync.rs` (`sync_all_observed`) |
| Journaux : collecteur `tracing-subscriber` (niveau INFO au plus) vers `norelly.log` du dossier des données, 2 Mio par fichier, rotation vers `norelly.log.1` (une seule archive), entrée trop longue tronquée ; sans fichier utilisable, l'application continue sans journaux | `app/src-tauri/src/logs.rs` (`BoundedLog`, `subscriber`, `init`), `app/src-tauri/src/lib.rs` |
| Au fil de l'eau : `sync_all_observed` signale `SyncStep::Folders` (liste lue), `Progress { done, total }` (0 au départ, puis après chaque boîte sélectionnable), `Headers` (lot d'en-têtes, de suppressions ou de drapeaux enregistré, avant les corps) et `MailboxDone` (une boîte réinitialisée, `UIDVALIDITY` changée, compte comme modifiée même sans autre changement : `MailboxReport::reset`) ; l'acteur relaie pendant le tour un signal regroupé (`FoldersChanged`, au plus un par `LIVE_REFRESH_INTERVAL` d'une seconde, `LiveRefresh`) et la progression au même rythme, puis un changement encore regroupé est signalé une dernière fois sans attendre l'intervalle (`take_pending`), et, après `reapply` et `purge_placeholders`, les `FolderChanged` de fin et en dernier la progression finale (`SyncProgress` avec `done == total`, DTO `sync_progress`) ; après chaque lot visible, `reapply_mailbox` (actions en attente sur les messages du dossier, port `PendingOpStore`, suite de contrat) rétablit leur effet avant le signal, et `sync_named` le limite au dossier synchronisé ; cadence unique (`LiveRefresh` : `note_change` sur un lot d'en-têtes, `flush_due` à chaque étape — `Headers`, `HeadersDone`, `Bodies` après chaque paquet de corps, progression, fin de dossier — au plus un signal par seconde, changement regroupé rattrapé à l'étape suivante ou en fin de tour ; `MailboxStep`) ; relais en retard (`Lagged`) : `folders_changed` du compte (`lagged_event`) et `outbox_changed` pour la boîte d'envoi (`outbox_lagged_event`) ; l'interface ignore une relecture des dossiers périmée (`loadFolders`) et relance la recherche affichée sur `folders_changed` ; l'interface garde la progression (`MailState::syncing`), l'efface à la fin (`done == total`) ou à la coupure, et l'affiche sous le compte (`role="status"`) | `crates/application/src/sync.rs`, `crates/application/src/account_actor.rs` (`full_sync`), `app/src-tauri/src/mail.rs`, `app/src-tauri/src/ipc.rs`, `app/src/lib/mail.svelte.ts`, `Sidebar.svelte` |

Orchestration : acteur de compte à deux connexions (commandes ; IDLE sur la
boîte de réception), `SyncAccount` boîte par boîte, enregistrement par lots
pour une reprise propre (`crates/application/src/account_actor.rs`,
`sync.rs`).

### Clés des messages

`common_offline`, `sidebar_offline_reason`, `sidebar_syncing`, `sidebar_syncing_label`, `failure_*`, `message_notice_not_available`.

### Commandes

`account.sync`.

### Vérifications et compléments

- Coupure en pleine synchronisation : chaque lot intermédiaire est
  enregistré sans `HIGHESTMODSEQ` ; la reprise compare les UID. Vérifié
  par Toxiproxy (coupure, latence, connexion à moitié fermée).
- `VANISHED` d'UID inconnus localement : ignorés ; dépliage des plages
  borné.
- COMPRESS=DEFLATE : non activé (ADR-002).
- Partie au-dessus du seuil : téléchargée à la demande par la connexion de
  commandes.

## Liste des conversations et boîte unifiée

Fiche : [conversations.md](../fonctionnel/conversations.md).

### Règles et code

| Règle | Où |
|---|---|
| Regroupement JWZ : `References`, puis `In-Reply-To`, puis sujet en dernier recours | `crates/domain/src/conversation.rs` (`thread`) |
| Version incrémentale (`assign`) : un message rejoint toute conversation dont il connaît un identifiant ; fusion si besoin ; converge vers JWZ quel que soit l'ordre d'arrivée (propriété testée) | `conversation.rs` |
| Regroupement par sujet seulement pour une **réponse** (préfixe `Re:`, `Fwd:`… ou références sans correspondance), dans une fenêtre de 31 jours | `conversation.rs` (`SUBJECT_WINDOW_SECS`) |
| Préfixes reconnus : `Re`, `RE`, `Fwd`, `Fw`, `TR`, `AW`, `WG`, `SV`, `VS`, `Antw`, `RIF`, `R`, `RE :`… (liste fixe) | `conversation.rs` (`strip_prefix`) |
| Une conversation n'est jamais scindée quand un message qui la reliait disparaît | ADR-003, `crates/adapters/sqlite/src/conversations.rs` |
| Pages de 500 au plus ; 500 messages au plus par conversation affichée | `crates/domain/src/ports.rs` (`MAX_PAGE_SIZE`, `MAX_CONVERSATION_MESSAGES`) |
| Lecture paginée par curseur (CQRS léger, ADR-008) | ports `ConversationQuery`, `MessageListQuery` |

### Clés des messages

`list_empty`, `mailbox_unified_inbox`, `common_no_subject`, `common_unknown_sender`, `list_marked`.

### Commandes

`conversation.next`, `conversation.previous`, `conversation.open`, `conversation.back`, `mailbox.unified`.

### Vérifications et compléments

- Liste virtualisée : pages demandées au fil du défilement
  (`list_conversations`).
- Mesures (bench, 50 000 messages) : première page ≈ 0,3 ms ; changement
  de dossier de bout en bout 13 à 16 ms.

## Lecture sécurisée

Fiche : [lecture.md](../fonctionnel/lecture.md).

### Règles et code

| Règle | Où |
|---|---|
| Liens ouverts : schémas `http`, `https`, `mailto` uniquement, 2048 caractères au plus | `crates/domain/src/reading.rs` (`MAX_LINK_LEN`) |
| Images distantes : autorisation par adresse `From` (falsifiable : protège du pistage, pas de l'usurpation) | `reading.rs`, ADR-004 |
| Nom de fichier assaini : séparateurs, `..`, noms réservés Windows, caractères de contrôle et de sens d'écriture (`exe.pdf` déguisé), 200 octets au plus | `reading.rs` (`safe_file_name`) |
| Ouverture directe réservée à une liste blanche d'extensions (documents sans macros, images matricielles, texte, sons, vidéos) ; tout le reste s'enregistre seulement | `reading.rs` (`OPENABLE_EXTENSIONS`, `is_unsafe_to_open`) |
| Enregistrement : création exclusive dans le dossier choisi, jamais d'écrasement ni de lien suivi | `app/src-tauri/src/reading.rs`, ADR-004 |
| HTML assaini par liste blanche (`ammonia`), CSS assaini (`@import`, `url()` externes retirés), `cid:` → `mailpart://`, images distantes → espace réservé ou `remote-img://` | `crates/adapters/render/src/sanitize.rs`, `css.rs` |
| CSP du document : `default-src 'none'`, seul script autorisé : l'agent (empreinte SHA-256) ; cadre `sandbox="allow-scripts"` sans `allow-same-origin` | ADR-004, `app/src-tauri/src/protocols.rs` |
| Conversation restée ouverte : marqués lus les messages affichés non lus à l'ouverture et leurs exemplaires d'autres dossiers (même `Message-ID`) ; un message arrivé depuis, ou étranger à la conversation, n'est pas touché | `crates/domain/src/actions.rs` (`opened_seen_targets`), commande `mark_opened_seen` (`app/src-tauri/src/actions.rs`) |
| Marquage après le délai des réglages, sans message ni annulation ; minuteur annulé si l'on quitte la conversation, si l'utilisateur agit dessus ou si une autre ouverture la dépasse ; rien tant que les réglages ne sont pas chargés ; une action de l'utilisateur attend le marquage en cours d'envoi, pour l'emporter | `app/src/lib/mail.svelte.ts`, `mail.test.ts` |
| Délai du marquage comme lu : réglage de l'application, 5 s par défaut, 60 s au plus (0 : à l'ouverture) ; valeur stockée hors bornes ramenée à la valeur par défaut, réglage par réglage (`Settings::sanitized`) | `crates/domain/src/settings.rs` (`mark_seen_delay_secs`, `MAX_MARK_SEEN_DELAY_SECS`), `crates/adapters/sqlite/src/preferences.rs` (`app.mark_seen_delay`) |
| Proxy d'images : sans cookies ni `Referer`, User-Agent neutre, taille et types bornés, adresses publiques seulement, autorisation revérifiée à chaque image | `app/src-tauri/src/remote_images.rs` |

### Clés des messages

`message_images_blocked`, `message_show_images`, `message_show_text`, `message_show_html`, `message_open`, `message_save`, `message_saved_as`, `err_link_refused`, `err_executable_attachment`, `message_unreadable`.

### Commandes

`reading.plain-text`.

### Vérifications et compléments

- Cadre isolé servi par le protocole `mailview://` ; corps lus sans poser
  `\Seen` (`BODY.PEEK`).
- Touches pressées dans le cadre relayées par l'agent (`postMessage`) au
  registre de commandes.
- Tests e2e de sécurité (WebKitGTK) : zéro requête vers un serveur canari,
  aucun script du mail exécuté, IPC inaccessible depuis `mailview://`,
  `javascript:` jamais ouvert (`app/e2e/specs/reading-security.e2e.mjs`).

## Actions sur les messages, hors ligne et annulation

Fiche : [actions.md](../fonctionnel/actions.md).

### Règles et code

| Règle | Où |
|---|---|
| « Supprimer » = déplacer vers la Corbeille ; **depuis la Corbeille**, suppression définitive | `crates/domain/src/actions.rs` (`resolve`) |
| « Archiver » = déplacer vers le dossier `\Archive`, sinon vers le dossier de repli choisi par l'utilisateur | `actions.rs` |
| Opération durable (`PendingOp`) enregistrée avec l'état local nécessaire à son annulation ; effet local immédiat (`local_effect`) | `actions.rs` |
| Fenêtre d'annulation : 5 s (`UNDO_WINDOW_SECS`) ; au-delà, « trop tard » | `actions.rs` |
| Conflits au rejeu (`decide`) : UIDVALIDITY changée, message disparu, dossier de destination supprimé → opération abandonnée, état local rétabli (`revert_effect`) ; la synchronisation suivante fait foi | `actions.rs` |
| Stratégie serveur : `MOVE` ; sinon `COPY` + `STORE \Deleted` + `UID EXPUNGE` (UIDPLUS) ; sinon `EXPUNGE` seulement si aucun autre message de la boîte n'est marqué `\Deleted` (`expunge_is_safe`) | `actions.rs` |
| Drapeaux posés avec `UNCHANGEDSINCE` (CONDSTORE) | `crates/adapters/imap/src/session.rs` |
| Bornes : 1 000 messages par action, 5 essais de rejeu, 10 000 opérations en attente par compte | `actions.rs` (`MAX_ACTION_TARGETS`, `MAX_REPLAY_ATTEMPTS`, `MAX_PENDING_OPS`) |
| Copie provisoire d'un message déplacé : visible dans la destination, retirée à l'annulation ou à l'abandon, remplacée sans trou ni doublon par le vrai message à la synchronisation, ignorée par la synchronisation et les actions | migration 5, `crates/adapters/sqlite/src/actions.rs` |

Cas d'usage : `PerformAction`, `UndoActions`, `ReplayActions`
(`crates/application/src/actions.rs`).

Accès à la souris (interface) :

| Règle | Où |
|---|---|
| Actions de chaque zone : barre du message ouvert et menu « Plus… » (`readerActions`), survol d'une ligne (`rowActions`), menu contextuel (`menuActions`), barre de sélection (`bulkActions`) ; chaque élément exécute une commande du registre | `app/src/lib/visible-actions.ts` |
| Quand la place manque, les boutons passent dans « Plus… » par priorité (`fitByPriority`) | `visible-actions.ts`, `app/src/components/ActionBar.svelte` |
| Libellés des bascules selon l'état de toutes les cibles (`subjectOf`) | `visible-actions.ts` |
| Case à cocher d'une ligne sans changer la sélection (`toggleMarkAt`), tout décocher (`clearMarks`) | `app/src/lib/mail.svelte.ts` |
| Glisser-déposer sur un dossier : événements pointeur (souris, stylet), pas le glisser-déposer HTML, que l'interception native des fichiers déposés rend peu fiable dans la webview ; seuil de 5 px (`DRAG_THRESHOLD`) ; conversations emportées : la saisie, ou toutes les cochées si elle l'est, d'un seul compte (`dragPayload`) ; dossiers proposés : même compte, sélectionnable, pas le seul dossier d'origine (`canDrop`, affichage seulement : Rust revalide le déplacement) ; Échap annule ; le clic qui suit un glisser n'ouvre pas la conversation (`suppressClick`) | `app/src/lib/drag-move.svelte.ts` |
| Dépôt = action `move` sur les conversations emportées, sans toucher à la sélection ni à la conversation ouverte (`act(action, done, ids)`) | `app/src/lib/mail.svelte.ts`, `app/src/App.svelte` |
| Cibles du dépôt : attributs `data-drop-folder` / `data-drop-account` des dossiers, trouvés sous le pointeur (`elementFromPoint`) | `app/src/components/Sidebar.svelte`, `App.svelte` |

### Clés des messages

`action_archived`, `action_deleted`, `action_moved`, `action_flagged`, `action_unflagged`, `action_marked_read`, `action_marked_unread`, `action_undo`, `action_undone`, `action_too_late`, `action_nothing`, `err_no_trash`, `err_no_archive`, `err_pending_queue_full`, `action_archive_folder_set`, `action_more`, `action_follow`, `action_unfollow`, `list_marked`, `list_select_row`, `list_selection`, `list_bulk_actions`, `cmd_clear_selection`, `drag_conversations`.

### Commandes

`action.archive`, `action.delete`, `action.flag`, `action.mark-read`, `action.mark-unread`, `action.move`, `action.toggle-mark`, `selection.clear`, `action.undo`, `account.archive-folder`.

### Vérifications et compléments

- Suppressions faites par un autre client pendant un `EXPUNGE` sans
  UIDPLUS : comptées (`ReplayReport::foreign_expunges`), la boîte est
  resynchronisée.

## Dossiers : création, renommage, déplacement, suppression

Fiche : [dossiers.md](../fonctionnel/dossiers.md).

### Règles et code

| Règle | Où |
|---|---|
| Nom saisi : espaces de bord retirés, non vide, 100 caractères au plus, ni contrôle, ni caractère de mise en forme, ni `*` / `%` (`folder_label`, `MAX_FOLDER_LABEL_CHARS`) | `crates/domain/src/mailbox.rs` |
| Ordre du panneau de gauche (`folder_display_order`) : en arbre, chaque dossier suivi de ses sous-dossiers par nom ; au premier niveau, dossiers spéciaux dans l'ordre de `MailboxRole::rank`, puis les autres par nom ; un dossier spécial reste au premier niveau même rangé dans un autre (`[Gmail]/Sent Mail`, `INBOX/Sent`) ; parent non listé : plus proche ancêtre listé ; `INBOX` reconnu à la casse près ; appliqué par `FolderTreeQuery::tree` (SQLite et faux, contrat) | `mailbox.rs` |
| Libellé d'un dossier spécial traduit d'après son rôle (`folderLabel`), nom du serveur en info-bulle s'il diffère (`folderServerName`, sauf `INBOX`) ; repris par le panneau, le titre de la liste, « Déplacer vers… » (recherche sur le libellé et le nom), les emplacements et les notifications ; affichage seulement, le rôle est décidé en Rust | `app/src/lib/folder-label.ts` |
| Nom complet (`new_folder_name`) : parent + séparateur du parent ; au premier niveau, préfixe `INBOX<séparateur>` si tous les autres dossiers sont sous INBOX (Courier, Cyrus) ; séparateur interdit dans le nom ; doublon refusé à la casse près ; parent inconnu, sans hiérarchie ou virtuel (`\All`, `\Flagged`) refusé | `mailbox.rs` |
| Décision prise sur la liste relue par `LIST` juste avant `CREATE`, pas sur la copie locale | `crates/application/src/folders.rs` (`CreateFolder`) |
| `CREATE` sur la connexion de commandes de l'acteur ; relecture de la liste (`sync_folders`) puis `FoldersChanged` ; un dossier créé reste annoncé comme créé si la relecture échoue (reconnexion ensuite) ; une demande abandonnée par l'interface (délai dépassé) avant d'être servie n'est pas envoyée | `crates/application/src/account_actor.rs` |
| Nom envoyé en UTF-7 modifié ; refus `NO` → `ConnectionError::Rejected` | `crates/adapters/imap/src/session.rs` (`create`) |
| Commande IPC : compte et parent revalidés (le parent doit être un dossier du compte), nom vérifié seul avant l'envoi, 30 s au plus (`CREATE_FOLDER_TIMEOUT`), erreurs traduites en codes | `app/src-tauri/src/folders.rs`, `app/src-tauri/src/mail.rs` |
| Emplacements proposés : premier niveau puis dossiers du compte, sans les dossiers virtuels `\All` et `\Flagged` (`parentChoices`, affichage seulement : la règle est dans le domaine) | `app/src/lib/new-folder.svelte.ts` |
| Nouveau nom (`renamed_folder_name`) : même parent, mêmes règles qu'un nouveau dossier ; seul le dossier lui-même peut changer de casse ; nom inchangé : rien n'est envoyé | `mailbox.rs`, `folders.rs` (`RenameFolder`) |
| Déplacement (`moved_folder_name`) : même nom sous le nouveau parent, emplacement calculé comme pour un nouveau dossier (`destination` : parent connu, non virtuel, avec hiérarchie ; préfixe `INBOX<séparateur>` au premier niveau s'il y a lieu) ; ni dans lui-même ni dans un sous-dossier (`IntoItself`) ; doublon à l'arrivée refusé à la casse près ; déjà à cet endroit : rien n'est envoyé ; un seul `RENAME`, les sous-dossiers suivent | `mailbox.rs`, `folders.rs` (`MoveFolder`) |
| Dossiers protégés (`FolderChangeError::Protected`) : INBOX et tout rôle autre que `Normal` (SPECIAL-USE ou nom reconnu, dossiers virtuels) ; dossier avec sous-dossiers non supprimable (`deletable_folder`, `HasSubfolders`) | `mailbox.rs` |
| Renommage, déplacement et suppression décidés sur la liste relue par `LIST` juste avant `RENAME` / `DELETE` | `folders.rs` (`RenameFolder`, `MoveFolder`, `DeleteFolder`) |
| Une seule commande d'acteur (`FolderChange`) pour créer, renommer, déplacer, supprimer ; dossier renommé ou déplacé et ses sous-dossiers resynchronisés aussitôt (le cache local de l'ancien nom est oublié avec la liste, `replace_mailboxes`) | `account_actor.rs` |
| Actions en attente (file `pending_ops`) qui visent un dossier renommé ou déplacé, en source ou en destination, lui ou un sous-dossier : noms réécrits juste après le `RENAME` (`PendingOp::follow_folder`, `MailboxName::relocated`, cas d'usage `FollowFolder`, port `PendingOpStore::relocate`), sinon le rejeu ne trouve plus le dossier et abandonne l'action | `crates/domain/src/actions.rs`, `crates/application/src/actions.rs`, `account_actor.rs`, `crates/adapters/sqlite/src/actions.rs` |
| Boîte sélectionnée quittée avant `RENAME` / `DELETE` (`UNSELECT`, sinon `EXAMINE INBOX`, jamais `CLOSE`) : Dovecot coupe la session si la boîte sélectionnée disparaît | `crates/adapters/imap/src/lib.rs` (`leave_selected`) |
| Ligne d'état (`OK`/`NO`/`BAD`/`BYE`/`PREAUTH`) sans littéral : octets non ASCII remplacés par `?` avant l'analyse (Dovecot cite le nom de la boîte en UTF-8 dans un refus) | `crates/adapters/imap/src/connection.rs` (`ascii_status_text`) |
| Commandes IPC `rename_folder`, `move_folder`, `delete_folder` : compte, dossier et destination revalidés (dossiers du compte), nom vérifié seul avant l'envoi, 30 s au plus (`FOLDER_CHANGE_TIMEOUT`), code d'échec propre à l'opération | `app/src-tauri/src/folders.rs`, `app/src-tauri/src/mail.rs` |
| Confirmation de suppression sans nombre de messages ni « vide » : le compte local peut être incomplet (dossier pas encore synchronisé) | `app/src/components/DeleteFolderDialog.svelte` |
| Emplacements proposés au déplacement : ceux d'un nouveau dossier, sans le dossier ni ses sous-dossiers ; emplacement actuel présélectionné, **Déplacer** grisé tant qu'il est inchangé ; hiérarchie lue dans les noms, la liste suivant l'ordre des rôles (`destinationChoices`, `MoveFolderState`, affichage seulement : la règle est dans le domaine) | `app/src/lib/folder-edit.svelte.ts` |
| Glisser-déposer d'un dossier : même geste que les conversations (`DragMove`, charge `kind: "folder"`, `folderPayload` : dossiers ordinaires seulement) ; cibles : un dossier du même compte, non virtuel, ni le dossier ni ses sous-dossiers, pas son parent actuel, ou la ligne du compte pour le premier niveau (`DropTarget.folder` à `null`, `data-drop-account`) (`canDrop`, affichage seulement : `move_folder` revalide) ; dépôt : `api.moveFolder` puis même suite que la fenêtre (`dropFolder`, `afterFolderChange`) | `app/src/lib/drag-move.svelte.ts`, `Sidebar.svelte`, `App.svelte` |
| Menu « ⋯ » proposé sur les seuls dossiers ordinaires (`isEditableFolder`, affichage seulement : la règle est dans le domaine) ; cible des commandes : dossier du menu, sinon dossier affiché (`AppContext.folder`) | `app/src/lib/folder-edit.svelte.ts`, `app/src/lib/app.svelte.ts` |

Port : `RemoteMailboxSession::create_mailbox`, `rename_mailbox`,
`delete_mailbox` ; contrat
`folders_session_contract` (`crates/test-support/src/session_contract.rs`),
(dont le déplacement sous un autre parent, sous-dossier et message
compris), joué contre le faux serveur et contre Dovecot
(`crates/adapters/imap/tests/integration_sync.rs`).

Accès à la souris : menu « ⋯ » d'un compte (`Sidebar.svelte`), fenêtre
`NewFolderDialog.svelte` ; menu « ⋯ » d'un dossier (`Sidebar.svelte`, au
survol, au focus et sur le dossier affiché), fenêtres
`RenameFolderDialog.svelte`, `MoveFolderDialog.svelte` et
`DeleteFolderDialog.svelte`.

### Clés des messages

`cmd_new_folder`, `cmd_new_folder_keywords`, `new_folder_title`, `new_folder_name`, `new_folder_parent`, `new_folder_top_level`, `new_folder_create`, `new_folder_creating`, `action_folder_created`, `err_folder_name_empty`, `err_folder_name_too_long`, `err_folder_name_invalid`, `err_folder_name_delimiter`, `err_folder_exists`, `err_no_subfolders`, `err_folder_create_failed`, `cmd_rename_folder`, `cmd_rename_folder_keywords`, `cmd_delete_folder`, `cmd_delete_folder_keywords`, `sidebar_folder_menu`, `rename_folder_title`, `rename_folder_name`, `rename_folder_confirm`, `rename_folder_busy`, `action_folder_renamed`, `delete_folder_title`, `delete_folder_explanation`, `delete_folder_irreversible`, `delete_folder_confirm`, `delete_folder_busy`, `action_folder_deleted`, `err_folder_protected`, `err_folder_has_subfolders`, `err_folder_rename_failed`, `err_folder_delete_failed`, `cmd_move_folder`, `cmd_move_folder_keywords`, `move_folder_title`, `move_folder_explanation`, `move_folder_parent`, `move_folder_confirm`, `move_folder_busy`, `action_folder_moved`, `err_folder_into_itself`, `err_folder_move_failed`.

### Commandes

`account.new-folder`, `folder.rename`, `folder.move`, `folder.delete`.

## Rédaction : nouveau message, réponse, transfert

Fiche : [redaction.md](../fonctionnel/redaction.md).

### Règles et code

| Règle | Où |
|---|---|
| Destinataires de « répondre » : `Reply-To` prioritaire, sinon `From` ; « répondre à tous » ajoute À et Cc ; ses propres adresses exclues, doublons retirés, adresses invalides ignorées ; répondre à son propre message vise ses destinataires | `crates/domain/src/compose.rs` (`reply_recipients`) |
| Sujet : toujours `Re:` / `Fwd:`, sans empilement (préfixes étrangers reconnus puis retirés : `RE :`, `AW:`, `TR:`, `SV:`, `Antw:`…) | `compose.rs` (`reply_subject`, `forward_subject`), ADR-011 |
| `In-Reply-To` et `References` (bornées à 20 : racine et plus récents) | `compose.rs` (`reply_threading`, `MAX_REFERENCES`) |
| Citation : en-tête « Le … a écrit : » construit par l'interface dans la langue de l'utilisateur, inséré tel quel par le domaine | `compose.rs` (`quote`), ADR-011 |
| Destinataire saisi ou collé lu en Rust (`Recipient::parse`, commande `parse_recipient`) : « adresse », « Nom <adresse> », nom entre guillemets ; nom imitant une adresse écarté (`Recipient::new`, aussi pour tout brouillon relu) ; l'interface n'en décide pas | `crates/domain/src/compose/draft.rs`, `app/src-tauri/src/compose.rs` |
| Validation avant envoi : au moins un destinataire, 100 au plus ; sujet sans caractère de contrôle, 998 caractères au plus ; corps ≤ 5 Mio ; pièces jointes ≤ 25 Mio au total et 100 au plus | `crates/domain/src/compose/draft.rs` (`check_sendable`) |
| HTML de l'éditeur réassaini en Rust (liste blanche minimale : p, br, gras, italique, barré, liens http/https/mailto, listes, citations) ; version texte dérivée du HTML assaini ; envoi en `multipart/alternative` | `crates/adapters/render/src/compose.rs`, ADR-010 |
| `Bcc` seulement dans les copies locales, jamais dans les octets transmis ; `Message-ID` aléatoire de 128 bits, sans nom de machine | `render/src/compose.rs` |
| Type MIME des pièces jointes recalculé en Rust d'après le nom ; taille réelle revérifiée ; chemins des fichiers jamais transmis par l'interface (jeton de dépôt) | `crates/application/src/compose.rs` (`QueueMessage`) |
| Transfert : pièces relues depuis le stockage local, message entier téléchargé si le corps est partiel ; mêmes bornes que l'envoi | `compose.rs` (`PrepareReply`), port `RemoteBodyFetcher` |

### Clés des messages

`compose_title_new`, `compose_title_reply`, `compose_title_forward`, `compose_attribution`, `compose_forward_intro`, `err_no_recipient`, `err_too_many_recipients`, `err_invalid_subject`, `err_body_too_large`, `err_attachments_too_large`, `err_too_many_attachments`, `compose_queued`.

### Commandes

`compose.new`, `compose.reply`, `compose.reply-all`, `compose.forward`.

### Vérifications et compléments

- Éditeur : Tiptap (ADR-010) ; sélecteur de fichiers ouvert depuis Rust.
- HTML saisi littéralement dans l'éditeur : envoyé comme du texte (test
  e2e `compose.e2e.mjs`).
- Brouillon relu depuis la base : revalidé (`check_storable`).

## Brouillons

Fiche : [brouillons.md](../fonctionnel/brouillons.md).

### Règles et code

| Règle | Où |
|---|---|
| Chaque enregistrement local incrémente la révision (`DraftSync::edited`) | `crates/domain/src/compose/draft.rs` |
| Copie sur le serveur seulement si la révision a changé, **au plus toutes les 30 s** (`REMOTE_SAVE_INTERVAL_SECS`) ; elle remplace l'ancienne | `draft.rs` (`remote_save_due`) |
| Sans UIDPLUS, les brouillons restent locaux (pas de remplacement fiable) | `crates/adapters/imap/src/session.rs` |
| Une ancienne copie n'est supprimée que si la boîte a la même UIDVALIDITY et que le message porte `\Draft` | ADR-002 |
| Copie refusée par le serveur (`NO`) ou impossible à construire : journalisée, nouvel essai une heure plus tard ; les brouillons suivants continuent | `draft.rs` (`remote_refused`, `REMOTE_REFUSED_RETRY_SECS`) |
| Brouillon d'un autre client repris avec son fil (`resumed_threading`) | `crates/domain/src/compose.rs`, `crates/application/src/compose.rs` (`PrepareReply::resume`) |

### Clés des messages

`drafts_title`, `drafts_empty`, `drafts_to`, `drafts_server`, `compose_discard`.

### Commandes

`drafts.open`.

### Vérifications et compléments

- Dossier « Brouillons » disparu ou refusé : la boîte est enregistrée avec
  chaque copie (migration 8), les copies en attente de suppression sont
  oubliées.

## Envoi : file d'envoi, nouvelles tentatives, envoi différé

Fiche : [envoi.md](../fonctionnel/envoi.md).

### Règles et code

États (`crates/domain/src/compose/outbox.rs`) :

```text
Queued ──start──▶ Sending ──succeeded──▶ Sent
  ▲                  │
  └──failed (temporaire, injoignable)
                     └──failed (refus, authentification, trop d'essais)──▶ Failed
```

| Règle | Où |
|---|---|
| Serveur injoignable : essai **non compté**, envoi différé | `outbox.rs` |
| Échec temporaire (SMTP 4xx, coupure) : nouvel essai avec attente croissante (30 s à 1 h), **8 essais** au plus | `outbox.rs` (`MAX_SEND_ATTEMPTS`, `RETRY_BASE_SECS`, `RETRY_MAX_SECS`) |
| Refus définitif (5xx), authentification, sécurité : échec immédiat | `outbox.rs` |
| Envoi interrompu par l'arrêt de l'application (`Sending` au démarrage) ou transmission interrompue sans réponse : **incertain**, jamais renvoyé automatiquement ; l'utilisateur décide | `outbox.rs` (`SendFailure::Uncertain`), `crates/application/src/compose_worker.rs` (`RecoverOutbox`) |
| Envoi différé : un an au plus ; brouillon gardé avec le message ; repris d'un bloc seulement s'il est encore en attente | `outbox.rs` (`MAX_SCHEDULE_SECS`), migration 10 |
| Annuler : seulement si le message n'est pas en cours d'envoi | `app/src-tauri/src/compose.rs` (`cancel_send`) |
| Connexion SMTP : TLS implicite (465) ou STARTTLS exigé (587), TLS négocié par `norelly-tls` | ADR-002, `crates/adapters/smtp` |
| Copie dans Envoyés : APPEND, sauf serveurs qui la font eux-mêmes (Gmail, `X-GM-EXT-1`) | `crates/application/src/compose.rs` (`FileSentCopies`) |

### Clés des messages

`outbox_title`, `outbox_empty`, `outbox_state_queued`, `outbox_state_sending`, `outbox_state_sent`, `outbox_state_scheduled`, `compose_send_failed`, `send_failure_*`, `outbox_retry`, `outbox_cancel`, `outbox_edit`, `compose_later`, `compose_schedule`, `compose_scheduled`, `err_schedule_too_far`, `err_outbox_busy`.

### Commandes

`compose.outbox`.

## Recherche plein texte

Fiche : [recherche.md](../fonctionnel/recherche.md).

### Règles et code

Syntaxe (`crates/domain/src/search.rs`, `parse_query`) :

| Syntaxe | Effet |
|---|---|
| mots libres | cherchés partout (sujet, expéditeur, destinataires, texte), **en préfixe** à partir de deux lettres (`fact` trouve « facture ») |
| `"phrase exacte"` | expression ; guillemet final facultatif |
| `from:`, `to:` (À et Cc), `subject:` | sur un champ, suivis d'un mot ou d'une phrase |
| `has:attachment`, `is:unread`, `is:read`, `is:flagged` | filtres |
| `before:AAAA-MM-JJ`, `after:AAAA-MM-JJ` | `before` exclut le jour, `after` l'inclut ; début du jour selon le fuseau de l'utilisateur |
| `-clause` | exclusion |

| Règle | Où |
|---|---|
| Clauses combinées par « et » ; opérateur inconnu ou valeur invalide (`is:bleu`, `before:hier`) cherchés comme du texte : une faute de frappe ne fait jamais échouer | `search.rs` |
| Requête bornée : 512 caractères, 16 clauses | `search.rs` (`MAX_QUERY_CHARS`, `MAX_CLAUSES`) |
| Casse et diacritiques ignorés (pliage aligné sur `unicode61 remove_diacritics 2`) ; surlignage en segments de texte, jamais en HTML | `search.rs` |
| Chaque clause devient une chaîne FTS5 entre guillemets, en paramètre lié : aucun mot de l'utilisateur n'est lu comme un opérateur | `crates/adapters/sqlite/src/search.rs` |
| Classement BM25 pondéré (sujet 10, expéditeur 5, destinataires 3, texte 1) ; 1 000 résultats au plus ; doublons de `\All` (Gmail) écartés | `search.rs` (sqlite), ADR-003 |
| Texte indexé : partie texte, sinon HTML réassaini réduit au texte, pièces jointes exclues, 100 000 caractères au plus | `crates/adapters/render/src/search_text.rs` |
| Une recherche est interrompue au-delà de 2 s | `crates/adapters/sqlite/src/search.rs` |

### Clés des messages

`search_placeholder`, `search_none`, `search_close`.

### Commandes

`search.open`, `conversation.next`, `conversation.previous`, `conversation.open`, `conversation.back`.

### Vérifications et compléments

- Requêtes hostiles (`OR`, `NEAR`, `col:`, `^`, `*`, parenthèses…) :
  cherchées comme du texte (tests de contrat).
- Requête trop longue : `err_invalid_input`, champ `field_query`.
- Mesure (bench, 50 000 messages de 300 mots) : 0,3 à 89 ms selon la
  requête, seuil 100 ms.

## Carnet d'adresses : contacts, favoris, écrire à un contact, nom des expéditeurs

Fiche : [contacts.md](../fonctionnel/contacts.md). Décision : ADR-012.

### Règles et code

| Règle | Où |
|---|---|
| Fiche validée (`ContactDraft::new`) : au moins une adresse, 20 adresses et 20 téléphones au plus, adresses distinctes à la casse ASCII près, une seule préférée (la première sinon), champs bornés, sans caractère de contrôle ni de mise en forme ; champ vide : absent | `crates/domain/src/contacts.rs` |
| Nom affiché (`display_label`) : nom affiché, sinon prénom et nom, sinon surnom, sinon adresse préférée ; tri sur ce nom en minuscules (`sort_key`) | `contacts.rs` |
| Une adresse n'appartient qu'à un contact du carnet (`AddressKey`, index unique) : écriture refusée sans effet (`ContactStoreError::DuplicateAddress`) | `contacts.rs`, `crates/adapters/sqlite/src/contacts.rs` |
| Filtre de la liste (`ContactFilter`) : texte borné, sans casse, cherché dans noms, organisation et adresses ; favoris seulement | `contacts.rs`, `sqlite/src/contacts.rs` |
| Sélection reçue de l'interface (`contact_selection`) : identifiants valides, dédoublonnés, 1 000 au plus | `contacts.rs` |
| Curseur reçu de l'interface (`ContactCursor::parse`) : identifiant valide, clé de tri bornée (`MAX_CONTACT_SORT_KEY_CHARS`) | `crates/domain/src/ports.rs` |
| Cas d'usage : la saisie est validée par le domaine avant toute écriture | `crates/application/src/contacts.rs` |
| Commandes IPC `list_contacts`, `get_contact`, `create_contact`, `update_contact`, `delete_contacts` : entrées revalidées, refus traduits en codes (`contact_no_address`, `contact_invalid`, `contact_too_long`, `contact_repeated_address`, `contact_duplicate_address`, `contact_not_found`), adresse refusée citée en `detail` | `app/src-tauri/src/contacts.rs`, `app/src-tauri/src/ipc.rs` |
| État de la vue : pages de 100 par curseur, réponse dépassée ignorée, contacts visés (menu d'une ligne, sinon cochés, sinon fiche ouverte), lignes vides non envoyées, destinataires « Écrire à » (adresse préférée, sans nom si le nom est l'adresse) | `app/src/lib/contacts.svelte.ts`, `app/src/lib/app.svelte.ts` |
| Vue Contacts : commandes du courrier neutralisées (`AppContext` sans conversation visée), retour au courrier par la boîte unifiée ou un dossier | `app/src/lib/app.svelte.ts`, `App.svelte` |
| Actions visibles : barre de la fiche, « Plus… », menu contextuel d'une ligne, barre des contacts cochés, bouton du pied du panneau de gauche | `app/src/lib/visible-actions.ts`, `ContactList.svelte`, `ContactCard.svelte`, `Sidebar.svelte` |
| Saisie affichée en texte, jamais interprétée | `ContactCard.svelte`, `ContactList.svelte` |
| Nom affiché d'un correspondant (`sender_label`) : nom du contact qui porte l'adresse (même `AddressKey`), vide pour une fiche sans nom ; sinon nom de l'en-tête, vide s'il imite une adresse (`imitates_address` : `@`, `<`, `>`) | `crates/domain/src/sender.rs` |
| Adresses cherchées dans le carnet (`sender_keys`) : valides, dédoublonnées dans l'ordre (expéditeurs d'abord), `MAX_CONTACT_LOOKUP` au plus ; carnet non interrogé sans adresse valide | `sender.rs`, `crates/application/src/contacts.rs` (`NameSenders`) |
| Liste des conversations, messages d'une conversation (expéditeur, destinataires) et résultats de recherche nommés d'après le carnet ; DTO `CorrespondentDto.contact` ; sans carnet (notification), nom imitant une adresse écarté | `app/src-tauri/src/reading.rs`, `app/src-tauri/src/search.rs`, `app/src-tauri/src/ipc.rs` |
| Personne visée par « Ajouter au carnet » / « Voir le contact » : celle du menu, sinon l'expéditeur du dernier message ouvert ; fiche préremplie (`ContactFormState.prefill`) ; courrier relu au retour du carnet et après un ajout | `app/src/lib/app.svelte.ts`, `app/src/lib/contacts.svelte.ts`, `App.svelte` |
| Actions visibles : nom cliquable de l'expéditeur et des destinataires, pastille du carnet, menu contextuel d'une ligne, « Plus… » du message ouvert | `PersonMenu.svelte`, `ConversationView.svelte`, `ConversationList.svelte`, `visible-actions.ts` (`personActions`) |
| Nom d'une fiche qui imite une adresse écarté à l'affichage (`contact_name`), comme un nom d'en-tête | `sender.rs` |
| Suggestions de destinataires (`recipient_suggestions`) : contacts d'abord (favoris, puis ordre du carnet), nom du carnet ; trouvé par un nom, toutes ses adresses (la préférée d'abord), sinon les adresses qui contiennent la saisie ; puis adresses collectées ; une fois par `AddressKey`, le carnet l'emporte ; adresses invalides écartées ; `MAX_CONTACT_SUGGESTIONS` au plus | `crates/domain/src/suggestions.rs` |
| Cas d'usage `SuggestRecipients` : carnet lu par `ContactDirectory::page` (favoris, puis tous) puis fiche par fiche, sans jointure SQL avec les messages (ADR-012, écart du lot 4) ; carnet illisible : adresses collectées seules | `crates/application/src/contacts.rs` |
| Commande `suggest_contacts` : DTO `RecipientSuggestionDto.contact` ; pastille « Dans vos contacts » (icône et texte, pas la couleur seule) | `app/src-tauri/src/compose.rs`, `app/src-tauri/src/ipc.rs`, `RecipientInput.svelte` |
| Liste de diffusion validée (`ContactListDraft::new`) : nom obligatoire, borné, sans caractère de contrôle ni de mise en forme ; description d'une ligne, bornée ; membres = contacts (identifiants valides), chacun une fois, `MAX_CONTACT_LIST_MEMBERS` (100, la limite d'un envoi) au plus ; liste vide permise | `crates/domain/src/contact_lists.rs` |
| Nom unique sans la casse (`name_key`, minuscules Unicode) : écriture refusée sans effet (`ContactListStoreError::DuplicateName`) ; membre absent du carnet refusé (`UnknownContact`) ; supprimer un contact le retire de ses listes, supprimer une liste garde ses contacts | `contact_lists.rs`, `crates/adapters/sqlite/src/contact_lists.rs`, `0012_contact_lists.sql` |
| Expansion (`expand`) : chaque membre encore au carnet, à son **adresse préférée du moment**, sous son nom du carnet (vide s'il n'en a pas ou s'il imite une adresse) ; une fois par `AddressKey` ; liste sans destinataire refusée (`Empty`) ; choix « en Cci » gardé | `contact_lists.rs` |
| Ajout de contacts à une liste (`with_members`) : absents ajoutés à la suite, borne respectée | `contact_lists.rs`, `crates/application/src/contact_lists.rs` (`AddToContactList`) |
| Listes proposées à la saisie (`list_suggestions`) : nom qui contient la saisie, listes vides écartées, `MAX_LIST_SUGGESTIONS` (5) au plus | `contact_lists.rs`, `SuggestContactLists` |
| Commandes IPC `list_contact_lists`, `get_contact_list`, `create_contact_list`, `update_contact_list`, `delete_contact_list`, `add_to_contact_list`, `expand_contact_list`, `suggest_contact_lists` : entrées revalidées (`ContactListId`, `contact_selection`, `ContactListCursor::parse`), refus traduits en codes (`contact_list_no_name`, `contact_list_invalid`, `contact_list_too_long`, `contact_list_duplicate_name`, `contact_list_too_many_members`, `contact_list_empty`, `contact_list_not_found`, `contact_not_found`) | `app/src-tauri/src/contact_lists.rs`, `app/src-tauri/src/ipc.rs` |
| Liste choisie à la rédaction : dépliée par Rust (`expand_contact_list`), qui décide aussi du champ (`Expansion::field` : la copie cachée pour une liste « en Cci », quel que soit le champ de saisie) ; chaque membre relu comme une saisie (`parse_recipient`), adresse déjà présente ignorée ; rien n'est ajouté si le brouillon a été refermé ou remplacé entre-temps | `crates/domain/src/contact_lists.rs`, `app/src-tauri/src/contact_lists.rs`, `app/src/lib/compose.svelte.ts` (`addList`), `RecipientInput.svelte` |
| Section « Listes » de la vue Contacts : 3e filtre, liste ouverte à droite, commandes des contacts neutralisées pendant ce temps ; listes visées : menu d'une ligne, sinon liste ouverte | `app/src/lib/contact-lists.svelte.ts`, `app/src/lib/app.svelte.ts`, `ContactList.svelte`, `ContactListCard.svelte` |
| Actions visibles : bouton « Nouvelle liste », barre de la liste ouverte, menu contextuel d'une ligne, « Ajouter à une liste… » dans « Plus… » de la fiche, le menu d'un contact et la barre des contacts cochés | `visible-actions.ts` (`contactListToolbar`, `contactListMenuActions`), `ContactListDialog.svelte`, `AddToListDialog.svelte`, `DeleteContactListDialog.svelte` |
| Import, lecture du fichier (`ContactCodec`, adaptateur `contacts-io`) : vCard 2.1/3.0/4.0 écrit pour le projet (dépliage, sauts souples quoted-printable, `CHARSET`, `ENCODING`, échappements, groupes `item1.`, cartes imbriquées ignorées, `KIND:group` → liste), CSV Google/Outlook/Thunderbird par la crate `csv` (format reconnu à l'en-tête, séparateur `,` `;` tabulation, BOM, repli Windows-1252, UTF-16), LDIF (repli, commentaires, base64, URL ignorées, `groupOfNames` → liste) ; format reconnu au contenu | `crates/adapters/contacts-io/src/{lib,vcard,tabular,ldif,text}.rs` |
| Entrées bornées : fichier `MAX_IMPORT_FILE_BYTES` (20 Mio, vérifié avant lecture et lecture bornée), `MAX_IMPORT_CARDS` (25 000) fiches, champ lu `MAX_FIELD_BYTES` (64 Kio), 100 adresses ou téléphones lus par fiche ; au-delà fichier refusé ou fiche illisible, jamais tronqué ; propriété ignorée (photo) jamais décodée | `crates/domain/src/contact_import.rs`, `contacts-io/src/lib.rs`, `app/src-tauri/src/contact_import.rs` (`read_contact_file`) |
| Plan d'import (`plan_import`) : textes en NFC (port `TextNormalizer`, `IcuNfc`), nom imitant une adresse retiré (fiche gardée, comptée), adresses vides ou répétées écartées, une seule préférée, caractères invisibles (Cf, Zl, Zp, sélecteurs de variante, remplisseurs) refusés, fiche revalidée (`ContactDraft::new`) sinon rejetée avec sa raison et un repère nettoyé et borné ; doublons par `AddressKey` contre le carnet et entre fiches du fichier ; « ignorer » ou « compléter » (`complete_contact` : champs vides remplis, adresses et téléphones manquants ajoutés, rien d'écrasé, adresse d'un autre contact jamais prise, limites d'une fiche respectées) | `crates/domain/src/contact_import.rs` |
| Import en une transaction (`ContactBook::import`) : tout ou rien, adresse prise ou contact inconnu annule tout | `crates/adapters/sqlite/src/contacts.rs`, `crates/test-support/src/contacts_contracts.rs` (`contact_import_contract`) |
| Cas d'usage : aperçu (deux politiques chiffrées, contacts du carnet lus par `by_addresses` par paquets de `MAX_CONTACT_LOOKUP`), import qui recalcule le plan contre le carnet du moment | `crates/application/src/contact_import.rs` |
| Commandes IPC `preview_contact_import` (sélecteur natif, fichier lu en Rust, aperçu chiffré et jeton) et `apply_contact_import` (jeton à usage unique, rendu après un échec) ; codes `contact_import_too_large`, `contact_import_too_many_cards`, `contact_import_unknown_format`, `contact_import_read_failed`, `contact_import_expired` ; 100 rejets détaillés au plus, nom de fichier nettoyé | `app/src-tauri/src/contact_import.rs`, `app/src-tauri/src/ipc.rs` |
| Export, règles de sortie (`export_card`) : adresse préférée en tête ; `FN` vCard = nom affiché, vide s'il se replierait sur l'adresse ou dépasserait la longueur d'un nom ; étiquettes vCard en minuscules, séparateurs de la syntaxe remplacés, mots réservés (`pref`, `internet`, `x-…`, encodages) écartés ; étiquettes Google sans « * » de tête ; téléphones LDIF rangés en `work`/`home`/`cell`/`fax`/`pager` ; favori non exporté ; neutralisation des cellules CSV (`neutralize_csv_cell` : apostrophe devant `=`, `+`, `-`, `@`, tabulation, CR, apostrophes de tête comprises) et son inverse à l'import (`restore_csv_cell`) ; sélection revalidée (`ContactSelection::of`, non vide, `MAX_CONTACT_SELECTION`) ; bornes de l'import (`MAX_EXPORT_CONTACTS`, `MAX_EXPORT_FILE_BYTES`) | `crates/domain/src/contact_export.rs` |
| Écriture des fichiers (`ContactCodec::encode`) : vCard 4.0/3.0 (échappements `\\` `,` `;` saut de ligne, lignes repliées à 75 octets sans couper un caractère, CRLF, `PREF=1` ou `TYPE=PREF`), CSV Google (BOM UTF-8, CRLF, colonnes `Name`… `E-mail n - Label/Value`, `Phone n - …`, crate `csv`), LDIF Thunderbird (`dn` échappé RFC 4514, base64 hors `SAFE-STRING`, lignes repliées à 76 octets) ; sortie bornée sans rien produire au-delà | `crates/adapters/contacts-io/src/encode.rs`, `encode/{vcard,csv,ldif}.rs` |
| Lecture en bloc des fiches (`ContactDirectory::contacts`, tout ou sélection, ordre de la liste, `limit`) | `crates/adapters/sqlite/src/contacts.rs`, `crates/test-support/src/contacts_contracts.rs` (`contact_selection_contract`) |
| Cas d'usage `PrepareContactExport` (vide refusé, au-delà de 25 000 refusé) ; commande IPC `export_contacts` : sélection revalidée, fichier produit hors des fils asynchrones avant le sélecteur d'enregistrement natif, chemin choisi par le système seulement, écriture atomique (`write_atomic` : temporaire voisin créé en exclusivité, synchronisé, renommé, lien remplacé sans être suivi, borne revérifiée) ; codes `contact_export_empty`, `contact_export_too_large`, `write_failed` | `crates/application/src/contact_export.rs`, `app/src-tauri/src/contact_export.rs`, `app/src-tauri/src/ipc.rs` |
| Interface : « Exporter… » dans l'en-tête de la vue (replié dans « Plus… » le premier) et dans la barre des contacts cochés ; fenêtre du format (vCard 4.0 par défaut, format repris la fois suivante), enregistrement annulé = fenêtre gardée, erreur gardée pour réessayer, message du nombre exporté | `app/src/lib/contact-export.svelte.ts`, `ContactExportDialog.svelte`, `visible-actions.ts` (`contactsHeaderActions`, `contactBulkActions`) |
| Interface : « Importer… » dans l'en-tête de la vue (repli dans « Plus… ») et dans le carnet vide ; aperçu (fichier, format, ajoutés, complétés, doublons, rejetés et leur raison, noms retirés), choix du sort des doublons proposé seulement s'il y en a, import impossible sans rien à écrire, erreur gardée pour réessayer | `app/src/lib/contact-import.svelte.ts`, `ContactImportDialog.svelte`, `ContactList.svelte`, `visible-actions.ts` (`contactsHeaderActions`) |
| Doublons (`find_duplicates`) : même nom (`name_key` : nom affiché en NFC par le port `TextNormalizer`, minuscules, mots triés ; fiche sans nom ou nom imitant une adresse écartés) ou même numéro (`phone_key`, `MIN_DUPLICATE_PHONE_DIGITS` = 6 chiffres), groupes par transitivité (union-find), raisons du groupe ; fiche proposée = la plus complète (`completeness`), puis favorite, puis la plus ancienne ; groupes dans l'ordre du nom de leur première fiche ; au plus `MAX_DUPLICATE_GROUPS` (500) groupes de `MAX_DUPLICATE_GROUP_SIZE` (20) fiches, les plus complètes, groupe signalé tronqué | `crates/domain/src/contact_duplicates.rs` |
| Fusion (`merge_contacts`) : champs vides de la fiche gardée complétés dans l'ordre, rien d'écrasé ; adresses réunies (préférée de la fiche gardée), téléphones réunis au chiffre près, notes distinctes bout à bout, favori si l'une l'est ; bornes d'une fiche revérifiées (`ContactDraft::new`) ; demande revalidée (`MergeSelection` : identifiants valides, 1 à `MAX_MERGED_CONTACTS` fiches absorbées distinctes, sans la fiche gardée) | `crates/domain/src/contact_duplicates.rs` |
| Fusion en une transaction (`ContactBook::merge`) : fiches relues et comparées à celles qui ont servi au calcul (changée entre-temps : `Conflict`, rien n'est écrit), fiche gardée réécrite, listes de diffusion reportées (à la place de la fiche absorbée, sans doublon de membre), fiches absorbées supprimées ; fiche inconnue ou adresse d'un contact étranger à la fusion : rien n'est écrit | `crates/adapters/sqlite/src/contacts.rs`, `crates/test-support/src/contact_lists_contracts.rs` (`contact_merge_contract`) |
| Cas d'usage `FindDuplicates` (carnet lu d'un bloc, au-delà de `MAX_DUPLICATE_SCAN` = 25 000 refusé), `PreviewMerge` (fusion calculée sans écriture) et `MergeContacts` (fiches relues, fusion recalculée par le domaine) ; commandes IPC `find_contact_duplicates` (fiches réduites aux champs affichés, sans les notes), `preview_contact_merge` (nom et adresse préférée gardés), `merge_contacts` (l'interface ne désigne que des identifiants) ; codes `contact_duplicates_too_many`, `contact_merge_too_large`, `contact_merge_conflict`, `contact_not_found` | `crates/application/src/contact_duplicates.rs`, `app/src-tauri/src/contact_duplicates.rs`, `app/src-tauri/src/ipc.rs` |
| Interface : « Rechercher les doublons… » toujours dans « Plus… » de l'en-tête de la vue ; fenêtre « Doublons possibles », un groupe à la fois (« Groupe n sur N », raisons), fiche gardée choisie (cochée d'office), fiches à réunir cochées, aperçu « Après la fusion » (nom, adresse préférée) redemandé à chaque changement, réponse périmée ignorée, « Fusionner » (au moins deux fiches et un aperçu valide), groupe tronqué signalé, « Passer », erreur gardée pour réessayer, message du nombre de groupes fusionnés | `app/src/lib/contact-duplicates.svelte.ts`, `ContactDuplicatesDialog.svelte`, `visible-actions.ts` (`contactsHeaderActions`) |

### Clés des messages

`contacts_*`, `contact_*`, `delete_contacts_*`, `err_contact_*`,
`field_contact*`, `field_given_name`, `field_family_name`,
`field_nickname`, `field_organization`, `field_notes`, `sidebar_contacts`,
`key_delete`, `cmd_person_*`, `person_menu`, `person_in_contacts`,
`conversation_to_label`, `conversation_message_toggle`,
`compose_suggestions`, `compose_suggestion_contact`, `compose_suggestion_list`,
`compose_list_option`, `lists_*`, `list_*`, `add_to_list_*`,
`delete_list_*`, `err_contact_list_*`, `field_contact_list`,
`field_list_name`, `field_list_description`, `contacts_filter_lists`,
`contacts_import_*`, `contacts_header_actions`, `cmd_contacts_import*`,
`err_contact_import_*`, `contacts_export_*`, `cmd_contacts_export*`,
`err_contact_export_*`, `contacts_duplicates_*`,
`cmd_contacts_find_duplicates*`, `err_contact_duplicates_*`,
`err_contact_merge_*`.

### Commandes

`contacts.open`, `contacts.new`, `contacts.edit`, `contacts.write`,
`contacts.favorite`, `contacts.delete`, `contacts.clear-selection`,
`contacts.add-person`, `contacts.show-person`, `contacts.add-to-list`,
`lists.new`, `lists.edit`, `lists.write`, `lists.delete`, `contacts.import`,
`contacts.export`, `contacts.export-selected`, `contacts.find-duplicates`.

### Vérifications et compléments

- Contrats du carnet joués contre le faux et SQLite
  (`crates/test-support/src/contacts_contracts.rs`), dont la recherche sur
  chaque nom et toutes les adresses d'une fiche, et l'import tout ou rien.
- Contrats de la normalisation NFC et de la lecture des fichiers joués
  contre les faux et `contacts-io`
  (`crates/test-support/src/contact_import_contracts.rs`) ; jeux d'essai
  des exports Gmail (CSV, vCard), Outlook (CSV anglais et français, vCard
  2.1 en Windows-1252), Apple (vCard 3.0) et Thunderbird (CSV, LDIF)
  (`crates/adapters/contacts-io/tests/`) ; propriétés (proptest) et cible
  de fuzzing `contact_import` (lecture puis plan).
- Contrat de l'écriture des fichiers joué contre le faux et `contacts-io`
  (`contact_encoding_contract` : relu à l'identique dans chaque format,
  trop de fiches ou fichier trop gros refusés) ; aller-retour export puis
  import dans un carnet vide par format, favori non exporté, propriétés
  (relecture à l'identique, réexport stable d'un carnet valide, une
  adresse n'appartenant qu'à un contact) dans
  `crates/adapters/contacts-io/tests/roundtrip.rs` ; cible de fuzzing
  `contact_export` (fichier quelconque lu, fiches validées, exportées et
  relues à l'identique).
- Scénario de bout en bout des doublons
  (`app/e2e/specs/contacts-duplicates.e2e.mjs`) : deux homonymes (prénom
  et nom inversés) et un contact au même téléphone, « Rechercher les
  doublons… » dans « Plus… », fiche la plus complète gardée, fiche
  décochée laissée de côté, fusion, puis nouvelle recherche.
- Scénario de bout en bout de l'export
  (`app/e2e/specs/contacts-export.e2e.mjs`) : tout le carnet en vCard 4.0
  depuis l'en-tête, contact coché en CSV depuis la barre de sélection,
  fichier écrit vérifié (CRLF, 75 octets, BOM, « +33 » neutralisé), puis
  réimport qui redonne les mêmes contacts.
- Scénario de bout en bout de l'import (`app/e2e/specs/contacts-import.e2e.mjs`) :
  `.vcf` puis CSV Gmail choisis dans le sélecteur natif, doublons
  reconnus, fiche sans adresse rejetée avec sa raison.
- Charges d'injection SQL dans tous les champs d'une fiche, comme filtre
  et comme saisie d'un destinataire
  (`crates/application/tests/hostile_message_sql.rs`).
- Commandes IPC sur l'application factice : aller-retour complet et
  entrées refusées (`app/src-tauri/src/command_tests.rs`).
- Scénario de bout en bout à la souris (`app/e2e/specs/contacts.e2e.mjs`),
  dont le nom du carnet dans la liste et le message, un nom d'en-tête qui
  imite une adresse, « Voir le contact », « Ajouter au carnet
  d'adresses » et les suggestions de destinataires (contact d'abord, une
  fois par adresse, destinataire collé imitant une adresse).
- Contrat des listes de diffusion joué contre le faux et SQLite
  (`crates/test-support/src/contact_lists_contracts.rs`) : nom unique sans
  la casse, membres inconnus refusés, contact supprimé retiré de ses
  listes, pagination ; schéma et lignes corrompues
  (`crates/adapters/sqlite/src/contact_lists.rs`).
- Scénario de bout en bout des listes (`app/e2e/specs/contact-lists.e2e.mjs`) :
  création à la souris, nom en double refusé, « Ajouter à une liste… »,
  « Écrire à la liste », liste proposée à la saisie, dépliée en Cci et
  envoyée vers Mailpit (membres en copie cachée seulement), suppression.
- Nom des correspondants sur l'application factice : liste, conversation,
  recherche (`command_tests.rs`, `senders_in_the_book_are_named_from_their_card`).

## Notifications des nouveaux messages

Fiche : [notifications.md](../fonctionnel/notifications.md).

### Règles et code

| Règle | Où |
|---|---|
| Seuls les messages **non lus** comptent (un message lu ailleurs n'est pas signalé) | `crates/domain/src/notify.rs` |
| Rien à la **première synchronisation** d'une boîte | `notify.rs` |
| Arrivées d'une synchronisation **regroupées** : un nombre et, au plus, le message le plus récent | `notify.rs` |
| Confidentialité : contenu retiré **en Rust** (`NewMail::without_content`) avant d'atteindre l'interface | `notify.rs`, `app/src-tauri/src/settings.rs` (`new_mail_event`) |
| Texte composé par l'interface dans la langue de l'utilisateur, revalidé par la commande `show_notification` (balisage freedesktop neutralisé) | ADR-011, `app/src-tauri/src/settings.rs` |
| Affichage par `notify-rust` (sans zbus) depuis Rust seulement | dérogation à consigner |

### Clés des messages

`notify_new_one`, `notify_new_many`, `notify_latest`, `settings_notifications`, `settings_privacy`.

### Vérifications et compléments

- Balisage dans le sujet ou l'expéditeur : neutralisé.
- Affichage réel vérifié à la main (`docs/MANUAL-TESTS.md`).

## Paramètres, langue et mises à jour

Fiche : [parametres.md](../fonctionnel/parametres.md).

### Règles et code

| Règle | Où |
|---|---|
| Recherche de mises à jour : demandée au premier lancement, « oui » présélectionné (`UpdateCheck::PROPOSED`), modifiable | `crates/domain/src/settings.rs`, ADR-006 |
| Vérification au plus une fois par jour, sans identifiant envoyé ; mise à jour signée (minisign) vérifiée avant installation | `settings.rs` (`UPDATE_CHECK_INTERVAL_SECS`), `app/src-tauri/src/updates.rs` |
| Module de mise à jour compilé seulement avec la fonctionnalité `updater` (NSIS) | ADR-006, `app/src-tauri/Cargo.toml` |
| Seuil de rapatriement entre 64 Kio et 64 Mio ; valeur illisible ou hors bornes remplacée par la valeur par défaut | `settings.rs`, port `SettingsStore` |
| Langue : choix de la locale parmi les préférences du système, anglais en repli ; dates, heures, tailles par `Intl` avec la locale active | `app/src/lib/i18n.ts`, ADR-011 |

### Clés des messages

`settings_title`, `settings_restart_note`, `update_question`, `update_available`, `update_install`, `update_later`, `err_update_failed`.

### Commandes

`settings.open`.

## Commandes : souris, clavier, palette, accessibilité

Fiche : [clavier.md](../fonctionnel/clavier.md).

### Règles et code

| Règle | Où |
|---|---|
| Un identifiant ou un raccourci déjà pris est refusé à l'enregistrement : un raccourci ne déclenche jamais deux commandes (`Ctrl+K` et `mod+k` sont le même) | `app/src/lib/commands/registry.ts` |
| `Maj` ignoré pour les caractères qui en dépendent (`?`), pas pour les lettres (`Maj+I` ≠ `i`) | `app/src/lib/commands/shortcut.ts` |
| Le registre ne prend aucune décision de sécurité : une commande appelle l'interface ou une commande IPC revalidée en Rust | `registry.ts` |
| Messages `postMessage` de l'agent validés (source, schéma, taille) | `app/src/lib/agent.ts`, ADR-004 |
| Chaque commande figure dans l'aide-mémoire de la documentation utilisateur, dans les deux langues | `app/src/docs.test.ts` (via `scripts/check-docs.sh`) |
| Infobulle « Libellé — Touches » et état d'un élément visible, tirés du registre | `app/src/lib/commands/describe.ts` |
| Les composants relisent le registre à l'abonnement (commandes enregistrées avant) | `app/src/lib/commands/watch.svelte.ts` |
| Touches de la fenêtre de rédaction et de l'éditeur, affichées dans les infobulles et l'aide | `app/src/lib/fixed-shortcuts.ts` |
| Bouton Fermer de chaque fenêtre, dernier dans l'ordre de tabulation | `app/src/components/Modal.svelte` |
| Bouton « ? », « Nouveau message », menu « ⋯ » des comptes, « Ajouter un compte » et « Paramètres » toujours visibles | `app/src/components/Sidebar.svelte` |

### Clés des messages

`palette_placeholder`, `palette_empty`, `help_title`, `help_palette_hint`, `help_group_compose_window`, `help_send`, `help_close_compose`, `tooltip_shortcut`, `common_close`, `key_shift`, `key_escape`, `key_space`, `key_enter`, `sidebar_account_menu`, `sidebar_sync`, `sidebar_remove`, `sidebar_settings`, `search_button`.


### Commandes

| Groupe | Identifiants (raccourci) |
|---|---|
| Général | `palette.open` (`Ctrl+K`), `help.shortcuts` (`?`), `settings.open` (`Ctrl+,`) |
| Lecture | `conversation.next` (`j`), `conversation.previous` (`k`), `conversation.open` (`Entrée`), `conversation.back` (`Échap`), `search.open` (`/`), `mailbox.unified`, `reading.plain-text` |
| Actions | `action.archive` (`e`), `action.delete` (`#`), `action.flag` (`s`), `action.mark-read` (`Maj+I`), `action.mark-unread` (`Maj+U`), `action.move` (`v`), `action.toggle-mark` (`x`), `selection.clear`, `action.undo` (`z`) |
| Rédaction | `compose.new` (`c`), `compose.reply` (`r`), `compose.reply-all` (`a`), `compose.forward` (`f`), `drafts.open`, `compose.outbox` |
| Comptes | `account.add`, `account.new-folder`, `folder.rename`, `folder.move`, `folder.delete`, `account.archive-folder`, `account.sync`, `account.remove` |

Accessibilité : listes en `listbox`/`option` avec `aria-activedescendant`.

## Vocabulaire : termes fonctionnels et noms dans le code

| Terme fonctionnel | Dans le code |
|---|---|
| Action | `MessageAction`, `PendingOp` |
| Boîte, dossier | `Mailbox` (rôle : attributs SPECIAL-USE, sinon nom) |
| Brouillon | `Draft`, `DraftSync` |
| Compte | `Account` |
| Conversation | algorithme JWZ (`conversation.rs`) |
| Empreinte de certificat | `CertificateFingerprint` |
| Envoi incertain | `SendFailure::Uncertain` |
| File d'actions | table `pending_ops` |
| File d'envoi | `outbox` |
| Rapatriement | `fetch_policy` |
| Suivre | drapeau IMAP `\Flagged` |
| Trousseau | port `SecretStore` |

Termes sans équivalent fonctionnel :

- **Agent** : seul script autorisé dans le cadre d'un message
  (`crates/adapters/render/src/agent.js`) ; il transmet la hauteur, les
  clics sur les liens et les touches à l'interface par `postMessage`.
- **Blob** : message brut (`.eml`) compressé (`.zst`), stocké hors de la
  base, par compte et par empreinte de contenu.
- **Port** : trait du domaine implémenté par un adaptateur (voir
  [l'architecture](architecture.md)).
- **UID, UIDVALIDITY, MODSEQ** : identifiants IMAP d'un message dans une
  boîte, de la « génération » de la boîte et de la version de ses drapeaux.
  Un changement d'UIDVALIDITY oblige à tout resynchroniser.
