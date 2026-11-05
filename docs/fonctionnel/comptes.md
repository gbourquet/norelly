# Comptes : ajout, configuration, retrait

[← Fonctionnalités](README.md)

## But

Ajouter une adresse électronique avec le moins de saisie possible (adresse
et mot de passe), sans jamais affaiblir la sécurité de la connexion pour y
parvenir ; retirer un compte proprement.

## Comportement attendu

### Ajout

1. L'utilisateur saisit l'**adresse**, le **mot de passe** et,
   facultativement, un **nom affiché**, puis **Continuer**.
2. Norelly cherche les serveurs (progression affichée, source par source).
3. Si IMAP et SMTP sont trouvés par une source sûre, Norelly **teste la
   connexion** (TLS, authentification) aux deux serveurs, puis enregistre le
   compte : le mot de passe va dans le trousseau, le compte dans la base. La
   synchronisation démarre aussitôt.
4. Sinon, ou en cas d'échec, l'écran de **saisie manuelle** s'affiche,
   prérempli avec ce qui a été trouvé : serveur, port, sécurité (TLS,
   STARTTLS, aucune) et utilisateur, pour IMAP et pour SMTP.
5. Le test peut s'arrêter sur un écran de décision :
   - **certificat non reconnu** : raison, serveur, empreinte SHA-256 ;
     « Faire confiance à ce serveur » si le problème le permet ;
   - **connexion non chiffrée** : consentement explicite pour cet hôte et
     ce port ;
   - **aucun trousseau** (Linux sans Secret Service) : consentement pour un
     fichier de mots de passe non chiffré ;
   - **serveur trouvé par le DNS hors du domaine** de l'adresse : case à
     cocher de confirmation.

### Retrait

Depuis l'icône de corbeille du compte (panneau de gauche) ou la commande
« Supprimer le compte sélectionné… », après confirmation : l'acteur du
compte est arrêté, le secret est effacé du trousseau, les données locales
(messages, blobs, files, brouillons, exceptions de certificat) sont
supprimées. **Rien n'est supprimé sur le serveur.**

## Règles métier

| Règle | Où |
|---|---|
| Adresse validée strictement : `dot-atom` RFC 5322, UTF-8 admis dans la partie locale (RFC 6531), domaine validé comme nom d'hôte ; guillemets et IP littérales refusés | `crates/domain/src/address.rs` |
| Ordre d'autodécouverte : autoconfig du fournisseur (HTTPS) → ISPDB de Mozilla (HTTPS, seul le domaine transmis) → SRV RFC 6186 → sondage des ports usuels (`imap.`, `smtp.`, `mail.`, domaine) en TLS implicite ou STARTTLS → saisie manuelle ; arrêt dès que IMAP et SMTP sont trouvés | `crates/domain/src/discovery.rs` |
| Préférence de sécurité : TLS implicite > STARTTLS > clair | `discovery.rs` |
| Jamais de HTTP en clair pour l'autoconfiguration ; serveur SRV hors du domaine de l'adresse : confirmation obligatoire | `discovery.rs` |
| Certificat : valide pour la plateforme, ou exception mémorisée (hôte + empreinte SHA-256) ; sinon rejet avec un `CertificateIssue` détaillé ; une empreinte différente de l'exception est signalée comme changement | `crates/domain/src/security.rs` (`TlsTrustPolicy`) |
| Clair refusé sans consentement explicite par serveur ; STARTTLS obligatoire s'il est annoncé ou demandé (aucun repli silencieux) ; capacités relues après TLS | `security.rs` (`PlaintextPolicy`), `capabilities.rs` |
| Authentification : `AUTHENTICATE PLAIN` ou `LOGIN` selon les capacités, `LOGINDISABLED` respecté ; `XOAUTH2` non pris en charge | `crates/domain/src/auth.rs` |
| Le secret n'est jamais dans `Debug`, n'a pas de `Display`, n'est pas sérialisable | `crates/domain/src/credentials.rs` |
| Le mot de passe n'appartient pas au compte : il vit dans le trousseau, sous des clés propres au compte | `crates/domain/src/account.rs` |
| Fichier de secrets non chiffré (droits 0600) seulement si aucun trousseau ne répond, après consentement (dérogation ADR-002) | `crates/adapters/keyring/src/file.rs` |

Cas d'usage : `DiscoverSettings`, `TestConnection`, `AddAccount`,
`RemoveAccount` (`crates/application/src/accounts.rs`,
`discover_settings.rs`, `test_connection.rs`).

## Cas limites

- Adresse déjà enregistrée : refus (`add_msg_already_exists`).
- Découverte partielle (IMAP sans SMTP, par exemple) : saisie manuelle
  préremplie (`add_msg_incomplete`).
- Certificat non acceptable (révoqué, signature invalide) : pas de bouton
  de confiance (`add_cert_cannot_trust`).
- Changement d'empreinte d'un serveur déjà accepté : avertissement renforcé
  et case « J'ai vérifié auprès de l'administrateur » obligatoire.
- Annulation pendant le test : retour à la saisie (`add_msg_cancelled`).
- Gmail et Outlook.com exigent un mot de passe d'application ; Outlook.com
  peut refuser tout accès par mot de passe (OAuth2 non pris en charge).
- TLS 1.0 et 1.1 non pris en charge (ADR-002) : échec de sécurité.

## Messages affichés

| Clé | Texte (fr) |
|---|---|
| `add_discovering` | Recherche des paramètres de {address}… |
| `add_testing` | Test de la connexion à {imap} et {smtp}… |
| `add_msg_incomplete` | Les serveurs n'ont pas tous été trouvés automatiquement : vérifiez ou complétez les paramètres. |
| `add_msg_dns_outside` | Serveur trouvé par le DNS en dehors du domaine de votre adresse : {hosts}. … |
| `add_cert_title` | Certificat non reconnu |
| `add_issue_*` | raison du rejet (autorité inconnue, expiré, pas encore valide, nom différent, révoqué, signature invalide, illisible, autre) |
| `add_cert_changed` | ce serveur présente un certificat différent de celui que vous aviez accepté. … |
| `add_plain_warning` | La connexion à {host}:{port} ne serait pas chiffrée : … |
| `add_secrets_warning_*` | Aucun trousseau du système … ne répond. … fichier non chiffré … |
| `add_fail_authentication` | {prefix}identifiants refusés. … (un mot de passe d'application est parfois nécessaire). |
| `add_fail_network` | {prefix}serveur injoignable. Vérifiez le nom et le port. |
| `add_fail_security` | {prefix}connexion refusée par sécurité. |
| `add_msg_already_exists` | Un compte existe déjà pour cette adresse. |
| `remove_explanation_*` | … son mot de passe est effacé du trousseau et ses données locales sont supprimées. Les messages restent sur le serveur. |

`{prefix}` vaut « Serveur de réception (IMAP) : » ou « Serveur d'envoi
(SMTP) : ».

## Raccourcis

| Commande | Raccourci |
|---|---|
| `account.add` (Ajouter un compte…) | palette |
| `account.remove` (Supprimer le compte sélectionné…) | palette |
