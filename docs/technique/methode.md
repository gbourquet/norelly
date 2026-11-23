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
| `release.yml` | tag de version | NSIS (Windows, mise à jour signée minisign dans un environnement séparé, signature vérifiée avec la clé publique embarquée par le job `update-signature`, sans secret), .deb et .rpm, manifeste `latest.json`, manifestes winget, `SHA256SUMS` et attestation de provenance Sigstore de chaque fichier (`scripts/release-assets.sh`, `actions/attest`), revérifiées avant le brouillon de version ; préversion (`X.Y.Z-suffixe`) : pas de .rpm (le tiret est refusé dans une version rpm) ni de manifestes winget, brouillon marqué préversion ; version stable : .deb et .rpm exigés |
| `release.yml` (job `site`) | publication d'une version stable par le mainteneur (`release: released`) | lancement du workflow `deploy.yml` du dépôt du site `gbourquet/norelly-site` sur `main`, qui se reconstruit au dernier tag de version (ADR-017) ; voir « Signal de publication au site » |
| `release.yml` (job `aur`) | publication d'une version stable par le mainteneur (`release: released`) | paquet AUR `norelly-bin` : .deb publié vérifié (somme et attestation de provenance), PKGBUILD et `.SRCINFO` régénérés (`scripts/aur-pkgbuild.sh`), envoyés sur `aur.archlinux.org` ; voir « Paquet AUR » |

### Signal de publication au site

Le site norelly.fr lit le guide, la politique de confidentialité et les
captures au tag de la dernière version publiée. Quand le mainteneur publie
le brouillon de version (jamais avant : un brouillon ne se télécharge
pas), ou promeut une préversion en version stable, le job `site` de
`release.yml` lance le déploiement du site ; les jobs de construction ne
tournent pas à cette occasion. Une préversion ne prévient pas le site, qui
suit la dernière version stable.

Mise en place par le mainteneur, une fois :

1. Créer un **jeton d'accès personnel fin** (*fine-grained personal access
   token*) : propriétaire `gbourquet`, accès au seul dépôt
   `gbourquet/norelly-site`, permission de dépôt **Actions : Read and
   write** (celle qu'exige l'API
   `POST /repos/{owner}/{repo}/actions/workflows/{workflow}/dispatches`),
   aucune autre (Metadata en lecture est ajoutée d'office), avec une date
   d'expiration courte. Ce jeton ne peut pas modifier le code du site ;
   il peut en revanche lancer, annuler, relancer ou désactiver ses
   workflows et supprimer ses artefacts, journaux et caches.
2. Dans ce dépôt, créer l'environnement GitHub **`site`**, limité aux tags
   `v*` (*Deployment branches and tags*), sans relecture obligatoire, et y
   ranger le jeton sous le nom **`SITE_DISPATCH_TOKEN`**.
3. Le workflow `deploy.yml` du dépôt du site accepte `workflow_dispatch`.
   Son envoi est automatique (pas de relecture obligatoire pour un dépôt
   privé avec l'offre GitHub gratuite) : il ne part que depuis `main`,
   pour son dernier commit, après une construction vérifiée.
4. Protéger les tags `v*` de ce dépôt par une règle (*Settings → Rules →
   Rulesets*, cible « Tags », motif `v*`) : création, mise à jour et
   suppression réservées au mainteneur. L'environnement `site` accepte tout
   tag `v*`, quel que soit le commit visé.

Secret absent, jeton expiré ou refusé : le job `site` échoue, avec un
message explicite, et la version reste publiée ; renouveler le jeton puis
relancer le job (ou déclencher le site à la main) met le site à jour.

### Enregistrer un secret

Un secret d'environnement s'enregistre par l'interface de GitHub
(*Settings → Environments → nom de l'environnement → Environment
secrets*), ou par `gh secret set NOM --env ENVIRONNEMENT` lancé dans un
**vrai terminal**. Sans terminal interactif (commande lancée par un agent,
préfixe `!` de Claude Code, script), `gh secret set` ne demande pas la
valeur : il lit une entrée vide et enregistre un secret vide, sans erreur.
Le job qui l'utilise échoue alors en le signalant absent.

### Paquet AUR

Le paquet `norelly-bin` de l'AUR (Arch Linux et dérivées) reprend le .deb
de la version publiée : même binaire que les autres paquets Linux,
connexion Google comprise. Le modèle est dans `packaging/aur`. À chaque
version stable publiée, le job `aur` de `release.yml` :

1. télécharge le .deb publié et le vérifie : somme dans `SHA256SUMS`,
   attestation de provenance du .deb et de `SHA256SUMS` émise par ce
   workflow à ce tag et à ce commit (`scripts/aur-deb.sh`) ;
2. remplit le PKGBUILD et le `.SRCINFO` (`scripts/aur-pkgbuild.sh`), et
   y joint la licence des fichiers du paquet : 0BSD au texte exact d'Arch
   (`LICENSE`, `REUSE.toml`, lien `LICENSES/0BSD.txt`), comme le demandent
   les règles de l'AUR (*AUR submission guidelines*) ;
3. les envoie sur l'AUR s'ils sont strictement plus récents
   (`scripts/aur-push.sh`) : version comparée sur `pkgver-pkgrel`, jamais
   de retour en arrière ; relance à l'identique : rien n'est envoyé ; même
   version, autre contenu : refusé.

Une préversion ne change pas l'AUR (tiret interdit dans `pkgver`). Clé
d'hôte de `aur.archlinux.org` commitée (`packaging/aur/known_hosts`,
empreinte vérifiée par `aur-push.test.sh`). Contrôles : tests des trois
scripts dans `check.sh` ; `check-aur.sh` dans `check.sh full` (Docker,
sur un .deb factice : `.SRCINFO` égal à `makepkg --printsrcinfo`, namcap,
`pkgctl license check`, construction et installation sous Arch). Les dépendances (`depends`) ont
été relevées sur le vrai binaire (`namcap` sur le paquet de la 0.1.0) :
à revoir si l'application lie une nouvelle bibliothèque.

Mise en place par le mainteneur, une fois :

1. Créer un compte sur <https://aur.archlinux.org> dédié à Norelly, qui ne
   maintiendra que `norelly-bin`. Rien ne l'impose côté AUR : la clé de la
   CI peut agir sur tout ce que le compte peut faire (voir « Clé AUR
   compromise »).
2. Créer une clé SSH dédiée, sans phrase de passe :
   `ssh-keygen -t ed25519 -N '' -C norelly-aur -f ~/norelly-aur`. Coller
   la clé publique (`~/norelly-aur.pub`) dans *My Account → SSH Public
   Key* du compte AUR.
3. Premier envoi, pour une version publiée avant l'existence du job `aur`
   (la 0.1.0 : le workflow de son tag n'a pas ce job, et un tag publié ne
   se déplace ni ne se recrée jamais). Depuis une copie de travail de
   `master` à jour, avec les mêmes vérifications que le job :

   ```
   git fetch --tags
   git diff --exit-code v0.1.0 HEAD -- LICENSE-MIT
   ./scripts/aur-deb.sh v0.1.0 "$(git rev-parse 'v0.1.0^{commit}')" /tmp/aur-deb
   ./scripts/aur-pkgbuild.sh 0.1.0 /tmp/aur-deb/Norelly_0.1.0_amd64.deb /tmp/aur-pkg
   GIT_SSH_COMMAND="ssh -F /dev/null -i $HOME/norelly-aur -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=$PWD/packaging/aur/known_hosts -o GlobalKnownHostsFile=/dev/null" \
     ./scripts/aur-push.sh /tmp/aur-pkg ssh://aur@aur.archlinux.org/norelly-bin.git
   ```

   La deuxième ligne vérifie que la licence n'a pas changé depuis le tag
   (le PKGBUILD la télécharge au tag). Le premier envoi crée le paquet sur
   l'AUR.
4. Dans ce dépôt, créer l'environnement GitHub **`aur`**, limité aux tags
   `v*` (*Deployment branches and tags*), avec **relecture obligatoire**
   par le mainteneur (*Required reviewers*) : la clé permet de publier un
   PKGBUILD exécuté chez les utilisateurs, comme la clé de mise à jour de
   l'environnement `release`. Y ranger **`AUR_SSH_PRIVATE_KEY`** depuis le
   fichier, sans passer par le presse-papiers (un gestionnaire de
   presse-papiers en garderait l'historique) :

   ```
   gh secret set AUR_SSH_PRIVATE_KEY --env aur -R gbourquet/norelly < ~/norelly-aur
   shred -u ~/norelly-aur
   ```

5. La règle des tags `v*` (étape 4 de « Signal de publication au site ») :
   création, mise à jour et suppression réservées au mainteneur, sans
   liste de contournement. Les environnements `site` et `aur` acceptent
   tout tag `v*`.

À chaque version stable publiée ensuite, GitHub demande d'approuver le job
`aur`. Les règles de l'AUR rappellent qu'une mise à jour automatique ne
dispense pas de vérifier le paquet : avant d'approuver, s'assurer que la
version n'ajoute ni bibliothèque liée (dépendances du PKGBUILD) ni
changement de licence ; sinon, corriger `packaging/aur` d'abord. Secret absent ou mal formé, clé refusée : le job échoue avec un
message explicite et la version reste publiée ; corriger le secret puis
relancer le job met l'AUR à jour.

#### Clé AUR compromise

L'interface SSH de l'AUR ne sert pas qu'à pousser : la clé permet aussi
d'ajouter des co-mainteneurs, d'adopter ou d'abandonner des paquets, et de
créer un paquet par un premier envoi. En cas de doute sur la clé :

1. retirer la clé publique du compte AUR et changer le mot de passe du
   compte ;
2. vider la liste des co-mainteneurs (`ssh aur@aur.archlinux.org
   set-comaintainers norelly-bin` sans nom, avec une nouvelle clé) et
   vérifier les paquets du compte (`list-repos`) ;
3. relire l'historique git de `norelly-bin` et rétablir le dernier
   PKGBUILD légitime par un nouvel envoi (`pkgrel` augmenté si la version
   ne change pas) ;
4. créer une nouvelle clé et remplacer le secret (étapes 2 et 4).

`./scripts/check.sh` (rapide) enchaîne : build du front, format Rust,
Clippy (avec et sans `updater`), tests unitaires avec couverture, tests du
module de mise à jour, doctests, `cargo deny` (workspace principal puis
workspace de fuzzing `fuzz/`), alignement du verrou du fuzzing sur le
verrou racine, règle de dépendance du domaine, contrôle SQL,
**documentation** (`check-docs.sh`), versions figées de la chaîne et des
outils de la CI (`check-toolchain.mjs`), métadonnées de bureau et scripts de
publication (Linux), `svelte-check`, lint et tests du front avec couverture.
`./scripts/check.sh full` ajoute le build du workspace, les tests
d'intégration et le contrôle du paquet AUR sous Arch (`check-aur.sh`,
Docker).

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
- Contrôle AUR (`scripts/check-aur.sh`) : bases et paquets Arch de
  l'instantané daté de l'archive, dans `~/.cache/norelly-aur/<date>` en
  local comme en CI (clé : `scripts/check-aur.sh`). L'archive Arch, lente
  et parfois injoignable, n'est lue qu'au premier lancement pour une date ;
  ensuite le contrôle se fait sans réseau, en quelques secondes. Changer
  la date efface le dossier de l'ancienne.

### Flatpak et Flathub

Le Flatpak n'est pas produit par `release.yml` : Flathub le construit
depuis le dépôt de l'application chez Flathub, qui contient le manifeste
et les sources vendorisées.

- **Build local**, depuis l'arbre de travail (source `type: dir` du
  manifeste du dépôt) :
  `flatpak-builder --force-clean build packaging/flatpak/io.github.gbourquet.Norelly.yml`.
- **Sources vendorisées** (crates et paquets npm) :
  `./scripts/flatpak-sources.py` après tout changement de `Cargo.lock` ou
  de `app/pnpm-lock.yaml` (`--check` dans `check-packaging.sh`). Une
  dépendance git n'est acceptée que si son dépôt figure dans `allow-git`
  de `deny.toml` et qu'elle est figée par commit complet. Le dépôt est
  alors cloné à ce commit au moment des sources, et ses crates sont
  recopiées dans le répertoire vendorisé. Le build s'arrête si un
  manifeste hérite encore d'un réglage du workspace du dépôt.
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
- **Note de version** : le tag n'est accepté que si la première entrée
  `<release>` du fichier AppStream (`packaging/linux/…metainfo.xml`), au
  même commit, est celle de cette version. Une préversion y est notée
  avec un tilde à la place du tiret (`0.1.0~beta.1`,
  `type="development"`) : AppStream classe `0.1.0-beta.1` après `0.1.0`,
  et `appstreamcli validate` refuse alors l'entrée de la version stable
  placée au-dessus. Le tag `v0.1.0-beta.1`, antérieur à cette règle, n'a
  pas de manifeste Flathub (préversion non publiée sur Flathub).

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
empreinte SHA. `dtolnay/rust-toolchain`, sans étiquette de version, est
figée sur un commit de sa branche `master` (`# master`), dont l'historique
n'est pas réécrit ; la chaîne installée est celle de `toolchain:`. Ses
branches de canal (`stable`, `nightly`) sont régénérées par son auteur et
sont refusées par `scripts/check-actions.mjs`. Le contrôle porte aussi sur les formes qui passeraient outre
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
