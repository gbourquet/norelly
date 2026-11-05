# Actions sur les messages, hors ligne et annulation

[← Fonctionnalités](README.md)

## But

Ranger vite (archiver, supprimer, déplacer, suivre, marquer lu ou non lu),
avec un effet visible immédiatement, même hors ligne, et pouvoir se
rattraper.

## Comportement attendu

- Les actions portent sur la **cible** : les conversations cochées (`x`),
  sinon la conversation ouverte, sinon celle pointée dans la liste. Une
  action sur une conversation vise tous ses messages du dossier affiché.
- Accès : raccourcis, palette, barre d'outils de la vue de lecture, boutons
  au survol d'une ligne, menu contextuel (clic droit) ; tous exécutent la
  même commande du registre.
- **Application optimiste** : l'effet est visible tout de suite (drapeau
  changé, conversation retirée de la liste, copie provisoire dans le
  dossier de destination), puis envoyé au serveur.
- Un bandeau confirme l'action, avec **Annuler** pendant **5 secondes**
  (aussi `z`). Passé ce délai, l'opération part au serveur.
- **Hors ligne**, les opérations s'accumulent dans une file durable et sont
  rejouées à la reconnexion, dans l'ordre.
- « Suivre » est une bascule : si toutes les cibles sont suivies, le suivi
  est retiré, sinon il est ajouté.

## Règles métier

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

## Cas limites

- Compte sans Corbeille : suppression refusée (`err_no_trash`).
- Compte sans `\Archive` ni dossier de repli : `err_no_archive`, qui invite à
  choisir le dossier (« Choisir le dossier d'archives… »).
- Déplacer vers le dossier courant : `err_same_folder` ; vers un dossier non
  sélectionnable : `err_folder_not_selectable`.
- Déplacer des conversations de plusieurs comptes : refusé
  (`action_one_account`), un dossier n'appartenant qu'à un compte.
- Suppressions faites par un autre client pendant un `EXPUNGE` sans
  UIDPLUS : comptées et signalées (`ReplayReport::foreign_expunges`), la
  boîte est resynchronisée.
- File pleine : `err_pending_queue_full`.

## Messages affichés

| Clé | Texte (fr) |
|---|---|
| `action_archived`, `action_deleted` | Archivé., Supprimé. |
| `action_moved` | Déplacé vers « {folder} ». |
| `action_flagged`, `action_unflagged` | Suivi activé., Suivi retiré. |
| `action_marked_read`, `action_marked_unread` | Marqué comme lu., Marqué comme non lu. |
| `action_undo`, `action_undone` | Annuler, Action annulée. |
| `action_too_late` | Trop tard pour annuler : l'action a déjà été envoyée au serveur. |
| `action_nothing` | Rien à changer. |
| `err_no_trash`, `err_no_archive` | Aucun dossier Corbeille sur ce compte., Aucun dossier d'archives sur ce compte : choisissez-en un … |
| `err_pending_queue_full` | Trop d'actions attendent le serveur sur ce compte : réessayez une fois reconnecté. |
| `action_archive_folder_set` | Dossier d'archives : « {folder} ». |

## Raccourcis

| Commande | Raccourci |
|---|---|
| `action.archive` (Archiver) | `e` |
| `action.delete` (Supprimer) | `#` |
| `action.flag` (Suivre (ou ne plus suivre)) | `s` |
| `action.mark-read` (Marquer comme lu) | `Maj+I` |
| `action.mark-unread` (Marquer comme non lu) | `Maj+U` |
| `action.move` (Déplacer vers…) | `v` |
| `action.toggle-mark` (Sélectionner la conversation) | `x` |
| `action.undo` (Annuler la dernière action) | `z` |
| `account.archive-folder` (Choisir le dossier d'archives…) | palette |
