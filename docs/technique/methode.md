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
| `release.yml` | tag de version | NSIS (Windows, mise à jour signée minisign dans un environnement séparé), .deb et .rpm, manifeste `latest.json`, manifestes winget, `SHA256SUMS` et attestation de provenance Sigstore de chaque fichier (`scripts/release-assets.sh`, `actions/attest`), revérifiées avant le brouillon de version |

`./scripts/check.sh` (rapide) enchaîne : build du front, format Rust,
Clippy (avec et sans `updater`), tests unitaires avec couverture, tests du
module de mise à jour, doctests, `cargo deny` (workspace principal puis
workspace de fuzzing `fuzz/`), alignement du verrou du fuzzing sur le
verrou racine, règle de dépendance du domaine, contrôle SQL,
**documentation** (`check-docs.sh`), versions figées de la chaîne et des
outils de la CI (`check-toolchain.mjs`), métadonnées de bureau et scripts de
publication (Linux), `svelte-check`, lint et tests du front avec couverture.
`./scripts/check.sh full` ajoute le build du workspace et les tests
d'intégration.

Caches de la CI (10 Go par dépôt sur GitHub) : ceux des jobs Rust et
end-to-end ne sont **enregistrés que depuis `master`**, pour ne pas
évincer ceux de `master` par des caches de PR. Une PR ne peut de toute
façon écrire que dans sa propre portée, jamais lue par `master` ni par une
autre PR.

- Compilation Rust des jobs `check.sh` : `Swatinem/rust-cache`.
- End-to-end : l'image Docker (clé : `app/e2e/Dockerfile` et la semaine ;
  chaîne Rust et paquets apt figés par le Dockerfile) et le dossier de build `~/.cache/norelly-e2e`
  (clé : la même, plus `Cargo.lock`), réduit à `target/` (sans
  `incremental`) et aux archives `.crate` ; une image restaurée n'est
  acceptée que sous son tag attendu. Le
  build y est fait sans compilation incrémentale ni informations de
  débogage (`CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`, transmis
  au conteneur par `app/e2e/run.sh`).

### Flatpak et Flathub

Le Flatpak n'est pas produit par `release.yml` : Flathub le construit
depuis le dépôt de l'application chez Flathub, qui contient le manifeste
et les sources vendorisées.

- **Build local**, depuis l'arbre de travail (source `type: dir` du
  manifeste du dépôt) :
  `flatpak-builder --force-clean build packaging/flatpak/io.github.gbourquet.Norelly.yml`.
- **Sources vendorisées** (crates et paquets npm) :
  `./scripts/flatpak-sources.py` après tout changement de `Cargo.lock` ou
  de `app/pnpm-lock.yaml` (`--check` dans `check-packaging.sh`).
- **Manifeste Flathub**, une fois le tag `vX.Y.Z` publié et récupéré
  (`git fetch --tags`) :
  `./scripts/flathub-manifest.py --tag vX.Y.Z --out <clone du dépôt Flathub>`.
  Le script lit le manifeste et les sources vendorisées **au commit du
  tag** (pas dans l'arbre de travail), vérifie que le tag est `v` suivi
  de la version de `tauri.conf.json` à ce commit, et remplace la source
  locale par une source `git` (`url`, `tag`, `commit`) ; flatpak-builder
  refuse la construction si le tag ne désigne plus ce commit. Il écrit
  le manifeste, `cargo-sources.json` et `node-sources.json` dans le
  dossier donné, sans toucher à ses autres fichiers ; il reste à les
  commiter dans le dépôt Flathub. Tests : `scripts/flathub-manifest.test.py`.
- **Autorisations** : liste fermée (`ALLOWED_FINISH_ARGS` de
  `flathub-manifest.py`, une valeur par ligne, indentée de deux espaces,
  sans guillemets ni échappements), vérifiée par `check-packaging.sh` et à la production du
  manifeste Flathub ; aucun `--filesystem`, les fichiers passent par les
  portails (ADR-006). Ajouter une autorisation, c'est modifier cette
  liste, en revue. Pour que cette lecture ligne à ligne soit celle de
  flatpak-builder, le manifeste s'en tient à un YAML simple : une clé de
  premier niveau unique par ligne en colonne 0 (ni étiquette, ancre,
  alias, clé explicite, séparateur de document), aucune chaîne entre
  apostrophes ou sur plusieurs lignes, aucun commentaire en fin de ligne ;
  un test compare la liste à la lecture de `flatpak-builder --show-manifest`
  quand l'outil est installé.
- **Note de version** : le tag n'est accepté que si le fichier AppStream
  (`packaging/linux/…metainfo.xml`) a, au même commit, une entrée
  `<release>` pour cette version.

### Versions figées de la chaîne et des outils

La chaîne Rust et les outils cargo de la CI ne sont jamais installés en
version flottante (`stable`, `nightly`, outil sans version) : une nouvelle
version n'entre dans la CI que par une PR, qui la fait passer par toutes
les vérifications.

| Quoi | Où | Forme |
|---|---|---|
| Rust (postes avec rustup, CI, image e2e) | `rust-toolchain.toml` : `channel` et `components` | version exacte `X.Y.Z` |
| Rust dans les workflows | entrée `toolchain:` (et `components:`) de chaque étape `dtolnay/rust-toolchain` | la même version, les mêmes composants |
| Rust dans l'image e2e | `ARG RUST_VERSION` et `--component` de `app/e2e/Dockerfile` | la même version, les mêmes composants |
| Nightly du fuzzing | `toolchain:`, `RUSTUP_TOOLCHAIN` (installation de `cargo-fuzz`) et `cargo +…` de `nightly.yml` | nightly daté `nightly-AAAA-MM-JJ` |
| Outils cargo de la CI | `tool:` de chaque étape `taiki-e/install-action` | `outil@X.Y.Z`, avec `fallback: none` (binaire du manifeste de l'action, somme vérifiée) ou `fallback: cargo-install` (outil absent du manifeste, compilé depuis crates.io), jamais le repli par défaut `cargo-binstall` (binaire non vérifié) |
| Autres outils cargo (`run:`, Dockerfile) | `cargo install` | `--version X.Y.Z --locked` (ou `outil@X.Y.Z --locked`) ; `cargo-binstall` refusé (binaires non vérifiés) |

`scripts/check-toolchain.mjs` (lancé par `check.sh`) vérifie ces règles et
la cohérence entre les fichiers : une version oubliée à un endroit fait
échouer la vérification. Les actions elles-mêmes restent figées par
empreinte SHA (le commentaire `# stable` ou `# nightly` d'une étape
`dtolnay/rust-toolchain` désigne la branche de l'action, pas la chaîne
installée). Le contrôle porte aussi sur les formes qui passeraient outre
`rust-toolchain.toml` : `cargo +…`, `rustc +…` et `RUSTUP_TOOLCHAIN` (chaîne
installée par le même job), `rustup … stable`, un second fichier de chaîne, une
autre action d'installation de Rust. C'est un garde-fou contre les oublis
(lecture ligne à ligne, sans analyseur YAML), pas contre une modification
volontaire, qui passe par la revue.

Hors de ce contrôle, et toujours flottants : Node.js et pnpm des
workflows (version majeure seulement, `setup-node` et `pnpm/action-setup`),
les paquets apt des jobs (ceux de l'image e2e sont figés par un
instantané daté, voir [tests](tests.md)) et le Rust du
Flatpak, qui compile avec l'extension `rust-stable` de son SDK, pas avec la
version de `rust-toolchain.toml`. La version minimale déclarée
(`rust-version` de `Cargo.toml`) n'est pas vérifiée par la CI, qui ne
compile qu'avec la version figée : la remonter, ou ajouter un job qui la
vérifie, est une décision du mainteneur.

Effets de `rust-toolchain.toml` : toute commande `cargo` lancée dans le
dépôt l'applique. Un job qui utilise une autre chaîne (le fuzzing, en
nightly daté) la nomme donc à chaque appel : `cargo +nightly-…`, ou
`RUSTUP_TOOLCHAIN` sur l'étape qui compile `cargo-fuzz` ; sinon rustup y
téléchargerait aussi la version figée.

Monter Rust (à chaque version stable, toutes les six semaines environ) :

1. remplacer la version dans `rust-toolchain.toml`, puis la reporter dans
   chaque `toolchain:` des workflows et dans `ARG RUST_VERSION` de
   `app/e2e/Dockerfile` (`node scripts/check-toolchain-cli.mjs` liste les
   endroits oubliés) ;
2. corriger les nouveaux avertissements de Clippy et de `rustfmt`, sans
   les désactiver ;
3. `./scripts/check.sh full`, puis une PR `ci: Rust X.Y.Z` : l'image e2e
   est reconstruite (son empreinte change).

Monter le nightly du fuzzing : choisir une date récente pour laquelle le
nightly existe (`https://static.rust-lang.org/dist/AAAA-MM-JJ/channel-rust-nightly.toml`),
la reporter dans `toolchain:`, `RUSTUP_TOOLCHAIN` et
`cargo +nightly-AAAA-MM-JJ` de `nightly.yml`, puis lancer le workflow à la main (`workflow_dispatch`)
depuis la branche.

Monter un outil cargo : choisir une version publiée (notes de version
lues, licence inchangée), la reporter dans chaque `tool:` qui le cite
(`ci.yml`, `nightly.yml`), et de préférence installer la même en local
(`cargo install --locked outil@X.Y.Z`). Avec `fallback: none`, une version
absente du manifeste de l'action figée fait échouer l'étape : monter
l'action (nouvelle empreinte), ou passer l'outil en
`fallback: cargo-install`.
