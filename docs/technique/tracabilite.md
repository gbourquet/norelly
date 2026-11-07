# Traçabilité : des fonctionnalités au code

[← Documentation technique](README.md)

La [documentation fonctionnelle](../fonctionnel/README.md) décrit ce que
fait Norelly, sans rien dire de sa construction. Cette page fait le lien
inverse : pour chaque fiche, les modules qui portent ses règles, les cas
d'usage, les clés des catalogues de messages (`app/messages/{fr,en}.json`,
ADR-011) et les identifiants des commandes du registre
(`registerAppCommands`, `app/src/lib/app.svelte.ts`).

En cas de désaccord entre une fiche et le code, **le domaine fait foi**
(`crates/domain`) : la fiche est à corriger, ou l'écart est un défaut à
signaler. Une règle modifiée met à jour sa fiche **et** cette page.

## Comptes : ajout, configuration, retrait

Fiche : [comptes.md](../fonctionnel/comptes.md).

### Règles et code

| Règle | Où |
|---|---|
| Adresse validée strictement : `dot-atom` RFC 5322, UTF-8 admis dans la partie locale (RFC 6531), domaine validé comme nom d'hôte ; guillemets et IP littérales refusés | `crates/domain/src/address.rs` |
| Ordre d'autodécouverte : autoconfig du fournisseur (HTTPS) → ISPDB de Mozilla (HTTPS, seul le domaine transmis) → SRV RFC 6186 → sondage des ports usuels (`imap.`, `smtp.`, `mail.`, domaine) en TLS implicite ou STARTTLS → saisie manuelle ; arrêt dès que IMAP et SMTP sont trouvés | `crates/domain/src/discovery.rs` |
| Préférence de sécurité : TLS implicite > STARTTLS > clair | `discovery.rs` |
| Jamais de HTTP en clair pour l'autoconfiguration ; serveur SRV hors du domaine de l'adresse : confirmation obligatoire | `discovery.rs` |
| Certificat : valide pour la plateforme, ou exception mémorisée (hôte + empreinte SHA-256) ; sinon rejet avec un `CertificateIssue` détaillé ; une empreinte différente de l'exception est signalée comme changement | `crates/domain/src/security.rs` (`TlsTrustPolicy`) |
| Clair refusé sans consentement explicite par serveur ; STARTTLS obligatoire s'il est annoncé ou demandé (aucun repli silencieux) ; capacités relues après TLS | `security.rs` (`PlaintextPolicy`), `capabilities.rs` |
| Authentification : `AUTHENTICATE PLAIN` ou `LOGIN` selon les capacités, `LOGINDISABLED` respecté ; `XOAUTH2` non pris en charge | `crates/domain/src/auth.rs` |
| Le secret n'est jamais dans `Debug`, n'a pas de `Display`, n'est pas sérialisable | `crates/domain/src/credentials.rs` |
| Le mot de passe n'appartient pas au compte : il vit dans le trousseau, sous des clés propres au compte | `crates/domain/src/account.rs` |
| Fichier de secrets non chiffré (droits 0600) seulement si aucun trousseau ne répond, après consentement (dérogation ADR-002) | `crates/adapters/keyring/src/file.rs` |

Cas d'usage : `DiscoverSettings`, `TestConnection`, `AddAccount`,
`RemoveAccount` (`crates/application/src/accounts.rs`,
`discover_settings.rs`, `test_connection.rs`).

### Clés des messages

`add_discovering`, `add_testing`, `add_msg_incomplete`, `add_msg_dns_outside`, `add_cert_title`, `add_issue_*`, `add_cert_changed`, `add_plain_warning`, `add_secrets_warning_*`, `add_fail_authentication`, `add_fail_network`, `add_fail_security`, `add_msg_already_exists`, `remove_explanation_*`.

### Commandes

`account.add`, `account.remove`.

## Synchronisation

Fiche : [synchronisation.md](../fonctionnel/synchronisation.md).

### Règles et code

| Règle | Où |
|---|---|
| Rôle d'un dossier : attribut SPECIAL-USE (`\Sent`, `\Drafts`, `\Trash`, `\Junk`, `\Archive`, `\All`), sinon heuristique de nom | `crates/domain/src/mailbox.rs` |
| Stratégie par dossier : complète (première fois ou UIDVALIDITY changée) ; QRESYNC (`VANISHED`) ; CONDSTORE (drapeaux depuis `HIGHESTMODSEQ`) ; sans extension (comparaison des UID et drapeaux) ; rien si `HIGHESTMODSEQ` et `UIDNEXT` n'ont pas bougé | `crates/domain/src/sync.rs` (`plan`) |
| Propriétés : convergence (local = serveur), idempotence, application double sans effet (tests `proptest`) | `sync.rs`, `sync/tests.rs` |
| Rapatriement : en-têtes et texte pour tous ; pièces jointes sous le seuil ; structure MIME bornée (profondeur 16, 256 parties) | `crates/domain/src/fetch_policy.rs` |
| Seuil de rapatriement borné entre 64 Kio et 64 Mio | `crates/domain/src/settings.rs` |
| IDLE renouvelé toutes les 25 minutes (avant la limite des 29 minutes de la RFC 2177) | `sync.rs` (`IDLE_RENEWAL_SECS`) |
| Reconnexion : attente exponentielle de 1 s à 5 min, avec gigue, remise à zéro après un succès | `sync.rs` (`Backoff`) |
| Tout texte venu du serveur est assaini (contrôles retirés, longueur bornée) ; noms de boîte ≤ 1024 caractères, 10 000 boîtes au plus | `mailbox.rs` |

Orchestration : acteur de compte à deux connexions (commandes ; IDLE sur la
boîte de réception), `SyncAccount` boîte par boîte, enregistrement par lots
pour une reprise propre (`crates/application/src/account_actor.rs`,
`sync.rs`).

### Clés des messages

`common_offline`, `sidebar_offline_reason`, `failure_*`, `message_notice_not_available`.

### Commandes

`account.sync`.

### Vérifications et compléments

- Coupure en pleine synchronisation : chaque lot intermédiaire est
  enregistré sans `HIGHESTMODSEQ` ; la reprise compare les UID. Vérifié
  par Toxiproxy (coupure, latence, connexion à moitié fermée).
- `VANISHED` d'UID inconnus localement : ignorés ; dépliage des plages
  borné.
- COMPRESS=DEFLATE : non activé (ADR-002).
- Partie au-dessus du seuil : téléchargée à la demande par la connexion de
  commandes.

## Liste des conversations et boîte unifiée

Fiche : [conversations.md](../fonctionnel/conversations.md).

### Règles et code

| Règle | Où |
|---|---|
| Regroupement JWZ : `References`, puis `In-Reply-To`, puis sujet en dernier recours | `crates/domain/src/conversation.rs` (`thread`) |
| Version incrémentale (`assign`) : un message rejoint toute conversation dont il connaît un identifiant ; fusion si besoin ; converge vers JWZ quel que soit l'ordre d'arrivée (propriété testée) | `conversation.rs` |
| Regroupement par sujet seulement pour une **réponse** (préfixe `Re:`, `Fwd:`… ou références sans correspondance), dans une fenêtre de 31 jours | `conversation.rs` (`SUBJECT_WINDOW_SECS`) |
| Préfixes reconnus : `Re`, `RE`, `Fwd`, `Fw`, `TR`, `AW`, `WG`, `SV`, `VS`, `Antw`, `RIF`, `R`, `RE :`… (liste fixe) | `conversation.rs` (`strip_prefix`) |
| Une conversation n'est jamais scindée quand un message qui la reliait disparaît | ADR-003, `crates/adapters/sqlite/src/conversations.rs` |
| Pages de 500 au plus ; 500 messages au plus par conversation affichée | `crates/domain/src/ports.rs` (`MAX_PAGE_SIZE`, `MAX_CONVERSATION_MESSAGES`) |
| Lecture paginée par curseur (CQRS léger, ADR-008) | ports `ConversationQuery`, `MessageListQuery` |

### Clés des messages

`list_empty`, `mailbox_unified_inbox`, `common_no_subject`, `common_unknown_sender`, `list_marked`.

### Commandes

`conversation.next`, `conversation.previous`, `conversation.open`, `conversation.back`, `mailbox.unified`.

### Vérifications et compléments

- Liste virtualisée : pages demandées au fil du défilement
  (`list_conversations`).
- Mesures (bench, 50 000 messages) : première page ≈ 0,3 ms ; changement
  de dossier de bout en bout 13 à 16 ms.

## Lecture sécurisée

Fiche : [lecture.md](../fonctionnel/lecture.md).

### Règles et code

| Règle | Où |
|---|---|
| Liens ouverts : schémas `http`, `https`, `mailto` uniquement, 2048 caractères au plus | `crates/domain/src/reading.rs` (`MAX_LINK_LEN`) |
| Images distantes : autorisation par adresse `From` (falsifiable : protège du pistage, pas de l'usurpation) | `reading.rs`, ADR-004 |
| Nom de fichier assaini : séparateurs, `..`, noms réservés Windows, caractères de contrôle et de sens d'écriture (`exe.pdf` déguisé), 200 octets au plus | `reading.rs` (`safe_file_name`) |
| Ouverture directe réservée à une liste blanche d'extensions (documents sans macros, images matricielles, texte, sons, vidéos) ; tout le reste s'enregistre seulement | `reading.rs` (`OPENABLE_EXTENSIONS`, `is_unsafe_to_open`) |
| Enregistrement : création exclusive dans le dossier choisi, jamais d'écrasement ni de lien suivi | `app/src-tauri/src/reading.rs`, ADR-004 |
| HTML assaini par liste blanche (`ammonia`), CSS assaini (`@import`, `url()` externes retirés), `cid:` → `mailpart://`, images distantes → espace réservé ou `remote-img://` | `crates/adapters/render/src/sanitize.rs`, `css.rs` |
| CSP du document : `default-src 'none'`, seul script autorisé : l'agent (empreinte SHA-256) ; cadre `sandbox="allow-scripts"` sans `allow-same-origin` | ADR-004, `app/src-tauri/src/protocols.rs` |
| Proxy d'images : sans cookies ni `Referer`, User-Agent neutre, taille et types bornés, adresses publiques seulement, autorisation revérifiée à chaque image | `app/src-tauri/src/remote_images.rs` |

### Clés des messages

`message_images_blocked`, `message_show_images`, `message_show_text`, `message_show_html`, `message_open`, `message_save`, `message_saved_as`, `err_link_refused`, `err_executable_attachment`, `message_unreadable`.

### Commandes

`reading.plain-text`.

### Vérifications et compléments

- Cadre isolé servi par le protocole `mailview://` ; corps lus sans poser
  `\Seen` (`BODY.PEEK`).
- Touches pressées dans le cadre relayées par l'agent (`postMessage`) au
  registre de commandes.
- Tests e2e de sécurité (WebKitGTK) : zéro requête vers un serveur canari,
  aucun script du mail exécuté, IPC inaccessible depuis `mailview://`,
  `javascript:` jamais ouvert (`app/e2e/specs/reading-security.e2e.mjs`).

## Actions sur les messages, hors ligne et annulation

Fiche : [actions.md](../fonctionnel/actions.md).

### Règles et code

| Règle | Où |
|---|---|
| « Supprimer » = déplacer vers la Corbeille ; **depuis la Corbeille**, suppression définitive | `crates/domain/src/actions.rs` (`resolve`) |
| « Archiver » = déplacer vers le dossier `\Archive`, sinon vers le dossier de repli choisi par l'utilisateur | `actions.rs` |
| Opération durable (`PendingOp`) enregistrée avec l'état local nécessaire à son annulation ; effet local immédiat (`local_effect`) | `actions.rs` |
| Fenêtre d'annulation : 5 s (`UNDO_WINDOW_SECS`) ; au-delà, « trop tard » | `actions.rs` |
| Conflits au rejeu (`decide`) : UIDVALIDITY changée, message disparu, dossier de destination supprimé → opération abandonnée, état local rétabli (`revert_effect`) ; la synchronisation suivante fait foi | `actions.rs` |
| Stratégie serveur : `MOVE` ; sinon `COPY` + `STORE \Deleted` + `UID EXPUNGE` (UIDPLUS) ; sinon `EXPUNGE` seulement si aucun autre message de la boîte n'est marqué `\Deleted` (`expunge_is_safe`) | `actions.rs` |
| Drapeaux posés avec `UNCHANGEDSINCE` (CONDSTORE) | `crates/adapters/imap/src/session.rs` |
| Bornes : 1 000 messages par action, 5 essais de rejeu, 10 000 opérations en attente par compte | `actions.rs` (`MAX_ACTION_TARGETS`, `MAX_REPLAY_ATTEMPTS`, `MAX_PENDING_OPS`) |
| Copie provisoire d'un message déplacé : visible dans la destination, retirée à l'annulation ou à l'abandon, remplacée sans trou ni doublon par le vrai message à la synchronisation, ignorée par la synchronisation et les actions | migration 5, `crates/adapters/sqlite/src/actions.rs` |

Cas d'usage : `PerformAction`, `UndoActions`, `ReplayActions`
(`crates/application/src/actions.rs`).

Accès à la souris (interface) :

| Règle | Où |
|---|---|
| Actions de chaque zone : barre du message ouvert et menu « Plus… » (`readerActions`), survol d'une ligne (`rowActions`), menu contextuel (`menuActions`), barre de sélection (`bulkActions`) ; chaque élément exécute une commande du registre | `app/src/lib/visible-actions.ts` |
| Quand la place manque, les boutons passent dans « Plus… » par priorité (`fitByPriority`) | `visible-actions.ts`, `app/src/components/ActionBar.svelte` |
| Libellés des bascules selon l'état de toutes les cibles (`subjectOf`) | `visible-actions.ts` |
| Case à cocher d'une ligne sans changer la sélection (`toggleMarkAt`), tout décocher (`clearMarks`) | `app/src/lib/mail.svelte.ts` |
| Glisser-déposer sur un dossier : événements pointeur (souris, stylet), pas le glisser-déposer HTML, que l'interception native des fichiers déposés rend peu fiable dans la webview ; seuil de 5 px (`DRAG_THRESHOLD`) ; conversations emportées : la saisie, ou toutes les cochées si elle l'est, d'un seul compte (`dragPayload`) ; dossiers proposés : même compte, sélectionnable, pas le seul dossier d'origine (`canDrop`, affichage seulement : Rust revalide le déplacement) ; Échap annule ; le clic qui suit un glisser n'ouvre pas la conversation (`suppressClick`) | `app/src/lib/drag-move.svelte.ts` |
| Dépôt = action `move` sur les conversations emportées, sans toucher à la sélection ni à la conversation ouverte (`act(action, done, ids)`) | `app/src/lib/mail.svelte.ts`, `app/src/App.svelte` |
| Cibles du dépôt : attributs `data-drop-folder` / `data-drop-account` des dossiers, trouvés sous le pointeur (`elementFromPoint`) | `app/src/components/Sidebar.svelte`, `App.svelte` |

### Clés des messages

`action_archived`, `action_deleted`, `action_moved`, `action_flagged`, `action_unflagged`, `action_marked_read`, `action_marked_unread`, `action_undo`, `action_undone`, `action_too_late`, `action_nothing`, `err_no_trash`, `err_no_archive`, `err_pending_queue_full`, `action_archive_folder_set`, `action_more`, `action_follow`, `action_unfollow`, `list_marked`, `list_select_row`, `list_selection`, `list_bulk_actions`, `cmd_clear_selection`, `drag_conversations`.

### Commandes

`action.archive`, `action.delete`, `action.flag`, `action.mark-read`, `action.mark-unread`, `action.move`, `action.toggle-mark`, `selection.clear`, `action.undo`, `account.archive-folder`.

### Vérifications et compléments

- Suppressions faites par un autre client pendant un `EXPUNGE` sans
  UIDPLUS : comptées (`ReplayReport::foreign_expunges`), la boîte est
  resynchronisée.

## Dossiers : création, renommage, déplacement, suppression

Fiche : [dossiers.md](../fonctionnel/dossiers.md).

### Règles et code

| Règle | Où |
|---|---|
| Nom saisi : espaces de bord retirés, non vide, 100 caractères au plus, ni contrôle, ni caractère de mise en forme, ni `*` / `%` (`folder_label`, `MAX_FOLDER_LABEL_CHARS`) | `crates/domain/src/mailbox.rs` |
| Nom complet (`new_folder_name`) : parent + séparateur du parent ; au premier niveau, préfixe `INBOX<séparateur>` si tous les autres dossiers sont sous INBOX (Courier, Cyrus) ; séparateur interdit dans le nom ; doublon refusé à la casse près ; parent inconnu, sans hiérarchie ou virtuel (`\All`, `\Flagged`) refusé | `mailbox.rs` |
| Décision prise sur la liste relue par `LIST` juste avant `CREATE`, pas sur la copie locale | `crates/application/src/folders.rs` (`CreateFolder`) |
| `CREATE` sur la connexion de commandes de l'acteur ; relecture de la liste (`sync_folders`) puis `FoldersChanged` ; un dossier créé reste annoncé comme créé si la relecture échoue (reconnexion ensuite) ; une demande abandonnée par l'interface (délai dépassé) avant d'être servie n'est pas envoyée | `crates/application/src/account_actor.rs` |
| Nom envoyé en UTF-7 modifié ; refus `NO` → `ConnectionError::Rejected` | `crates/adapters/imap/src/session.rs` (`create`) |
| Commande IPC : compte et parent revalidés (le parent doit être un dossier du compte), nom vérifié seul avant l'envoi, 30 s au plus (`CREATE_FOLDER_TIMEOUT`), erreurs traduites en codes | `app/src-tauri/src/folders.rs`, `app/src-tauri/src/mail.rs` |
| Emplacements proposés : premier niveau puis dossiers du compte, sans les dossiers virtuels `\All` et `\Flagged` (`parentChoices`, affichage seulement : la règle est dans le domaine) | `app/src/lib/new-folder.svelte.ts` |
| Nouveau nom (`renamed_folder_name`) : même parent, mêmes règles qu'un nouveau dossier ; seul le dossier lui-même peut changer de casse ; nom inchangé : rien n'est envoyé | `mailbox.rs`, `folders.rs` (`RenameFolder`) |
| Déplacement (`moved_folder_name`) : même nom sous le nouveau parent, emplacement calculé comme pour un nouveau dossier (`destination` : parent connu, non virtuel, avec hiérarchie ; préfixe `INBOX<séparateur>` au premier niveau s'il y a lieu) ; ni dans lui-même ni dans un sous-dossier (`IntoItself`) ; doublon à l'arrivée refusé à la casse près ; déjà à cet endroit : rien n'est envoyé ; un seul `RENAME`, les sous-dossiers suivent | `mailbox.rs`, `folders.rs` (`MoveFolder`) |
| Dossiers protégés (`FolderChangeError::Protected`) : INBOX et tout rôle autre que `Normal` (SPECIAL-USE ou nom reconnu, dossiers virtuels) ; dossier avec sous-dossiers non supprimable (`deletable_folder`, `HasSubfolders`) | `mailbox.rs` |
| Renommage, déplacement et suppression décidés sur la liste relue par `LIST` juste avant `RENAME` / `DELETE` | `folders.rs` (`RenameFolder`, `MoveFolder`, `DeleteFolder`) |
| Une seule commande d'acteur (`FolderChange`) pour créer, renommer, déplacer, supprimer ; dossier renommé ou déplacé et ses sous-dossiers resynchronisés aussitôt (le cache local de l'ancien nom est oublié avec la liste, `replace_mailboxes`) | `account_actor.rs` |
| Boîte sélectionnée quittée avant `RENAME` / `DELETE` (`UNSELECT`, sinon `EXAMINE INBOX`, jamais `CLOSE`) : Dovecot coupe la session si la boîte sélectionnée disparaît | `crates/adapters/imap/src/lib.rs` (`leave_selected`) |
| Ligne d'état (`OK`/`NO`/`BAD`/`BYE`/`PREAUTH`) sans littéral : octets non ASCII remplacés par `?` avant l'analyse (Dovecot cite le nom de la boîte en UTF-8 dans un refus) | `crates/adapters/imap/src/connection.rs` (`ascii_status_text`) |
| Commandes IPC `rename_folder`, `move_folder`, `delete_folder` : compte, dossier et destination revalidés (dossiers du compte), nom vérifié seul avant l'envoi, 30 s au plus (`FOLDER_CHANGE_TIMEOUT`), code d'échec propre à l'opération | `app/src-tauri/src/folders.rs`, `app/src-tauri/src/mail.rs` |
| Confirmation de suppression sans nombre de messages ni « vide » : le compte local peut être incomplet (dossier pas encore synchronisé) | `app/src/components/DeleteFolderDialog.svelte` |
| Emplacements proposés au déplacement : ceux d'un nouveau dossier, sans le dossier ni ses sous-dossiers ; emplacement actuel présélectionné, **Déplacer** grisé tant qu'il est inchangé ; hiérarchie lue dans les noms, la liste suivant l'ordre des rôles (`destinationChoices`, `MoveFolderState`, affichage seulement : la règle est dans le domaine) | `app/src/lib/folder-edit.svelte.ts` |
| Menu « ⋯ » proposé sur les seuls dossiers ordinaires (`isEditableFolder`, affichage seulement : la règle est dans le domaine) ; cible des commandes : dossier du menu, sinon dossier affiché (`AppContext.folder`) | `app/src/lib/folder-edit.svelte.ts`, `app/src/lib/app.svelte.ts` |

Port : `RemoteMailboxSession::create_mailbox`, `rename_mailbox`,
`delete_mailbox` ; contrat
`folders_session_contract` (`crates/test-support/src/session_contract.rs`),
(dont le déplacement sous un autre parent, sous-dossier et message
compris), joué contre le faux serveur et contre Dovecot
(`crates/adapters/imap/tests/integration_sync.rs`).

Accès à la souris : menu « ⋯ » d'un compte (`Sidebar.svelte`), fenêtre
`NewFolderDialog.svelte` ; menu « ⋯ » d'un dossier (`Sidebar.svelte`, au
survol, au focus et sur le dossier affiché), fenêtres
`RenameFolderDialog.svelte`, `MoveFolderDialog.svelte` et
`DeleteFolderDialog.svelte`.

### Clés des messages

`cmd_new_folder`, `cmd_new_folder_keywords`, `new_folder_title`, `new_folder_name`, `new_folder_parent`, `new_folder_top_level`, `new_folder_create`, `new_folder_creating`, `action_folder_created`, `err_folder_name_empty`, `err_folder_name_too_long`, `err_folder_name_invalid`, `err_folder_name_delimiter`, `err_folder_exists`, `err_no_subfolders`, `err_folder_create_failed`, `cmd_rename_folder`, `cmd_rename_folder_keywords`, `cmd_delete_folder`, `cmd_delete_folder_keywords`, `sidebar_folder_menu`, `rename_folder_title`, `rename_folder_name`, `rename_folder_confirm`, `rename_folder_busy`, `action_folder_renamed`, `delete_folder_title`, `delete_folder_explanation`, `delete_folder_irreversible`, `delete_folder_confirm`, `delete_folder_busy`, `action_folder_deleted`, `err_folder_protected`, `err_folder_has_subfolders`, `err_folder_rename_failed`, `err_folder_delete_failed`, `cmd_move_folder`, `cmd_move_folder_keywords`, `move_folder_title`, `move_folder_explanation`, `move_folder_parent`, `move_folder_confirm`, `move_folder_busy`, `action_folder_moved`, `err_folder_into_itself`, `err_folder_move_failed`.

### Commandes

`account.new-folder`, `folder.rename`, `folder.move`, `folder.delete`.

## Rédaction : nouveau message, réponse, transfert

Fiche : [redaction.md](../fonctionnel/redaction.md).

### Règles et code

| Règle | Où |
|---|---|
| Destinataires de « répondre » : `Reply-To` prioritaire, sinon `From` ; « répondre à tous » ajoute À et Cc ; ses propres adresses exclues, doublons retirés, adresses invalides ignorées ; répondre à son propre message vise ses destinataires | `crates/domain/src/compose.rs` (`reply_recipients`) |
| Sujet : toujours `Re:` / `Fwd:`, sans empilement (préfixes étrangers reconnus puis retirés : `RE :`, `AW:`, `TR:`, `SV:`, `Antw:`…) | `compose.rs` (`reply_subject`, `forward_subject`), ADR-011 |
| `In-Reply-To` et `References` (bornées à 20 : racine et plus récents) | `compose.rs` (`reply_threading`, `MAX_REFERENCES`) |
| Citation : en-tête « Le … a écrit : » construit par l'interface dans la langue de l'utilisateur, inséré tel quel par le domaine | `compose.rs` (`quote`), ADR-011 |
| Validation avant envoi : au moins un destinataire, 100 au plus ; sujet sans caractère de contrôle, 998 caractères au plus ; corps ≤ 5 Mio ; pièces jointes ≤ 25 Mio au total et 100 au plus | `crates/domain/src/compose/draft.rs` (`check_sendable`) |
| HTML de l'éditeur réassaini en Rust (liste blanche minimale : p, br, gras, italique, barré, liens http/https/mailto, listes, citations) ; version texte dérivée du HTML assaini ; envoi en `multipart/alternative` | `crates/adapters/render/src/compose.rs`, ADR-010 |
| `Bcc` seulement dans les copies locales, jamais dans les octets transmis ; `Message-ID` aléatoire de 128 bits, sans nom de machine | `render/src/compose.rs` |
| Type MIME des pièces jointes recalculé en Rust d'après le nom ; taille réelle revérifiée ; chemins des fichiers jamais transmis par l'interface (jeton de dépôt) | `crates/application/src/compose.rs` (`QueueMessage`) |
| Transfert : pièces relues depuis le stockage local, message entier téléchargé si le corps est partiel ; mêmes bornes que l'envoi | `compose.rs` (`PrepareReply`), port `RemoteBodyFetcher` |

### Clés des messages

`compose_title_new`, `compose_title_reply`, `compose_title_forward`, `compose_attribution`, `compose_forward_intro`, `err_no_recipient`, `err_too_many_recipients`, `err_invalid_subject`, `err_body_too_large`, `err_attachments_too_large`, `err_too_many_attachments`, `compose_queued`.

### Commandes

`compose.new`, `compose.reply`, `compose.reply-all`, `compose.forward`.

### Vérifications et compléments

- Éditeur : Tiptap (ADR-010) ; sélecteur de fichiers ouvert depuis Rust.
- HTML saisi littéralement dans l'éditeur : envoyé comme du texte (test
  e2e `compose.e2e.mjs`).
- Brouillon relu depuis la base : revalidé (`check_storable`).

## Brouillons

Fiche : [brouillons.md](../fonctionnel/brouillons.md).

### Règles et code

| Règle | Où |
|---|---|
| Chaque enregistrement local incrémente la révision (`DraftSync::edited`) | `crates/domain/src/compose/draft.rs` |
| Copie sur le serveur seulement si la révision a changé, **au plus toutes les 30 s** (`REMOTE_SAVE_INTERVAL_SECS`) ; elle remplace l'ancienne | `draft.rs` (`remote_save_due`) |
| Sans UIDPLUS, les brouillons restent locaux (pas de remplacement fiable) | `crates/adapters/imap/src/session.rs` |
| Une ancienne copie n'est supprimée que si la boîte a la même UIDVALIDITY et que le message porte `\Draft` | ADR-002 |
| Copie refusée par le serveur (`NO`) ou impossible à construire : journalisée, nouvel essai une heure plus tard ; les brouillons suivants continuent | `draft.rs` (`remote_refused`, `REMOTE_REFUSED_RETRY_SECS`) |
| Brouillon d'un autre client repris avec son fil (`resumed_threading`) | `crates/domain/src/compose.rs`, `crates/application/src/compose.rs` (`PrepareReply::resume`) |

### Clés des messages

`drafts_title`, `drafts_empty`, `drafts_to`, `drafts_server`, `compose_discard`.

### Commandes

`drafts.open`.

### Vérifications et compléments

- Dossier « Brouillons » disparu ou refusé : la boîte est enregistrée avec
  chaque copie (migration 8), les copies en attente de suppression sont
  oubliées.

## Envoi : file d'envoi, nouvelles tentatives, envoi différé

Fiche : [envoi.md](../fonctionnel/envoi.md).

### Règles et code

États (`crates/domain/src/compose/outbox.rs`) :

```text
Queued ──start──▶ Sending ──succeeded──▶ Sent
  ▲                  │
  └──failed (temporaire, injoignable)
                     └──failed (refus, authentification, trop d'essais)──▶ Failed
```

| Règle | Où |
|---|---|
| Serveur injoignable : essai **non compté**, envoi différé | `outbox.rs` |
| Échec temporaire (SMTP 4xx, coupure) : nouvel essai avec attente croissante (30 s à 1 h), **8 essais** au plus | `outbox.rs` (`MAX_SEND_ATTEMPTS`, `RETRY_BASE_SECS`, `RETRY_MAX_SECS`) |
| Refus définitif (5xx), authentification, sécurité : échec immédiat | `outbox.rs` |
| Envoi interrompu par l'arrêt de l'application (`Sending` au démarrage) ou transmission interrompue sans réponse : **incertain**, jamais renvoyé automatiquement ; l'utilisateur décide | `outbox.rs` (`SendFailure::Uncertain`), `crates/application/src/compose_worker.rs` (`RecoverOutbox`) |
| Envoi différé : un an au plus ; brouillon gardé avec le message ; repris d'un bloc seulement s'il est encore en attente | `outbox.rs` (`MAX_SCHEDULE_SECS`), migration 10 |
| Annuler : seulement si le message n'est pas en cours d'envoi | `app/src-tauri/src/compose.rs` (`cancel_send`) |
| Connexion SMTP : TLS implicite (465) ou STARTTLS exigé (587), TLS négocié par `norelly-tls` | ADR-002, `crates/adapters/smtp` |
| Copie dans Envoyés : APPEND, sauf serveurs qui la font eux-mêmes (Gmail, `X-GM-EXT-1`) | `crates/application/src/compose.rs` (`FileSentCopies`) |

### Clés des messages

`outbox_title`, `outbox_empty`, `outbox_state_queued`, `outbox_state_sending`, `outbox_state_sent`, `outbox_state_scheduled`, `compose_send_failed`, `send_failure_*`, `outbox_retry`, `outbox_cancel`, `outbox_edit`, `compose_later`, `compose_schedule`, `compose_scheduled`, `err_schedule_too_far`, `err_outbox_busy`.

### Commandes

`compose.outbox`.

## Recherche plein texte

Fiche : [recherche.md](../fonctionnel/recherche.md).

### Règles et code

Syntaxe (`crates/domain/src/search.rs`, `parse_query`) :

| Syntaxe | Effet |
|---|---|
| mots libres | cherchés partout (sujet, expéditeur, destinataires, texte), **en préfixe** à partir de deux lettres (`fact` trouve « facture ») |
| `"phrase exacte"` | expression ; guillemet final facultatif |
| `from:`, `to:` (À et Cc), `subject:` | sur un champ, suivis d'un mot ou d'une phrase |
| `has:attachment`, `is:unread`, `is:read`, `is:flagged` | filtres |
| `before:AAAA-MM-JJ`, `after:AAAA-MM-JJ` | `before` exclut le jour, `after` l'inclut ; début du jour selon le fuseau de l'utilisateur |
| `-clause` | exclusion |

| Règle | Où |
|---|---|
| Clauses combinées par « et » ; opérateur inconnu ou valeur invalide (`is:bleu`, `before:hier`) cherchés comme du texte : une faute de frappe ne fait jamais échouer | `search.rs` |
| Requête bornée : 512 caractères, 16 clauses | `search.rs` (`MAX_QUERY_CHARS`, `MAX_CLAUSES`) |
| Casse et diacritiques ignorés (pliage aligné sur `unicode61 remove_diacritics 2`) ; surlignage en segments de texte, jamais en HTML | `search.rs` |
| Chaque clause devient une chaîne FTS5 entre guillemets, en paramètre lié : aucun mot de l'utilisateur n'est lu comme un opérateur | `crates/adapters/sqlite/src/search.rs` |
| Classement BM25 pondéré (sujet 10, expéditeur 5, destinataires 3, texte 1) ; 1 000 résultats au plus ; doublons de `\All` (Gmail) écartés | `search.rs` (sqlite), ADR-003 |
| Texte indexé : partie texte, sinon HTML réassaini réduit au texte, pièces jointes exclues, 100 000 caractères au plus | `crates/adapters/render/src/search_text.rs` |
| Une recherche est interrompue au-delà de 2 s | `crates/adapters/sqlite/src/search.rs` |

### Clés des messages

`search_placeholder`, `search_none`, `search_close`.

### Commandes

`search.open`, `conversation.next`, `conversation.previous`, `conversation.open`, `conversation.back`.

### Vérifications et compléments

- Requêtes hostiles (`OR`, `NEAR`, `col:`, `^`, `*`, parenthèses…) :
  cherchées comme du texte (tests de contrat).
- Requête trop longue : `err_invalid_input`, champ `field_query`.
- Mesure (bench, 50 000 messages de 300 mots) : 0,3 à 89 ms selon la
  requête, seuil 100 ms.

## Notifications des nouveaux messages

Fiche : [notifications.md](../fonctionnel/notifications.md).

### Règles et code

| Règle | Où |
|---|---|
| Seuls les messages **non lus** comptent (un message lu ailleurs n'est pas signalé) | `crates/domain/src/notify.rs` |
| Rien à la **première synchronisation** d'une boîte | `notify.rs` |
| Arrivées d'une synchronisation **regroupées** : un nombre et, au plus, le message le plus récent | `notify.rs` |
| Confidentialité : contenu retiré **en Rust** (`NewMail::without_content`) avant d'atteindre l'interface | `notify.rs`, `app/src-tauri/src/settings.rs` (`new_mail_event`) |
| Texte composé par l'interface dans la langue de l'utilisateur, revalidé par la commande `show_notification` (balisage freedesktop neutralisé) | ADR-011, `app/src-tauri/src/settings.rs` |
| Affichage par `notify-rust` (sans zbus) depuis Rust seulement | dérogation à consigner ([BACKLOG](../BACKLOG.md)) |

### Clés des messages

`notify_new_one`, `notify_new_many`, `notify_latest`, `settings_notifications`, `settings_privacy`.

### Vérifications et compléments

- Balisage dans le sujet ou l'expéditeur : neutralisé.
- Affichage réel vérifié à la main (`docs/MANUAL-TESTS.md`).

## Paramètres, langue et mises à jour

Fiche : [parametres.md](../fonctionnel/parametres.md).

### Règles et code

| Règle | Où |
|---|---|
| Recherche de mises à jour : demandée au premier lancement, « oui » présélectionné (`UpdateCheck::PROPOSED`), modifiable | `crates/domain/src/settings.rs`, ADR-006 |
| Vérification au plus une fois par jour, sans identifiant envoyé ; mise à jour signée (minisign) vérifiée avant installation | `settings.rs` (`UPDATE_CHECK_INTERVAL_SECS`), `app/src-tauri/src/updates.rs` |
| Module de mise à jour compilé seulement avec la fonctionnalité `updater` (NSIS) | ADR-006, `app/src-tauri/Cargo.toml` |
| Seuil de rapatriement entre 64 Kio et 64 Mio ; valeur illisible ou hors bornes remplacée par la valeur par défaut | `settings.rs`, port `SettingsStore` |
| Langue : choix de la locale parmi les préférences du système, anglais en repli ; dates, heures, tailles par `Intl` avec la locale active | `app/src/lib/i18n.ts`, ADR-011 |

### Clés des messages

`settings_title`, `settings_restart_note`, `update_question`, `update_available`, `update_install`, `update_later`, `err_update_failed`.

### Commandes

`settings.open`.

## Commandes : souris, clavier, palette, accessibilité

Fiche : [clavier.md](../fonctionnel/clavier.md).

### Règles et code

| Règle | Où |
|---|---|
| Un identifiant ou un raccourci déjà pris est refusé à l'enregistrement : un raccourci ne déclenche jamais deux commandes (`Ctrl+K` et `mod+k` sont le même) | `app/src/lib/commands/registry.ts` |
| `Maj` ignoré pour les caractères qui en dépendent (`?`), pas pour les lettres (`Maj+I` ≠ `i`) | `app/src/lib/commands/shortcut.ts` |
| Le registre ne prend aucune décision de sécurité : une commande appelle l'interface ou une commande IPC revalidée en Rust | `registry.ts` |
| Messages `postMessage` de l'agent validés (source, schéma, taille) | `app/src/lib/agent.ts`, ADR-004 |
| Chaque commande figure dans l'aide-mémoire de la documentation utilisateur, dans les deux langues | `app/src/docs.test.ts` (via `scripts/check-docs.sh`) |
| Infobulle « Libellé — Touches » et état d'un élément visible, tirés du registre | `app/src/lib/commands/describe.ts` |
| Les composants relisent le registre à l'abonnement (commandes enregistrées avant) | `app/src/lib/commands/watch.svelte.ts` |
| Touches de la fenêtre de rédaction et de l'éditeur, affichées dans les infobulles et l'aide | `app/src/lib/fixed-shortcuts.ts` |
| Bouton Fermer de chaque fenêtre, dernier dans l'ordre de tabulation | `app/src/components/Modal.svelte` |
| Bouton « ? », « Nouveau message », menu « ⋯ » des comptes, « Ajouter un compte » et « Paramètres » toujours visibles | `app/src/components/Sidebar.svelte` |

### Clés des messages

`palette_placeholder`, `palette_empty`, `help_title`, `help_palette_hint`, `help_group_compose_window`, `help_send`, `help_close_compose`, `tooltip_shortcut`, `common_close`, `key_shift`, `key_escape`, `key_space`, `key_enter`, `sidebar_account_menu`, `sidebar_sync`, `sidebar_remove`, `sidebar_settings`, `search_button`.


### Commandes

| Groupe | Identifiants (raccourci) |
|---|---|
| Général | `palette.open` (`Ctrl+K`), `help.shortcuts` (`?`), `settings.open` (`Ctrl+,`) |
| Lecture | `conversation.next` (`j`), `conversation.previous` (`k`), `conversation.open` (`Entrée`), `conversation.back` (`Échap`), `search.open` (`/`), `mailbox.unified`, `reading.plain-text` |
| Actions | `action.archive` (`e`), `action.delete` (`#`), `action.flag` (`s`), `action.mark-read` (`Maj+I`), `action.mark-unread` (`Maj+U`), `action.move` (`v`), `action.toggle-mark` (`x`), `selection.clear`, `action.undo` (`z`) |
| Rédaction | `compose.new` (`c`), `compose.reply` (`r`), `compose.reply-all` (`a`), `compose.forward` (`f`), `drafts.open`, `compose.outbox` |
| Comptes | `account.add`, `account.new-folder`, `folder.rename`, `folder.move`, `folder.delete`, `account.archive-folder`, `account.sync`, `account.remove` |

Accessibilité : listes en `listbox`/`option` avec `aria-activedescendant`.

## Vocabulaire : termes fonctionnels et noms dans le code

| Terme fonctionnel | Dans le code |
|---|---|
| Action | `MessageAction`, `PendingOp` |
| Boîte, dossier | `Mailbox` (rôle : attributs SPECIAL-USE, sinon nom) |
| Brouillon | `Draft`, `DraftSync` |
| Compte | `Account` |
| Conversation | algorithme JWZ (`conversation.rs`) |
| Empreinte de certificat | `CertificateFingerprint` |
| Envoi incertain | `SendFailure::Uncertain` |
| File d'actions | table `pending_ops` |
| File d'envoi | `outbox` |
| Rapatriement | `fetch_policy` |
| Suivre | drapeau IMAP `\Flagged` |
| Trousseau | port `SecretStore` |

Termes sans équivalent fonctionnel :

- **Agent** : seul script autorisé dans le cadre d'un message
  (`crates/adapters/render/src/agent.js`) ; il transmet la hauteur, les
  clics sur les liens et les touches à l'interface par `postMessage`.
- **Blob** : message brut (`.eml`) compressé (`.zst`), stocké hors de la
  base, par compte et par empreinte de contenu.
- **Port** : trait du domaine implémenté par un adaptateur (voir
  [l'architecture](architecture.md)).
- **UID, UIDVALIDITY, MODSEQ** : identifiants IMAP d'un message dans une
  boîte, de la « génération » de la boîte et de la version de ses drapeaux.
  Un changement d'UIDVALIDITY oblige à tout resynchroniser.
