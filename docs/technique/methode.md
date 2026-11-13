# Méthode de travail

[← Documentation technique](README.md)

Les règles de référence sont dans [CLAUDE.md](../../CLAUDE.md) (pour les
contributions assistées) et [CONTRIBUTING.md](../../CONTRIBUTING.md) ; cette
page les explique.

## TDD obligatoire

Tout développement suit **rouge → vert → refactor** :

1. **Rouge** : écrire d'abord le ou les tests du comportement attendu, les
   lancer et constater qu'ils échouent **pour la bonne raison** (une
   assertion, pas une erreur de compilation ou d'import).
2. **Vert** : écrire le code minimal qui les fait passer.
3. **Refactor** : nettoyer ; les tests restent verts.

Règles :

- pas de code de production sans test rouge préalable ;
- un test qui passe dès sa première exécution est suspect : vérifier qu'il
  teste vraiment quelque chose ;
- correction de bug = d'abord un test qui reproduit le bug ;
- TDD strict pour toute la logique (protocoles, MIME, synchronisation,
  stockage, recherche, état de l'interface) ; le rendu visuel pur est couvert
  par des tests e2e écrits après coup ;
- tests réseau contre les serveurs de test en conteneur, jamais contre un
  vrai serveur.

**Interdit** : ramollir, désactiver ou contourner un test, affaiblir
`scripts/check.sh`, `deny.toml` ou un seuil de couverture. Un blocage se
documente dans la PR (ou dans le suivi des sujets ouverts), et on s'arrête.

## Maintenance

Le projet est en maintenance : nouvelles
fonctionnalités, interface, corrections, durcissement. Les sujets ouverts
sont suivis par le mainteneur (conventions dans `CLAUDE.md`, « Suivi des
sujets »). Une modification est prête pour une
PR quand :

1. `./scripts/check.sh full` passe, ainsi que les e2e ou les bancs si elle
   les concerne ;
2. la documentation fonctionnelle, utilisateur et technique décrit le
   nouveau comportement ;
3. si elle touche une zone sensible, la revue `security-reviewer` est faite,
   constats critiques et hauts corrigés, rapport résumé dans la PR et autres
   constats reportés dans le suivi des sujets ouverts ;
4. la PR cite le sujet traité ; ce qui a été découvert et laissé de côté
   devient un nouveau sujet.

Les choix d'architecture font foi dans [DECISIONS.md](../DECISIONS.md) :
toute dérogation est validée par le mainteneur **avant** d'être
implémentée, puis consignée.

## Conventions de code

- **Langue** : code, commentaires, documentation et messages de commit en
  **français** ; identifiants de code en **anglais**.
- **Rust** : pas d'`unwrap`/`expect` sur des données externes (lint
  `unwrap_used`) ; `unsafe` interdit ; `cargo fmt` (`rustfmt.toml`),
  `cargo clippy -- -D warnings`. Le domaine reste sans E/S et ne dépend que
  de `thiserror` et `serde`.
- **SQL** : paramètres liés pour toute donnée ; une constante interpolée
  porte le commentaire `sql-const`.
- **Svelte 5** uniquement (runes) ; jamais de `{@html}` ni d'`innerHTML` ;
  Prettier, ESLint, `svelte-check --fail-on-warnings`.
- **i18n** (ADR-011) : toute chaîne d'interface est une clé de
  `app/messages/{fr,en}.json` (mêmes clés, testé) ; Rust n'envoie que des
  codes et un `detail` brut.
- **IPC** : types dans `app/src-tauri/src/ipc.rs`, bindings générés et
  commités ; toute entrée revalidée par le domaine.
- **Ports** : chaque port a une suite de contrat jouée contre son faux et son
  implémentation réelle.
- **Documentation** : toute commande ajoutée au registre figure dans
  l'aide-mémoire `docs/utilisateur/{fr,en}/shortcuts.md` (testé) ; toute
  page utilisateur existe dans les deux langues (testé).

Un hook Claude Code (`.claude/hooks/format-lint.sh`) formate et lint chaque
fichier modifié par l'assistant.

## Commits

- **Atomiques** : un changement cohérent par commit.
- Message : `<zone>: <résumé>` en français, par exemple
  `imap: rejette STARTTLS absent`, `docs: aide-mémoire des raccourcis`.
  Zones courantes : `domaine`, `imap`, `smtp`, `sqlite`, `render`, `ui`,
  `ipc`, `e2e`, `tests`, `docs`, `ci`, `sécurité`, `paquets`.

## Branches et PR

- Branche principale : `master`, **protégée**. Ne jamais pousser dessus.
- Travailler sur une branche `<zone>/<sujet>` (par exemple `ui/bouton-repondre`),
  la pousser, puis ouvrir une PR vers `master` (`gh pr create`).
- **Seul le mainteneur valide et merge les PR.**

## Intégration continue

| Workflow | Déclencheur | Contenu |
|---|---|---|
| `ci.yml` | push sur `master`, PR | trois jobs en parallèle : Linux `./scripts/check.sh full` ; Linux `pnpm -C app e2e` ; Windows `./scripts/check.sh` puis `cargo build --workspace` |
| `nightly.yml` | chaque nuit | fuzzing (5 min par cible), bancs (liste 50k, démarrage de bout en bout), Stalwart en second serveur |
| `release.yml` | tag de version | NSIS (Windows, mise à jour signée minisign dans un environnement séparé), .deb et .rpm, sommes SHA-256, manifeste `latest.json`, manifestes winget, brouillon de version |

`./scripts/check.sh` (rapide) enchaîne : build du front, format Rust,
Clippy (avec et sans `updater`), tests unitaires avec couverture, tests du
module de mise à jour, doctests, `cargo deny`, règle de dépendance du
domaine, contrôle SQL, **documentation** (`check-docs.sh`), métadonnées de
bureau (Linux), `svelte-check`, lint et tests du front avec couverture.
`./scripts/check.sh full` ajoute le build du workspace et les tests
d'intégration.

Caches de la CI (10 Go par dépôt sur GitHub) : ceux des jobs Rust et
end-to-end ne sont **enregistrés que depuis `master`**, pour ne pas
évincer ceux de `master` par des caches de PR. Une PR ne peut de toute
façon écrire que dans sa propre portée, jamais lue par `master` ni par une
autre PR.

- Compilation Rust des jobs `check.sh` : `Swatinem/rust-cache`.
- End-to-end : l'image Docker (clé : `app/e2e/Dockerfile` et la semaine,
  pour suivre Rust stable) et le dossier de build `~/.cache/norelly-e2e`
  (clé : la même, plus `Cargo.lock`), réduit à `target/` (sans
  `incremental`) et aux archives `.crate` ; une image restaurée n'est
  acceptée que sous son tag attendu. Le
  build y est fait sans compilation incrémentale ni informations de
  débogage (`CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`, transmis
  au conteneur par `app/e2e/run.sh`).
