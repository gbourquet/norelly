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

- [ ] Domaine : `Account`, `EmailAddress` (validation), stratégie d'autodécouverte (ordre ISPDB fournisseur, puis ISPDB Mozilla, puis SRV RFC 6186, puis sondage, puis saisie manuelle ; préférence TLS implicite > STARTTLS > clair)
- [ ] `adapters/discovery` : client autoconfig (fournisseur + ISPDB) avec `reqwest`/rustls et parsing `quick-xml` (fixtures dans `fixtures/ispdb/`) ; SRV via `hickory-resolver` derrière un trait ; sondage des ports avec délai court et en parallèle
- [ ] Tests : `wiremock` pour l'autoconfig, résolveur simulé, sondage contre le faux serveur ; XML hostile ou malformé sans panique
- [ ] Application : `DiscoverSettings`, `AddAccount` (découverte, test de connexion, stockage du secret dans le trousseau, persistance du compte), `RemoveAccount` (supprime aussi le secret et les données locales)
- [ ] Persistance minimale des comptes (premier schéma SQLite et migration), avec un test de contrat `AccountRepository` (mémoire et SQLite)
- [ ] `tauri-specta` : génération des types TS ; test CI qui échoue si les bindings générés sont périmés
- [ ] Interface :
  - jetons de design (`tokens.css`, clair et sombre) ;
  - mise en page en 3 panneaux redimensionnables ;
  - police système, icônes Lucide
- [ ] Interface : registre de commandes central, avec des tests vitest (enregistrement, conflits de raccourcis, conditions d'activation)
- [ ] Interface : palette Ctrl+K (bits-ui Command) et aide des raccourcis (`?`)
- [ ] Interface, parcours d'ajout de compte (adresse + mot de passe) :
  - progression de la découverte ;
  - écran d'avertissement de certificat (empreinte, raison, « faire confiance à ce serveur ») ;
  - avertissement en cas de clair ;
  - saisie manuelle en dernier recours
- [ ] Interface : suppression de compte avec confirmation
- [ ] Test end-to-end (tauri-driver) : ajout d'un compte Dovecot de test de bout en bout
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; moyennes corrigées)

**Condition de fin** : `./scripts/check.sh full && pnpm -C app e2e` passent sans erreur.

## Phase 4 : stockage et synchronisation

- [ ] Domaine : `Mailbox` (rôle SPECIAL-USE ou heuristique de nom), `MessageMeta`, `Flags`, `Uid`, `UidValidity`, `ModSeq`
- [ ] Domaine : machine d'états pure de rapprochement :
  - synchronisation initiale ;
  - incrémentale CONDSTORE ;
  - QRESYNC (VANISHED) ;
  - repli sans extension (comparaison des UID) ;
  - changement d'UIDVALIDITY (resynchronisation complète)

  Tests exhaustifs et `proptest` (idempotence, convergence)
- [ ] Domaine : politique de rapatriement (en-têtes + texte pour tous ; pièces jointes sous un seuil configurable)
- [ ] `adapters/imap` : LIST (SPECIAL-USE), SELECT/EXAMINE (CONDSTORE/QRESYNC), FETCH (en-têtes, BODYSTRUCTURE, corps par parties), ENABLE, COMPRESS si disponible
- [ ] `adapters/imap` : IDLE avec renouvellement (< 29 min), reconnexion avec backoff exponentiel
- [ ] `adapters/sqlite` :
  - schéma messages, dossiers et état de synchro ;
  - index composites `(dossier, date DESC)` ;
  - mode WAL, un rédacteur et un pool de lecteurs ;
  - migrations testées (montée de version depuis chaque version précédente)
- [ ] `adapters/blobs` : stockage `.eml.zst` par compte et empreinte, écriture atomique (fichier temporaire + renommage), test de contrat `BlobStore`
- [ ] Application : acteur de compte (une connexion IDLE sur la boîte de réception, une connexion de commandes), `SyncAccount`, événements « dossier modifié »
- [ ] Ports de requête : `FolderTreeQuery`, `MessageListQuery::page(dossier, curseur, n)`
- [ ] Intégration Dovecot : synchronisation initiale, incrémentale, IDLE (message ajouté par APPEND côté serveur et reçu en moins de 5 s), UIDVALIDITY changée
- [ ] Toxiproxy : coupure en pleine synchro, latence, connexion à moitié fermée ; l'état converge après la reprise
- [ ] Générateur de boîte synthétique de 50k messages + bench `criterion` : première page < 50 ms et changement de dossier < 50 ms (requête seule)
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; moyennes corrigées)

**Condition de fin** : `./scripts/check.sh full && cargo bench -p norelly-sqlite -- --test` passent sans erreur.

## Phase 5 : liste, conversations, lecture

- [ ] Domaine : fils de conversation JWZ (`References`/`In-Reply-To`, regroupement par sujet en dernier recours), avec des tests à partir de cas réels anonymisés ; boîte unifiée
- [ ] `adapters/sqlite` : table des conversations précalculée, mise à jour incrémentale ; `ConversationQuery`, liste unifiée paginée
- [ ] `adapters/render` : MIME vers vue (`mail-parser`), choix de la meilleure alternative, charsets, pièces jointes listées
- [ ] `adapters/render` :
  - liste blanche `ammonia` ;
  - réécriture `cid:` vers `mailpart://` et des images distantes vers un espace réservé ou `remote-img://` ;
  - suppression de `@import` ;
  - suite de tests sur `fixtures/hostile-html/` (au moins 40 vecteurs)
- [ ] Agent `postMessage` (hauteur, liens, touches) ; hash SHA-256 calculé au build ; CSP générée
- [ ] Protocoles Tauri `mailview://`, `mailpart://`, `remote-img://` (proxy : sans cookies ni Referer, User-Agent neutre, taille et types bornés, autorisation par expéditeur vérifiée en Rust)
- [ ] Commande `open_external` : schémas `http`, `https`, `mailto` uniquement, ouverture dans le navigateur système
- [ ] Domaine et persistance : autorisation des images distantes par expéditeur, préférence « texte brut »
- [ ] Pièces jointes :
  - ouverture et enregistrement ;
  - nom de fichier assaini (séparateurs, `..`, noms réservés Windows, caractères de contrôle, longueur) ;
  - écriture uniquement dans le dossier choisi via le sélecteur de fichiers
- [ ] Interface : liste virtualisée (au-dessus de `@tanstack/virtual-core`), boîte unifiée, vue conversation (messages empilés, hauteur via l'agent), bannière « images bloquées »
- [ ] Interface : navigation j/k, ouverture, retour, via le registre de commandes
- [ ] Tests end-to-end de sécurité A+ :
  - zéro requête vers un serveur canari ;
  - aucun script du mail exécuté ;
  - IPC inaccessible depuis `mailview://` ;
  - `javascript:` jamais ouvert ;
  - touches renvoyées correctement
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; moyennes corrigées)

**Condition de fin** : `./scripts/check.sh full && pnpm -C app e2e` passent sans erreur.

## Phase 6 : actions et hors ligne

- [ ] Domaine : actions (déplacer, supprimer, archiver, lu/non lu, drapeau), application optimiste, `pending_ops`, rejeu, résolution de conflits (message disparu, dossier supprimé, UIDVALIDITY changée)
- [ ] Domaine : sémantique de « supprimer » (déplacement vers la Corbeille ; suppression définitive depuis la Corbeille) et d'« archiver » (dossier `\Archive` ou repli configurable)
- [ ] `adapters/imap` : MOVE, ou à défaut COPY + STORE `\Deleted` + UID EXPUNGE (UIDPLUS), ou EXPUNGE avec garde-fou ; STORE des drapeaux avec UNCHANGEDSINCE
- [ ] `adapters/sqlite` : table `pending_ops` durable, test de contrat
- [ ] Application : file d'actions rejouée à la reconnexion, avec un test de bout en bout hors ligne puis en ligne (Toxiproxy)
- [ ] Interface : raccourcis `e`, `#`, `s` (drapeau), `Shift+i`/`Shift+u`, `v` (déplacer, avec sélection de dossier dans la palette), sélection multiple (`x`), annulation de la dernière action
- [ ] Intégration Dovecot : chaque action vérifiée côté serveur, avec et sans MOVE (capacité masquée par le faux serveur)
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; moyennes corrigées)

**Condition de fin** : `./scripts/check.sh full && pnpm -C app e2e` passent sans erreur.

## Phase 7 : rédaction et envoi

- [ ] Domaine :
  - destinataires de répondre, répondre à tous et transférer (exclure ses propres adresses, dédoublonner, `Reply-To`) ;
  - sujet `Re:`/`Fwd:` sans empilement ;
  - `In-Reply-To`/`References` ;
  - citation
- [ ] Domaine : brouillon (cycle de vie, sauvegarde périodique), file d'envoi (états, nouvelles tentatives, échec définitif)
- [ ] `adapters/render` : réassainissement du HTML Tiptap ; construction `multipart/alternative` + pièces jointes avec `lettre` ; tests du MIME produit (relu par `mail-parser`)
- [ ] `adapters/imap` : APPEND des brouillons (remplacement de l'ancien via UIDPLUS), copie dans Envoyés (sauf serveurs qui le font eux-mêmes : Gmail)
- [ ] Application : `SendMessage` (file d'envoi persistante, envoi différé hors ligne), `SaveDraft`
- [ ] Interface : éditeur Tiptap (gras, italique, liens, listes, citation), destinataires avec autocomplétion depuis les adresses connues, pièces jointes par glisser-déposer et par sélecteur, `c`/`r`/`a`/`f`, Ctrl+Entrée pour envoyer
- [ ] Intégration : envoi via Mailpit vérifié par son API, brouillon présent sur Dovecot, copie dans Envoyés présente
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; moyennes corrigées)

**Condition de fin** : `./scripts/check.sh full && pnpm -C app e2e` passent sans erreur.

## Phase 8 : recherche plein texte

- [ ] Domaine : analyse de la syntaxe de requête (`from:`, `to:`, `subject:`, `has:attachment`, `is:unread`, `before:`/`after:`, guillemets, négation) en arbre syntaxique, avec des tests et `proptest` (aucune panique)
- [ ] `adapters/sqlite` : FTS5 à contenu externe (`unicode61 remove_diacritics 2`), indexation dans la même transaction que l'insertion, préfixe automatique, `SearchQuery` paginée avec BM25
- [ ] Test de contrat `SearchIndex` ; protection contre l'injection de syntaxe FTS5 (les termes de l'utilisateur sont échappés)
- [ ] Bench : recherche dans 50k messages < 100 ms
- [ ] Interface : barre de recherche (`/`), résultats en liste, surlignage, fonctionnement hors ligne
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; moyennes corrigées)

**Condition de fin** : `./scripts/check.sh full && cargo bench -p norelly-sqlite -- --test && pnpm -C app e2e` passent sans erreur.

## Phase 9 : notifications et finitions

- [ ] Notifications natives (`tauri-plugin-notification`) pour les nouveaux messages (regroupées, sans contenu si l'option de confidentialité est active)
- [ ] Paramètres : question de mise à jour au premier lancement (oui présélectionné), modifiable ; préférences d'affichage, de rapatriement et de notifications
- [ ] Mesure de bout en bout : démarrage et changement de dossier < 200 ms sur la boîte de 50k messages (test end-to-end avec mesure, seuil dans le job nocturne)
- [ ] Passe accessibilité : focus visible, rôles ARIA, navigation entièrement au clavier, contrastes
- [ ] Checklist de tests manuels Gmail, Fastmail, Outlook.com (IMAP par mot de passe d'application), consignée dans `docs/MANUAL-TESTS.md`
- [x] Revue security-reviewer, consignée dans REVIEWS.md (0 critique, 0 haute ; moyennes corrigées)

**Condition de fin** : `./scripts/check.sh full && pnpm -C app e2e` passent sans erreur.

## Phase 10 : distribution

- [ ] `release.yml` : build NSIS (Windows), .deb et .rpm (Linux) sur tag, artefacts publiés sur GitHub Releases
- [ ] `tauri-plugin-updater` (NSIS uniquement, feature `updater`), signature minisign, manifeste JSON sur GitHub Releases
- [ ] Manifeste Flatpak `io.github.gbourquet.Norelly` (runtime GNOME, sources cargo et npm vendorisées, autorisations minimales), avec un build local qui passe
- [ ] Manifeste winget
- [ ] Métadonnées AppStream, icônes, fichier `.desktop`
- [ ] Dossier de candidature SignPath Foundation
- [ ] `nightly.yml` : fuzzing court, bench, Stalwart en second serveur
- [ ] Revue security-reviewer finale (chaîne d'approvisionnement, autorisations Flatpak, capabilities Tauri), consignée dans REVIEWS.md

**Condition de fin** : `./scripts/check.sh full && pnpm -C app tauri build` passent sans erreur, et `flatpak-builder --force-clean build packaging/flatpak/io.github.gbourquet.Norelly.yml` réussit.
