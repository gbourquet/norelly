# Norelly : client mail desktop (Tauri 2 + Rust + Svelte 5)

Client IMAP/SMTP pour Windows et Linux : rapide, utilisable entièrement
à la souris par un débutant, raccourcis clavier en accélérateurs, sans
télémétrie. Le projet est en **maintenance** : nouvelles
fonctionnalités, interface, corrections, durcissement. Les sujets ouverts
sont suivis dans Linear (voir « Suivi des sujets »).

Les choix d'architecture font foi dans `docs/DECISIONS.md` (ADR) : toute
dérogation est validée par le mainteneur **avant** d'être implémentée, puis
consignée dans l'ADR concerné.

## Utilisateur cible et interface
Utilisateur cible : une personne grand public, habituée à Gmail ou
Outlook, qui utilise la souris pour tout, ne connaît aucun raccourci
clavier et ne lira jamais de documentation. Elle doit comprendre comment
faire chaque action simplement en regardant l'écran. Les raccourcis
clavier restent, comme accélérateurs pour ceux qui les connaissent.

- **Aucune fonctionnalité ne doit être accessible uniquement par raccourci
  clavier. Toute action doit avoir un équivalent visible à la souris.**
  (bouton libellé, menu « Plus… », menu d'un compte, menu contextuel ;
  `app/src/lib/visible-actions.ts`). Si l'écran est trop chargé, déplacer
  les actions les moins fréquentes dans « Plus… », ne pas en retirer.
- **Après toute modification d'interface, prendre une capture d'écran et
  la vérifier avant de considérer le travail terminé.** (alignement,
  encombrement, libellés tronqués, thème sombre, survol, menus ;
  `pnpm -C app tauri dev`, ou les captures de `pnpm -C app e2e`).
- Nouveaux éléments : jetons du design system seulement
  (`app/src/styles/tokens.css`), libellé accessible, focus visible,
  infobulle « Libellé — Touche » (`describeCommand`).

## Architecture hexagonale dosée (ADR-008)
Description complète, modules et flux : `docs/technique/architecture.md`.

| Couche | Rôle | Dépend de |
|---|---|---|
| `crates/domain` | Règles métier pures, valeurs validées, ports (traits). **Aucune E/S**, ni async runtime, ni horloge, ni aléa : ce qui en a besoin le reçoit en paramètre. Zone de relecture prioritaire du mainteneur. | `thiserror`, `serde` seulement (`scripts/check-domain-deps.sh`) |
| `crates/application` | Cas d'usage async qui orchestrent les ports (acteur par compte, synchronisation, rédaction, actions). **Aucune règle métier propre.** | `domain` (+ `tokio`, `tracing`) |
| `crates/adapters/*` | Implémentations des ports : `tls`, `imap`, `smtp`, `sqlite` (seul à dépendre de `rusqlite`), `blobs`, `keyring`, `discovery`, `oauth`, `render`, `contacts-io`. Traduisent entre le monde extérieur et les types du domaine, sans décider. | `domain` ; `tls` pour `imap`, `smtp`, `discovery`, `oauth` ; jamais un autre adaptateur |
| `app/src-tauri` | Adaptateur entrant (commandes IPC, protocoles `mailview://`, `mailpart://`, `remote-img://`) et point d'assemblage : `state.rs` est le **seul** endroit qui choisit les implémentations. | tout |
| `app/src` | Interface Svelte 5. Présentation seulement : **toute décision de sécurité est prise en Rust.** | IPC seulement |
| `crates/test-support` | Conteneurs, faux serveurs IMAP/SMTP, faux des ports, suites de contrat. | dev-dependency uniquement |

Où mettre quoi :
- Une règle (validation, décision, calcul, politique de sécurité) va dans
  le domaine, testée sans E/S. Si elle apparaît dans un cas d'usage, une
  commande Tauri, un adaptateur ou l'interface, la remonter dans le domaine.
- Un besoin d'E/S nouveau = un port (trait) dans `crates/domain/src/ports.rs`,
  son implémentation dans un adaptateur, son faux et sa suite de contrat
  dans `test-support`, jouée contre les deux.
- Une commande Tauri revalide les entrées par les constructeurs du domaine,
  appelle un cas d'usage, traduit les erreurs en codes. Les DTO vivent dans
  `ipc.rs` : jamais de `specta` ni de type Tauri dans le domaine.
- CQRS léger : les écritures passent par le domaine ; les lectures d'écran
  passent par des ports de requête taillés pour l'écran (pagination par
  curseur), implémentés en SQL.
- Ports async en `impl Future + Send` (dispatch statique dans les chemins
  chauds) ; `dyn` seulement au point d'assemblage.
- Une entorse à ces règles est une dérogation : à faire valider avant.

Documentation : `docs/technique/` (architecture, données, sécurité, tests,
pas à pas dans `premiers-pas.md`), `docs/fonctionnel/` (une fiche par
fonctionnalité), `docs/utilisateur/{fr,en}/`.

## Commandes
| Action | Commande |
|---|---|
| Installer le front | `pnpm -C app install` |
| Lancer l'application | `pnpm -C app tauri dev` |
| Vérifications rapides (sans Docker) | `./scripts/check.sh` |
| Tout, avec Docker (avant toute PR) | `./scripts/check.sh full` |
| Tests Rust | `cargo nextest run --workspace` (ou `-p <crate> <filtre>`) |
| Tests d'intégration (Docker) | `cargo nextest run --workspace --profile integration` |
| Module de mise à jour | `cargo nextest run -p norelly-app --features updater` |
| Tests front | `pnpm -C app test` |
| End-to-end (Docker) | `pnpm -C app e2e` |
| Bancs liste et recherche 50k | `cargo bench -p norelly-sqlite -- --test` |
| Bindings IPC | `./scripts/gen-bindings.sh` |
| Catalogues i18n | `pnpm -C app i18n` (fait aussi par `check`, `lint` et Vite) |
| Documentation | `./scripts/check-docs.sh` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings`, `pnpm -C app lint`, `pnpm -C app check` |
| Format | `cargo fmt --all`, `pnpm -C app format` |

- Un hook PostToolUse (`.claude/hooks/format-lint.sh`) formate et lint
  chaque fichier modifié ; un échec de lint revient comme retour à corriger.
- `check.sh` construit d'abord le front (`app/dist` est embarqué dans le
  binaire) et impose des seuils de couverture (Rust : lignes ;
  front : `app/vite.config.ts`), à relever, jamais à baisser.
- `pnpm -C app e2e` réécrit les captures de `docs/utilisateur/*/captures` :
  ne les commiter que si le changement d'interface le justifie.

## Méthode : TDD obligatoire
Tout développement suit le cycle rouge → vert → refactor :
1. ROUGE : écris d'abord le ou les tests décrivant le comportement attendu.
   Lance-les et montre-moi qu'ils échouent, pour la bonne raison (une
   assertion qui échoue, pas une erreur de compilation ou d'import).
2. VERT : écris le code minimal pour les faire passer. Relance les tests.
3. REFACTOR : nettoie le code, les tests doivent rester verts.

Règles :
- Interdit d'écrire du code de production sans test rouge préalable.
- Un test qui passe dès sa première exécution est suspect : vérifie qu'il
  teste vraiment quelque chose avant de continuer.
- Correction de bug = d'abord un test qui reproduit le bug.
- TDD strict pour toute la logique (protocoles IMAP/SMTP, MIME,
  synchronisation, stockage, recherche, état de l'interface). Le rendu
  visuel pur est couvert par des tests e2e écrits après coup.
- Tests réseau contre les serveurs de test en conteneur (Dovecot, Mailpit,
  Toxiproxy), jamais contre un vrai serveur ni avec de vrais identifiants.
- Tests d'intégration dans des fichiers `tests/integration*.rs` ; chaque
  port a une suite de contrat jouée contre son faux et son implémentation
  réelle.

## Règles non négociables
- **Ne jamais ramollir, désactiver ou contourner un test**, ni affaiblir
  `scripts/check.sh`, `deny.toml` ou un seuil de couverture. Un blocage se
  documente dans la PR (ou un ticket Linear), puis on s'arrête.
- Rust : pas d'`unwrap`/`expect` sur des données externes (lint
  `unwrap_used`) ; `unsafe` interdit dans tout le workspace.
- Secrets : uniquement dans le trousseau (`SecretStore`), jamais dans
  SQLite, les journaux, les erreurs, `Debug`/`Display` ou les événements
  IPC. Le niveau TRACE est supprimé à la compilation (`imap-next` y
  journalise les mots de passe) : ne pas le réactiver.
- TLS : jamais de vérificateur qui accepte tout ni de repli en clair si
  STARTTLS est exigé (ADR-002).
- SQL : paramètres liés pour toute donnée ; une constante interpolée dans
  le texte d'une requête porte le commentaire `sql-const`
  (`scripts/check-sql.sh`).
- Svelte 5 uniquement (runes) ; jamais de `{@html}`, d'`innerHTML` ni
  d'`eval`. Le contenu d'un mail est toujours affiché en texte échappé.
- i18n (ADR-011) : toute chaîne d'interface est une clé de
  `app/messages/fr.json` **et** `en.json` (mêmes clés, testé) ; Rust
  n'envoie aucun texte destiné à l'utilisateur, seulement des codes et un
  `detail` brut.
- Contrat IPC : types dans `app/src-tauri/src/ipc.rs`, bindings TS générés
  et commités (`app/src/lib/ipc/bindings.ts`) ; toute entrée de
  l'interface est revalidée par le domaine.
- Dépendances : licences permissives (GPL et AGPL refusées, ADR-009),
  contrôlées par `cargo deny` ; toute nouvelle dépendance se justifie dans
  la PR. Une exception dans `deny.toml` exige l'accord du mainteneur.

## Recettes (détail dans `docs/technique/premiers-pas.md`)
- **Commande IPC** : règle dans le domaine, cas d'usage dans
  `application`, commande qui revalide et traduit les erreurs en codes.
  L'enregistrer à **trois** endroits : `collect_commands![…]` dans
  `app/src-tauri/src/lib.rs`, `APP_COMMANDS` dans
  `app/src-tauri/ipc-commands.rs`, `app/src-tauri/permissions/main-window.toml`
  (`acl_tests` échoue sinon). Puis `./scripts/gen-bindings.sh` et
  l'enveloppe dans `app/src/lib/ipc/api.ts`.
- **Migration SQLite** : nouveau fichier `migrations/00NN_<sujet>.sql`
  déclaré à la fin de `migrations()` (`crates/adapters/sqlite/src/lib.rs`) ;
  **ne jamais modifier une migration publiée**. Documenter dans
  `docs/technique/donnees.md`.
- **Action utilisateur** : une commande du registre (`registerAppCommands`,
  `app/src/lib/app.svelte.ts`), libellés dans les deux catalogues, ligne
  dans `docs/utilisateur/{fr,en}/shortcuts.md` (testé), **et** son élément
  visible à la souris : `app/src/lib/visible-actions.ts` (barre, « Plus… »,
  survol, menu contextuel, barre de sélection) ou menu « ⋯ » d'un compte
  (`Sidebar.svelte`) ; capture vérifiée.
- **Code d'erreur** : variante de `ErrorCodeDto` (`ipc.rs`), texte dans
  `app/src/lib/errors.ts` et les catalogues.

## Git et PR
- Code, commentaires, documentation et commits en français ; identifiants
  en anglais. Les commentaires décrivent le comportement, pas l'historique
  (c'est le rôle de git).
- Commits atomiques, message `<zone>: <résumé>` (par exemple
  `imap: rejette STARTTLS absent`). Zones courantes : `domaine`, `imap`,
  `smtp`, `sqlite`, `render`, `ui`, `ipc`, `e2e`, `tests`, `docs`, `ci`,
  `sécurité`, `deps`.
- `master` est protégée : **ne jamais pousser dessus**. Travailler sur une
  branche `<zone>/<sujet>` (par exemple `ui/bouton-repondre`), la pousser,
  ouvrir une PR vers `master` avec `gh pr create`. Seul le mainteneur
  valide et merge ; ne jamais merger soi-même.
- La description de la PR cite le ticket traité (`NOR-12`) pour que
  Linear les relie.

## Suivi des sujets (Linear)
Équipe **Norelly** (`NOR`), projet **Norelly**, par le serveur MCP
`linear` (`.mcp.json`).
- Chaque ticket est le sous-ticket d'une epic (ticket parent au label
  `Epic`) et porte un type : `Feature`, `Bug` ou `Improvement`.
- Labels : un seul label de `Zone` (`connexion`, `carnet`, `dossiers`,
  `lecture`, `interface`, `chaîne-appro`, `docs`) ; `revue-sécurité` avec
  une `Gravité` (`moyen`, `bas`, `info`) pour un constat que le mainteneur
  a accepté de laisser de côté (voir « Fin de tâche ») ;
  `à-trancher` quand une décision du mainteneur est attendue (ne pas
  commencer) ; `tâche-humaine` pour ce qu'un agent ne peut pas faire.
- États : `Backlog` (à trier), `Todo` (priorisé), `In Progress`,
  `In Review`, `Done`, `Canceled`. Toujours, pour tout ticket traité :
  `In Progress` **avant** de commencer, `In Review` **dès** que la PR est
  ouverte ; seul le mainteneur merge : quand il annonce un merge, passer
  le ticket en `Done` s'il n'y est pas déjà.
- Description : une partie lisible (constat, attendu), puis les fichiers et
  le plan dans une section repliable « Agent notes ».
- Avant de créer un ticket, chercher s'il existe déjà.

## Fin de tâche
- `./scripts/check.sh full` passe (plus les e2e ou les bancs si la
  modification les concerne).
- Documentation à jour si le comportement change : fiche de
  `docs/fonctionnel/` (comportement seulement, sans vocabulaire technique ni
  ADR, objectifs plutôt que seuils), `docs/technique/tracabilite.md`,
  `docs/utilisateur/{fr,en}/` (mêmes pages dans les deux langues). Les
  captures sont régénérées par les e2e, jamais retouchées à la main.
- Linear : lier la PR au ticket traité et le passer en `In Review`. Ce
  qui reste à faire pour la PR se traite
  dans la même PR : ticket repassé en `In Progress`, ajustements sur la
  même branche, PR mise à jour, ticket repassé en `In Review`.
- Revue de sécurité : lancer le sous-agent `security-reviewer` dès que la
  branche touche une zone sensible (rendu HTML et protocoles, TLS, IMAP,
  SMTP, autodécouverte, trousseau, SQL, pièces jointes et fichiers, IPC et
  capabilities Tauri, dépendances, CI, empaquetage ; liste des fichiers
  dans `docs/technique/securite.md#fichiers-sensibles`). **Tous** les
  constats, quelle que soit leur gravité, se corrigent dans la PR du
  ticket, avant de la proposer ; le rapport est résumé dans sa
  description.
- **Pas de ticket par réflexe** : tout ce qui est découvert pendant un
  ticket (constat de revue, défaut voisin, robustesse, test manquant) se
  traite dans ce ticket et sa PR. Ne devient un ticket que ce qu'un agent
  ne peut pas faire (`tâche-humaine`, décision `à-trancher`), et seulement
  après accord du mainteneur ; la partie faisable par l'agent reste dans
  la PR.
- Récapitulatif : fait, reste, fichiers à relire en priorité (y compris les
  fichiers d'infra sensibles).
