# Décisions d'architecture (ADR)

Format court : contexte, options, décision, conséquences. Toute dérogation
à une décision « Acceptée » doit être validée par le mainteneur avant
d'être implémentée, puis consignée ici dans une nouvelle entrée.

Contexte général : client IMAP/SMTP desktop (Windows, Linux), open source,
budget nul, faible empreinte, boîtes d'environ 50 000 messages sur 2 ou 3
comptes. Le mainteneur maîtrise Rust (et Qt/Dart), pas TypeScript. Le code
est majoritairement écrit par Claude et relu par le mainteneur.

---

## ADR-001 : framework applicatif Tauri 2

- **Statut** : Acceptée (2026-11-01)
- **Contexte** : il faut un moteur web pour le HTML des mails, une
  faible empreinte et des licences permissives.
- **Options** : Tauri 2 (Rust + webview système) ; Rust natif (iced/gpui)
  + webview embarqué ; Rust + Qt/QML (cxx-qt) ; Flutter + Rust ;
  Electron (écarté : empreinte).
- **Décision** : **Tauri 2.x** (stable, pas la 3.0 alpha), cœur en Rust,
  interface web légère.
- **Conséquences** :
  - Toute la logique métier et toutes les décisions de sécurité sont en
    Rust. Le TypeScript ne fait que de la présentation.
  - Risque WebKitGTK sous Linux (performances, versions selon les
    distributions), atténué par Flatpak (ADR-006).
  - Les listes doivent être paginées côté Rust, jamais transmises en masse
    par l'IPC.

## ADR-002 : bibliothèques IMAP, SMTP, MIME et TLS

- **Statut** : Acceptée (2026-11-01)
- **Contexte** : extensions requises (IDLE, CONDSTORE, QRESYNC, UIDPLUS,
  MOVE, SPECIAL-USE, COMPRESS) ; données reçues hostiles par nature.
- **Options IMAP** : `async-imap` (éprouvée, QRESYNC incomplet) ;
  `imap-codec` + `imap-next` (codec strict, fuzzé, extensions modélisées) ;
  parseur maison (rejeté).
- **Décision** :
  - IMAP : `imap-codec` + `imap-next` (feature `ext_condstore_qresync`,
    `starttls`), avec une couche de session maison derrière un port
    `RemoteMailbox` ;
  - SMTP : `lettre` (et construction des messages MIME) ;
  - MIME : `mail-parser` ;
  - TLS : `rustls` 0.23 + `rustls-platform-verifier`, avec un vérificateur
    qui gère les exceptions par empreinte ;
  - autres : `keyring`, `hickory-resolver`, `reqwest` (rustls),
    `quick-xml`.
- **Conséquences** :
  - Plus de code à écrire pour la session IMAP (pipelining, IDLE,
    reconnexion).
  - L'interopérabilité est sous notre responsabilité : on la vérifie contre
    Dovecot et un faux serveur scripté (ADR-007).
  - TLS 1.0 et 1.1 ne sont pas supportés : message d'erreur explicite.
  - Couverture effective des extensions : voir la vérification
    ci-dessous. Si une extension manque, on le signale avant de la
    contourner.
- **Vérification** (2026-11-01, `imap-next` 0.3.4,
  `imap-codec` 2.0.0-alpha.9, `imap-types` 2.0.0-alpha.7, `lettre`
  0.11.23), faite dans le code source :

  | Extension | Couverture | Où |
  |---|---|---|
  | IDLE | complète (commande, `+`, `DONE`) | `CommandBody::Idle`, `Client::set_idle_done` |
  | CONDSTORE / QRESYNC | complète avec `ext_condstore_qresync` : paramètres de SELECT/EXAMINE, `CHANGEDSINCE`, `VANISHED`, `UNCHANGEDSINCE`, `HIGHESTMODSEQ`, `MODSEQ` | `command.rs`, `response.rs` |
  | UIDPLUS | complète : `UID EXPUNGE`, codes `APPENDUID` / `COPYUID` | `CommandBody::ExpungeUid`, `Code::AppendUid` |
  | MOVE | complète (`MOVE`, `UID MOVE`) | `CommandBody::Move` |
  | ENABLE | complète (`ENABLE`, réponse `ENABLED`) | `CommandBody::Enable` |
  | SPECIAL-USE | **partielle** : attributs `\Sent`, `\Drafts`… lus comme attributs d'extension génériques, capacité lue comme capacité inconnue ; `LIST … RETURN (SPECIAL-USE)` et `CREATE … (USE …)` non modélisés | `FlagNameAttribute::Extension` |
  | COMPRESS | **partielle** : commande et capacité `COMPRESS=DEFLATE` modélisées dans le codec, mais aucune compression dans la couche d'E/S d'`imap-next` | `extensions/compress.rs` |

  Manques signalés au mainteneur, **sans contournement** :
  - SPECIAL-USE : on s'appuiera sur les attributs renvoyés par un `LIST`
    ordinaire (vérifié contre Dovecot, voir plus bas) et sur l'heuristique
    de nom déjà prévue ; `RETURN (SPECIAL-USE)` à demander en amont si
    nécessaire.
  - COMPRESS=DEFLATE : optimisation non indispensable au MVP, reportée.
    L'activer demanderait un flux de décompression dans notre pilote d'E/S
    (avec une borne contre les bombes de décompression).

  Défauts trouvés et contournés (à valider par le mainteneur, à signaler
  en amont) :
  - `imap-next` journalise en TRACE chaque fragment envoyé, **mot de passe
    compris** (`src/fragment.rs`, sans garde `debug_assertions`). Le niveau
    TRACE est supprimé à la compilation pour tout le binaire
    (`tracing/max_level_debug`, `Cargo.toml` du workspace). Le test
    `credentials_never_appear_in_logs_or_debug` échoue si on retire ce
    plafond (vérifié).
  - `imap-codec` 2.0.0-alpha.9 utilise des fonctions `nom` de la feature
    `alloc` sans l'activer : `nom` est ajouté en dépendance directe du crate
    `imap` avec cette feature.
  - Le flux `imap_next::stream::Stream` n'est pas utilisé (feature `stream`
    désactivée) : tampon de lecture non borné, et pas de contrôle des
    octets reçus avant STARTTLS. Notre pilote d'E/S
    (`crates/adapters/imap/src/connection.rs`) fournit les lignes une à une
    à la machine d'états, borne les tailles, applique les délais et rejette
    toute donnée reçue après la réponse à `STARTTLS`.
  - `imap-next` 0.3.4 ne libère une commande à littéral synchrone
    (`APPEND`) que si le serveur la refuse par `BAD` avant la
    continuation ; sur un `NO` (`[TRYCREATE]`…), le littéral reste en
    attente et toutes les commandes suivantes sont bloquées. Constaté par le test
    `append_sends_a_literal_and_reads_appenduid`.
    Contournement : une réponse étiquetée reçue avant `CommandSent`
    renvoie bien `Rejected`, mais la session est ensuite déclarée
    inutilisable (`literal_pending`) et seulement fermée ; l'appelant se
    reconnecte. À signaler en amont.
  - `lettre` 0.11.23 n'accepte pas de `rustls::ClientConfig` fourni par
    l'appelant (`TlsParameters` aux champs privés ; seuls des réglages
    `dangerous_accept_invalid_*` sont exposés). Contournement par l'API
    publique `AsyncSmtpConnection::connect_with_transport` : la
    négociation TLS (et STARTTLS) est faite par `norelly-tls`, puis le flux
    chiffré est confié à `lettre` ; après STARTTLS, une bannière `220`
    locale est placée devant le flux car `lettre` en attend une. `lettre`
    est compilé sans backend TLS. À proposer en amont : un constructeur
    `TlsParameters` à partir d'un `ClientConfig`.

- **Vérifications contre Dovecot** (2026-11-02, Dovecot 2.4.5) :
  - SPECIAL-USE : un `LIST "" "*"` ordinaire renvoie bien les attributs
    `\Sent`, `\Drafts`, `\Trash`, `\Junk` (test
    `dovecot_special_use_attributes_are_read_from_plain_list`) ; le rôle
    est déduit de ces attributs, puis du nom à défaut. `RETURN
    (SPECIAL-USE)` reste inutile.
  - QRESYNC : le `SELECT … (QRESYNC (uidvalidity modseq))` est envoyé
    **sans** liste d'UID connus. Avec elle, le serveur ne signale que les
    changements parmi ces UID (RFC 7162, 3.2.5.1) et omet les nouveaux
    messages : défaut révélé par la suite de contrat contre Dovecot. Les
    `VANISHED` d'UID inconnus localement sont ignorés par le domaine, et
    leur dépliage est borné.
  - COMPRESS=DEFLATE : toujours reporté (voir ci-dessus) ; la
    synchronisation fonctionne sans.

- **Dérogation validée par le mainteneur (2026-11-02)** : sans trousseau
  du système (aucun Secret Service sous Linux), les mots de passe peuvent
  être enregistrés dans un fichier **non chiffré**
  (`<données>/mots-de-passe.json`, droits 0600, écriture atomique), après
  consentement explicite dans le parcours d'ajout de compte. Le trousseau
  reste prioritaire : le fichier ne sert que s'il ne répond pas. Solution
  provisoire : à remplacer par mieux (fichier chiffré par un mot de passe
  principal, ou saisie à chaque lancement).

## ADR-003 : stockage local et recherche plein texte

- **Statut** : Acceptée (2026-11-01)
- **Options** : SQLite + FTS5 ; SQLite + Tantivy ; base clé-valeur +
  Tantivy.
- **Décision** :
  - **une base SQLite globale** (`rusqlite`, SQLite embarqué), en mode
    WAL, avec un seul thread rédacteur et plusieurs lecteurs ;
  - migrations avec `rusqlite_migration` ;
  - FTS5 à contenu externe, tokenizer `unicode61 remove_diacritics 2`,
    préfixe automatique sur les termes ;
  - mails bruts hors base, en fichiers `.eml.zst` classés par compte et
    par empreinte de contenu ;
  - pas de chiffrement au repos au MVP (on s'appuie sur le chiffrement
    disque de l'OS).
- **Conséquences** :
  - La recherche reste derrière le port `SearchIndex`, ce qui permet de
    passer à Tantivy plus tard.
  - Pas de racinisation ni de tolérance aux fautes au MVP.
  - L'objectif des 200 ms repose sur des index composites et des
    conversations précalculées.

- **Mise en œuvre (2026-11-02)** : conversations précalculées
  (migration 3). Le rattachement d'un message est décidé à l'insertion
  par le domaine (`conversation::assign`, version incrémentale de JWZ :
  identifiants connus par conversation, fusion, sujet en dernier recours
  pour une réponse seulement, dans une fenêtre de 31 jours) ; les agrégats
  par dossier (`conversation_mailboxes`, index par dossier et par rôle
  pour la boîte unifiée) sont tenus par des déclencheurs SQLite, quel que
  soit le chemin d'écriture. Une propriété vérifie que la version
  incrémentale converge vers JWZ quel que soit l'ordre d'arrivée. Une
  conversation n'est jamais scindée quand un message qui la reliait
  disparaît. Mesure (bench, 50k messages) : première page de
  conversations ≈ 0,3 ms, boîte unifiée ≈ 0,3 ms.

- **Mise en œuvre (2026-11-03)** : recherche derrière le port
  `SearchIndex`, requête analysée par le domaine (`search::parse_query`,
  arbre sans syntaxe de moteur). Migration 7 : table de contenu
  `message_texts` et index FTS5 `message_search` (préfixes 2 et 3 indexés),
  tenus par des déclencheurs dans la transaction de chaque écriture ; texte
  du corps extrait par l'adaptateur `render` (port `SearchTextExtractor`,
  passé en `&dyn` aux cas d'usage) et enregistré avec le corps. Chaque
  clause de texte devient une chaîne FTS5 entre guillemets, en paramètre
  lié : aucun mot de l'utilisateur n'est lu comme un opérateur. Classement
  BM25 pondéré (sujet 10, expéditeur 5, destinataires 3, texte 1),
  pagination par décalage plafonnée à 1 000 résultats. Mesure (bench,
  50k messages avec leur texte) : 0,4 à 43 ms selon la requête.

## ADR-004 : rendu sécurisé du HTML des mails (« A+ »)

- **Statut** : Acceptée (2026-11-01)
- **Options** : iframe `srcdoc` avec `allow-same-origin` (A) ; iframe
  `sandbox=""` opaque sans agent (B) ; webview Tauri séparé (C, rejeté :
  overlays masqués, clavier perdu, API `unstable`) ; conversion en texte
  (D).
- **Décision** :
  - **A+** : `<iframe src="mailview://msg/<id>" sandbox="allow-scripts">`,
    **sans** `allow-same-origin`, donc origine opaque ;
  - le protocole Rust `mailview://` sert le HTML assaini par `ammonia`,
    avec la CSP en en-tête HTTP : `default-src 'none'`,
    `script-src 'sha256-<agent>'`, `img-src mailpart: remote-img: data:`,
    `style-src 'unsafe-inline'` ;
  - un seul script autorisé : notre agent, qui communique par
    `postMessage` (hauteur, clic sur lien, touches de raccourci) ;
  - images `cid:` servies par `mailpart://`, images distantes par le
    proxy Rust `remote-img://` (sans cookies ni Referer, User-Agent
    neutre, taille bornée), sur autorisation par expéditeur décidée en
    Rust ;
  - liens ouverts par une commande Rust qui valide le schéma (`http`,
    `https`, `mailto`), dans le navigateur système ;
  - mode texte brut (D) disponible en option utilisateur.
- **Conséquences** :
  - Le protocole `postMessage` est une surface critique : messages
    validés côté interface (source, schéma, taille).
  - Tests end-to-end obligatoires : zéro requête vers un serveur canari,
    aucun script du mail exécuté, IPC Tauri inaccessible depuis
    `mailview://`. Si WebKitGTK se comporte différemment : repli
    documenté vers A, après validation du mainteneur.

- **Mise en œuvre (2026-11-02)** :
  - Adresses des protocoles selon la plateforme : `scheme://localhost/`
    sous Linux (WebKitGTK), `http://scheme.localhost/` sous Windows
    (WebView2) ; la CSP du document et celle de la fenêtre principale
    (`frame-src`) couvrent les deux formes.
  - L'agent (`crates/adapters/render/src/agent.js`) est embarqué tel quel ;
    son empreinte SHA-256 est calculée par `build.rs` et vérifiée par un
    test indépendant. CSP du document : `default-src 'none'`, agent par
    empreinte, `style-src 'unsafe-inline'`, images `mailpart:`,
    `remote-img:` et `data:` seulement, `form-action`, `base-uri`,
    `connect-src`, `frame-src`… à `'none'`.
  - Le CSS des mails est assaini en plus de la CSP (défense en
    profondeur) : échappements résolus, `@import` supprimé, `url()`
    externes retirées.
  - `remote-img://<message>/<index>` : l'index renvoie à la liste
    d'adresses établie par le dernier rendu (le mail ne peut pas désigner
    une adresse arbitraire) ; l'autorisation de l'expéditeur est revérifiée
    à chaque image. Le proxy n'accepte que des adresses publiques
    (résolution DNS filtrée, adresses IP littérales et redirections
    vérifiées) : un mail ne peut pas faire sonder le réseau local.
  - L'autorisation porte sur l'adresse `From`, falsifiable : elle ne
    protège que contre le pistage, les images passant toujours par le
    proxy sans cookies ni `Referer`.
  - Ouverture des liens et des pièces jointes : crate `open` (xdg-open,
    ShellExecute) appelé depuis Rust. `tauri-plugin-opener` est écarté car
    il tire zbus 5 (RUSTSEC-2026-0370 à 0372, voir `deny.toml`). Sélecteur
    de dossier : `tauri-plugin-dialog`, appelé depuis Rust seulement
    (aucune permission accordée à l'interface).
  - Pièces jointes : écriture par création exclusive dans le dossier
    choisi (jamais d'écrasement ni de lien suivi), nom assaini par le
    domaine ; les types exécutables ou à macros ne s'ouvrent pas, ils
    s'enregistrent seulement. Les pièces ouvertes sont écrites dans un
    dossier privé du cache de l'application.
  - Messages partiels : chaque partie est téléchargée avec ses
    en-têtes MIME (`BODY.PEEK[n.MIME]`), ce qui permet de rendre le corps
    et d'extraire une pièce jointe sans le message complet ; les parties
    différées sont téléchargées à la demande par la connexion de
    commandes de l'acteur du compte.
  - Résultats des tests end-to-end sous WebKitGTK :
    `app/e2e/specs/reading-security.e2e.mjs` (serveur canari
    `app/e2e/canary.mjs`) : A+ tient tel que décrit, sans repli vers A.

## ADR-005 : interface utilisateur

- **Statut** : Acceptée (2026-11-01)
- **Options** : Svelte 5 ; SolidJS ; React 19.
- **Décision** :
  - **Svelte 5** (runes) + TypeScript strict + Vite, gestionnaire de
    paquets pnpm ;
  - `bits-ui` pour les primitives accessibles ;
  - virtualisation maison au-dessus de `@tanstack/virtual-core` ;
  - jetons de design en variables CSS et CSS scopé, **sans Tailwind** ;
  - police système, icônes Lucide embarquées, aucune ressource réseau ;
  - contrat IPC généré depuis Rust par `tauri-specta` ;
  - **registre de commandes central**, qui alimente la palette Ctrl+K,
    les raccourcis (style Gmail), les menus contextuels et l'aide ;
  - thème clair ou sombre selon le système.
- **Conséquences** :
  - Le mainteneur relit le contrat côté Rust.
  - Pas de syntaxe Svelte 4.
- **Mise en œuvre (2026-11-02)** :
  - `tauri-specta` 2.0.0-rc.26, `specta` 2.0.0-rc.26 et
    `specta-typescript` 0.0.13, versions figées (RC). Le contrat est dans
    `app/src-tauri/src/ipc.rs` (vues, sans dépendance de `specta` dans le
    domaine) ; les bindings sont commités dans
    `app/src/lib/ipc/bindings.ts` et le test `bindings_are_up_to_date`
    échoue s'ils sont périmés (`./scripts/gen-bindings.sh` les régénère).
  - `specta` dépend de `paste`, signalé non maintenu (RUSTSEC-2024-0436,
    sans vulnérabilité) : exception ajoutée dans `deny.toml`, validée par
    le mainteneur le 2026-11-02, à réévaluer à chaque montée de specta.
- **Mise en œuvre (2026-11-06)** : le registre alimente aussi les
  éléments visibles à la souris (barres d'actions libellées, menus
  « Plus… » et « ⋯ » des comptes, boutons au survol, barre de sélection),
  décrits par zone dans `app/src/lib/visible-actions.ts` : aucune action
  n'est réservée au clavier.

## ADR-006 : packaging et mises à jour

- **Statut** : Acceptée (2026-11-01)
- **Options** : artefacts directs non signés (avec AppImage) ; canaux
  libres et gratuits ; magasins (Microsoft Store + Flathub).
- **Décision** :
  - **Windows** : NSIS par utilisateur, signé via SignPath Foundation
    (dès que le projet est accepté), et winget ;
  - **Linux** : Flathub en canal principal, .deb et .rpm en secondaire,
    **pas d'AppImage** ;
  - `tauri-plugin-updater` (signature minisign) pour NSIS uniquement, le
    module étant désactivé à la compilation pour les autres canaux ;
  - vérification des mises à jour **demandée au premier lancement**
    (oui présélectionné), modifiable dans les paramètres, sans aucun
    identifiant envoyé ;
  - identifiant d'application : `io.github.gbourquet.Norelly`.
- **Conséquences** :
  - Binaires Windows non signés tant que SignPath n'a pas validé le
    projet.
  - Build Flathub hors ligne (dépendances cargo et npm vendorisées).
- **Dérogations validées par le mainteneur (2026-11-05)**, autorisations
  du Flatpak :
  - `--talk-name=org.freedesktop.secrets` : accès direct au Secret
    Service, donc à tout le trousseau de session. Le portail Secret, qui donne un trousseau propre à
    l'application, passe par `oo7` et donc par zbus, écarté par
    `deny.toml`. On garde l'accès direct, comme les autres clients mail
    sous Flatpak. À réexaminer si zbus redevient acceptable.
  - `--filesystem=xdg-download` (lecture et écriture) et
    `--filesystem=xdg-documents:ro` : le sélecteur de fichiers GTK3 de
    `tauri-plugin-dialog` ne passe pas par le portail. Les pièces jointes
    s'enregistrent dans Téléchargements et se joignent depuis
    Téléchargements ou Documents. À retirer quand le sélecteur passera par
    le portail.

## ADR-007 : stratégie de tests

- **Statut** : Acceptée (2026-11-01)
- **Options serveurs** : GreenMail (extensions insuffisantes) ; Dovecot +
  Mailpit + Toxiproxy + faux serveur scripté ; Stalwart tout-en-un.
- **Décision** :
  - **Rust** : `cargo nextest`, `proptest`, `cargo-fuzz` (job nocturne) ;
  - **front** : `vitest` + `@testing-library/svelte` ;
  - **end-to-end** : `tauri-driver` + WebdriverIO, sous Linux ;
  - **performance** : `criterion` + générateur de boîte de 50k messages ;
  - **intégration** via `testcontainers`, dans des fichiers de test nommés
    `tests/integration*.rs`, exclus du profil nextest par défaut et lancés
    par `--profile integration`. Les briques :
    - Dovecot et Mailpit ;
    - faux serveur scripté et certificats `rcgen` ;
    - Toxiproxy ;
    - Stalwart dans le job nocturne ;
  - **tests de contrat** : chaque port est testé contre son faux en
    mémoire et contre son implémentation réelle.
- **Dérogation validée par le mainteneur (2026-11-05)** : l'image Stalwart
  du job nocturne est figée en v0.11.8 (par empreinte,
  `crates/test-support/src/containers.rs`). C'est la dernière version
  configurable par un seul `config.toml` ; les suivantes demandent une
  configuration par l'API au démarrage. À réexaminer si une version
  récente permet de nouveau une configuration déclarative simple.
- **Mise en œuvre de l'end-to-end (2026-11-02)** : WebKitWebDriver
  n'est pas fourni par toutes les distributions (absent d'Arch, par
  exemple). `pnpm -C app e2e` (`app/e2e/run.sh`) exécute donc tauri-driver
  et WebdriverIO dans une image Ubuntu 24.04 (`app/e2e/Dockerfile`), avec
  Xvfb, D-Bus et GNOME Keyring (vrai Secret Service), contre Dovecot et
  Mailpit sur un réseau Docker interne (`--internal`, sans accès extérieur ; la
  compilation se fait avant, dans une étape séparée), avec des certificats autosignés
  générés à chaque exécution.
- **Conséquences** :
  - Tests d'intégration uniquement sous Linux (les runners Windows
    n'exécutent pas de conteneurs Linux).
  - Le seuil des 200 ms est vérifié en local et dans le job nocturne, pas
    à chaque commit.
  - Tests manuels sur Gmail, Fastmail et Outlook, sans identifiants en
    CI.

## ADR-008 : architecture hexagonale dosée

- **Statut** : Acceptée (2026-11-01)
- **Contexte** : le mainteneur relit en priorité le domaine Rust et veut
  pouvoir remplacer une brique d'infrastructure.
- **Décision** :
  - **`crates/domain`** : règles pures, sans E/S, et ports (traits). Il
    contient :
    - le rapprochement de synchronisation ;
    - la file d'actions hors ligne ;
    - les fils de conversation (JWZ) ;
    - les règles de rédaction ;
    - les politiques de sécurité ;
    - la stratégie d'autodécouverte ;
    - l'analyse des requêtes de recherche.

    Dépendances autorisées : `thiserror`, `serde` (vérifié en CI).
  - **`crates/application`** : cas d'usage async qui orchestrent les
    ports.
  - **`crates/adapters/*`** : implémentations des ports.
  - **`app/src-tauri`** : adaptateur entrant et point d'assemblage.
  - **CQRS léger** : les écritures passent par le domaine ; les lectures
    d'écran passent par des ports de requête taillés pour l'écran
    (pagination par curseur), implémentés en SQL.
  - Dispatch statique dans les chemins chauds, `dyn` au point
    d'assemblage.
- **Conséquences** :
  - Environ 10 à 15 % de code en plus.
  - Le code de sécurité vit aussi dans l'infra : TLS, STARTTLS,
    assainisseur, protocoles personnalisés, écriture des pièces jointes.
    Ces fichiers sont signalés au mainteneur dans chaque PR qui les
    touche.

## ADR-009 : licence et politique de dépendances

- **Statut** : Acceptée (2026-11-01)
- **Décision** :
  - application sous **`MIT OR Apache-2.0`** ;
  - dépendances permissives ; MPL-2.0 tolérée ; LGPL tolérée si liée
    dynamiquement ; **GPL et AGPL refusées** ;
  - contrôle par `cargo-deny` (licences et avis de sécurité) et
    `pnpm audit`.
- **Conséquences** : les serveurs de test sous AGPL (Stalwart) sont
  admis, car ils ne sont pas liés à notre code.
- **Dérogation temporaire validée par le mainteneur (2026-11-06)** :
  l'avis RUSTSEC-2026-0440 (`astral-tokio-tar` non maintenu depuis le
  2026-11-03, sans vulnérabilité connue) est ignoré dans `deny.toml`. La
  bibliothèque est tirée par `testcontainers` 0.28.0, dernière version, et
  ne sert qu'aux tests d'intégration. Un correctif est proposé chez
  testcontainers-rs : à la prochaine version, mettre à jour
  `testcontainers` et retirer l'exception.

## ADR-010 : éditeur de rédaction

- **Statut** : Acceptée (2026-11-01)
- **Options** : Tiptap avec mise en forme minimale ; texte brut seul.
- **Décision** :
  - **Tiptap** (ProseMirror, MIT) : gras, italique, liens, listes,
    citation ;
  - le HTML produit est réassaini en Rust ;
  - envoi en `multipart/alternative` avec une version texte.
- **Conséquences** : dépendance front supplémentaire ; notre propre HTML
  est traité comme non fiable.

## ADR-011 : internationalisation

- **Statut** : Acceptée (2026-11-02)
- **Contexte** : au moment de la décision, l'interface est écrite en dur en
  français. `app/src/lib/format.ts` impose `"fr-FR"`, et les erreurs
  passent à l'interface sous forme de texte français produit par le
  `Display` des erreurs Rust (`ErrorDto.message`, `Failed.message`). Le
  coût d'une reprise augmente à chaque écran. La rédaction fait aussi
  entrer la langue dans le domaine : préfixes de réponse étrangers et
  en-tête de citation.
- **Options** : Paraglide JS (catalogues compilés, typés) ; `svelte-i18n`
  (ICU à l'exécution, stores Svelte 4) ; catalogues maison ; traduction
  côté Rust (Fluent) avec du texte prêt à afficher envoyé par l'IPC.
- **Décision** :
  - **La traduction se fait uniquement dans l'interface.** Le front est
    déjà la couche de présentation. Fluent côté Rust demanderait une
    seconde chaîne d'outils et ferait transiter du texte par l'IPC.
  - **Paraglide JS 2** (inlang) :
    - chaque message devient une fonction TypeScript typée ;
    - une clé absente ou un paramètre manquant fait échouer
      `pnpm -C app check` ;
    - aucun moteur ne s'exécute au rendu, les catalogues inutilisés sont
      éliminés au build ;
    - les pluriels suivent ICU. La version est figée et la licence
      contrôlée à l'ajout (ADR-009).
  - Catalogues dans `app/messages/{fr,en}.json`. **Le français est la
    langue source** ; l'anglais sert de repli pour toute autre locale.
  - **Choix de la langue** : la langue du système (`navigator.language`
    dans la webview) par défaut, puis une préférence enregistrée dans les
    paramètres. Dates, heures, tailles et nombres passent par `Intl` avec
    la locale active, jamais une locale écrite en dur.
  - **Contrat IPC** : Rust n'envoie **aucun texte destiné à
    l'utilisateur**.
    - Les erreurs et les issues deviennent des **codes énumérés avec
      des paramètres structurés** (hôte, protocole, dossier…).
    - Un champ `detail: Option<String>` peut transporter un texte
      technique brut, par exemple la réponse `NO` d'un serveur IMAP. Il
      est affiché tel quel, comme une citation, et n'est jamais traduit.
    - Le `Display` des erreurs Rust reste en français et sert uniquement
      aux journaux.
  - **Domaine** : il ne connaît aucune langue, sauf deux exceptions
    explicites.
    - **Reconnaissance des préfixes de sujet** (`Re`, `RE`, `Fwd`, `Fw`,
      `TR`, `AW`, `WG`, `SV`, `VS`, `Antw`, `RIF`, `R`, et `RE :` avec
      espace avant les deux-points…) : c'est une liste de données fixe,
      testée par `proptest`, qui sert à ne pas empiler les préfixes.
      **Les préfixes produits restent toujours `Re:` et `Fwd:`**, quelle
      que soit la langue (usage RFC 5322, et lisibles par tous les
      clients).
    - **En-tête de citation** (« Le … a écrit : ») : l'interface le
      construit avec la langue et le format de date de l'utilisateur, et
      le transmet au domaine comme une donnée de rédaction ; le domaine
      ne fait que l'insérer.
  - **Textes émis hors de la fenêtre** (notifications natives) : ils sont produits par l'interface à partir d'un événement
    Rust, car la webview tourne encore quand la fenêtre est masquée. Si
    ce n'est pas possible, on reviendra devant le mainteneur avant
    d'introduire un catalogue côté Rust.
- **Conséquences** :
  - Le contrat IPC change : `ErrorDto`, `AddAccountResultDto::Failed`
    et `TestConnectionOutcome` passent à des codes. C'est un changement
    cassant, mais les bindings sont régénérés et l'application n'est pas
    encore distribuée.
  - Une nouvelle chaîne d'interface = une clé dans les deux catalogues.
    Un test front vérifie que `fr.json` et `en.json` ont exactement les
    mêmes clés, et `./scripts/check.sh` l'exécute.
  - Les tests end-to-end tournent avec la locale `fr` imposée et ciblent
    de préférence les rôles ARIA plutôt que les textes.
  - Les métadonnées AppStream et le fichier `.desktop` sont traduits.

## ADR-012 : carnet d'adresses

- **Statut** : Acceptée (2026-11-07) ([plan de l'epic](plans/carnet-adresses.md)).
- **Contexte** : les seules adresses connues sont calculées à la volée à
  partir des messages (expéditeurs reçus, destinataires envoyés), pour
  l'autocomplétion. Il manque un carnet que l'utilisateur alimente,
  importe depuis son ancien client (Gmail, Outlook, Thunderbird, Apple) et
  exporte, ainsi que des listes de diffusion.
- **Options** :
  - stockage : local (SQLite) ; local et synchronisé en CardDAV ;
  - portée : un carnet unique ; un carnet par compte ;
  - formats : bibliothèque vCard existante ; analyseur écrit pour le
    projet.
- **Décision** :
  - **Carnet local** dans la base SQLite, modèle calqué sur vCard
    (nom affiché, prénom, nom, surnom, plusieurs adresses dont une
    préférée, téléphones, organisation, notes, favori) pour qu'une
    synchronisation **CardDAV** puisse s'ajouter plus tard sans reprise
    du schéma ;
  - **un carnet unique** pour tous les comptes (la boîte unifiée affiche
    le même nom quel que soit le compte) ;
  - règles (validation, nom affiché, plan d'import, doublons, listes) dans
    le domaine ; lectures d'écran par un port de requête paginé ;
  - formats dans un adaptateur `contacts-io` derrière un port
    `ContactCodec` : vCard 2.1/3.0/4.0 et LDIF **écrits pour le projet**
    (formats simples, limites et fuzzing maîtrisés), CSV Google, Outlook
    et Thunderbird par la crate **`csv`** (MIT ou Unlicense) ;
  - le fichier est choisi par le sélecteur natif et lu en Rust ;
    l'interface ne reçoit qu'un aperçu chiffré.
- **Conséquences** :
  - deux migrations (contacts, listes) ; nom du carnet dans les listes
    d'écran du courrier : **dérogation validée par le mainteneur le
    2026-11-07 (lot 3)**, pas de jointure SQL sur l'adresse normalisée
    mais une requête par page par `ContactDirectory::by_addresses` (index
    unique sur `address_key`), pour que la comparaison des adresses reste
    celle du domaine (`AddressKey`) ; requêtes du courrier et bancs 50k
    inchangés ;
  - suggestions de destinataires : **écart au plan validé par le
    mainteneur le 2026-11-07 (lot 4)**, `ContactQuery::suggest` reste
    limité aux adresses collectées ; le cas d'usage lit le carnet par
    `ContactDirectory::page` (favoris, puis tous) et une règle du domaine
    fusionne les deux listes (carnet d'abord, une fois par `AddressKey`),
    sans nouvelle requête SQL ni jointure entre contacts et messages ;
  - un nom de fiche qui imite une adresse est écarté à l'affichage
    (courrier, suggestions), comme un nom d'en-tête ; le refus à la
    saisie attend l'import (lot 6) et l'assainissement des fiches
    existantes ;
  - le nom du carnet s'affiche pour une adresse `From` qui peut être
    falsifiée : il ne vaut pas preuve d'identité (constat à porter par la
    revue de sécurité) ;
  - listes de diffusion : **écart au plan validé par le mainteneur le
    2026-11-07 (lot 5)**, une liste regroupe des **contacts**, pas des
    adresses : pas d'adresse choisie par membre, chacun est écrit à son
    adresse préférée au moment de l'envoi (modifier une fiche ne laisse
    jamais une adresse périmée dans une liste). L'envoi en Cci est une
    propriété de la liste, choisie dans son dialogue ; les listes sont
    proposées par une commande à part (`suggest_contact_lists`),
    `suggest_contacts` restant inchangé ;
  - import (lot 6) : **écarts au plan validés par le mainteneur le
    2026-11-08**. La forme NFC des noms importés est décidée par
    `plan_import` à travers un port `TextNormalizer` (le domaine
    n'embarque pas les tables Unicode), implémenté dans `contacts-io` par
    `icu_normalizer`, déjà présent dans l'arbre (`idna`, licence
    Unicode-3.0) ; un nom importé qui imite une adresse est **retiré**, la
    fiche gardée (les exports de Gmail mettent souvent l'adresse comme
    nom) ; les caractères invisibles (Cf, Zl, Zp, sélecteurs de variante,
    remplisseurs) font rejeter une fiche **importée** seulement : la
    saisie et les fiches existantes gardent la règle de
    `ContactDraft::new`. Les deux politiques de doublons (« ignorer »,
    « compléter » sans rien écraser) sont livrées, choisies dans
    l'aperçu ; les listes d'un fichier ne sont pas importées ; jeux
    d'essai **reconstitués** dans la structure des exports réels avec des
    données fictives, et non de vrais exports anonymisés (écart validé
    par le mainteneur après coup, le 2026-11-08) ;
  - export (lot 7) : **choix validés par le mainteneur le 2026-11-08**,
    avant le code sauf mention contraire. Formats vCard 4.0 (par défaut)
    et 3.0, CSV aux colonnes de Google (plus `Name` pour le nom affiché :
    écart validé **après coup**), LDIF de
    Thunderbird, écrits pour le projet sans nouvelle dépendance (la crate
    `csv` écrit les cellules). Ce que chaque format retient est une règle
    du domaine (`export_card`) : adresse préférée écrite en premier
    partout ; étiquettes vCard en minuscules ; pas d'étiquette d'adresse
    en LDIF, téléphones rangés dans les attributs de Thunderbird
    (étiquette inconnue → `telephoneNumber`) ; `FN` vCard toujours écrit
    (nom composé à défaut de nom affiché, vide sans nom ou trop long :
    règle validée **après coup**). Le favori n'est pas exporté.
    Toute cellule CSV qui commence par `=`, `+`, `-`, `@`, tabulation ou
    CR, téléphones compris, est préfixée d'une apostrophe, et l'import
    retire cette apostrophe (aller-retour exact dans Norelly ; Gmail ou
    Outlook montrent « '+33… »). Les bornes de l'export sont celles de
    l'import (25 000 fiches, 20 Mio) : tout export se réimporte.
    « Exporter… » dans l'en-tête de la vue Contacts (repli dans
    « Plus… ») et dans la barre des contacts cochés, format choisi dans
    une fenêtre ; listes de diffusion non exportées. Conséquence vue sur
    les captures, validée **après coup** : à 1 280 px, « Importer… »
    passe aussi dans « Plus… » de l'en-tête (gardé tel quel) ;
  - doublons (lot 8) : **règles validées par le mainteneur le
    2026-11-08, avant le code**. Écart au plan : deux contacts ne
    partagent jamais une adresse (index unique, l'import « ignorer »
    écarte la fiche entière), le critère « adresses qui se recoupent »
    est remplacé par le **téléphone**. Sont proposés comme doublons les
    contacts de **même nom** (nom affiché comparé mot à mot en NFC, sans
    casse ni ordre des mots, un seul mot suffit ; fiche sans nom ou nom
    imitant une adresse écartés) ou de **même numéro** (chiffres et `+`,
    au moins 6 chiffres), regroupés par transitivité. La **fusion**
    garde la fiche choisie par l'utilisateur (par défaut la plus
    complète), complète ses champs vides par les autres fiches sans rien
    écraser, réunit adresses (la préférée reste celle de la fiche
    gardée) et téléphones (au chiffre près), met les notes distinctes
    bout à bout (ligne vide entre deux), garde le favori si l'une l'est,
    reporte les listes de diffusion des fiches absorbées et supprime
    celles-ci, en une transaction. Une fusion qui dépasserait les bornes
    d'une fiche (20 adresses, 20 téléphones, 10 000 caractères de notes)
    est refusée. « Rechercher les doublons… » dans « Plus… » de l'en-tête
    de la vue Contacts ouvre une fenêtre qui présente les groupes un par
    un (« Fusionner », « Passer ») ; un groupe passé n'est pas mémorisé
    (backlog). Suites de la revue de sécurité, **validées par le
    mainteneur le 2026-11-08, avant le code** : un groupe garde au plus
    20 fiches (les plus complètes ; groupe signalé « tronqué »), une
    recherche renvoie au plus 500 groupes, et l'écran ne reçoit que les
    champs affichés (pas les notes) ; la fusion relit les fiches dans sa
    transaction et refuse sans rien écrire si l'une a changé depuis la
    recherche (`Conflict`) ; avant de fusionner, la fenêtre montre le nom
    et l'adresse préférée que gardera la fiche, calculés par Rust pour
    les fiches cochées (aperçu qui signale aussi une fusion trop grosse).
    Conséquence vue sur les captures, validée **après coup** : « Plus… »
    est désormais toujours dans l'en-tête de la vue Contacts, et à
    1 280 px « Export… » y passe aussi en anglais (gardé tel quel).
    Vérification de l'interface, écart de méthode validé **après coup**
    par le mainteneur : les e2e ne capturent qu'en clair ; le sombre, le
    survol, le menu « Plus… » et les libellés à 1 280 px ont été vérifiés
    en montant seuls la fenêtre des doublons et l'en-tête de la vue
    Contacts (vrais composants, données factices) dans Chrome sans
    interface, et non dans `tauri dev` ;
  - photos de contact et CardDAV restent au backlog.

## ADR-013 : connexion OAuth 2.0 (XOAUTH2)

- **Statut** : Acceptée (2026-11-11), validée par le mainteneur avant le
  code (ticket « Se connecter avec Microsoft »).
- **Contexte** : Outlook.com et Hotmail refusent le mot de passe en IMAP
  et SMTP, et Microsoft ne propose pas de mot de passe d'application aux
  comptes personnels. Google suivra (même mécanisme).
- **Options** :
  - bibliothèque `oauth2` ; flux écrit pour le projet sur les dépendances
    existantes ;
  - redirection : schéma d'URL propre à l'application ; boucle locale
    (RFC 8252) ;
  - stockage : jeton d'accès persistant ; jeton de rafraîchissement seul,
    dans le trousseau.
- **Décision** :
  - flux « code d'autorisation + PKCE » (RFC 7636) **écrit pour le
    projet** : règles (URL, formulaires, contrôle du retour, expiration,
    classement des erreurs, destination du jeton) dans le domaine
    (`oauth.rs`) ; adaptateur `crates/adapters/oauth` sur `reqwest` +
    `norelly-tls`, `sha2`, `getrandom` et `serde_json`, déjà au workspace :
    **aucune dépendance nouvelle** ;
  - navigateur du système (crate `open`, déjà utilisée), redirection
    `http://localhost:<port>/` écoutée sur `127.0.0.1`, port éphémère ;
  - configuration par fournisseur, valeur du domaine : Microsoft
    (`consumers`, scopes `outlook.office.com` IMAP et SMTP,
    `offline_access`, identifiant client public de l'inscription
    Entra ID ; `openid email profile` à la connexion interactive, pour
    vérifier que le compte choisi est celui de l'adresse) ; Google ne sera qu'une configuration de plus ;
  - Google (2026-11-14) : adresses `gmail.com`, scope
    `https://mail.google.com/`, `openid email` à la connexion interactive,
    `access_type=offline` et `prompt=select_account consent` (sans quoi
    Google ne redonne pas de jeton de rafraîchissement), rafraîchissement
    sans scope ; le client « application de bureau » de Google exige son
    `client_secret` au point d'accès des jetons. Ni lui ni l'identifiant
    client ne sont écrits dans le dépôt (public) : la protection au push de
    GitHub les refuse, et un secret publié peut être révoqué par Google.
    Décision du mainteneur : ils sont fournis **à la compilation**
    (`NORELLY_GOOGLE_CLIENT_ID`, `NORELLY_GOOGLE_CLIENT_SECRET`, lus par
    `option_env!` dans le domaine, sans E/S), depuis les secrets de la CI
    pour les versions publiées (build refusé s'ils manquent). Ils restent
    non confidentiels (même valeur dans chaque copie, code protégé par
    PKCE). Sans eux (développement, tests), « Se connecter avec Google »
    n'est pas proposé et une adresse Gmail garde le mot de passe
    d'application (`OAuthProvider::offered_for`), et un compte Google déjà
    enregistré n'envoie plus rien à Google (refus définitif,
    `ProviderNotConfigured`) ; les e2e compilent avec des valeurs
    factices ; secrets rangés dans l'environnement `release` (jobs Linux et
    Windows) ;
  - identité Google : `iss`, `aud` et `email_verified` vérifiés ; scope de
    messagerie exigé parmi les scopes accordés (Google permet d'en
    décocher) ;
    fournisseur `google` accepté en base (migration 14) ;
  - jeton de rafraîchissement dans le trousseau, sous la clé IMAP du
    compte ; jeton d'accès en mémoire ; fournisseur enregistré sur le
    compte (migration 13) ;
  - un jeton n'est présenté qu'aux serveurs du fournisseur, en TLS ou
    STARTTLS (suite de la revue de sécurité) ;
  - le jeton est renouvelé avant chaque connexion par une enveloppe des
    connecteurs IMAP et SMTP (`FreshTokens`), posée au point d'assemblage :
    l'acteur de compte et l'envoi restent inchangés.
- **Conséquences** :
  - « Reconnexion nécessaire » quand le jeton de rafraîchissement est
    refusé : bouton « Se reconnecter » visible sur le compte ;
  - la page renvoyée au navigateur après la connexion est le seul texte
    affiché hors de l'interface, neutre et bilingue, sans donnée ;
  - un compte OAuth ne passe jamais par l'autodécouverte ni la saisie
    manuelle des serveurs ;
  - application Google non validée : avertissement de Google expliqué à
    l'écran avant le navigateur, et nombre d'utilisateurs limité par
    Google ; une connexion Google annulée à l'ajout (refus, abandon, délai)
    devient `GoogleSignInRefused`, qui oriente vers le mot de passe
    d'application, toujours proposé à côté.
