# Technologies et versions

[← Documentation technique](README.md)

Les versions exactes font foi dans `Cargo.toml` (workspace),
`Cargo.lock`, `app/package.json` (versions figées) et
`app/pnpm-lock.yaml`. Les raisons sont détaillées dans
[DECISIONS.md](../DECISIONS.md).

## Socle

| Brique | Version | Pourquoi | ADR |
|---|---|---|---|
| Rust | édition 2024, ≥ 1.90 | Logique métier et sécurité dans un langage sûr, maîtrisé par le mainteneur ; `unsafe` interdit (`unsafe_code = "forbid"`) | ADR-001, ADR-008 |
| Tauri | 2.12 | Webview du système (faible empreinte), indispensable pour le HTML des mails ; cœur Rust | ADR-001 |
| Svelte | 5.57 (runes uniquement) | Interface légère, réactive, sans moteur lourd ; présentation seulement | ADR-005 |
| TypeScript | 6.0, strict | Typage du contrat IPC généré et de l'interface | ADR-005 |
| Vite | 8.3 | Build du front | ADR-005 |
| pnpm | 10 | Gestionnaire de paquets du front (verrouillage strict) | ADR-005 |
| Node | ≥ 22 (24 en CI) | Outils du front, e2e | — |

## Protocoles et données (Rust)

| Brique | Version | Pourquoi | ADR |
|---|---|---|---|
| `imap-codec` + `imap-next` | 2.0.0-alpha.9 / 0.3.4 | Codec IMAP strict, fuzzé, extensions modélisées (IDLE, CONDSTORE, QRESYNC, UIDPLUS, MOVE) ; session maison au-dessus | ADR-002 |
| `lettre` | 0.11.23 (sans backend TLS) | SMTP et construction MIME ; TLS fourni par `norelly-tls` via `connect_with_transport` | ADR-002 |
| `mail-parser` | 0.11 | Analyse MIME tolérante des messages reçus | ADR-002 |
| `rustls` + `rustls-platform-verifier` | 0.23 / 0.7 (`ring`) | TLS sans OpenSSL ; vérificateur de la plateforme combiné aux exceptions par empreinte ; TLS 1.2 minimum | ADR-002 |
| `ammonia` | 4.2 | Assainissement HTML par liste blanche | ADR-004 |
| `rusqlite` (SQLite embarqué) | 0.40 (`bundled`) | Une base globale, WAL, FTS5 ; `rusqlite_migration` 2.6 | ADR-003 |
| `zstd` | 0.13 | Compression des messages bruts (`.eml.zst`) | ADR-003 |
| `reqwest`, `quick-xml`, `hickory-resolver` | 0.13 / 0.42 / 0.26 | Autoconfiguration (HTTPS), XML, SRV | ADR-002 |
| `csv` | 1.4 | Lecture des exports CSV de contacts (guillemets, champs sur plusieurs lignes) ; vCard et LDIF écrits pour le projet | ADR-012 |
| `icu_normalizer` | 2.4 (`compiled_data` seule, déjà tirée par `idna`) | Normalisation NFC des noms importés, derrière le port `TextNormalizer` | ADR-012 |
| `keyring-core` + magasins Secret Service et Windows | 1 | Trousseau du système | ADR-002 |
| `tokio` | 1.54 | Asynchrone | — |
| `tracing` | 0.1 (plafonné à DEBUG à la compilation) | Journaux ; TRACE supprimé car `imap-next` y journaliserait les mots de passe | ADR-002 |
| `tauri-specta`, `specta` | =2.0.0-rc.26 (figées) | Génération des types TypeScript du contrat IPC | ADR-005 |
| `notify-rust` | — | Notifications natives sans zbus (zbus 5 a des avis RUSTSEC) | [BACKLOG](../BACKLOG.md) (dérogation à consigner) |
| `tauri-plugin-dialog`, `tauri-plugin-updater` | 2.6 / 2 | Sélecteurs de fichiers (appelés depuis Rust) ; mises à jour signées minisign (NSIS seulement) | ADR-004, ADR-006 |

## Interface

| Brique | Version | Pourquoi | ADR |
|---|---|---|---|
| `bits-ui` | 2.19 | Primitives accessibles (dialogue, palette) | ADR-005 |
| `@tanstack/virtual-core` | 3.19 | Virtualisation de la liste (50 000 lignes) | ADR-005 |
| Tiptap | 3.32 (figé) | Éditeur limité à ce que le réassainissement Rust conserve | ADR-010 |
| Paraglide JS | 2.29 | i18n compilée et typée, catalogues `app/messages/{fr,en}.json` | ADR-011 |
| Lucide | 1.59 | Icônes embarquées, aucune ressource réseau | ADR-005 |

Pas de Tailwind : jetons de design en variables CSS (`app/src/styles/tokens.css`)
et CSS scopé (ADR-005).

## Outils de qualité et de test

| Outil | Rôle | ADR |
|---|---|---|
| `cargo nextest` | Exécution des tests Rust, profils `default`, `integration`, `stalwart` | ADR-007 |
| `proptest` | Propriétés (synchronisation, conversations, recherche, rédaction, fichiers de contacts) | ADR-007 |
| `cargo-fuzz` | Fuzzing nocturne (MIME, HTML, CSS, autoconfig, requêtes, fichiers de contacts) | ADR-007 |
| `testcontainers` | Dovecot, Mailpit, Toxiproxy, Stalwart en conteneurs | ADR-007 |
| `criterion` | Bancs de performance (liste et recherche sur 50 000 messages) | ADR-007 |
| `cargo-llvm-cov`, `@vitest/coverage-v8` | Couverture avec seuils | — |
| `cargo-deny` | Licences (ADR-009) et avis de sécurité | ADR-009 |
| Vitest + Testing Library | Tests du front | ADR-007 |
| tauri-driver + WebdriverIO | End-to-end sous Linux (conteneur Ubuntu 24.04) | ADR-007 |
| ESLint, Prettier, svelte-check | Lint et format du front | — |

## Licences

Application sous `MIT OR Apache-2.0`. Dépendances permissives, MPL-2.0
tolérée, LGPL tolérée si liée dynamiquement, GPL et AGPL refusées ; contrôle
par `cargo deny` (`deny.toml`) et `pnpm audit` (ADR-009).
