# Tests

[← Documentation technique](README.md)

Stratégie : ADR-007. Méthode : **TDD obligatoire** (voir
[méthode](methode.md)). Aucun test ne parle à un vrai serveur de
messagerie ni à un vrai DNS : conteneurs, faux serveurs en processus.

## Niveaux

| Niveau | Où | Outils | Lancement |
|---|---|---|---|
| **Unitaires Rust** | modules `#[cfg(test)]`, fichiers `*/tests.rs` | `cargo nextest`, `proptest` | `cargo nextest run --workspace` |
| **Contrats de ports** | `crates/test-support/src/*_contracts.rs` | suites génériques jouées contre le faux en mémoire **et** l'implémentation réelle | dans les tests unitaires (faux, SQLite) et d'intégration (serveurs) |
| **Intégration** (conteneurs) | `crates/*/tests/integration*.rs` | `testcontainers` : Dovecot, Mailpit, Toxiproxy ; Stalwart la nuit | `cargo nextest run --workspace --profile integration` (Docker) |
| **Shell Tauri sans webview** | `app/src-tauri/src/command_tests.rs`, `acl_tests.rs`, `test_app.rs` | `tauri::test` (application factice, état réel sur dossier temporaire) | `cargo nextest run -p norelly-app` |
| **Front** | `app/src/**/*.test.ts` | Vitest, Testing Library, jsdom | `pnpm -C app test` |
| **End-to-end** | `app/e2e/specs/*.e2e.mjs` | tauri-driver + WebdriverIO dans un conteneur Ubuntu 24.04, Xvfb, GNOME Keyring | `pnpm -C app e2e` (Docker) |
| **Captures de la documentation** | `app/e2e/captures/captures.e2e.mjs` | même chaîne que l'e2e, une fois par langue | lancé par `pnpm -C app e2e` |
| **Performance** | `crates/adapters/sqlite/benches/`, `app/e2e/perf/` | `criterion`, scénario e2e mesuré | voir plus bas |
| **Fuzzing** | `fuzz/fuzz_targets/` | `cargo-fuzz` | job nocturne |
| **Documentation** | `scripts/check-docs.*`, `app/src/docs.test.ts` | Node, Vitest | `./scripts/check-docs.sh` |

Profils nextest (`.config/nextest.toml`) : `default` exclut les binaires
`integration*` ; `integration` les lance (sauf Stalwart) ; `stalwart` le
second serveur, dans le job nocturne. Un fichier de test d'intégration
s'appelle toujours `tests/integration*.rs`.

## Faux serveurs et outils de test (`crates/test-support`)

- `fake_imap.rs`, `fake_server.rs` : faux serveurs IMAP et SMTP **scriptés**
  (tokio, étapes « attendre / envoyer », TLS optionnel). Ils reproduisent ce
  qu'un vrai serveur ne fait pas sur commande : STARTTLS retiré, injection
  après `STARTTLS`, `LOGINDISABLED`, réponses malformées ou tronquées,
  `BAD`, délais, capacités masquées (`FakeImap::set_move_extensions`).
- `certs.rs` : certificats `rcgen` (CA de test, autosigné, expiré, mauvais
  nom d'hôte).
- `containers.rs` : Dovecot 2.4.5, Mailpit, Toxiproxy (coupure, latence,
  connexion à moitié fermée), Stalwart ; images figées par empreinte.
- `imap_control.rs` : manipulation directe d'une boîte (APPEND, changement
  d'UIDVALIDITY…).
- `synthetic.rs` : générateur de boîtes synthétiques (50 000 messages).
- `fakes.rs` : implémentations en mémoire des ports.
- `*_contracts.rs` : suites de contrat (`SecretStore`, `AccountRepository`,
  `MailStore`, `BlobStore`, `PendingOpStore`, `DraftStore`, `OutboxStore`,
  `SearchIndex`, session IMAP…).

Contre Dovecot, les capacités sont masquées par le connecteur
(`ImapConnector::hiding_capabilities`) pour tester les chemins sans MOVE ou
sans UIDPLUS. Le trousseau réel (Secret Service) est testé dans le conteneur
des e2e (`integration_secret_service.rs`), jamais sur le poste d'un
développeur.

## End-to-end

`app/e2e/run.sh` construit l'image (`app/e2e/Dockerfile`), compile
l'application dans le conteneur, démarre Dovecot, Mailpit et un **serveur
canari** sur un réseau Docker **interne** (sans accès extérieur), avec des
certificats autosignés neufs, puis lance les scénarios avec un `HOME`
éphémère et la locale `fr`.

| Scénario | Vérifie |
|---|---|
| `add-account.e2e.mjs` | Ajout d'un compte de bout en bout (saisie manuelle, confiance dans les certificats) |
| `reading-security.e2e.mjs` | Zéro requête vers le canari, aucun script du mail exécuté, IPC inaccessible depuis `mailview://`, `javascript:` jamais ouvert, touches relayées |
| `compose.e2e.mjs` | Réponse `r` + Ctrl+Entrée reçue par Mailpit (In-Reply-To, texte et HTML), copie dans Envoyés, Cci hors des en-têtes, HTML saisi jamais interprété |
| `search.e2e.mjs` | Mot du texte trouvé et surligné, échappement, filtre `from:` |
| `captures/captures.e2e.mjs` | Produit les captures de `docs/utilisateur/{fr,en}/captures/` sur un profil neuf par langue (boîte Dovecot dédiée) ; vérifie la langue de l'interface |
| `perf/perf.e2e.mjs` | Démarrage et changement de dossier sur 50 000 + 20 000 messages (`NORELLY_E2E_PERF=1`, build release) |

Les captures ne sont recopiées dans `docs/` que si tous les scénarios ont
réussi. Elles ne se modifient jamais à la main : relancer
`pnpm -C app e2e`, puis commiter les PNG modifiés.

## Couverture et seuils

Vérifiés par `./scripts/check.sh`, **à relever, jamais à baisser** :

| Mesure | Outil | Seuil |
|---|---|---|
| Rust, lignes (tests unitaires) | `cargo llvm-cov nextest --fail-under-lines` | 86 % |
| Front, lignes / instructions / branches / fonctions | `vitest --coverage` (`app/vite.config.ts`) | 81 / 77 / 71 / 74 % |

Le code d'adaptateur entrant couvert surtout par l'intégration et l'e2e
(`app/src-tauri/src/mail.rs`, chemin Secret Service) reste plus faible en
mesure unitaire (PLAN, phase 7).

## Bancs de performance

| Banc | Commande | Seuil |
|---|---|---|
| Liste : première page et changement de dossier, 50 000 messages (requête seule) | `cargo bench -p norelly-sqlite -- --test` | < 50 ms (mesuré ≈ 0,2 ms) |
| Recherche, 50 000 messages de 300 mots | même commande (bancs `message_list` et `search`) | < 100 ms (mesuré 0,3 à 89 ms) |
| Démarrage de bout en bout | `NORELLY_E2E_PERF=1 pnpm -C app e2e` | < 400 ms (seuil porté par le mainteneur, PLAN phase 9) |
| Changement de dossier de bout en bout | idem | < 200 ms (mesuré 13 à 16 ms) |

Les bancs de bout en bout tournent dans le job nocturne
(`.github/workflows/nightly.yml`), avec cinq minutes de fuzzing par cible
(`mime_message`, `sanitize_html`, `sanitize_css`, `autoconfig_xml`,
`search_query`) et la suite de contrat contre Stalwart.

## Tests manuels

Ce que l'automatisation ne peut pas vérifier (vrais fournisseurs,
notifications du système, trousseau réel) : grille dans
[MANUAL-TESTS.md](../MANUAL-TESTS.md), à rejouer avant chaque version.
