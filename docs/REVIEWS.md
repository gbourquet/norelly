# Revues de sécurité

Rapports du sous-agent `security-reviewer`, du plus récent au plus ancien.
Les constats critiques et hauts doivent être corrigés avant de déclarer une
phase terminée. Pour chaque constat, on indique son statut : corrigé (commit),
accepté (justification) ou reporté (phase).

---

# Revue sécurité : phase 3, comptes, autodécouverte et interface (2026-11-02)

Périmètre : `git diff afb655e..HEAD` et les fichiers e2e non commités au
moment de la revue (domaine, `discovery`, `sqlite`, `blobs`, `application`,
`app/src-tauri`, `app/src`, `app/e2e`, CI, `deny.toml`). Revue faite par
l'agent `security-reviewer`, en lecture seule.

Synthèse : 0 critique, 1 haute, 1 moyenne, 6 basses, 8 infos. La haute et
la moyenne sont corrigées, ainsi que cinq basses ; le reste est accepté ou
reporté ci-dessous.

## [HAUTE] Serveur SRV hors domaine non signalé si la découverte est partielle ; confirmation non imposée par Rust
- Problème : l'interface testait la complétude avant `requires_confirmation`
  (avertissement perdu si un seul protocole était trouvé), le message ne
  nommait pas l'hôte, et `add_account` ne distinguait pas un serveur
  confirmé d'un serveur repris de la découverte.
- Scénario : réponse DNS falsifiée (`_imaps._tcp`) vers un serveur au
  certificat valide : le mot de passe part chez l'attaquant sans alerte.
- **Statut : corrigé.**
  - Rust (`app/src-tauri/src/state.rs`, `commands.rs`) : les hôtes à
    confirmer présentés par `discover_settings` sont mémorisés ;
    `add_account` renvoie `confirmation_required` sans aucune connexion
    tant que l'hôte ne figure pas dans `confirmed_hosts`. Tests
    `unconfirmed_hosts_block_until_confirmed` et, côté application,
    `srv_outside_domain_is_never_used_without_review`.
  - Interface (`add-account.svelte.ts`, `AddAccountDialog.svelte`) :
    avertissement d'abord, nommant l'hôte et la source, case à cocher par
    hôte, envoi bloqué sans confirmation ; tests vitest du cas partiel.

## [MOYENNE] Clé de trousseau commune à deux comptes (alias) : retour arrière et suppression effacent le secret d'un autre compte
- **Statut : corrigé.** Clé propre au compte (`SecretKey::for_account`,
  `account:<id>:<protocole>`) ; `AddAccount` insère le compte d'abord (un
  doublon, y compris par course, échoue sans toucher au trousseau), écrit
  ensuite les secrets et, en cas d'échec, ne retire que ce qu'il a créé
  (secrets puis compte). Identifiants SQLite en `AUTOINCREMENT` (jamais
  réattribués). Tests `aliases_on_the_same_login_keep_separate_secrets`,
  `duplicate_at_insert_leaves_the_keyring_untouched`.

## [BASSE] Garde « seule une empreinte présentée » et filtre `requires_confirmation` non testés
- **Statut : corrigé.** Tests `only_a_presented_rejection_can_be_taken_once`,
  `pending_rejections_are_bounded` (`state.rs`) et
  `srv_outside_domain_is_never_used_without_review`.

## [BASSE] Le test « ne suit pas les redirections HTTP » ne testait aucune redirection
- **Statut : corrigé.** Décision extraite dans `redirect_decision`
  (`discovery/src/http.rs`) et testée (HTTPS → HTTP refusé, chaîne bornée) ;
  l'ancien test est renommé selon ce qu'il vérifie.

## [BASSE] Exceptions de certificat accumulées et conservées après le compte
- **Statut : corrigé en partie.** Accepter une empreinte changée remplace
  les anciennes (`trust_certificate`, test
  `trusting_a_changed_fingerprint_replaces_the_old_one`) et exige une case
  à cocher supplémentaire dans l'interface. **Reporté (phase 4)** : purge
  des exceptions des hôtes qu'aucun compte n'utilise plus, à faire avec la
  gestion des dossiers et la reconnexion (une exception peut servir à
  plusieurs comptes du même hôte).

## [BASSE] Infrastructure e2e : chaîne d'approvisionnement non figée, `curl | bash`
- **Statut : corrigé en partie.** Images Ubuntu, Dovecot et Mailpit figées
  par digest. **Accepté pour l'instant** : NodeSource et rustup installés
  par script (image de test uniquement, jamais distribuée, reconstruite
  seulement quand le Dockerfile change) ; à remplacer par des archives
  vérifiées par SHA-256 avec le durcissement de la CI (phase 10).

## [BASSE] Infrastructure e2e : réseau non isolé, dépôt en écriture, `PATH` préfixé par le cache
- **Statut : corrigé.** Compilation et scénarios dans deux conteneurs : les
  scénarios tournent sur un réseau `--internal` (aucun accès extérieur,
  donc aussi aucune requête vers l'ISPDB) avec le dépôt monté en lecture
  seule ; le cache n'est plus dans le `PATH`. La compilation garde le dépôt
  en écriture (`app/dist`), comme un build local.

## [BASSE] Répertoire de données créé avec les permissions par défaut
- **Statut : corrigé.** Répertoire en 0700 sous Unix, droits restreints s'il
  existait déjà (`create_private_dir`, test `data_dir_is_private`).

## [INFO] e2e : chemin `wdio.conf.ts` erroné
- **Statut : corrigé** (`wdio.conf.mjs`). Le scénario s'exécutait tout de
  même (WebdriverIO résout l'extension), le rapport e2e le montrait.

## [INFO] Commandes IPC non restreintes par capability
- **Reporté (phase 5)**, avant l'arrivée de `mailview://` : déclarer les
  commandes dans le manifeste applicatif et ne les accorder qu'à `main`.

## [INFO] Vie privée de la découverte
- Accepté : l'ISPDB reçoit le domaine, `autoconfig.<domaine>` l'adresse ;
  les variables de proxy sont respectées (TLS de bout en bout). À
  documenter dans l'aide utilisateur (phase 9).

## [INFO] Copies du mot de passe non effacées (JSON IPC, état Svelte)
- Accepté : effacé à `reset()` et après succès ; inévitable côté JS.

## [INFO] `Username` admettait les caractères bidirectionnels
- **Statut : corrigé.** `Username::parse` refuse aussi les caractères de
  mise en forme invisibles (`is_format_char`).

## [INFO] `dangerous()` dans `standard_client_config`, nouvelles dépendances, audit npm
- Sans constat : vérificateur de la plateforme sans exception ; licences
  conformes ; avis `pnpm audit` uniquement en développement (wdio, mocha),
  à suivre.

## Résultats outils
- `cargo deny --locked check` : ok (avertissement `license-not-encountered`
  pour `Unicode-DFS-2016`, sans effet).
- `pnpm -C app audit --prod` : aucune vulnérabilité.

---

# Revue sécurité : phase 2, transport et sécurité de connexion (2026-11-01)

Périmètre : `git diff 1dd5378..HEAD` (domaine, `tls`, `imap`, `smtp`,
`keyring`, `application`, `test-support`, `Cargo.toml`, ADR-002).

Note : l'agent `security-reviewer` n'était pas chargé dans la session (elle
avait démarré dans le dossier parent). La revue a été faite par un agent
généraliste à qui l'on a fait suivre `.claude/agents/security-reviewer.md` à
la lettre, en lecture seule.

Synthèse : 0 critique, 0 haute, 3 moyennes, 4 basses, 5 infos.

Aucune fuite d'identifiants exploitable, aucun contournement TLS et aucun
repli en clair n'a été trouvé.

## [MOYENNE] IMAP : réponses non étiquetées accumulées sans limite, y compris avant STARTTLS
- Emplacement : `crates/adapters/imap/src/connection.rs` (`execute`, `capability`).
- Problème : aucune limite sur le nombre de réponses non étiquetées conservées
  pour une commande ; la limite de 64 Mio vaut par réponse, et s'applique
  aussi avant TLS.
- Scénario : un attaquant actif avant STARTTLS répond au `CAPABILITY` par un
  flot sans fin de `* CAPABILITY …` sans jamais `OK` ; la mémoire sature.
- Statut : **corrigé** : au plus 10 000 réponses non étiquetées par commande ;
  limite d'une réponse ramenée à 64 Kio avant TLS ; test de flot sans fin.

## [MOYENNE] IMAP : recherche de fin de ligne quadratique
- Emplacement : `crates/adapters/imap/src/connection.rs` (`feed_line`).
- Problème : tout le tampon est reparcouru à chaque bloc reçu ; une ligne de
  64 Mio sans `\n` coûte des dizaines de Gio de comparaisons.
- Statut : **corrigé** : la recherche reprend à la position déjà examinée.

## [MOYENNE] `cargo deny` en échec : RUSTSEC-2026-0370/0371/0372 (zbus 5 non maintenu via `keyring` 4)
- Statut : **corrigé** sans toucher à `deny.toml` : `keyring` remplacé par
  `keyring-core` + `dbus-secret-service-keyring-store` (libdbus du système,
  liée dynamiquement) sous Linux et `windows-native-keyring-store` sous
  Windows. Plus aucune dépendance zbus. La CI installe `libdbus-1-dev`
  (fichier d'infra à relire).

## [BASSE] Pas de délai global par commande (délai d'inactivité par lecture seulement)
- Emplacement : `imap/src/connection.rs`, `smtp/src/stream.rs`.
- Scénario : un octet toutes les 59 s maintient un test de connexion ouvert
  indéfiniment.
- Statut : **reporté (phase 4)**, avec la reconnexion et le backoff : délai
  global par phase (salutation, STARTTLS, authentification) et par commande.

## [BASSE] Effacement partiel des secrets (copies dans imap-types, lettre)
- Statut : **accepté** : effacement « au mieux », documenté dans
  `credentials.rs` ; les copies internes à `imap-types` (`CommandBody::login`,
  réponses SASL) et à `lettre` (`Credentials` clonées) échappent à notre
  contrôle. Piste pour plus tard : LOGIN en littéral depuis un tampon
  `Zeroizing`.

## [BASSE] SMTP via lettre : ligne de réponse lue en entier avant le contrôle de taille
- Statut : **reporté (phase 6, envoi)** : seulement après TLS (serveur
  authentifié ou accepté par exception), borné par le délai de 60 s. Piste :
  compteur d'octets dans `LettreStream::poll_read`, et signalement en amont.

## [BASSE] `server_text` laisse passer les caractères de mise en forme Unicode (U+202E…)
- Statut : **corrigé** : les caractères de mise en forme bidirectionnelle et
  invisibles (U+00AD, U+061C, U+200B–U+200F, U+202A–U+202E, U+2060–U+2069,
  U+FEFF) sont remplacés ; cas ajouté au test.

## [INFO] Plafond `tracing` à DEBUG fragile
- Le test `credentials_never_appear_in_logs_or_debug` échoue si on retire le
  plafond (vérifié). À revérifier à chaque mise à jour d'`imap-next` ;
  défaut à signaler en amont (ADR-002).

## [INFO] SMTP smuggling (LF nu) non géré par lettre
- À traiter en phase 6 (rédaction) : normaliser `OutgoingMessage.raw` en CRLF
  strict avant `send`.

## [INFO] Dépendances nouvelles
- `imap-codec`/`imap-types` en préversion analysent des données hostiles ;
  `nom` ajouté pour un défaut de features. `rustls-platform-verifier` ne
  vérifie pas la révocation sous Linux. `pub use rustls` dans `norelly-tls`
  expose `dangerous()` : à surveiller en relecture.

## [INFO] Tests d'injection dépendants de la segmentation TCP
- Fiables sur la boucle locale (OK et données injectées en un seul `write`).

## [INFO] Exceptions de certificat par hôte, pas par port
- Conforme au cahier des charges ; sans risque (même certificat exact).

## Résultats outils
- cargo deny : échec initial (avis zbus) ; **ok** après correction
  (advisories, bans, licenses, sources).
- pnpm audit : non exécuté (pas d'interface dans cette phase).
