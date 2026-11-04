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
  - Couverture effective des extensions à confirmer en phase 2 (voir
    PLAN.md). Si une extension manque, on le signale avant de la
    contourner.
- **Vérification de la phase 2** (2026-11-01, `imap-next` 0.3.4,
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
    ordinaire (à vérifier contre Dovecot en phase 4) et sur l'heuristique
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
    attente et toutes les commandes suivantes sont bloquées. Constaté en
    phase 7 (test `append_sends_a_literal_and_reads_appenduid`).
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

- **Vérifications de la phase 4** (2026-11-02, Dovecot 2.4.5) :
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

- **Mise en œuvre (phase 5, 2026-11-02)** : conversations précalculées
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

- **Mise en œuvre (phase 8, 2026-11-03)** : recherche derrière le port
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

- **Mise en œuvre (phase 5, 2026-11-02)** :
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
  - Messages partiels (phase 4) : chaque partie est téléchargée avec ses
    en-têtes MIME (`BODY.PEEK[n.MIME]`), ce qui permet de rendre le corps
    et d'extraire une pièce jointe sans le message complet ; les parties
    différées sont téléchargées à la demande par la connexion de
    commandes de l'acteur du compte.
  - Résultats des tests end-to-end sous WebKitGTK : voir PLAN.md, phase 5.

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
- **Mise en œuvre (phase 3, 2026-11-02)** :
  - `tauri-specta` 2.0.0-rc.26, `specta` 2.0.0-rc.26 et
    `specta-typescript` 0.0.13, versions figées (RC). Le contrat est dans
    `app/src-tauri/src/ipc.rs` (vues, sans dépendance de `specta` dans le
    domaine) ; les bindings sont commités dans
    `app/src/lib/ipc/bindings.ts` et le test `bindings_are_up_to_date`
    échoue s'ils sont périmés (`./scripts/gen-bindings.sh` les régénère).
  - `specta` dépend de `paste`, signalé non maintenu (RUSTSEC-2024-0436,
    sans vulnérabilité) : exception ajoutée dans `deny.toml`, validée par
    le mainteneur le 2026-11-02, à réévaluer à chaque montée de specta.

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
    par `--profile integration`. Les briques sont ajoutées progressivement :
    - Dovecot et Mailpit dès la phase 1 ;
    - faux serveur scripté et certificats `rcgen` en phase 2 ;
    - Toxiproxy en phase 4 ;
    - Stalwart dans le job nocturne, après le MVP ;
  - **tests de contrat** : chaque port est testé contre son faux en
    mémoire et contre son implémentation réelle.
- **Mise en œuvre de l'end-to-end (phase 3, 2026-11-02)** : WebKitWebDriver
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
    Ces fichiers sont signalés au mainteneur à chaque fin de phase.

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
- **Contexte** : jusqu'à la phase 5, l'interface est écrite en dur en
  français. `app/src/lib/format.ts` impose `"fr-FR"`, et les erreurs
  passent à l'interface sous forme de texte français produit par le
  `Display` des erreurs Rust (`ErrorDto.message`, `Failed.message`). Le
  coût d'une reprise augmente à chaque écran. La phase 7 fait aussi
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
    dans la webview) par défaut, puis une préférence enregistrée en
    phase 9. Dates, heures, tailles et nombres passent par `Intl` avec
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
  - **Textes émis hors de la fenêtre** (notifications natives,
    phase 9) : ils sont produits par l'interface à partir d'un événement
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
  - Les métadonnées AppStream et le fichier `.desktop` sont traduits
    (phase 10).
