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
| **Scripts de publication** | `scripts/release-assets.test.sh` | Bash et coreutils, fichiers factices (Linux) | `./scripts/release-assets.test.sh` (lancé par `check.sh`) |
| **Signature de mise à jour** | `tools/verify-update-signature/src/main.rs` | `minisign-verify`, clés publiques et signature factices (`tools/verify-update-signature/tests/fixtures/`, clés privées jetées) | `cargo nextest run -p norelly-verify-update-signature` (lancé par `check.sh`) |

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
- `certs.rs` : certificats `rcgen` (CA de test, autosigné, autosigné marqué
  autorité comme `openssl req -x509`, expiré, mauvais nom d'hôte).
- `containers.rs` : Dovecot 2.4.5, Mailpit, Toxiproxy (coupure, latence,
  connexion à moitié fermée), Stalwart ; images figées par empreinte.
- `imap_control.rs` : manipulation directe d'une boîte (APPEND, changement
  d'UIDVALIDITY…).
- `synthetic.rs` : générateur de boîtes synthétiques (50 000 messages).
- `fakes.rs` : implémentations en mémoire des ports.
- `*_contracts.rs` : suites de contrat (`SecretStore`, `AccountRepository`,
  `MailStore`, `BlobStore`, `PendingOpStore`, `DraftStore`, `OutboxStore`,
  `SearchIndex`, carnet d'adresses, session IMAP…).

Contre Dovecot, les capacités sont masquées par le connecteur
(`ImapConnector::hiding_capabilities`) pour tester les chemins sans MOVE ou
sans UIDPLUS. L'échange XOAUTH2 est joué contre les faux serveurs scriptés :
un jeton ne part que vers l'hôte de son fournisseur, si bien que les tests
gardent cet hôte dans le point de connexion (certificat de test émis pour
lui) et redirigent seulement la connexion TCP vers le faux serveur local
(`connecting_to`, compilé pour les tests seulement, dans `ImapConnector` et
`SmtpSender`). Le trousseau réel (Secret Service) est testé dans le conteneur
des e2e (`integration_secret_service.rs`), jamais sur le poste d'un
développeur.

## End-to-end

`app/e2e/run.sh` construit l'image (`app/e2e/Dockerfile`), compile
l'application dans le conteneur, démarre Dovecot, Mailpit et un **serveur
canari** sur un réseau Docker **interne** (sans accès extérieur), avec des
certificats autosignés neufs, puis lance les scénarios avec un `HOME`
éphémère et la locale `fr`. Un second Dovecot, alias `imap.gmail.com` sur
ce réseau, joue le serveur IMAP de Google : son certificat est signé par une
autorité de test neuve, contrainte à `gmail.com`, à laquelle seul le
conteneur des scénarios fait confiance (`SSL_CERT_FILE`, qui remplace les
autorités du système dans ce conteneur sans Internet). Le conseil « mot de
passe d'application » exige en effet des serveurs de Google joints sans
exception de certificat.

L'image part d'Ubuntu 24.04 figée par empreinte ; ses paquets apt viennent
d'un instantané daté de l'archive (`snapshot.ubuntu.com`, date `snapshot=`
écrite dans le Dockerfile), si bien que deux constructions installent les
mêmes versions ; la construction échoue si un index ne vient pas de cet
instantané. Rustup, Node.js et pnpm sont des archives figées vérifiées par
SHA-256. Les correctifs Ubuntu n'arrivent donc qu'avec une montée de la
date : au moins une fois par mois, et dès qu'un avis de sécurité Ubuntu
touche un paquet de l'image (WebKitGTK, libsoup, OpenSSL, curl…) ;
`./scripts/check.sh` avertit, sans échouer, au-delà de 35 jours
(`scripts/check-apt-snapshot.mjs`, testé par
`scripts/check-apt-snapshot.test.mjs`). Montée :
remplacer la date par une date récente, postérieure à l'image de base (UTC,
format `AAAAMMJJTHHMMSSZ`), et relancer `pnpm -C app e2e` ; l'empreinte du
Dockerfile change, la CI reconstruit donc son image.

| Scénario | Vérifie |
|---|---|
| `add-account.e2e.mjs` | Ajout d'un compte de bout en bout (saisie manuelle, confiance dans les certificats) |
| `reading-security.e2e.mjs` | Zéro requête vers le canari, aucun script du mail exécuté, IPC inaccessible depuis `mailview://`, `javascript:` jamais ouvert, touches relayées |
| `compose.e2e.mjs` | Réponse `r` + Ctrl+Entrée reçue par Mailpit (In-Reply-To, texte et HTML), copie dans Envoyés, Cci hors des en-têtes, HTML saisi jamais interprété |
| `search.e2e.mjs` | Mot du texte trouvé et surligné, échappement, filtre `from:` |
| `captures/captures.e2e.mjs` | Produit les captures de `docs/utilisateur/{fr,en}/captures/` sur un profil neuf par langue (boîte Dovecot dédiée), en passant par la souris comme un utilisateur débutant (boutons, cases à cocher, clic droit, menus « Plus… » et « ⋯ ») ; vérifie la langue de l'interface |
| `captures/inbox-dark.e2e.mjs` | Lancé juste avant, sur son propre profil neuf et avec `GTK_THEME=Adwaita:dark` : capture `inbox-dark` (même compte, mêmes messages, même taille que `inbox`, `captures/shared.mjs`) ; vérifie la langue et que le thème sombre est appliqué (`prefers-color-scheme`), puis vide la boîte Dovecot de la capture |
| `perf/perf.e2e.mjs` | Démarrage et changement de dossier sur 50 000 + 20 000 messages (`NORELLY_E2E_PERF=1`, build release) |

Menus sous WebKitWebDriver : il ne synthétise que `mousedown`, `mouseup`
et `click`, sans `pointerdown`, qui ouvre les menus déroulants (« Plus… »,
menu d'un compte) ; et son clic droit émet aussi un clic, qui ouvrirait la
conversation. Les scénarios passent donc par `openMenu` et
`openContextMenu` (`app/e2e/specs/helpers.mjs`), qui envoient ces
événements tels que la souris les produit. Viser les boutons par leur nom
exact, hors de la copie invisible de mesure des barres d'actions
(`aria-hidden`).

**Règle permanente** : à chaque montée de version de Tauri, rejouer
`reading-security.e2e.mjs`, qui vérifie qu'aucun accès à l'IPC n'est
possible depuis l'iframe `mailview://`. La capability ne distingue pas
l'iframe de lecture, et `tauri::test` n'enregistre pas les protocoles :
seul l'e2e le prouve.

Les captures ne sont recopiées dans `docs/` que si tous les scénarios ont
réussi. Elles ne se modifient jamais à la main : relancer
`pnpm -C app e2e`, puis commiter les PNG modifiés.

### Cache de build

Le build de l'application dans le conteneur garde ses fichiers compilés
dans `~/.cache/norelly-e2e` (ou `$XDG_CACHE_HOME/norelly-e2e`) ; le
dossier utilisé est affiché au lancement (« Cache de build : … ») :

| Dossier | Contenu | Partagé entre copies de travail |
|---|---|---|
| `target-<empreinte>/` | Fichiers compilés (`CARGO_TARGET_DIR`) | Non : un par copie de travail, seul monté dans le conteneur |
| `cargo/registry/cache/`, `cargo/registry/index/` | Archives `.crate` et index du registre | Oui ; archives revérifiées contre `Cargo.lock` à chaque lancement |

Les sources extraites du registre (`cargo/registry/src`) et le `HOME` des
conteneurs sont éphémères (`tmpfs`) : cargo réextrait les sources des
archives à chaque lancement, en quelques secondes, sans recompiler les
dépendances.

Les dépôts git des dépendances (`cargo/git`, clones et extractions) sont
retirés au début de chaque lancement (`app/e2e/run.sh`) : cargo réutilise
une extraction existante sans la revérifier, et ce dossier est partagé par
toutes les copies de travail. Le build reclone donc le dépôt, puis les
scénarios (`cargo test --offline`) se servent de cette extraction.

Toutes les copies de travail (worktrees git) sont montées sur `/work` dans
le conteneur : avec un seul dossier de build, cargo reprendrait les
fichiers compilés d'une autre branche, les dates de modification ne
suffisant pas à les distinguer. L'empreinte est donc tirée du chemin réel
des sources (`app/e2e/target-name.sh`) : chaque copie de travail
recompile seulement ce qui a changé depuis son lancement précédent. La CI
calcule le même nom pour le chemin du dépôt sur le runner, qui est
stable, et sauve ou restaure ce dossier.

cargo ne contrôle l'empreinte d'une archive `.crate` qu'à son
téléchargement, pas quand elle est déjà dans le cache. Au lancement,
`app/e2e/verify-crates.sh` compare donc chaque archive d'un paquet de
`Cargo.lock` à son `checksum` et retire celle qui diffère (ou qui n'est
pas un fichier ordinaire) : cargo la télécharge et la vérifie à nouveau.
Si une archive ne peut pas être lue, le lancement s'arrête. Le script
ne lit que les empreintes de `sha256sum`, jamais les chemins qu'il
réécrit (préfixe `*` du mode binaire sous Windows, chemins échappés).

Nettoyage : chaque dossier de build note dans `.source` le chemin des
sources qui l'ont produit. Au lancement, `app/e2e/prune-targets.sh` retire
ceux dont la copie de travail n'existe plus ; un dossier d'origine
inconnue, un lien symbolique et le registre de cargo ne sont jamais
touchés. Une copie de travail neuve compile tout à son premier lancement.
Pour tout libérer :
`rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}"/norelly-e2e/target-*` (chaque
dossier sera recompilé au lancement suivant) ; le nom propre à une copie
de travail s'obtient avec `app/e2e/target-name.sh <chemin>`. Les dossiers
`target/`, `home/` et `cargo/registry/src/` des versions précédentes ne
servent plus : les supprimer à la main.

Ces trois scripts sont testés par `app/e2e/*.test.sh`, lancés par
`./scripts/check.sh`.

## Couverture et seuils

Vérifiés par `./scripts/check.sh`, **à relever, jamais à baisser** :

| Mesure | Outil | Seuil |
|---|---|---|
| Rust, lignes (tests unitaires) | `cargo llvm-cov nextest --fail-under-lines` | 86 % |
| Front, lignes / instructions / branches / fonctions | `vitest --coverage` (`app/vite.config.ts`) | 84 / 81 / 74 / 79 % |

Le code d'adaptateur entrant couvert surtout par l'intégration et l'e2e
(`app/src-tauri/src/mail.rs`, chemin Secret Service) reste plus faible en
mesure unitaire.

## Bancs de performance

| Banc | Commande | Seuil |
|---|---|---|
| Liste : première page et changement de dossier, 50 000 messages (requête seule) | `cargo bench -p norelly-sqlite -- --test` | < 50 ms (mesuré ≈ 0,2 ms) |
| Recherche, 50 000 messages de 300 mots | même commande (bancs `message_list` et `search`) | < 100 ms (mesuré 0,3 à 89 ms) |
| Suggestions de destinataires, carnet plein (`MAX_CONTACTS` = 25 000 fiches) : lecture des fiches trouvées en une fois | même commande (banc `contacts_suggestions`) | < 10 ms (mesuré 0,06 à 1,1 ms) |
| Carnet plein (`MAX_CONTACTS` = 25 000 fiches) : première page, page suivante, liste filtrée | même commande (banc `contacts`) | < 50 ms (mesuré 0,7 à 18 ms) |
| Carnet plein : suggestions de destinataires (lectures du carnet) | même commande (banc `contacts`) | < 100 ms (mesuré 13 à 45 ms) |
| Import d'un fichier de 25 000 fiches qui remplit le carnet | même commande (banc `contacts`) | < 20 s (mesuré ≈ 2,2 s) |
| Démarrage de bout en bout | `NORELLY_E2E_PERF=1 pnpm -C app e2e` | < 400 ms (seuil porté par le mainteneur) |
| Changement de dossier de bout en bout | idem | < 200 ms (mesuré 13 à 16 ms) |

Taille maximale du carnet : le banc `contacts` mesure aussi d'autres
tailles (`NORELLY_BENCH_CONTACT_SIZES=25000,50000,100000`, au plus le
plafond ; au-delà, le relever le temps de la mesure). Médianes mesurées
(plafond relevé) :

| Fiches | Première page | Liste filtrée, texte absent | Suggestions, texte courant | Suggestions, texte absent | Import de 25 000 fiches |
|---|---|---|---|---|---|
| 25 000 | 0,7 ms | 18 ms | 13 ms | 31 ms | 2,2 s |
| 50 000 | 0,7 ms | 47 ms | 18 ms | 87 ms | 2,4 s |
| 100 000 | 0,8 ms | 114 ms | 24 ms | 211 ms | 3,1 s |
| 200 000 | 0,8 ms | 255 ms | 29 ms | 484 ms | 3,2 s |

Un texte que ne contient aucune fiche fait parcourir tout le carnet (la
liste filtrée et les suggestions croissent avec lui) : à 50 000 fiches,
la liste filtrée (47 ms) et une suggestion (87 ms) sont déjà au bord de
leurs seuils sur une machine calme, et les dépassent sur une machine
chargée ; à 100 000, elles les dépassent toujours. Le plafond
retenu est aussi la borne d'un fichier importé, de l'export, de la
recherche des doublons et de CardDAV.

Les bancs de bout en bout tournent dans le job nocturne
(`.github/workflows/nightly.yml`), avec cinq minutes de fuzzing par cible
(`mime_message`, `sanitize_html`, `sanitize_css`, `autoconfig_xml`,
`search_query`, `contact_import`, `contact_export`) et la suite de contrat contre Stalwart.
La cible `sanitize_html` vérifie chaque sortie avec l'oracle
`norelly_render::oracle` (fonctionnalité `fuzz-oracle` du crate, activée
par `fuzz/` seulement, jamais par l'application ; ses tests unitaires
tournent avec ceux du crate) : aucune balise active, aucun attribut
`on…`, aucune adresse `javascript:`, `vbscript:` ou `data:` hors image
matricielle, aucune adresse d'image ou de lien inattendue (ni
`remote-img://` quand les images distantes sont bloquées), la sortie
étant analysée comme par le tokeniseur HTML du navigateur.

Le workspace de fuzzing a son propre verrou, `fuzz/Cargo.lock`. Dans
`./scripts/check.sh`, `cargo deny` le contrôle avec `--locked` (verrou à
jour), et `scripts/check-fuzz-lock.mjs` vérifie que chaque crate qu'il
partage avec le `Cargo.lock` racine y a la même version et la même
empreinte : le fuzzing porte ainsi sur le code livré. Après un changement
de dépendances à la racine, resynchroniser en repartant du verrou racine :
`cp Cargo.lock fuzz/Cargo.lock` puis
`cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 > /dev/null`
(cargo retire les paquets inutiles au fuzzing et résout les siens,
`libfuzzer-sys` et `arbitrary`, à relire dans le diff). Le job nocturne
refuse un verrou périmé avant de compiler.

`./scripts/check.sh full` compile aussi toutes les cibles
(`cargo check --manifest-path fuzz/Cargo.toml --locked --bins`, Rust
stable) : un changement d'API qui casserait une cible se voit dans la PR,
pas seulement au job nocturne.

## Tests manuels

Ce que l'automatisation ne peut pas vérifier (vrais fournisseurs,
notifications du système, trousseau réel) : grille dans
[MANUAL-TESTS.md](../MANUAL-TESTS.md), à rejouer avant chaque version.
