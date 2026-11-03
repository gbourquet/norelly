# Plan de développement de Norelly

Règles :
- On travaille uniquement sur la phase en cours et on coche les cases au fur
  et à mesure.
- Une phase n'est terminée que si sa **condition de fin** passe et si la
  revue sécurité est consignée dans `docs/REVIEWS.md`, avec les constats
  critiques et hauts corrigés.
- Un test impossible à faire passer : on documente le blocage ici, sous la
  tâche concernée, et on s'arrête. Ramollir, désactiver ou contourner un
  test est interdit.

Commandes de référence (détail dans `scripts/check.sh`) :
- `./scripts/check.sh` : rapide, sans Docker. Format, lint Rust et front,
  tests unitaires Rust et front, `cargo deny`, vérification de la règle de
  dépendance du domaine.
- `./scripts/check.sh full` : tout ce qui précède, plus le build du front,
  le build du workspace et les tests d'intégration (`cargo nextest run
  --workspace --profile integration`, avec Docker).

---

## Phase 0 : choix techniques ✅

- [x] Questions de contexte
- [x] Décisions a à g, plus architecture, licence et éditeur (voir DECISIONS.md)

## Phase 1 : mise en place

- [x] `docs/DECISIONS.md` (ADR-001 à ADR-010)
- [x] `docs/PLAN.md` (ce fichier)
- [x] `CLAUDE.md`
- [x] `.claude/settings.json` : hook PostToolUse (Edit|Write), qui formate et lint les fichiers modifiés
- [x] `.claude/agents/security-reviewer.md`
- [x] Workspace Cargo : `domain`, `application`, `adapters/*`, `test-support`, `app/src-tauri`
- [x] Le crate `domain` ne dépend que de `thiserror` et `serde`, vérifié par `scripts/check-domain-deps.sh`
- [x] Front Svelte 5 + TS strict + Vite dans `app/` ; scripts pnpm `check`, `lint`, `format`, `test`, `build`
- [x] Shell Tauri 2 minimal qui compile et affiche la fenêtre vide « Norelly »
- [x] Test vide qui passe dans chaque crate et côté front
- [x] Profil nextest `integration` + test de fumée : Dovecot et Mailpit démarrent via testcontainers et répondent (bannière IMAP, bannière SMTP)
- [x] `deny.toml` (politique de licences ADR-009, avis de sécurité), `rustfmt.toml`
- [x] `scripts/check.sh` (modes rapide et `full`)
- [x] CI GitHub Actions `ci.yml` : Linux (check full) et Windows (check rapide + build)
- [x] `LICENSE-MIT`, `LICENSE-APACHE`, `README.md` minimal
- [x] Premier commit

**Condition de fin** : `./scripts/check.sh full` passe sans erreur.

## Phase 2 : transport et sécurité de connexion

- [x] Vérifier dans le code source d'`imap-codec`/`imap-next` la couverture de IDLE, CONDSTORE, QRESYNC, UIDPLUS, MOVE, SPECIAL-USE, COMPRESS, ENABLE, puis mettre ADR-002 à jour (signaler les manques au mainteneur avant tout contournement)
- [x] Vérifier que `lettre` accepte notre `rustls::ClientConfig` (vérificateur personnalisé), sinon documenter le blocage
  - Non : `TlsParameters` n'expose pas de constructeur à partir d'un `ClientConfig`. Contournement par `connect_with_transport` (TLS négocié par `norelly-tls`), détaillé dans ADR-002.
- [x] Domaine, types :
  - `ServerEndpoint` (hôte, port, `Security::{Tls, StartTls, Plain}`) ;
  - `Credentials`, avec un `Debug`/`Display` qui masque le secret ;
  - `ServerCapabilities`
- [x] Domaine, politiques :
  - `TlsTrustPolicy` : certificat valide, exception mémorisée (hôte + empreinte SHA-256), ou rejet avec `CertificateIssue` détaillé ;
  - `PlaintextPolicy` : refus sans consentement explicite par serveur.

  Le tout avec des tests unitaires exhaustifs.
- [x] Domaine, ports : `SecretStore`, `CertificateExceptionStore`, `RemoteMailboxConnector`, `MailSender`, `Authenticator` (PLAIN, LOGIN ; XOAUTH2 prévu, renvoyant `Unsupported`)
- [x] `test-support` : faux serveur IMAP scripté (tokio, étapes « attendre / envoyer », TLS optionnel) et faux serveur SMTP scripté
- [x] `test-support` : générateur de certificats `rcgen` (CA de test, autosigné, expiré, mauvais nom d'hôte)
- [x] `adapters/tls` : `ClientConfig` qui combine le vérificateur de la plateforme et l'exception par empreinte, plus des tests (autosigné rejeté, expiré rejeté, mauvais hôte rejeté, exception acceptée pour la même empreinte et le même hôte seulement, empreinte changée rejetée)
- [x] `adapters/imap`, connexion TLS implicite (993) et STARTTLS (143) :
  - STARTTLS obligatoire si annoncé, sinon erreur (aucun repli silencieux en clair) ;
  - capacités relues après TLS ;
  - données injectées avant la poignée de main TLS rejetées
- [x] `adapters/imap` : authentification LOGIN et AUTHENTICATE PLAIN, respect de LOGINDISABLED, LOGOUT, délais d'attente
- [x] Tests faux serveur :
  - STARTTLS stripping (aucun identifiant envoyé) ;
  - injection après `STARTTLS` ;
  - LOGINDISABLED ;
  - clair refusé sans consentement ;
  - réponses malformées sans panique
- [x] `adapters/smtp` : transport `lettre` en 465 (TLS) et 587 (STARTTLS exigé), authentification, vérification de connexion. Tests faux serveur, dont STARTTLS stripping
- [x] `adapters/keyring` : `SecretStore` (Credential Manager / Secret Service) + faux en mémoire ; suite de contrat exécutée sur le faux (et sur le vrai en intégration si un Secret Service est disponible)
  - Sur le poste du mainteneur (2026-11-01), aucun Secret Service D-Bus n'est actif : le test `integration_keyring` le détecte et ne lance pas le contrat sur le vrai trousseau. À exécuter une fois sur un bureau avec GNOME Keyring ou KWallet, et sous Windows.
- [x] Test qui vérifie que les identifiants n'apparaissent jamais dans les logs `tracing` ni dans `Debug`
- [x] Intégration Dovecot : connexion en 993 et en 143 + STARTTLS avec la CA de test, authentification, capacités attendues présentes
- [x] Intégration Mailpit : envoi en 465 et en 587, message reçu vérifié via l'API Mailpit
- [x] Application : cas d'usage `TestConnection(paramètres)`, qui renvoie un résultat structuré (succès, problème de certificat, authentification, réseau)
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; moyennes corrigées)

**Condition de fin** : `./scripts/check.sh full` passe sans erreur.

## Phase 3 : comptes, autodécouverte, squelette de l'interface

- [x] Domaine : `Account`, `EmailAddress` (validation), stratégie d'autodécouverte (ordre ISPDB fournisseur, puis ISPDB Mozilla, puis SRV RFC 6186, puis sondage, puis saisie manuelle ; préférence TLS implicite > STARTTLS > clair)
- [x] `adapters/discovery` : client autoconfig (fournisseur + ISPDB) avec `reqwest`/rustls et parsing `quick-xml` (fixtures dans `fixtures/ispdb/`) ; SRV via `hickory-resolver` derrière un trait ; sondage des ports avec délai court et en parallèle
- [x] Tests : `wiremock` pour l'autoconfig, résolveur simulé, sondage contre le faux serveur ; XML hostile ou malformé sans panique
- [x] Application : `DiscoverSettings`, `AddAccount` (découverte, test de connexion, stockage du secret dans le trousseau, persistance du compte), `RemoveAccount` (supprime aussi le secret et les données locales)
- [x] Persistance minimale des comptes (premier schéma SQLite et migration), avec un test de contrat `AccountRepository` (mémoire et SQLite)
- [x] `tauri-specta` : génération des types TS ; test CI qui échoue si les bindings générés sont périmés
- [x] Interface :
  - jetons de design (`tokens.css`, clair et sombre) ;
  - mise en page en 3 panneaux redimensionnables ;
  - police système, icônes Lucide
- [x] Interface : registre de commandes central, avec des tests vitest (enregistrement, conflits de raccourcis, conditions d'activation)
- [x] Interface : palette Ctrl+K (bits-ui Command) et aide des raccourcis (`?`)
- [x] Interface, parcours d'ajout de compte (adresse + mot de passe) :
  - progression de la découverte ;
  - écran d'avertissement de certificat (empreinte, raison, « faire confiance à ce serveur ») ;
  - avertissement en cas de clair ;
  - saisie manuelle en dernier recours
- [x] Interface : suppression de compte avec confirmation
- [x] Test end-to-end (tauri-driver) : ajout d'un compte Dovecot de test de bout en bout
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; moyennes corrigées)

**Condition de fin** : `./scripts/check.sh full && pnpm -C app e2e` passent sans erreur.

## Phase 4 : stockage et synchronisation

- [x] Domaine : `Mailbox` (rôle SPECIAL-USE ou heuristique de nom), `MessageMeta`, `Flags`, `Uid`, `UidValidity`, `ModSeq`
- [x] Domaine : machine d'états pure de rapprochement :
  - synchronisation initiale ;
  - incrémentale CONDSTORE ;
  - QRESYNC (VANISHED) ;
  - repli sans extension (comparaison des UID) ;
  - changement d'UIDVALIDITY (resynchronisation complète)

  Tests exhaustifs et `proptest` (idempotence, convergence)
- [x] Domaine : politique de rapatriement (en-têtes + texte pour tous ; pièces jointes sous un seuil configurable)
- [x] `adapters/imap` : LIST (SPECIAL-USE), SELECT/EXAMINE (CONDSTORE/QRESYNC), FETCH (en-têtes, BODYSTRUCTURE, corps par parties), ENABLE, COMPRESS si disponible
  - COMPRESS=DEFLATE non activé : reporté par ADR-002 (aucune décompression dans la couche d'E/S d'`imap-next`) ; la synchronisation fonctionne sans. SPECIAL-USE et QRESYNC vérifiés contre Dovecot (ADR-002, vérifications de la phase 4).
- [x] `adapters/imap` : IDLE avec renouvellement (< 29 min), reconnexion avec backoff exponentiel
- [x] `adapters/sqlite` :
  - schéma messages, dossiers et état de synchro ;
  - index composites `(dossier, date DESC)` ;
  - mode WAL, un rédacteur et un pool de lecteurs ;
  - migrations testées (montée de version depuis chaque version précédente)
- [x] `adapters/blobs` : stockage `.eml.zst` par compte et empreinte, écriture atomique (fichier temporaire + renommage), test de contrat `BlobStore`
- [x] Application : acteur de compte (une connexion IDLE sur la boîte de réception, une connexion de commandes), `SyncAccount`, événements « dossier modifié »
- [x] Ports de requête : `FolderTreeQuery`, `MessageListQuery::page(dossier, curseur, n)`
- [x] Intégration Dovecot : synchronisation initiale, incrémentale, IDLE (message ajouté par APPEND côté serveur et reçu en moins de 5 s), UIDVALIDITY changée
- [x] Toxiproxy : coupure en pleine synchro, latence, connexion à moitié fermée ; l'état converge après la reprise
- [x] Générateur de boîte synthétique de 50k messages + bench `criterion` : première page < 50 ms et changement de dossier < 50 ms (requête seule)
  - Mesuré le 2026-11-02 (poste du mainteneur, profil bench) : première page ≈ 0,17 ms, changement de dossier ≈ 0,16 ms ; le bench échoue au-delà de 50 ms.
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; moyennes corrigées)

**Condition de fin** : `./scripts/check.sh full && cargo bench -p norelly-sqlite -- --test` passent sans erreur.

## Phase 5 : liste, conversations, lecture

- [x] Domaine : fils de conversation JWZ (`References`/`In-Reply-To`, regroupement par sujet en dernier recours), avec des tests à partir de cas réels anonymisés ; boîte unifiée
- [x] `adapters/sqlite` : table des conversations précalculée, mise à jour incrémentale ; `ConversationQuery`, liste unifiée paginée
- [x] `adapters/render` : MIME vers vue (`mail-parser`), choix de la meilleure alternative, charsets, pièces jointes listées
- [x] `adapters/render` :
  - liste blanche `ammonia` ;
  - réécriture `cid:` vers `mailpart://` et des images distantes vers un espace réservé ou `remote-img://` ;
  - suppression de `@import` ;
  - suite de tests sur `fixtures/hostile-html/` (au moins 40 vecteurs)
  - 56 vecteurs, audit balise par balise (`crates/adapters/render/tests/hostile_html.rs`).
- [x] Agent `postMessage` (hauteur, liens, touches) ; hash SHA-256 calculé au build ; CSP générée
- [x] Protocoles Tauri `mailview://`, `mailpart://`, `remote-img://` (proxy : sans cookies ni Referer, User-Agent neutre, taille et types bornés, autorisation par expéditeur vérifiée en Rust)
- [x] Commande `open_external` : schémas `http`, `https`, `mailto` uniquement, ouverture dans le navigateur système
- [x] Domaine et persistance : autorisation des images distantes par expéditeur, préférence « texte brut »
- [x] Pièces jointes :
  - ouverture et enregistrement ;
  - nom de fichier assaini (séparateurs, `..`, noms réservés Windows, caractères de contrôle, longueur) ;
  - écriture uniquement dans le dossier choisi via le sélecteur de fichiers
- [x] Interface : liste virtualisée (au-dessus de `@tanstack/virtual-core`), boîte unifiée, vue conversation (messages empilés, hauteur via l'agent), bannière « images bloquées »
- [x] Interface : navigation j/k, ouverture, retour, via le registre de commandes
- [x] Tests end-to-end de sécurité A+ :
  - zéro requête vers un serveur canari ;
  - aucun script du mail exécuté ;
  - IPC inaccessible depuis `mailview://` ;
  - `javascript:` jamais ouvert ;
  - touches renvoyées correctement
  - `app/e2e/specs/reading-security.e2e.mjs`, sous WebKitGTK (serveur canari `app/e2e/canary.mjs`) : A+ tient tel que décrit dans ADR-004, sans repli vers A.
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique ; 1 haute et 3 moyennes corrigées ; basses corrigées sauf compléments e2e reportés en phase 9)

**Condition de fin** : `./scripts/check.sh full && pnpm -C app e2e` passent sans erreur.

## Phase 6 : actions et hors ligne

- [x] Socle i18n (ADR-011) : Paraglide JS, catalogues `app/messages/{fr,en}.json`, extraction de toutes les chaînes existantes de l'interface, locale du système, `format.ts` sur la locale active, test d'égalité des clés des catalogues branché dans `check.sh`
- [x] Contrat IPC (ADR-011) : erreurs et issues en codes énumérés avec paramètres structurés et `detail` technique optionnel ; plus aucun texte utilisateur produit en Rust ; bindings régénérés
- [x] Domaine : actions (déplacer, supprimer, archiver, lu/non lu, drapeau), application optimiste, `pending_ops`, rejeu, résolution de conflits (message disparu, dossier supprimé, UIDVALIDITY changée)
- [x] Domaine : sémantique de « supprimer » (déplacement vers la Corbeille ; suppression définitive depuis la Corbeille) et d'« archiver » (dossier `\Archive` ou repli configurable)
- [x] `adapters/imap` : MOVE, ou à défaut COPY + STORE `\Deleted` + UID EXPUNGE (UIDPLUS), ou EXPUNGE avec garde-fou ; STORE des drapeaux avec UNCHANGEDSINCE
- [x] `adapters/sqlite` : table `pending_ops` durable, test de contrat
- [x] Application : file d'actions rejouée à la reconnexion, avec un test de bout en bout hors ligne puis en ligne (Toxiproxy)
- [x] Interface : raccourcis `e`, `#`, `s` (drapeau), `Shift+i`/`Shift+u`, `v` (déplacer, avec sélection de dossier dans la palette), sélection multiple (`x`), annulation de la dernière action
- [x] Interface : actions à la souris, branchées sur le registre de commandes (ADR-005) : barre d'outils de la vue de lecture (archiver, supprimer, déplacer, suivre, lu/non lu), mêmes actions au survol des lignes de la liste, menu contextuel au clic droit sur une conversation
- [x] Déplacement visible tout de suite dans le dossier de destination : copie locale provisoire créée avec l'opération (dossier de destination, compteurs), retirée à l'annulation ou à l'abandon, remplacée par le vrai message à la synchronisation qui suit le rejeu (sans trou ni doublon visible), ignorée par la synchronisation et par les actions
- [x] Intégration Dovecot : chaque action vérifiée côté serveur, avec et sans MOVE (capacité masquée par le faux serveur)
  - `crates/application/tests/integration_dovecot_actions.rs` : avec MOVE et UIDPLUS, sans MOVE, sans MOVE ni UIDPLUS. Contre Dovecot, les capacités sont masquées par le connecteur (`ImapConnector::hiding_capabilities`) ; le faux serveur sait aussi les masquer (`FakeImap::set_move_extensions`).
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; 2 moyennes corrigées ; basses reportées en phase 7 et 10)

**Condition de fin** : `./scripts/check.sh full && pnpm -C app e2e` passent sans erreur.

## Phase 7 : rédaction et envoi

- [x] Rattrapage de couverture (TDD, voir CLAUDE.md), mesuré le 2026-11-02 sur `master` (Rust : `cargo llvm-cov`, tests unitaires et d'intégration, 87 % des lignes ; front : Vitest + coverage-v8, 71 % des lignes). Le code non couvert reçoit des tests qui décrivent son comportement ; un test qui révèle un défaut suit la règle « bug = d'abord un test rouge » :
  - `app/src-tauri` (adaptateur entrant), le moins couvert : `mail.rs` (7 %), `actions.rs` (14 %), `reading.rs` (22 %), `protocols.rs` (26 %), `ipc.rs` (34 %), `commands.rs` (38 %), `remote_images.rs` (68 %). Commandes IPC et protocoles testés sans webview, contre le faux serveur ou Dovecot en conteneur : revalidation des entrées, codes d'erreur, `mailview://`, `mailpart://`, `remote-img://`, enregistrement et ouverture des pièces jointes ;
  - `adapters/keyring/src/lib.rs` (38 %) : chemin Secret Service, joué seulement si un trousseau répond (conteneur avec gnome-keyring, comme les e2e) ;
  - `adapters/discovery/src/srv.rs` (52 %) : résolution SRV contre un serveur DNS de test en conteneur, jamais un vrai DNS ;
  - `adapters/imap/src/connection.rs` (81 %), `adapters/smtp/src/stream.rs` (86 %) : chemins d'erreur (réponses tronquées, `BAD`, délais) via le faux serveur scripté ;
  - front : `ServerFields.svelte` (0 %), `AddAccountDialog.svelte` (17 %), `lib/errors.ts` (40 %, tous les codes), `lib/i18n.ts` (55 %, `applySystemLocale`), `Splitter.svelte` (18 %, clavier), `ThreePaneLayout.svelte` (68 %, largeurs mémorisées), `lib/add-account.svelte.ts` (77 %) ; `lib/ipc/api.ts` (0 %, enveloppe des erreurs IPC) ;
  - décision du mainteneur : ajouter `cargo-llvm-cov` et `@vitest/coverage-v8` au projet, avec un seuil vérifié par `./scripts/check.sh` (renforcement, pas assouplissement).
  - **Fait (2026-11-03)** : `cargo-llvm-cov` et `@vitest/coverage-v8` 5.0.5 branchés dans `./scripts/check.sh` avec des seuils au niveau mesuré (Rust : 83 % des lignes en tests unitaires ; front : 78 % des lignes, 74 % des instructions, 67 % des branches, 72 % des fonctions) ; front : `errors.ts` (tous les codes, champs et catégories, liste vérifiée par le typage, doublon de libellé corrigé), `i18n.ts` (`applySystemLocale`), `Splitter.svelte` (clavier), `lib/ipc/api.ts` (enveloppe des erreurs), `ServerFields.svelte`.
  - **Fait ensuite (2026-11-03)** :
    - `app/src-tauri` : commandes IPC appelées sans webview (application factice `tauri::test`, état réel sur dossier temporaire : actions, dossiers, conversations, lecture, rédaction, file d'envoi, suggestions, comptes) et protocoles `mailview://`, `mailpart://`, `remote-img://` (en-têtes de sécurité, refus) ;
    - `adapters/keyring` : contrat `SecretStore` joué contre GNOME Keyring dans le conteneur des e2e (`integration_secret_service.rs`, `in-container.sh`), jamais contre le trousseau d'un poste ;
    - `adapters/discovery/src/srv.rs` : serveur DNS de test en processus (UDP local, même principe que les faux serveurs scriptés d'ADR-007, plutôt qu'un conteneur de plus) ;
    - `imap/src/connection.rs` : IDLE (changement, délai, refus, `BYE`) ; front : `ThreePaneLayout.svelte`, `AddAccountDialog.svelte`.
    - Seuils relevés : Rust 86 % des lignes (tests unitaires) ; front 81 % des lignes, 77 % des instructions, 71 % des branches, 74 % des fonctions.
    - Encore faible, couvert par les tests d'intégration et e2e plutôt qu'unitaires : `app/src-tauri/src/mail.rs` (acteurs, rendu avec téléchargement) et le chemin Secret Service de `keyring` dans la mesure de l'hôte.
- [x] Domaine :
  - destinataires de répondre, répondre à tous et transférer (exclure ses propres adresses, dédoublonner, `Reply-To`) ;
  - sujet `Re:`/`Fwd:` sans empilement ;
  - `In-Reply-To`/`References` ;
  - citation
  - `crates/domain/src/compose.rs` ; répondre à son propre message vise ses destinataires ; `References` bornées à 20 (racine et plus récents).
- [x] Domaine (ADR-011) : reconnaissance des préfixes de sujet étrangers (`TR`, `AW`, `WG`, `SV`, `VS`, `Antw`, `RIF`, `RE :`…) avec `proptest` ; préfixes produits toujours `Re:`/`Fwd:` ; en-tête de citation reçu en paramètre, localisé par l'interface
  - liste partagée avec le regroupement des conversations (`conversation::strip_prefix`).
- [x] Domaine : brouillon (cycle de vie, sauvegarde périodique), file d'envoi (états, nouvelles tentatives, échec définitif)
  - `compose/draft.rs` : validation avant envoi (destinataires, sujet sans contrôle, tailles), copie sur le serveur au plus toutes les 30 s et seulement si modifié ; `compose/outbox.rs` : essais hors ligne non comptés, attente croissante, 8 essais, envoi interrompu par un arrêt = incertain, jamais renvoyé automatiquement.
- [x] `adapters/render` : réassainissement du HTML Tiptap ; construction `multipart/alternative` + pièces jointes avec `lettre` ; tests du MIME produit (relu par `mail-parser`)
  - `crates/adapters/render/src/compose.rs` : liste blanche minimale (p, br, gras, italique, barré, liens http/https/mailto, listes, citations) ; version texte dérivée du HTML assaini ; `Bcc` seulement dans les copies locales ; `Message-ID` aléatoire (128 bits) sans nom de machine ; imbrication bornée à 64 niveaux (au-delà, réduit au texte). `lettre` compilé avec `builder`, `getrandom` ajouté en dépendance directe.
- [x] `adapters/imap` : APPEND des brouillons (remplacement de l'ancien via UIDPLUS), copie dans Envoyés (sauf serveurs qui le font eux-mêmes : Gmail)
  - `RemoteMailboxSession::append` renvoie la copie (UIDVALIDITY et UID d'APPENDUID) ; sans UIDPLUS, les brouillons restent locaux ; une ancienne copie n'est supprimée que si la boîte a la même UIDVALIDITY et que le message porte `\Draft` ; Gmail (`X-GM-EXT-1`) dépose lui-même la copie dans Envoyés. Défaut d'`imap-next` (littéral refusé par `NO`) contourné, consigné dans ADR-002.
- [x] Application : `SendMessage` (file d'envoi persistante, envoi différé hors ligne), `SaveDraft`
  - `crates/application/src/compose.rs` (`SaveDraft`, `QueueMessage`, `PrepareReply`, `DeliverOutbox`, `RecoverOutbox`, `FileSentCopies`, `SyncDrafts`) ; `compose_worker.rs` : un travailleur par compte, connexion IMAP courte seulement s'il y a du travail.
- [x] Interface : éditeur Tiptap (gras, italique, liens, listes, citation), destinataires avec autocomplétion depuis les adresses connues, pièces jointes par glisser-déposer et par sélecteur, `c`/`r`/`a`/`f`, Ctrl+Entrée pour envoyer
  - Tiptap 3.32.0 (figé, MIT), limité à ce que le réassainissement Rust conserve ; chemins des fichiers joints jamais transmis par l'interface (sélecteur ouvert par Rust, dépôt natif désigné par un jeton) ; boîte d'envoi (relancer, annuler) ; scénario e2e `compose.e2e.mjs`.
- [x] Intégration : envoi via Mailpit vérifié par son API, brouillon présent sur Dovecot, copie dans Envoyés présente
  - `crates/application/tests/integration_compose.rs` (stockage réel, travailleur de rédaction) ; scénario e2e `app/e2e/specs/compose.e2e.mjs` (réponse par `r`, Cci hors des en-têtes, HTML saisi jamais interprété).
- [x] Reports de la revue de la phase 6 (REVIEWS.md) : `hostile_message_sql.rs` étendu aux actions ; garde-fou `EXPUNGE` sans UIDPLUS (compte des `* n EXPUNGE`) ; file `pending_ops` lue une fois, `record_all` transactionnel, plafond par compte
  - plafond `MAX_PENDING_OPS` (10 000 par compte, code IPC `pending_queue_full`) ; suppressions en trop signalées par `ReplayReport::foreign_expunges` (boîte resynchronisée).
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; moyennes corrigées)
  - 1 haute et 3 moyennes trouvées, toutes corrigées ; 3 basses corrigées, 3 reportées en phase 8.

**Condition de fin** : `./scripts/check.sh full && pnpm -C app e2e` passent sans erreur.

## Phase 8 : recherche plein texte

- [x] Domaine : analyse de la syntaxe de requête (`from:`, `to:`, `subject:`, `has:attachment`, `is:unread`, `before:`/`after:`, guillemets, négation) en arbre syntaxique, avec des tests et `proptest` (aucune panique)
  - `crates/domain/src/search.rs` : conjonction de clauses niables (texte sur un champ, ou filtre) ; aussi `is:read`, `is:flagged` ; opérateur inconnu ou valeur invalide cherchés comme du texte (jamais d'échec pour une faute de frappe) ; requête bornée (512 caractères, 16 clauses) ; `before:` exclut le jour, `after:` l'inclut, début du jour selon le décalage horaire reçu ; pliage casse et diacritiques aligné sur `unicode61 remove_diacritics 2` et surlignage en segments de texte (jamais de HTML).
- [x] `adapters/sqlite` : FTS5 à contenu externe (`unicode61 remove_diacritics 2`), indexation dans la même transaction que l'insertion, préfixe automatique, `SearchQuery` paginée avec BM25
  - migration 7 : `message_texts` (sujet, expéditeur, destinataires, texte) et `message_search`, tenus par des déclencheurs (synchronisation, copies provisoires, cascades) ; messages existants indexés à la montée de version ; texte du corps extrait par `render` (`MimeTextExtractor` : partie texte, sinon HTML réassaini réduit au texte, pièces jointes exclues, 100 000 caractères au plus) et enregistré par `set_body` dans la transaction du corps ; port `SearchIndex` : portée tous comptes, compte ou dossier, doublons de `\All` écartés, BM25 pondéré (sujet 10, expéditeur 5, destinataires 3, texte 1), sinon tri par date ; pagination par décalage, 1 000 résultats au plus ; `integrity_problems` vérifie aussi l'index (`integrity-check`).
- [x] Test de contrat `SearchIndex` ; protection contre l'injection de syntaxe FTS5 (les termes de l'utilisateur sont échappés)
  - `crates/test-support/src/search_contracts.rs`, joué contre le faux en mémoire et SQLite (mémoire, fichier avec pool de lecteurs) ; chaque clause devient une chaîne FTS5 entre guillemets (guillemets doublés), passée en paramètre lié ; SQL assemblé uniquement à partir de fragments constants (`check-sql.sh`) ; requêtes hostiles (`OR`, `NEAR`, `col:`, `^`, `*`, guillemets, parenthèses…) cherchées comme du texte.
- [x] Bench : recherche dans 50k messages < 100 ms
  - `crates/adapters/sqlite/benches/search.rs` (50 000 messages avec ~80 mots de texte) : de 0,4 ms (mot rare) à 43 ms (trois mots fréquents), médiane, mesuré le 2026-11-03.
- [x] Interface : barre de recherche (`/`), résultats en liste, surlignage, fonctionnement hors ligne
  - commande IPC `search_messages` (surlignage calculé en Rust, transmis en segments de texte) ; `SearchPanel.svelte` à la place de la liste (bouton de l'en-tête ou `/`), recherche pendant la frappe, `j`/`k`, Entrée ouvre la conversation dans le dossier du message (actions dans ce dossier), Échap revient ; index local uniquement, donc hors ligne (tests de commande sans aucune connexion) ; scénario e2e `search.e2e.mjs`.
- [x] Reports de la revue de la phase 7 (REVIEWS.md, 4 basses) :
  - boîte enregistrée avec chaque copie distante d'un brouillon (migration 8 : `drafts.remote_mailbox`, boîte dans la clé de `draft_remote_trash`) ; corbeille traitée boîte par boîte, boîte disparue ou refusée : copies oubliées ;
  - brouillon refusé (`NO`) ou impossible à construire : journalisé, nouvel essai une heure plus tard (`DraftSync::remote_refused`), les brouillons suivants et la corbeille continuent ;
  - pièces jointes : taille réelle des blobs revérifiée (`QueueMessage`, copie locale), 100 au plus (`ComposeError::TooManyAttachments`), type MIME recalculé en Rust d'après le nom ;
  - brouillon relu depuis SQLite revalidé (`check_storable`).
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; moyennes corrigées)
  - 0 critique, 0 haute, 1 moyenne (extraction du texte hors du fil async, en temps borné : corrigée), 3 basses (extrait nettoyé : corrigée ; préfixe d'une lettre : corrigé en partie ; budget de temps des recherches et `hostile_message_sql.rs` : reportés en phase 9).

**Condition de fin** : `./scripts/check.sh full && cargo bench -p norelly-sqlite -- --test && pnpm -C app e2e` passent sans erreur.
- Vérifiée le 2026-11-03 : les trois commandes passent (bench recherche : 0,2 à 42 ms ; liste : < 1 ms ; e2e : 4 scénarios).

## Phase 9 : notifications et finitions

- [x] Reports de la revue de la phase 8 (REVIEWS.md) : budget de temps des lectures de recherche (interruption quand une nouvelle requête arrive), bench avec des corps réalistes ; `hostile_message_sql.rs` étendu à la recherche et au nom de boîte des copies distantes
  - recherche interrompue au-delà de 2 s (`progress_handler` SQLite, retiré après chaque requête), plutôt qu'à l'arrivée d'une nouvelle requête : borne la durée quelle qu'en soit la cause ; bench à 300 mots par corps, plus une requête d'une lettre et 8 négations (de 0,3 à 89 ms, seuil 100 ms : marge faible pour trois mots fréquents) ; `hostile_message_sql.rs` : chaque charge comme requête, messages piégés retrouvés par leur texte, copie distante dans la boîte au nom piégé.

- [x] Notifications natives (`tauri-plugin-notification`) pour les nouveaux messages (regroupées, sans contenu si l'option de confidentialité est active)
  - domaine `notify` (non lus seulement, jamais à la première synchronisation d'une boîte, regroupés avec le plus récent) ; événement `SyncEvent::NewMail` de l'acteur pour la boîte de réception ; filtre des paramètres appliqué en Rust dans le relais (`settings::new_mail_event`) ; texte composé par l'interface, affiché si la fenêtre n'a pas le focus, revalidé par la commande `show_notification`.
  - **Dérogation à valider par le mainteneur** : `tauri-plugin-notification` tire zbus 5, dont `zbus_names` et `zvariant` sont signalés non maintenus (RUSTSEC-2026-0370, -0371) et font échouer `cargo deny` ; remplacé par `notify-rust` sans zbus (fonctionnalité `d`, libdbus du système sous Linux), appelé depuis Rust seulement. L'affichage réel est à vérifier à la main (`docs/MANUAL-TESTS.md`).
- [x] Traduction anglaise complète, relue ; choix de la langue dans les paramètres (système par défaut) ; textes des notifications localisés côté interface (ADR-011)
  - catalogue anglais relu en entier le 2026-11-03 (orthographe britannique, aucun reste de français) ; langue des paramètres appliquée avant le premier rendu (`main.ts`, `applyLanguage`), prise en compte au prochain démarrage.
- [x] Paramètres : question de mise à jour au premier lancement (oui présélectionné), modifiable ; préférences d'affichage, de rapatriement et de notifications
  - domaine `settings` (bornes du seuil de rapatriement, filtre des notifications), port `SettingsStore` (contrat, faux, SQLite : valeur illisible ou hors bornes remplacée par sa valeur par défaut), commandes `get_settings`/`save_settings`, dialogue Paramètres (`Ctrl+,`), question au premier lancement (`UpdateQuestion`, « Oui » a le focus) ; le seuil s'applique au démarrage des acteurs. La recherche de mises à jour elle-même arrive en phase 10 (`tauri-plugin-updater`).
- [ ] Mesure de bout en bout : démarrage et changement de dossier < 200 ms sur la boîte de 50k messages (test end-to-end avec mesure, seuil dans le job nocturne)
- [ ] Passe accessibilité : focus visible, rôles ARIA, navigation entièrement au clavier, contrastes
- [ ] Checklist de tests manuels Gmail, Fastmail, Outlook.com (IMAP par mot de passe d'application), consignée dans `docs/MANUAL-TESTS.md`
- [ ] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; moyennes corrigées)

**Condition de fin** : `./scripts/check.sh full && pnpm -C app e2e` passent sans erreur.

## Phase 10 : distribution

- [ ] `release.yml` : build NSIS (Windows), .deb et .rpm (Linux) sur tag, artefacts publiés sur GitHub Releases
- [ ] `tauri-plugin-updater` (NSIS uniquement, feature `updater`), signature minisign, manifeste JSON sur GitHub Releases
- [ ] Manifeste Flatpak `io.github.gbourquet.Norelly` (runtime GNOME, sources cargo et npm vendorisées, autorisations minimales), avec un build local qui passe
- [ ] Manifeste winget
- [ ] Métadonnées AppStream, icônes, fichier `.desktop` (traduits en français et en anglais, ADR-011)
- [ ] Dossier de candidature SignPath Foundation
- [ ] `nightly.yml` : fuzzing court, bench, Stalwart en second serveur
- [ ] Revue security-reviewer finale (chaîne d'approvisionnement, autorisations Flatpak, capabilities Tauri), consignée dans REVIEWS.md

**Condition de fin** : `./scripts/check.sh full && pnpm -C app tauri build` passent sans erreur, et `flatpak-builder --force-clean build packaging/flatpak/io.github.gbourquet.Norelly.yml` réussit.
