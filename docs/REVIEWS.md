# Revues de sécurité

Rapports du sous-agent `security-reviewer`, du plus récent au plus ancien.
Les constats critiques et hauts doivent être corrigés avant de déclarer une
phase terminée. Pour chaque constat, on indique son statut : corrigé (commit),
accepté (justification) ou reporté (phase).

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
