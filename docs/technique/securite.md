# Sécurité

[← Documentation technique](README.md)

Un client de messagerie lit des données écrites par n'importe qui et parle
à des serveurs à travers des réseaux non fiables. La sécurité est donc le
premier critère de conception : **toute décision de sécurité est prise en
Rust**, l'interface ne fait que présenter (ADR-001, ADR-008).

## Modèle de menace

| Adversaire | Capacité supposée | Objectifs à contrer |
|---|---|---|
| **Expéditeur malveillant** | Envoie un message arbitraire : HTML, CSS, MIME malformé, en-têtes hostiles, noms de fichiers piégés, pièces jointes exécutables | Exécuter du code, atteindre l'IPC, pister la lecture, sonder le réseau local, faire planter ou ralentir l'application, injecter du SQL ou de la syntaxe de recherche, usurper l'interface |
| **Réseau hostile** (Wi-Fi public, FAI) | Intercepte, modifie, rejoue le trafic ; falsifie le DNS | Lire le mot de passe ou les messages (STARTTLS retiré, certificat substitué, injection avant la poignée de main), rediriger l'autodécouverte |
| **Serveur hostile ou défaillant** | Réponses IMAP/SMTP arbitraires, tronquées, énormes | Plantage, boucle, consommation mémoire, corruption de l'état local |
| **Autoconfiguration falsifiée** | Réponse XML ou SRV choisie par l'attaquant | Envoyer le mot de passe à un serveur tiers |
| **Page ou programme local** | Envoie des requêtes à la redirection OAuth locale ; lit l'URL d'autorisation | Faire accepter un code forgé (CSRF), annuler la connexion, voler le code d'autorisation |
| **Chaîne d'approvisionnement** | Dépendance ou action CI compromise, mise à jour substituée | Exécuter du code chez l'utilisateur |

Hors périmètre : un attaquant qui contrôle déjà la session de l'utilisateur
(lecture du dossier de données, du trousseau déverrouillé). La base locale
n'est pas chiffrée (ADR-003) : on s'appuie sur le chiffrement du disque.

## Défenses en place

### Transport (ADR-002)

- TLS par `rustls` (1.2 minimum), vérificateur de la plateforme ; exception
  par **hôte + empreinte SHA-256**, décidée par l'utilisateur ; empreinte
  changée signalée (`crates/domain/src/security.rs`,
  `crates/adapters/tls`).
- STARTTLS **obligatoire** s'il est demandé ou annoncé : aucun repli en
  clair ; données reçues après la réponse à `STARTTLS` et avant la poignée
  de main rejetées ; capacités relues après TLS
  (`crates/adapters/imap/src/connection.rs`, `crates/adapters/smtp`).
- Clair refusé sans consentement explicite par serveur.
- Pilote d'E/S IMAP maison : lignes et littéraux bornés, délais, aucune
  panique sur réponse malformée.
- Niveau `TRACE` supprimé à la compilation : `imap-next` y journaliserait
  les mots de passe ; test `credentials_never_appear_in_logs_or_debug`.
- Secrets : type `Secret` sans `Display`, `Debug` masqué, non sérialisable,
  effacé (`zeroize`) ; stockés dans le trousseau du système.

### Connexion OAuth (ADR-013)

- Jeton d'accès porteur : il n'est envoyé qu'aux serveurs IMAP et SMTP de
  son fournisseur, en TLS ou STARTTLS, jamais en clair, même consenti
  (`crates/domain/src/oauth.rs`, `token_destination_allowed`, vérifié par
  les adaptateurs IMAP et SMTP avant toute authentification) ; un compte
  OAuth n'utilise que XOAUTH2.
- Code d'autorisation protégé par PKCE S256 ; `state` aléatoire
  (128 bits) vérifié avant tout le reste ; retour forgé ou illisible
  ignoré sans annuler la connexion ; redirection écoutée sur
  `127.0.0.1` seulement, port éphémère, requête bornée
  (`crates/adapters/oauth`).
- Compte vérifié : la connexion interactive demande aussi `openid email
  profile` ; l'`id_token` reçu du point d'accès des jetons (HTTPS vérifié,
  en échange du code et du vérificateur PKCE, donc signature non
  vérifiée) doit porter l'adresse du compte en `preferred_username` ou
  `email`, sinon les jetons sont refusés avant tout test, stockage ou
  remplacement, à l'ajout comme à la reconnexion
  (`crates/domain/src/oauth.rs`, `verify_identity`). Un code d'un autre
  compte injecté dans la redirection locale est ainsi refusé. Le
  rafraîchissement garde les seuls scopes de messagerie.
- Identifiant client public embarqué (application de bureau, sans
  secret). Jeton de rafraîchissement dans le trousseau seulement ; jeton
  d'accès en mémoire ; aucun jeton dans les journaux, les erreurs ni
  `Debug` ; le corps des réponses du fournisseur n'est jamais recopié, et
  un code d'erreur n'est repris que s'il est court et imprimable.
- Point d'accès des jetons en HTTPS, vérificateur de la plateforme, sans
  redirection ; rafraîchissement borné à 30 s, un verrou par compte.
- Ajout de compte échoué après la connexion chez le fournisseur : jetons
  gardés en mémoire seulement (jamais dans le trousseau ni sur disque), un
  seul ajout à la fois, rendus une seule fois et pour la même adresse et le
  même fournisseur, dix minutes au plus.

### Autodécouverte

HTTPS uniquement ; seul le domaine est transmis à l'ISPDB ; serveur SRV hors
du domaine de l'adresse soumis à confirmation ; XML hostile sans panique
(fuzzé) (`crates/domain/src/discovery.rs`, `crates/adapters/discovery`).

### Rendu des messages (ADR-004, « A+ »)

- HTML assaini par liste blanche (`ammonia`), CSS assaini (échappements
  résolus, `@import` et `url()` externes retirés) ; suite de 56 vecteurs
  hostiles (`crates/adapters/render/tests/hostile_html.rs`).
- Cadre `sandbox="allow-scripts"` sans `allow-same-origin` (origine opaque),
  servi par `mailview://` avec une CSP stricte ; seul l'agent s'exécute
  (empreinte SHA-256 vérifiée).
- Messages `postMessage` de l'agent validés par l'interface (source, type,
  taille, schéma).
- Images distantes bloquées par défaut ; proxy `remote-img://` sans cookies
  ni `Referer`, adresses publiques seulement, taille et type bornés,
  autorisation par expéditeur revérifiée en Rust à chaque image.
- Liens : `http`, `https`, `mailto` seulement, ouverts par Rust dans le
  navigateur du système ; navigation de la webview bloquée
  (`navigation_guard`).
- Pièces jointes : nom assaini par le domaine, ouverture directe réservée à
  une liste blanche d'extensions, écriture par création exclusive dans le
  dossier choisi.
- Interface : jamais de `{@html}`, d'`innerHTML` ni d'`eval`.

### Données

- SQL : paramètres liés uniquement (`scripts/check-sql.sh`) ; requêtes de
  recherche traduites en chaînes FTS5 citées, en paramètre lié ; tests
  `hostile_message_sql.rs` avec des charges dans chaque champ stocké.
- Tout texte venu du serveur est assaini et borné à l'entrée du domaine
  (`mailbox.rs`, `conversation.rs`).
- Bornes partout : profondeur MIME, nombre de parties, tailles, nombre de
  clauses, de destinataires, de pièces jointes, d'opérations en attente,
  durée d'une recherche (2 s).

### Rédaction

HTML de l'éditeur réassaini en Rust (ADR-010) ; `Bcc` jamais transmis ;
`Message-ID` aléatoire sans nom de machine ; chemins de fichiers jamais
reçus de l'interface ; types MIME recalculés.

### Application Tauri

- Liste de contrôle d'accès : les commandes ne sont accordées qu'à la
  fenêtre `main`, depuis l'origine de l'application (`ipc-commands.rs`,
  `permissions/main-window.toml`, test `acl_tests`).
- Aucune permission de plugin accordée à l'interface : dialogues et mises à
  jour appelés depuis Rust.
- Toute entrée IPC revalidée par les constructeurs du domaine.
- `unsafe` interdit dans tout le workspace.

### Chaîne d'approvisionnement et distribution

- `cargo deny` : licences (ADR-009) et avis RUSTSEC ; exceptions commentées
  et validées par le mainteneur dans `deny.toml`.
- Versions figées côté front ; actions GitHub et images Docker figées par
  empreinte.
- Mises à jour (NSIS) signées **minisign**, signature faite dans un
  environnement de publication séparé de la compilation ; délais bornés
  (`docs/CODE_SIGNING.md`).
- Flatpak : autorisations minimales.

## Fichiers sensibles

Signalés au mainteneur dans chaque PR qui les touche (ADR-008) :

- `crates/domain/src/security.rs`, `discovery.rs`, `reading.rs`,
  `credentials.rs`, `oauth.rs`, `auth.rs` ;
- `crates/adapters/oauth/src/lib.rs`, `crates/application/src/oauth.rs` ;
- `crates/adapters/tls/src/lib.rs`, `crates/adapters/imap/src/connection.rs`,
  `crates/adapters/smtp/src/stream.rs` ;
- `crates/adapters/render/src/sanitize.rs`, `css.rs`, `compose.rs`,
  `agent.js` ;
- `crates/adapters/contacts-io/src/` (lecture et écriture de fichiers
  externes), `crates/domain/src/contact_import.rs`, `contact_export.rs` ;
- `app/src-tauri/src/protocols.rs`, `remote_images.rs`, `reading.rs`
  (pièces jointes), `contact_import.rs` (fichier importé),
  `contact_export.rs` (fichier exporté), `ipc.rs`,
  `capabilities/`, `permissions/` ;
- `app/src/lib/agent.ts` ;
- infrastructure : `deny.toml`, `.github/workflows/*.yml`,
  `packaging/flatpak/*.yml`, `scripts/check.sh`.

## Revues

Toute modification d'une zone sensible passe par une revue de l'agent
`security-reviewer` (`.claude/agents/security-reviewer.md`) ; les constats
critiques et hauts sont corrigés avant la PR, les autres reportés dans le
suivi des sujets ouverts, avec leur gravité.

Signaler une vulnérabilité : voir [SECURITY.md](../../SECURITY.md)
(signalement privé GitHub), jamais dans une issue publique.
