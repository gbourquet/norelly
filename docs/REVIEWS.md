# Revues de sécurité

Rapports du sous-agent `security-reviewer`, du plus récent au plus ancien.
Les constats critiques et hauts doivent être corrigés avant de déclarer une
phase terminée. Pour chaque constat, on indique son statut : corrigé (commit),
accepté (justification) ou reporté (phase).

---

# Revue sécurité : phase 10, distribution (2026-11-05)

Périmètre : `git diff phase-9/finitions...HEAD` sur `phase-10/distribution`
(workflows `ci`, `release`, `nightly` ; image e2e ; Flatpak et sources
vendorisées ; métadonnées Linux ; winget ; ACL Tauri ; mises à jour ;
correctifs de la synchronisation et de la rédaction ; fuzzing ; Stalwart).
Code de Tauri 2.12.5, tauri-plugin-updater 2.14.1 et reqwest 0.13.5 relu
pour les points en suspens.

Bilan : 0 critique, 0 haute, 2 moyennes, 5 basses, points INFO. Corrigés :
une moyenne (en partie : le reste relève du mainteneur), quatre basses ; la
seconde moyenne et une basse sont acceptées provisoirement. `cargo deny`
(toutes fonctionnalités, `tauri-plugin-updater` compris), `pnpm audit --prod`
et `check-sql.sh` sans constat.

## [MOYENNE] Clé minisign exposée à toute la compilation, sans environnement protégé, sans mot de passe
- La clé était dans l'environnement de `tauri build` (build.rs, macros, plugins Vite des ~640 crates et paquets) ; secret de dépôt lisible par un workflow poussé sur n'importe quelle branche ; clé locale sans mot de passe. Une clé fuitée signe des installateurs acceptés par tous les clients déjà installés.
- Statut : **corrigé** : compilation sans la clé puis étape de signature séparée (`tauri signer sign` sur le seul installateur) ; job dans l'environnement GitHub `release` ; aucun cache. Fait par le mainteneur (2026-11-05) : environnement `release` créé (tags `v*`, relecteur obligatoire) avec les deux secrets, tags `v*` protégés (ruleset), clé régénérée avec un mot de passe (clé publique `112E2BD55AB163D2` dans `tauri.updater.conf.json`), copie hors ligne de la clé et du mot de passe. Rotation : une nouvelle clé exige une version de transition signée par l'ancienne qui embarque la nouvelle clé publique.

## [MOYENNE] Flatpak : `--talk-name=org.freedesktop.secrets` ouvre tout le trousseau de session
- Une exécution de code dans le processus (faille WebKitGTK ou MIME) lirait tous les secrets de l'utilisateur, pas seulement ceux de Norelly.
- Statut : **accepté** par le mainteneur (2026-11-05, dérogation consignée dans l'ADR-006) : le portail Secret (trousseau propre à l'application) passe par `oo7`, donc zbus, écarté par `deny.toml` ; l'accès direct au Secret Service est maintenu.

## [BASSE] Cache pnpm restauré dans les builds de publication
- Statut : **corrigé** : `cache: pnpm` retiré de `release.yml`.

## [BASSE] ACL : `mailview://` et `remote-img://` sont des origines locales pour Tauri, non testé
- La capability ne distingue pas l'iframe de lecture ; ce qui protège réellement : l'invoke key injectée dans le seul cadre principal, l'`Origin` « null » de l'iframe opaque rejeté, la sandbox et la CSP de `mailview://`.
- Statut : **accepté** : l'application factice de `tauri::test` n'enregistre pas les protocoles (une origine `mailview://` y serait « distante ») ; le garde-fou reste le test e2e de la phase 5 (aucun accès à l'IPC depuis l'iframe), exigé à chaque montée de version de Tauri.

## [BASSE] Repli « réponse SELECT incomplète » sans test rapide de l'adaptateur
- Statut : **corrigé** : test `undecodable_examine_response_marks_the_selection_incomplete` (faux serveur : `* VANISHED (EARLIER) 0:1` pendant `EXAMINE … QRESYNC`, puis réponse saine : compteur remis à zéro) ; suppression de la ligne vérifiée détectée.

## [BASSE] Recherche et téléchargement de mise à jour sans délai
- Statut : **corrigé** : 30 s par requête (`updater_builder().timeout`), 60 s par recherche, 15 min pour le téléchargement.

## [BASSE] winget distribue l'installateur qui se met à jour lui-même
- Statut : **corrigé** (documentation) : commentaires de `updates.rs` et `ipc.rs` alignés (winget = même installateur NSIS, mise à jour propre avec consentement).

## [INFO] Points relevés
- Corrigés : formulation « signature vérifiée » (`ipc.rs`, `api.ts`) : `check()` ne lit que `latest.json`, la signature est vérifiée au téléchargement ; `persist-credentials: false` dans `ci.yml` ; autorisation de licence `Unicode-DFS-2016` inutile retirée de `deny.toml`.
- Rétrogradation : la signature porte sur le fichier, pas sur la version ; exige l'écriture sur le dépôt (comparateur strict et URL fixe en HTTPS corrects, aucun identifiant envoyé).
- TLS de l'updater : rustls, vérificateur de la plateforme, aucun `dangerous_*` (ADR-002).
- À suivre : chaîne `stable` et outils de `install-action` flottants ; pas d'attestation de provenance ni de `SHA256SUMS` signé ; paquets apt de l'image e2e non figés ; `fuzz/Cargo.lock` hors `cargo deny` ; oracle de `sanitize_html` limité à `<script` (à enrichir : `on*`, `javascript:` en attribut, `<iframe>`, `<base>`, `<meta>`, `<form>`) ; Flathub demandera une source `git` (tag et commit) au lieu de `type: dir` ; `--filesystem=xdg-download` à revoir si le sélecteur passe par le portail ; installateur non signé (Authenticode) tant que SignPath n'a pas accepté le projet ; `core:default` réductible.
- Sans constat : interpolations `${{ }}` (aucune donnée contrôlable dans un `run:`), permissions des workflows, vérification tag = version, regex de version de `winget-manifests.sh`, `flatpak-sources.py` (crates.io et SHA-512 seulement, `--check`), registre npm local hors réseau doublement vérifié, finish-args sans `home` ni `session-bus`, ACL des 40 commandes, consentement et revalidation des mises à jour, repli `UidCompare`, Stalwart figé par empreinte, métadonnées de bureau.

---

# Revue sécurité : phase 9, notifications et finitions (2026-11-04)

Périmètre : `439e28c..HEAD` sur `phase-9/finitions` (notifications,
paramètres, traduction, accessibilité, mesure de bout en bout, envoi
différé, liste des brouillons, transfert avec pièces jointes, reprise d'un
brouillon distant, reports de la phase 8).

Bilan : 0 critique, 1 haute, 1 moyenne, 4 basses, 4 points INFO. La haute
et la moyenne sont corrigées, ainsi que trois basses ; la dernière est
reportée. `cargo deny`, `pnpm audit --prod` et `check-sql.sh` sans constat.

## [HAUTE] Texte des notifications envoyé au serveur D-Bus sans échappement du balisage
- Le serveur `org.freedesktop.Notifications` interprète un balisage (`<a href>`, `<img>`) ; sujet et expéditeur d'un mail pouvaient produire un lien `file://` ou `smb://` cliquable, hors de la règle des liens validés en Rust, ou charger une image.
- Statut : **corrigé** : `notification_text` échappe `&`, `<`, `>` après la troncature (aucune entité coupée), sous Linux et autres Unix (Windows et macOS affichent du texte brut) ; test `notification_markup_is_escaped`.

## [MOYENNE] Fermer la question des mises à jour vaut « Oui »
- Statut : **corrigé** : Échap ou un clic à côté masquent la question pour la session sans rien enregistrer (`SettingsState::dismissUpdate`) ; elle est reposée au lancement suivant ; tests `fermer la question sans répondre n'enregistre rien`, `Échap sur la question des mises à jour n'est pas un consentement`.

## [BASSE] Option de confidentialité ignorée si les paramètres ne se lisent pas
- Statut : **corrigé** : une valeur `notify.privacy` présente mais illisible vaut « masquer » (test `unreadable_privacy_fails_closed`) ; dans le relais, des paramètres illisibles donnent une notification sans contenu (repli non couvert par un test unitaire : tâche Tauri).

## [BASSE] `Resume` : origine du message non vérifiée en Rust, compte vérifié trop tard
- Statut : **corrigé** : `prepare_compose` (`Resume`) vérifie, avant tout téléchargement ou écriture, que le message appartient au compte et à son dossier « Brouillons » ; test `only_drafts_folder_messages_can_be_resumed`.

## [BASSE] Pièces jointes transférées : analyse complète avant les bornes, sur le fil async
- Statut : **corrigé en partie** : borne de taille (2 × 25 Mio) appliquée avant l'analyse, que le message soit local ou téléchargé (test `oversized_messages_are_refused_before_parsing`). **Corrigé** (phase 10) : analyse dans `spawn_blocking` (test `forwarded_attachments_are_parsed_off_the_async_thread`), arrêt dès 100 pièces ou 25 Mio pendant l'itération (`forwarded_attachments_stop_beyond_the_count_limit`, `…_size_limit`).

## [BASSE] `hostile_message_sql.rs` non étendu aux nouveaux champs stockés
- Statut : **corrigé** (phase 10) : transfert et reprise d'un message piégé à pièce jointe au nom piégé, envoi programmé (`outbox.draft_content`) puis repris, relus à l'identique (mutation du stockage vérifiée détectée).

## [INFO] Points relevés sans action immédiate
- `open_draft(id)` ne prend pas de compte (application mono-utilisateur) ; à aligner sur `unschedule_send`.
- Envoi différé : l'en-tête `Date` est celui de la programmation, pas de l'envoi ; à décider (reconstruire à l'échéance).
- Flatpak (phase 10) : `--talk-name=org.freedesktop.Notifications` ou portail pour notify-rust.
- Dérogation `notify-rust` (à valider par le mainteneur) : 4 paquets ajoutés (`notify-rust`, `futures-lite`, `parking`, `mac-notification-sys`, `tauri-winrt-notification`), licences MIT/Apache-2.0, aucune permission accordée à l'interface.

---

# Revue sécurité : phase 8, recherche plein texte (2026-11-03)

Périmètre : `git diff master...HEAD` sur `phase-8/recherche` :
- domaine `search` (analyse de la requête, pliage, surlignage), ports
  `SearchIndex` et `SearchTextExtractor` ;
- `sqlite` (migrations 7 et 8, `search.rs`, `set_body` avec texte,
  revalidation des brouillons relus), `render` (`search_text.rs`) ;
- application (`sync.rs`, `reading.rs`, corbeille et refus des brouillons
  dans `compose.rs`), commande IPC `search_messages`, interface
  (`search.svelte.ts`, `SearchPanel.svelte`, `app.svelte.ts`,
  `mail.svelte.ts`).

Bilan : 0 critique, 0 haute, 1 moyenne, 3 basses, 5 points INFO. La
moyenne est corrigée, ainsi que deux basses (l'une en partie) ; les autres
sont reportées en phase 9. Aucune injection SQL ou FTS5, aucune fuite entre
comptes ni de message masqué ou `\Deleted`, aucun chemin qui interprète du
HTML. `cargo deny`, `pnpm audit --prod` et `check-sql.sh` sans constat.

## [MOYENNE] Extraction du texte cherchable sur le fil async, sans délai maximal
- `searchable_text` (mail-parser, `ammonia`, réduction au texte) était
  appelé directement dans la tâche async de la synchronisation et de
  l'ouverture d'un message : un message hostile pouvait bloquer un worker
  tokio sans plafond de durée.
- Statut : **corrigé** : `sync::extract_text` exécute l'extraction dans
  `spawn_blocking`, sous `TEXT_EXTRACTION_TIMEOUT` (5 s) ; au-delà, le
  message est indexé sans son texte (tests `text_extraction_is_bounded_in_time`,
  `hostile_html_stays_fast` : 2 Mio de HTML dense en moins d'une seconde).

## [BASSE] Requête de recherche sans délai maximal ; préfixe d'une lettre non indexé
- Un mot d'une lettre devenait un préfixe non indexé (parcours de tous
  les termes) ; 16 clauses de ce type, ou une frappe rapide, pouvaient
  occuper les lecteurs plusieurs secondes sur une grosse boîte.
- Statut : **corrigé en partie** : un mot d'une lettre est cherché tel
  quel, sans préfixe (contrat `search_index_contract`). Puis, en phase 9 :
  recherche interrompue au-delà de 2 s (`progress_handler`, test
  `a_search_past_its_deadline_is_interrupted`), bench à 300 mots par corps.

## [BASSE] Extrait sans retrait des caractères de format (bidi, largeur nulle)
- Statut : **corrigé** : l'extrait est nettoyé par `header_text`, comme le
  sujet (cas U+202E et U+200B dans le contrat).

## [BASSE] `hostile_message_sql.rs` non étendu à la phase 8
- Le test de bout en bout ne cherche pas les charges piégées et ne couvre
  pas le nom de boîte des copies distantes.
- Statut : **corrigé** (phase 9) : recherche des charges et copie distante
  dans la boîte au nom piégé ajoutées au test.

## [INFO] Points relevés sans action immédiate
- Migration 8 : copies existantes rattachées au dossier qui porte
  aujourd'hui le rôle `\Drafts` (risque identique à l'avant-phase 8, limité
  aux données migrées, `\Draft` et UIDVALIDITY toujours vérifiés).
- Corbeille des brouillons : tout refus de `SELECT` fait oublier les copies
  de la boîte (au pire une copie reste sur le serveur).
- Au-delà de 64 niveaux d'imbrication, le repli de `sanitize_composed_html`
  indexe le contenu des `<script>`/`<style>` comme du texte (faux résultats
  possibles, sans danger).
- Copies provisoires des déplacements indexées sans leur texte jusqu'à la
  synchronisation suivante.
- `x` pendant une recherche peut cocher une conversation de la liste
  masquée ; aucune action ne peut la viser, et Rust ne vise que la
  conversation dans la portée transmise.

---

# Revue sécurité : phase 7, rédaction et envoi (2026-11-03)

Périmètre : `git diff master...HEAD` sur `phase-7/redaction` :
- domaine `compose` (réponses, préfixes, fil, citation, brouillons, file
  d'envoi), ports `DraftStore`, `OutboxStore`, `ContactQuery`,
  `MessageComposer`, `RemoteMailboxSession::append` ;
- `render` (HTML rédigé réassaini, version texte, MIME avec `lettre`,
  lecture d'un message pour y répondre) ;
- `imap` (APPEND, contournement d'`imap-next`), `smtp` (4xx/5xx),
  `sqlite` (migration 0006, suggestions d'adresses) ;
- application (`compose.rs`, `compose_worker.rs`), reports de la phase 6 ;
- commandes Tauri de rédaction, fichiers joints, interface (Tiptap,
  destinataires, boîte d'envoi), nouvelles dépendances.

Bilan : 0 critique, 1 haute, 3 moyennes, 6 basses, 3 points INFO. La
haute et les trois moyennes sont corrigées, ainsi que trois basses ; les
trois autres basses sont reportées. `cargo deny`, `pnpm audit --prod` et
`check-sql.sh` sans constat.

## [HAUTE] « Répondre » à un mail HTML piégé fait planter l'application (débordement de pile)
- `nesting_depth` décrémentait sur toute balise fermante, même orpheline
  (`</q>`) : `<b></q>` répété était estimé à une profondeur de 1 alors
  qu'html5ever imbrique les `<b>`. La version texte (analyseur récursif)
  débordait alors la pile ; `reply_source` lisait un HTML non borné.
- Statut : **corrigé** :
  - l'estimation ne compte que les fermetures d'éléments ouverts
    (arrêt à 4 × 512 niveaux, coût linéaire) ;
  - l'analyseur de la version texte ignore les ouvrantes au-delà de
    64 niveaux (récursion bornée) ;
  - le HTML lu pour une réponse est borné à 2 Mio (comme la lecture) ;
  - tests `orphan_closing_tags_do_not_overflow_the_stack` (pile de
    2 Mio), `orphan_closing_tags_stay_fast`,
    `orphan_closing_tags_do_not_hide_depth`.

## [MOYENNE] Le même contournement neutralise la protection du chemin de lecture
- `sanitize_html` devenait quadratique (24 s pour 560 Ko).
- Statut : **corrigé** par la même correction de `nesting_depth` (rejet
  « trop complexe » immédiat, testé sur 80 000 motifs).

## [MOYENNE] Autocomplétion empoisonnable, puce de destinataire usurpable
- Les expéditeurs d'Indésirables et de la Corbeille alimentaient les
  suggestions ; la puce n'affichait que le nom, qui pouvait imiter une
  adresse.
- Statut : **corrigé** :
  - Indésirables et Corbeille exclus (contrat `ContactQuery`, faux et
    SQLite) ;
  - la puce et la liste affichent toujours l'adresse réelle ; un nom qui
    contient `@`, `<` ou `>` est remplacé par l'adresse (test
    `RecipientInput.test.ts`).

## [MOYENNE] Envoi d'issue incertaine renvoyé automatiquement (doublons)
- Une coupure ou un délai dépassé pendant la transmission était classé
  temporaire et renvoyé jusqu'à 8 fois ; délai global de 60 s pour un
  message jusqu'à 34 Mio.
- Statut : **corrigé** : `SendError::Interrupted` (erreur sans réponse du
  serveur pendant l'envoi) devient `SendFailure::Uncertain`, jamais
  renvoyé automatiquement ; délai de transmission proportionnel à la
  taille (≥ 64 Kio/s). Tests `cut_after_data_is_an_uncertain_outcome`,
  `send_errors_map_to_failures`.

## [BASSE] Un Message-ID reçu non conforme empêche de répondre
- Statut : **corrigé** : `reply_threading` ignore les identifiants non
  conformes (test `invalid_received_identifiers_are_dropped_from_threading`).

## [BASSE] Échecs d'authentification ou de sécurité enchaînés sur toute la file
- Statut : **corrigé** : la tournée s'arrête aussi sur `Authentication`
  et `Security` (test `delivery_stops_on_authentication_or_security_failure`).

## [BASSE] Lecture des fichiers joints non bornée entre `metadata` et `read`
- Statut : **corrigé** : `read_bounded` (lecture limitée au reste du
  budget de 25 Mio, refus au-delà).

## [BASSE] Corbeille des copies distantes sans nom de boîte
- L'ancienne copie est supprimée dans le dossier qui porte aujourd'hui le
  rôle `\Drafts` ; UIDVALIDITY et `\Draft` sont vérifiés.
- Statut : **corrigé** (phase 8) : `RemoteCopy::mailbox`, migration 8,
  corbeille traitée dans la boîte de chaque copie (test
  `old_copies_are_removed_from_their_own_mailbox`).

## [BASSE] Brouillon refusé ou impossible à construire : synchronisation bloquée, reconnexion chaque minute
- Statut : **corrigé** (phase 8) : refus ou échec de construction
  journalisé, nouvel essai une heure plus tard, synchronisation poursuivie
  (test `a_refused_draft_does_not_block_the_others`).

## [BASSE] Taille des pièces jointes déclarée par l'interface, nombre non borné
- Statut : **corrigé** (phase 8) : taille réelle revérifiée
  (`load_attachments`), 100 pièces jointes au plus, type recalculé d'après
  le nom dans `draft_of` (tests `queue_message_checks_the_real_attachment_sizes`,
  `attachment_count_is_bounded`, `attachment_types_are_recomputed_in_rust`). Le nom de fichier et le type ne sont
  pas exploitables (`safe_file_name`, encodage RFC 2231, `mime` refusent
  CR/LF).

## [BASSE] Brouillon relu depuis SQLite sans `check_storable`
- Statut : **corrigé** (phase 8) : brouillon relu revalidé (test
  `hand_modified_draft_is_revalidated_on_read`).

## [INFO] Points relevés sans action immédiate
- Plafond `MAX_PENDING_OPS` vérifié hors de la transaction de
  `record_all` : dépassement léger possible entre deux commandes
  concurrentes, sans gravité.
- `hostile_message_sql.rs` : la rédaction couvre sujet, HTML, nom de
  destinataire, enveloppe, message brut et sujet de la file ; à compléter
  (Cc, Cci, fil, pièces jointes, `suggest`).
- `deny.toml` : licence `Unicode-DFS-2016` autorisée mais plus
  rencontrée ; la retirer serait un resserrement.

## Résultats outils
- `cargo deny --locked check` : advisories, bans, licenses, sources ok
  (avertissement `license-not-encountered` pour `Unicode-DFS-2016`).
- `pnpm -C app audit --prod` : aucune vulnérabilité connue.
- `scripts/check-sql.sh` : requêtes paramétrées uniquement.

---

# Revue sécurité : phase 6, actions et hors ligne (2026-11-02)

Périmètre : `git diff origin/master...HEAD` sur `phase-6/actions` :
- i18n Paraglide (ADR-011) ;
- contrat IPC en codes (`ipc.rs`, `errors.ts`) ;
- domaine `actions`, ports `RemoteMailboxSession` et `PendingOpStore` ;
- `imap` (`session.rs`, `lib.rs`), `sqlite` (`actions.rs`, migration 0004,
  filtres de visibilité) ;
- application (`actions.rs`, `account_actor.rs`), commandes Tauri
  (`actions.rs`) et interface (actions, annulation, `FolderPicker`,
  `ActionNotice`).

Bilan : 0 critique, 0 haute, 2 moyennes, 4 basses, 6 points INFO. Les deux
moyennes sont corrigées ; les basses sont reportées. Aucune injection IMAP
ni SQL ; aucun secret dans `detail` ; `cargo deny`, `pnpm audit --prod` et
`check-sql.sh` sans constat.

## [MOYENNE] Opération empoisonnée : une réponse `BAD` au rejeu bloque la synchronisation du compte
- Seul un refus `NO` abandonnait une opération. Une commande toujours
  refusée par `BAD`, ou toujours hors délai, faisait échouer le rejeu à
  chaque reconnexion, avant la synchronisation, qui n'avait donc plus
  jamais lieu.
- Statut : **corrigé** :
  - colonne `pending_ops.attempts`, incrémentée avant tout envoi au
    serveur (`PendingOpStore::claim`) ;
  - au-delà de `MAX_REPLAY_ATTEMPTS` (5), l'opération est abandonnée
    (`Conflict::TooManyAttempts`), l'état local est rétabli et la
    boîte est relue ;
  - test `an_op_refused_forever_is_dropped_after_too_many_attempts`.

## [MOYENNE] Annulation concurrente du rejeu : suppression définitive annoncée « annulée »
- `undo_actions` (et la fusion des bascules) pouvait retirer une opération
  pendant que l'acteur l'envoyait au serveur : le message réapparaissait,
  « Action annulée » s'affichait, mais l'`EXPUNGE` avait lieu.
- Statut : **corrigé** :
  - le rejeu réserve l'opération avant tout envoi
    (`pending_ops.claimed`, `claim`) ;
  - l'annulation par l'utilisateur et la fusion passent par `cancel`, qui
    ne touche qu'une opération non réservée (sinon « trop tard ») ; une
    bascule inverse sur une opération réservée est enregistrée à la suite ;
  - l'interface oublie la dernière action à la fin de la fenêtre
    d'annulation ;
  - tests `undo_during_replay_is_refused`, contrat `PendingOpStore`
    (réservation, `cancel`) et test front de la fenêtre.

## [BASSE] `hostile_message_sql.rs` non étendu à la phase 6
- Noms de boîte copiés dans `pending_ops.mailbox`/`target`, mots-clés relus
  par `action_target` et `action_settings` ne sont pas couverts de bout en
  bout par le test du message piégé (seulement par `check-sql.sh` et le
  contrat).
- Statut : **corrigé** (phase 7) : actions sur les messages piégés
  (drapeau, déplacement vers la boîte au nom piégé), relecture de la file,
  dossier de repli, rejeu sur le faux serveur et resynchronisation. Les
  écritures des brouillons y seront ajoutées avec eux.

## [BASSE] Fenêtre de concurrence du garde-fou `EXPUNGE` sans UIDPLUS
- Entre `UID SEARCH DELETED` et `EXPUNGE`, un autre client peut poser
  `\Deleted` sur un message, qui part aussi. Aucune opération atomique
  n'existe sans UIDPLUS ; les deux commandes s'enchaînent déjà sans autre
  commande entre elles.
- Statut : **corrigé** (phase 7) : `expunge_all` compte les réponses
  `* n EXPUNGE` ; au-delà de l'attendu, journalisation et
  `ReplayReport::foreign_expunges` (boîte resynchronisée) ; limite
  documentée dans `expunge_is_safe`.

## [BASSE] Commandes IPC accordées sans manifeste applicatif
- `perform_action`, `undo_actions` et `set_archive_folder` sont
  destructrices et ne sont pas restreintes à la fenêtre `main`.
  Aujourd'hui, la sandbox opaque de `mailview://` et la CSP empêchent tout
  accès à l'IPC (test e2e de la phase 5).
- Statut : **corrigé** (phase 10) : manifeste applicatif (`build.rs`,
  `ipc-commands.rs`), permissions accordées à la seule fenêtre `main` par
  la capability `default` (`permissions/main-window.toml`) ; test
  `acl_tests::app_commands_are_granted_to_the_main_window_only` (autre
  fenêtre et origine distante refusées, avec la configuration réelle).

## [BASSE] File `pending_ops` non bornée, fusion quadratique, application partielle
- La file est relue pour chaque opération planifiée ; chaque `record` est
  une transaction séparée (une erreur de stockage en cours de boucle laisse
  une action à moitié appliquée).
- Statut : **corrigé** (phase 7) : lecture unique de la file par compte,
  fusion et plafond décidés par `plan_queue`, `record_all` en une
  transaction, plafond `MAX_PENDING_OPS`.

## [INFO] Points relevés sans action immédiate
- `undo_actions` accepte tout identifiant d'opération en attente : pas de
  frontière de privilège entre comptes locaux, effet limité à une
  annulation.
- Texte rédigé en Rust dans certains `detail` (`commande refusée : …`,
  `MOVE non annoncé`, `rendu interrompu`) : nettoyé, sans secret ; à
  ramener au seul texte brut du serveur.
- Coupure entre `COPY` et `EXPUNGE` : le rejeu suivant recopie (doublon
  dans la destination, sans perte) ; piste `COPYUID`.
- Actions sur une conversation bornées à 1000 messages, sans signalement.
- `ImapConnector::hiding_capabilities` s'applique après authentification :
  ne peut masquer ni `STARTTLS` ni `LOGINDISABLED`.
- Dépendances npm i18n : versions figées, licences permissives, aucun
  script d'installation, greffon chargé localement (pas de téléchargement
  au build).

---

# Revue sécurité : phase 5, liste, conversations et lecture (2026-11-02)

Périmètre : `git diff master...HEAD` sur `phase-5/lecture` (domaine
`conversation`/`reading`, `sqlite` migration 0003/`conversations`/
`preferences`, `render` (ammonia, CSS, MIME, agent, CSP), `imap` sections
`n.MIME`, `application` lecture et acteur, `app/src-tauri` protocoles,
proxy d'images, commandes, front `agent.ts`/`MessageBody.svelte`, e2e).
Bilan : 0 critique, 1 haute, 3 moyennes, 5 basses, plus des points INFO.
Haute et moyennes corrigées ; basses corrigées sauf une (e2e, en partie).

## [HAUTE] Pièces jointes écrites sans Mark-of-the-Web (Windows)
- Fichiers enregistrés ou ouverts sans flux `Zone.Identifier` : SmartScreen,
  Mode protégé d'Office et blocage des macros venues d'Internet contournés.
- Statut : **corrigé** : `reading.rs::mark_from_internet` écrit
  `[ZoneTransfer] ZoneId=3` après chaque écriture (enregistrement et
  ouverture) ; test `#[cfg(windows)]` qui relit le flux.

## [MOYENNE] `<style>` avec attribut non assaini ; protocoles adressables pour tout message
- `<style class=x>` sortait d'ammonia avec son attribut et échappait à
  `sanitize_css` ; `mailpart://<autre message>/<partie>` déclenchait des
  téléchargements IMAP (déni de service réseau ; la CSP empêchait toute
  exfiltration).
- Statut : **corrigé** : aucun attribut sur `<style>` (attribute_filter) et
  repérage de toute balise `<style …>` ; `mailpart://` ne sert plus que
  les parties à `Content-ID` du dernier rendu du message, sans jamais
  télécharger (`MailServices::inline_part`) ; `remote-img://` lit la liste
  du rendu en cache. Vecteur `56-style-attribut.html` et test unitaire.

## [MOYENNE] Déni de service par HTML ou CSS pathologique
- Analyse quadratique selon la profondeur d'imbrication (80 000 `<div>` :
  34 s), `remove_at_rules` quadratique, rendu sur les workers async, fait
  deux fois par ouverture.
- Statut : **corrigé** : HTML borné à 2 Mio, profondeur estimée par un
  pré-scan linéaire (au-delà de 512 : message « trop complexe ») ;
  `remove_at_rules` en un passage ; rendu dans `spawn_blocking` avec délai
  de 10 s et cache par (message, images, texte brut). Tests de durée
  (80 000 `<div>`, 60 000 `@import;`).

## [MOYENNE] Liste noire « exécutables » incomplète
- `.html`, `.svg`, `.doc`, `.xls`, `.website`, `.search-ms`… s'ouvraient
  directement.
- Statut : **corrigé** : liste blanche des types ouvrables
  (`OPENABLE_EXTENSIONS` : PDF, images matricielles, texte, OOXML/ODF sans
  macros, audio, vidéo) ; tout le reste s'enregistre seulement. Tests
  ajoutés.

## [BASSE] Proxy d'images : variables `HTTP(S)_PROXY` honorées
- Statut : **corrigé** : `.no_proxy()`.

## [BASSE] Paramètres `mailto:` non filtrés (`attach=`)
- Statut : **corrigé** : seuls `to`, `cc`, `bcc`, `subject`, `body`,
  `in-reply-to` ; `LinkError::MailtoParameter`, tests.

## [BASSE] Aucun garde de navigation de premier niveau
- Statut : **corrigé** : greffon interne `navigation-guard`
  (`protocols::navigation_allowed`) : interface, `mailview://` et serveur
  Vite en debug seulement ; test unitaire.

## [BASSE] `hostile_message_sql.rs` non étendu à la phase 5
- Statut : **corrigé** : conversations (dossier et boîte unifiée),
  messages d'une conversation et préférences relus avec les charges ;
  schéma et intégrité revérifiés.

## [BASSE] Lacunes des tests end-to-end de sécurité
- Statut : **reporté en partie (phase 9, finitions)** : la CSP du document
  est vérifiée par test unitaire, et l'e2e vérifie l'effet (zéro requête,
  un seul script) ; restent à ajouter un scénario « expéditeur autorisé »
  (proxy observé par le canari) et un vecteur qui contourne volontairement
  l'assainisseur. Pas de WebView2 en CI (ADR-007).

## [INFO]
- `is_public_address` complété (`fec0::/10`, `2001::/32`, `192.88.99.0/24`).
- Noms réservés Windows en exposant (`COM¹`…) : à ajouter si besoin.
- `thread()` (JWZ complet) est récursif dans `prune` : utilisé en test
  seulement ; à rendre itératif avant tout usage en production.
- Intrusion dans un fil (JWZ) : comportement attendu ; indication visuelle
  d'un nouvel expéditeur envisageable.
- Sections IMAP non demandées acceptées dans la réponse : à filtrer.
- Extension masquée par l'ellipse CSS d'un nom long ; dossier
  `cache/pieces-jointes` jamais purgé (à traiter en phase 9).
- Dépendances : `ammonia` 4.2.4, `sha2`, `open` 5.4.5, `url`,
  `tauri-plugin-dialog` 2.6 (sans zbus, aucune permission côté interface),
  `@tanstack/virtual-core` 3.19.10.

## Résultats outils
- cargo deny : **ok** (advisories, bans, licenses, sources).
- pnpm audit --prod : aucune vulnérabilité connue.
- scripts/check-sql.sh : requêtes paramétrées uniquement.

---

# Revue sécurité : phase 4, stockage et synchronisation (2026-11-02)

Périmètre : `git diff master...HEAD` sur `phase-4/sync` (domaine
`mailbox`/`sync`/`fetch_policy`/`ports`, `imap` `session`/`utf7`/
`connection`, `sqlite` `mail`/migration 0002/pool de lecteurs, `blobs`,
`application` `sync`/`account_actor` ; `test-support` et tests
d'intégration en survol). Revue faite par l'agent `security-reviewer`, en
lecture seule, avec des programmes d'essai contre `imap-codec`
2.0.0-alpha.9.

Synthèse : 0 critique, 1 haute, 4 moyennes, 6 basses, 3 infos. La haute et
les quatre moyennes sont corrigées (commit `d4495fa`), ainsi que trois
basses ; le reste est reporté ou accepté ci-dessous.

## [HAUTE] Débordement de pile dans `imap-codec` : `message/rfc822` imbriqué sans limite, arrêt du processus, y compris avant STARTTLS
- Problème : `imap-codec` remet son compteur de récursion à 8 à chaque
  `message/rfc822` (`body.rs:73`). Environ 500 niveaux (43 Ko, sous la
  limite de 64 Kio d'avant TLS) font déborder une pile de 2 Mio : arrêt du
  processus, pas de panic. `MAX_MIME_DEPTH` n'intervenait qu'après
  l'analyse.
- Scénario : attaquant sur le réseau qui injecte la réponse en clair avant
  STARTTLS, ou serveur hostile : Norelly s'arrête à chaque connexion.
- **Statut : corrigé.** `crates/adapters/imap/src/nesting.rs` : chaque
  réponse est assemblée en entier (ligne, littéraux, suites) et sa
  profondeur de parenthèses mesurée hors chaînes et hors littéraux avant
  d'être transmise au décodeur. Au-delà de 48, elle est refusée avant
  authentification et écartée ensuite. Tests
  `deeply_nested_response_before_tls_is_refused_without_crash` (500
  niveaux, en clair) et `hostile_bodystructures_do_not_block_metadata`
  (après TLS). Défaut à signaler en amont (`body_type_msg_limited(input,
  remaining_recursions - 1)`). Aucun `[patch]` d'`imap-codec`.

## [MOYENNE] Message « poison » : un seul message bloque la synchronisation du compte pour toujours
- Problème : BODYSTRUCTURE au-delà de la limite d'`imap-codec`, message
  de plus de 64 Mio, ou `NO` sur un FETCH : l'erreur faisait échouer tout
  le lot, puis la reconnexion rejouait l'échec.
- **Statut : corrigé.**
  - Une fois authentifié, une réponse indécodable ou trop longue (déjà
    écartée par `imap-next`) est comptée et la commande continue.
  - `fetch_meta` redemande les UID manquants sans BODYSTRUCTURE, puis
    sous une forme minimale (drapeaux, taille, date).
  - `ensure_ok` : `NO` devient `ConnectionError::Rejected` (session
    intacte) et un `NO` partiel sur FETCH garde les données reçues ; un
    corps refusé est sauté.
  - `FetchPolicy` ne télécharge jamais un message d'un bloc au-delà de
    48 Mio (`MAX_FULL_FETCH`) : il est téléchargé par parties, les plus
    grandes étant différées.
  - Tests : `hostile_bodystructures_do_not_block_metadata`,
    `huge_messages_are_never_fetched_whole`.

## [MOYENNE] Dépliage de VANISHED : temps CPU quadratique, suppressions perdues au-delà du plafond
- **Statut : corrigé.** `VANISHED` est conservé en plages normalisées
  (`norelly_domain::sync::UidRanges` : tri, fusion, recherche
  dichotomique), puis croisé avec les UID connus (`BTreeMap::range`). Il
  n'est plus déplié et aucune suppression n'est perdue. Tests :
  `uid_ranges_are_normalized_and_never_expanded` (100 000 plages
  `1:4294967295`) et 10 000 lignes `VANISHED` répétées dans
  `hostile_sync_responses_are_contained`, avec une borne de temps.

## [MOYENNE] Aucun budget mémoire cumulé par commande
- **Statut : corrigé.** Les octets reçus par commande sont plafonnés à
  256 Mio (`MAX_COMMAND_BYTES`). Le nombre de réponses de données est
  passé en paramètre de `execute_with` : `fetch_meta` en accepte
  4 × le nombre d'UID demandés + 64, `fetch_body` 64.

## [MOYENNE] Délai global par commande absent ; arrêt de l'acteur impossible à interrompre
- **Statut : corrigé.**
  - Échéance par commande : 2 min pour une commande ordinaire
    (`COMMAND_DEADLINE`), 30 min pour une commande de synchronisation
    (`BULK_DEADLINE`). Un octet toutes les 59 s ne la prolonge plus. Cela
    règle aussi le constat bas de la phase 2.
  - Une commande abandonnée (`busy`) rend la session inutilisable, qui
    est alors fermée sans `LOGOUT`.
  - Acteur : les synchronisations sont dans un `select!` avec l'arrêt, et
    `shutdown` abandonne une tâche après 5 s (`SHUTDOWN_GRACE`). Test
    `shutdown_interrupts_a_long_sync`.

## [BASSE] Blobs jamais supprimés : contenu des messages effacés conservé sur disque
- **Statut : reporté (phase 6, actions et hors ligne).** Il faut un
  ramasse-miettes avec comptage des références, car un contenu est
  dédupliqué entre boîtes (Gmail `\All`). Seule la suppression du compte
  purge pour l'instant. La croissance du disque face à un serveur qui
  change l'UIDVALIDITY en boucle est couverte par la même tâche.

## [BASSE] Chaînes de `MimePart` ni bornées ni assainies
- **Statut : corrigé.** Type et sous-type passent par `mime_token`
  (jetons ASCII de 64 caractères au plus), le nom de fichier par
  `attachment_name` (`header_text`, 255 caractères). Test
  `mime_strings_are_bounded_and_sanitized`. L'assainissement pour
  l'écriture sur disque reste prévu en phase 5.

## [BASSE] Intégrité des blobs : pas de `fsync` du répertoire, blob corrompu jamais réparé
- **Statut : corrigé en partie.** Le répertoire est synchronisé après le
  renommage (Unix). Un fichier existant mais illisible ou corrompu est
  réécrit par `put` (test `corrupted_blob_is_an_error`). Reporté : le
  nettoyage des `.tmp-*` orphelins au démarrage (phase 5, avec le
  branchement de l'acteur dans l'application).

## [BASSE] Purge des exceptions de certificat orphelines
- **Statut : reporté (phase 5).** Elle sera faite avec le branchement des
  comptes et de l'acteur dans l'application.

## [BASSE] L'acteur fige consentement au clair et exceptions de certificat
- **Statut : reporté (phase 5).** L'acteur n'est pas encore lancé par
  l'application. Au branchement, il sera redémarré à chaque changement de
  ces réglages, avec un test. Le mot de passe reste en mémoire pendant la
  session : c'est nécessaire pour se reconnecter.

## [BASSE] `execute_bulk` annulé laisse `max_untagged` à 2 M
- **Statut : corrigé.** La limite est passée en paramètre et n'est plus
  stockée. Une commande annulée rend de toute façon la session
  inutilisable.

## [INFO] Liste `is_format_char` incomplète
- Reporté en phase 5 (affichage des expéditeurs) : caractères
  d'étiquette U+E0000–U+E007F, U+FFF9–U+FFFB, U+180B–U+180F, fillers
  Hangul, U+2028/U+2029, homoglyphes.

## [INFO] Images de conteneurs de test figées par étiquette, pas par empreinte
- Accepté pour l'instant : `dovecot/dovecot:2.4.5` et
  `ghcr.io/shopify/toxiproxy:2.12.0` servent uniquement aux tests
  d'intégration. À figer par digest avec les images e2e.

## [INFO] Nouvelles dépendances
- `mail-parser` 0.11, `zstd` 0.13 (C, décompression bornée et testée),
  `serde_json`, `criterion` (bench), `proptest` (tests du domaine).
  `deny.toml` inchangé.

## Vérification complémentaire : injection SQL par un message piégé (2026-11-02)
- Demande du mainteneur : un message piégé peut-il injecter du SQL et
  corrompre la base ?
- Audit : non. Toutes les valeurs issues d'un message ou du serveur
  passent par des paramètres liés. Les trois `format!` qui produisent du
  SQL n'interpolent que des constantes (`ACCOUNT_COLUMNS`, `LIST_COLUMNS`)
  et un filtre désormais typé `&'static str`. Il n'y a pas
  d'`execute_batch` hors tests, et `rusqlite` n'est utilisé que par
  `crates/adapters/sqlite`.
- Garde permanent : `scripts/check-sql.sh`, ajouté à `scripts/check.sh`.
  Il refuse `execute_batch` en production, tout SQL construit par
  `format!`/`concat!` sans commentaire `sql-const`, toute interpolation
  qui n'est ni une constante ni un `&'static str`, toute interpolation
  positionnelle, `push_str` et la concaténation sur du SQL, ainsi que toute
  dépendance à `rusqlite` hors du crate de stockage. Vérifié par mutation :
  quatre requêtes vulnérables introduites volontairement sont toutes
  refusées.
- Test de bout en bout `crates/application/tests/hostile_message_sql.rs`,
  exécuté sans Docker. Des messages piégés portent des charges dans le
  sujet, l'expéditeur, les destinataires, le Message-ID, les références,
  les mots-clés, le nom de boîte et le corps, et sont synchronisés deux
  fois (faux serveur IMAP, `mail-parser`, SQLite sur disque, fichiers).
  Charges : requêtes empilées, sous-requête sur `sqlite_schema`,
  `ATTACH`, `load_extension`, `writable_schema`, rupture du JSON, octet
  nul. Le test vérifie :
  - schéma identique et `integrity_check` sain, sans clé étrangère
    orpheline ;
  - comptes intacts et aucune boîte créée ou supprimée ;
  - valeurs relues comme données et corps identiques.
  Vérifié par mutation : le test échoue dès qu'une requête vulnérable est
  introduite.
- `Database::integrity_problems` et `Database::schema` sont ajoutées
  (diagnostic, réutilisable au démarrage).
- Point de contrôle n° 9 ajouté à l'agent `security-reviewer`.

## Résultats outils
- `cargo deny --locked check` : ok (avertissement préexistant :
  `Unicode-DFS-2016` autorisée mais inutilisée).
- `pnpm -C app audit --prod` : aucune vulnérabilité connue.
- `./scripts/check.sh full` et `cargo bench -p norelly-sqlite -- --test`
  passent après corrections.

---

# Revue sécurité : phase 3, comptes, autodécouverte et interface (2026-11-02)

Périmètre : `git diff afb655e..HEAD` et les fichiers e2e non commités au
moment de la revue (domaine, `discovery`, `sqlite`, `blobs`, `application`,
`app/src-tauri`, `app/src`, `app/e2e`, CI, `deny.toml`). Revue faite par
l'agent `security-reviewer`, en lecture seule.

Synthèse : 0 critique, 1 haute, 1 moyenne, 6 basses, 8 infos. La haute et
la moyenne sont corrigées, ainsi que cinq basses ; le reste est accepté ou
reporté ci-dessous.

## [HAUTE] Serveur SRV hors domaine non signalé si la découverte est partielle ; confirmation non imposée par Rust
- Problème : l'interface testait la complétude avant `requires_confirmation`
  (avertissement perdu si un seul protocole était trouvé), le message ne
  nommait pas l'hôte, et `add_account` ne distinguait pas un serveur
  confirmé d'un serveur repris de la découverte.
- Scénario : réponse DNS falsifiée (`_imaps._tcp`) vers un serveur au
  certificat valide : le mot de passe part chez l'attaquant sans alerte.
- **Statut : corrigé.**
  - Rust (`app/src-tauri/src/state.rs`, `commands.rs`) : les hôtes à
    confirmer présentés par `discover_settings` sont mémorisés ;
    `add_account` renvoie `confirmation_required` sans aucune connexion
    tant que l'hôte ne figure pas dans `confirmed_hosts`. Tests
    `unconfirmed_hosts_block_until_confirmed` et, côté application,
    `srv_outside_domain_is_never_used_without_review`.
  - Interface (`add-account.svelte.ts`, `AddAccountDialog.svelte`) :
    avertissement d'abord, nommant l'hôte et la source, case à cocher par
    hôte, envoi bloqué sans confirmation ; tests vitest du cas partiel.

## [MOYENNE] Clé de trousseau commune à deux comptes (alias) : retour arrière et suppression effacent le secret d'un autre compte
- **Statut : corrigé.** Clé propre au compte (`SecretKey::for_account`,
  `account:<id>:<protocole>`) ; `AddAccount` insère le compte d'abord (un
  doublon, y compris par course, échoue sans toucher au trousseau), écrit
  ensuite les secrets et, en cas d'échec, ne retire que ce qu'il a créé
  (secrets puis compte). Identifiants SQLite en `AUTOINCREMENT` (jamais
  réattribués). Tests `aliases_on_the_same_login_keep_separate_secrets`,
  `duplicate_at_insert_leaves_the_keyring_untouched`.

## [BASSE] Garde « seule une empreinte présentée » et filtre `requires_confirmation` non testés
- **Statut : corrigé.** Tests `only_a_presented_rejection_can_be_taken_once`,
  `pending_rejections_are_bounded` (`state.rs`) et
  `srv_outside_domain_is_never_used_without_review`.

## [BASSE] Le test « ne suit pas les redirections HTTP » ne testait aucune redirection
- **Statut : corrigé.** Décision extraite dans `redirect_decision`
  (`discovery/src/http.rs`) et testée (HTTPS → HTTP refusé, chaîne bornée) ;
  l'ancien test est renommé selon ce qu'il vérifie.

## [BASSE] Exceptions de certificat accumulées et conservées après le compte
- **Statut : corrigé en partie.** Accepter une empreinte changée remplace
  les anciennes (`trust_certificate`, test
  `trusting_a_changed_fingerprint_replaces_the_old_one`) et exige une case
  à cocher supplémentaire dans l'interface. **Reporté (phase 4, puis phase 5 : voir la revue de la phase 4)** : purge
  des exceptions des hôtes qu'aucun compte n'utilise plus, à faire avec la
  gestion des dossiers et la reconnexion (une exception peut servir à
  plusieurs comptes du même hôte).

## [BASSE] Infrastructure e2e : chaîne d'approvisionnement non figée, `curl | bash`
- **Statut : corrigé en partie.** Images Ubuntu, Dovecot et Mailpit figées
  par digest. **Accepté pour l'instant** : NodeSource et rustup installés
  par script (image de test uniquement, jamais distribuée, reconstruite
  seulement quand le Dockerfile change) ; à remplacer par des archives
  vérifiées par SHA-256 avec le durcissement de la CI (phase 10).
  **Corrigé (phase 10)** : rustup-init, Node.js et pnpm téléchargés en
  archives figées vérifiées par SHA-256 ; actions GitHub figées par SHA.

## [BASSE] Infrastructure e2e : réseau non isolé, dépôt en écriture, `PATH` préfixé par le cache
- **Statut : corrigé.** Compilation et scénarios dans deux conteneurs : les
  scénarios tournent sur un réseau `--internal` (aucun accès extérieur,
  donc aussi aucune requête vers l'ISPDB) avec le dépôt monté en lecture
  seule ; le cache n'est plus dans le `PATH`. La compilation garde le dépôt
  en écriture (`app/dist`), comme un build local.

## [BASSE] Répertoire de données créé avec les permissions par défaut
- **Statut : corrigé.** Répertoire en 0700 sous Unix, droits restreints s'il
  existait déjà (`create_private_dir`, test `data_dir_is_private`).

## [INFO] e2e : chemin `wdio.conf.ts` erroné
- **Statut : corrigé** (`wdio.conf.mjs`). Le scénario s'exécutait tout de
  même (WebdriverIO résout l'extension), le rapport e2e le montrait.

## [INFO] Commandes IPC non restreintes par capability
- **Reporté (phase 5)**, avant l'arrivée de `mailview://` : déclarer les
  commandes dans le manifeste applicatif et ne les accorder qu'à `main`.

## [INFO] Vie privée de la découverte
- Accepté : l'ISPDB reçoit le domaine, `autoconfig.<domaine>` l'adresse ;
  les variables de proxy sont respectées (TLS de bout en bout). À
  documenter dans l'aide utilisateur (phase 9).

## [INFO] Copies du mot de passe non effacées (JSON IPC, état Svelte)
- Accepté : effacé à `reset()` et après succès ; inévitable côté JS.

## [INFO] `Username` admettait les caractères bidirectionnels
- **Statut : corrigé.** `Username::parse` refuse aussi les caractères de
  mise en forme invisibles (`is_format_char`).

## [INFO] `dangerous()` dans `standard_client_config`, nouvelles dépendances, audit npm
- Sans constat : vérificateur de la plateforme sans exception ; licences
  conformes ; avis `pnpm audit` uniquement en développement (wdio, mocha),
  à suivre.

## Résultats outils
- `cargo deny --locked check` : ok (avertissement `license-not-encountered`
  pour `Unicode-DFS-2016`, sans effet).
- `pnpm -C app audit --prod` : aucune vulnérabilité.

---

# Revue sécurité : phase 2, transport et sécurité de connexion (2026-11-01)

Périmètre : `git diff 1dd5378..HEAD` (domaine, `tls`, `imap`, `smtp`,
`keyring`, `application`, `test-support`, `Cargo.toml`, ADR-002).

Note : l'agent `security-reviewer` n'était pas chargé dans la session (elle
avait démarré dans le dossier parent). La revue a été faite par un agent
généraliste à qui l'on a fait suivre `.claude/agents/security-reviewer.md` à
la lettre, en lecture seule.

Synthèse : 0 critique, 0 haute, 3 moyennes, 4 basses, 5 infos.

Aucune fuite d'identifiants exploitable, aucun contournement TLS et aucun
repli en clair n'a été trouvé.

## [MOYENNE] IMAP : réponses non étiquetées accumulées sans limite, y compris avant STARTTLS
- Emplacement : `crates/adapters/imap/src/connection.rs` (`execute`, `capability`).
- Problème : aucune limite sur le nombre de réponses non étiquetées conservées
  pour une commande ; la limite de 64 Mio vaut par réponse, et s'applique
  aussi avant TLS.
- Scénario : un attaquant actif avant STARTTLS répond au `CAPABILITY` par un
  flot sans fin de `* CAPABILITY …` sans jamais `OK` ; la mémoire sature.
- Statut : **corrigé** : au plus 10 000 réponses non étiquetées par commande ;
  limite d'une réponse ramenée à 64 Kio avant TLS ; test de flot sans fin.

## [MOYENNE] IMAP : recherche de fin de ligne quadratique
- Emplacement : `crates/adapters/imap/src/connection.rs` (`feed_line`).
- Problème : tout le tampon est reparcouru à chaque bloc reçu ; une ligne de
  64 Mio sans `\n` coûte des dizaines de Gio de comparaisons.
- Statut : **corrigé** : la recherche reprend à la position déjà examinée.

## [MOYENNE] `cargo deny` en échec : RUSTSEC-2026-0370/0371/0372 (zbus 5 non maintenu via `keyring` 4)
- Statut : **corrigé** sans toucher à `deny.toml` : `keyring` remplacé par
  `keyring-core` + `dbus-secret-service-keyring-store` (libdbus du système,
  liée dynamiquement) sous Linux et `windows-native-keyring-store` sous
  Windows. Plus aucune dépendance zbus. La CI installe `libdbus-1-dev`
  (fichier d'infra à relire).

## [BASSE] Pas de délai global par commande (délai d'inactivité par lecture seulement)
- Emplacement : `imap/src/connection.rs`, `smtp/src/stream.rs`.
- Scénario : un octet toutes les 59 s maintient un test de connexion ouvert
  indéfiniment.
- Statut : **IMAP corrigé en partie en phase 4** : échéance par commande
  (`COMMAND_DEADLINE`/`BULK_DEADLINE`) pour toutes les commandes passant
  par `execute`, dont CAPABILITY, STARTTLS et LOGIN. **Reporté** : la
  salutation et `AUTHENTICATE` (délai par lecture seulement) et SMTP, avec
  la rédaction et l'envoi (phase 7).

## [BASSE] Effacement partiel des secrets (copies dans imap-types, lettre)
- Statut : **accepté** : effacement « au mieux », documenté dans
  `credentials.rs` ; les copies internes à `imap-types` (`CommandBody::login`,
  réponses SASL) et à `lettre` (`Credentials` clonées) échappent à notre
  contrôle. Piste pour plus tard : LOGIN en littéral depuis un tampon
  `Zeroizing`.

## [BASSE] SMTP via lettre : ligne de réponse lue en entier avant le contrôle de taille
- Statut : **reporté (phase 6, envoi)** : seulement après TLS (serveur
  authentifié ou accepté par exception), borné par le délai de 60 s. Piste :
  compteur d'octets dans `LettreStream::poll_read`, et signalement en amont.

## [BASSE] `server_text` laisse passer les caractères de mise en forme Unicode (U+202E…)
- Statut : **corrigé** : les caractères de mise en forme bidirectionnelle et
  invisibles (U+00AD, U+061C, U+200B–U+200F, U+202A–U+202E, U+2060–U+2069,
  U+FEFF) sont remplacés ; cas ajouté au test.

## [INFO] Plafond `tracing` à DEBUG fragile
- Le test `credentials_never_appear_in_logs_or_debug` échoue si on retire le
  plafond (vérifié). À revérifier à chaque mise à jour d'`imap-next` ;
  défaut à signaler en amont (ADR-002).

## [INFO] SMTP smuggling (LF nu) non géré par lettre
- À traiter en phase 6 (rédaction) : normaliser `OutgoingMessage.raw` en CRLF
  strict avant `send`.

## [INFO] Dépendances nouvelles
- `imap-codec`/`imap-types` en préversion analysent des données hostiles ;
  `nom` ajouté pour un défaut de features. `rustls-platform-verifier` ne
  vérifie pas la révocation sous Linux. `pub use rustls` dans `norelly-tls`
  expose `dangerous()` : à surveiller en relecture.

## [INFO] Tests d'injection dépendants de la segmentation TCP
- Fiables sur la boucle locale (OK et données injectées en un seul `write`).

## [INFO] Exceptions de certificat par hôte, pas par port
- Conforme au cahier des charges ; sans risque (même certificat exact).

## Résultats outils
- cargo deny : échec initial (avis zbus) ; **ok** après correction
  (advisories, bans, licenses, sources).
- pnpm audit : non exécuté (pas d'interface dans cette phase).
