# Modèle de données et protocoles personnalisés

[← Documentation technique](README.md)

## Emplacements

Dossier de données de l'application (`app_data_dir` de Tauri, identifiant
`io.github.gbourquet.Norelly`) :

| Chemin | Contenu |
|---|---|
| `norelly.sqlite3` (+ `-wal`, `-shm`) | Base SQLite unique, tous comptes (ADR-003) |
| `accounts/<compte>/…` | Messages bruts `.eml.zst` par empreinte de contenu (`BlobStore`) et données locales du compte |
| `mots-de-passe.json` | Seulement en dérogation (aucun trousseau, consentement) : secrets en clair, droits 0600, écriture atomique |

Dossier de cache : `pieces-jointes/` (pièces ouvertes, dossier privé).
Aucun secret n'est en base : les mots de passe sont dans le trousseau du
système.

## Base SQLite

- `rusqlite` avec SQLite embarqué, mode **WAL**, **un seul rédacteur** et un
  pool de **4 lecteurs** (`crates/adapters/sqlite/src/lib.rs`).
- Migrations : `rusqlite_migration`, fichiers
  `crates/adapters/sqlite/migrations/NNNN_nom.sql`, listés dans
  `migrations()`. **Une migration publiée n'est jamais modifiée** : on en
  ajoute une.
- Test `upgrade_from_every_previous_version_keeps_data` : montée de version
  depuis chaque version précédente, données conservées.
- Requêtes : **paramètres liés uniquement** ; une constante interpolée porte
  le commentaire `sql-const` (`scripts/check-sql.sh`).
- `integrity_problems` (commande `integrity-check`) vérifie aussi l'index de
  recherche.

### Migrations

| Version | Fichier | Ajoute |
|---|---|---|
| 1 | `0001_accounts.sql` | `accounts` (adresse, nom, serveurs), `certificate_exceptions` (hôte, empreinte) |
| 2 | `0002_mail.sql` | `mailboxes` (rôle, état de synchronisation : UIDVALIDITY, HIGHESTMODSEQ, UIDNEXT), `messages` (UID, drapeaux, métadonnées, index `(dossier, date DESC)`), `message_parts` |
| 3 | `0003_conversations.sql` | `conversations`, `conversation_links` (identifiants connus), `conversation_mailboxes` (agrégats par dossier, index par dossier et par rôle pour la boîte unifiée), déclencheurs ; `remote_image_senders` (expéditeurs autorisés), `settings` |
| 4 | `0004_actions.sql` | `messages.hidden` (application optimiste), `pending_ops` (file durable), `action_settings` (dossier d'archives de repli) ; agrégats ignorant les messages masqués ou `\Deleted` |
| 5 | `0005_placeholder_copies.sql` | `messages.pending_op` : copies provisoires des messages déplacés (UID synthétique en haut de l'espace, ignorées par la synchronisation) |
| 6 | `0006_compose.sql` | `drafts` (document JSON revalidé, révision, copie distante), `draft_remote_trash`, `outbox` (`raw` sans Bcc, `sent_copy` avec Bcc, état) |
| 7 | `0007_search.sql` | `message_texts` (sujet, expéditeur, destinataires, texte) et index FTS5 à contenu externe `message_search` (`unicode61 remove_diacritics 2`, préfixes 2 et 3), tenus par déclencheurs ; indexation des messages existants |
| 8 | `0008_remote_copy_mailbox.sql` | boîte enregistrée avec chaque copie distante de brouillon |
| 9 | `0009_folder_counts.sql` | index couvrant `(mailbox_id, hidden, flags)` pour les compteurs de l'arbre |
| 10 | `0010_scheduled_send.sql` | envoi différé : échéance et contenu du brouillon dans `outbox` |

Le schéma courant complet est la somme de ces fichiers ; ils sont commentés
en tête et à chaque table.

### Invariants tenus par la base

- Conversations : le rattachement est décidé en Rust (domaine) à
  l'insertion ; les agrégats par dossier sont tenus par des **déclencheurs**,
  quel que soit le chemin d'écriture (synchronisation, cascade).
- Index de recherche : mis à jour **dans la même transaction** que
  l'écriture du message (déclencheurs) ; les messages ne sont jamais
  remplacés par `INSERT OR REPLACE`.
- Suppression d'un compte : cascade sur toutes ses données.

### Lectures d'écran (CQRS léger)

Les écritures passent par le domaine ; les lectures d'écran passent par des
ports de requête taillés pour l'écran, paginés par curseur
(`FolderTreeQuery`, `ConversationQuery`, `MessageListQuery`, `SearchIndex`,
`ContactQuery`), implémentés en SQL dans `crates/adapters/sqlite/src/`.
Bancs : `crates/adapters/sqlite/benches/` (voir [tests](tests.md)).

## Stockage des messages bruts

`crates/adapters/blobs` : un fichier `.eml.zst` par message, classé par
compte et par empreinte de contenu ; écriture atomique (fichier temporaire
puis renommage). Les parties différées (pièces au-dessus du seuil) sont
téléchargées à la demande, avec leurs en-têtes MIME (`BODY.PEEK[n.MIME]`).

## Protocoles personnalisés

Déclarés dans `app/src-tauri/src/protocols.rs` ; utilisables par la seule
fenêtre principale ; réponses `no-store`, `nosniff`, `no-referrer`.

| Protocole | Adresse | Sert | Contrôles |
|---|---|---|---|
| `mailview://` | `mailview://localhost/<message>` | Document HTML assaini du message | CSP en en-tête : `default-src 'none'`, `script-src 'sha256-<agent>'`, `style-src 'unsafe-inline'`, `img-src mailpart: remote-img: data:`, `form-action`, `base-uri`, `connect-src`, `frame-src` à `'none'` |
| `mailpart://` | `mailpart://localhost/<message>/<partie>` | Image en ligne (`cid:`) | types d'image matriciels seulement ; CSP `default-src 'none'; sandbox` |
| `remote-img://` | `remote-img://localhost/<message>/<index>` | Image distante, via le proxy | index dans la liste du dernier rendu ; expéditeur autorisé revérifié ; adresses publiques seulement (DNS filtré, IP littérales et redirections vérifiées) ; sans cookies ni `Referer`, User-Agent neutre ; taille et types bornés |

Forme des adresses selon la plateforme (ADR-004) : `scheme://localhost/…`
sous Linux (WebKitGTK), `http://scheme.localhost/…` sous Windows
(WebView2) ; la CSP du document et celle de la fenêtre principale
(`frame-src`) couvrent les deux.

Le cadre est `<iframe sandbox="allow-scripts">` **sans** `allow-same-origin`
(origine opaque) : le message n'a accès ni à l'IPC ni au DOM de
l'application. Le seul script est l'agent
(`crates/adapters/render/src/agent.js`), dont l'empreinte SHA-256 est
calculée par `build.rs` et vérifiée par un test.

## Contrat IPC

Voir [architecture](architecture.md#contrat-ipc). Les types sont dans
`app/src-tauri/src/ipc.rs` ; les bindings générés dans
`app/src/lib/ipc/bindings.ts`.
