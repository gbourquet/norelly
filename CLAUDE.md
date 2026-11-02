# Norelly : client mail desktop (Tauri 2 + Rust + Svelte 5)

**En début de session, lis `docs/PLAN.md` et reprends à la première case
non cochée de la phase en cours.** Les choix d'architecture font foi dans
`docs/DECISIONS.md` : toute dérogation doit être validée par le mainteneur
avant d'être implémentée.

## Architecture (hexagonale dosée, ADR-008)
- `crates/domain` : règles métier pures et ports (traits). **Aucune E/S.**
  Dépendances limitées à `thiserror` et `serde`, vérifié par
  `scripts/check-domain-deps.sh`. C'est la zone de relecture prioritaire du
  mainteneur.
- `crates/application` : cas d'usage async qui orchestrent les ports.
- `crates/adapters/*` : `tls`, `imap`, `smtp`, `sqlite`, `blobs`,
  `keyring`, `discovery`, `render`.
- `crates/test-support` : conteneurs, faux serveurs, fixtures, suites de
  contrat.
- `app/src-tauri` : adaptateur entrant (IPC, protocoles `mailview://`…) et
  point d'assemblage.
- `app/src` : interface Svelte 5. Présentation seulement : **toute décision
  de sécurité est prise en Rust**.
- CQRS léger : écritures via le domaine ; lectures d'écran via des ports de
  requête SQL paginés.

## Commandes
| Action | Commande |
|---|---|
| Installer le front | `pnpm -C app install` |
| Lancer l'application | `pnpm -C app tauri dev` |
| Vérifications rapides | `./scripts/check.sh` |
| Tout, avec Docker | `./scripts/check.sh full` |
| Tests Rust | `cargo nextest run --workspace` |
| Tests d'intégration | `cargo nextest run --workspace --profile integration` |
| Tests front | `pnpm -C app test` |
| Tests end-to-end (Docker) | `pnpm -C app e2e` |
| Bench liste 50k (seuil 50 ms) | `cargo bench -p norelly-sqlite -- --test` |
| Régénérer les bindings IPC | `./scripts/gen-bindings.sh` |
| Recompiler les catalogues i18n | `pnpm -C app i18n` (fait aussi par `check`, `lint` et Vite) |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings`, `pnpm -C app lint`, `pnpm -C app check` |
| Format | `cargo fmt --all`, `pnpm -C app format` |

Un hook PostToolUse formate et lint automatiquement chaque fichier modifié.

## Conventions
- Code, commentaires, documentation et messages de commit en français ;
  identifiants de code en anglais.
- Commits atomiques. Message : `<zone>: <résumé>` (par exemple
  `imap: rejette STARTTLS absent`).
- Branche principale : `master`, protégée. **Ne jamais pousser sur
  `master`** : travailler sur une branche (`<zone>/<sujet>`, par exemple
  `phase-4/sync`), la pousser, puis ouvrir une PR vers `master` avec
  `gh pr create`. Seul le mainteneur valide et merge les PR ; ne jamais
  merger soi-même.
- Pas d'`unwrap`/`expect` sur des données externes. Pas d'`unsafe`
  (interdit dans tout le workspace).
- Svelte 5 uniquement (runes) ; jamais de `{@html}` ni d'`innerHTML`.
- i18n (ADR-011) : toute chaîne d'interface est une clé de
  `app/messages/{fr,en}.json` (mêmes clés, testé) ; Rust n'envoie aucun
  texte destiné à l'utilisateur, seulement des codes et un `detail` brut.
- Tests d'intégration dans des fichiers `tests/integration*.rs`.
- Contrat IPC : types dans `app/src-tauri/src/ipc.rs`, bindings TS générés
  et commités (`app/src/lib/ipc/bindings.ts`) ; toute entrée de l'interface
  est revalidée par le domaine.
- Chaque port a une suite de contrat exécutée contre son faux et son
  implémentation réelle.
- SQL : uniquement des paramètres liés pour toute donnée ; une
  interpolation de constante dans le texte d'une requête porte le
  commentaire `sql-const` (vérifié par `scripts/check-sql.sh`).
- Interdit de ramollir, désactiver ou contourner un test, ou d'affaiblir
  `scripts/check.sh` ou `deny.toml`. Un blocage se documente dans
  `PLAN.md`, puis on s'arrête.

## Fin de phase
Lancer le sous-agent `security-reviewer`, consigner son rapport dans
`docs/REVIEWS.md` et corriger les constats critiques et hauts. Ensuite :
récapitulatif (fait, reste, fichiers à relire en priorité, y compris les
fichiers d'infra sensibles), mise à jour de ce fichier si besoin, puis la
commande `/goal` de la phase suivante.
