# Modèle de données et protocoles personnalisés

[← Documentation technique](README.md)

## Emplacements

Dossier de données de l'application (`app_data_dir` de Tauri, identifiant
`io.github.gbourquet.Norelly`) :

| Chemin | Contenu |
|---|---|
| `norelly.sqlite3` (+ `-wal`, `-shm`) | Base SQLite unique, tous comptes (ADR-003) |
| `accounts/<compte>/…` | Messages bruts `.eml.zst` par empreinte de contenu (`BlobStore`) et données locales du compte |
| `norelly.log` (+ `norelly.log.1`) | Journaux de l'application (niveau INFO au plus), 2 Mio par fichier, une seule archive, droits 0600 (`app/src-tauri/src/logs.rs`) ; aucun secret ni contenu de message, mais serveurs, noms de dossiers et parfois adresses |
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
  ajoute une. Seule dérogation (ADR-003, validée par le mainteneur) :
  l'étape Rust de la migration 18, corrigée pour ne plus bloquer
  l'ouverture sur une clé d'adresse en collision ; son fichier SQL est
  inchangé.
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
| 11 | `0011_contacts.sql` | carnet d'adresses unique (ADR-012) : `contacts` (champs vCard, favori, `sort_key` et `search_text` calculés par le domaine, index de tri), `contact_emails` (`address_key` unique dans le carnet, une seule préférée par contact), `contact_phones` ; cascade à la suppression d'un contact |
| 12 | `0012_contact_lists.sql` | listes de diffusion du carnet (ADR-012) : `contact_lists` (`name_key`, nom en minuscules calculé par le domaine, unique et ordonnant la liste ; celle d'une liste illisible dont le nom est repris est remplacée par une clé libre commençant par un caractère de contrôle (`\u{1}<id>`), jamais celle d'un nom, le reste de la ligne intact (la clé se recalcule depuis le nom) ; libération définitive, y compris pour une liste écartée par une règle plus stricte d'une autre version, journalisée par l'identifiant seul ; `bcc`), `contact_list_members` (contacts dans l'ordre de la liste, chacun une fois) ; cascade à la suppression d'une liste ou d'un contact |
| 13 | `0013_oauth_accounts.sql` | `accounts.oauth_provider` (`microsoft`, ou `NULL` pour un compte à mot de passe) : compte connecté par OAuth ; le jeton de rafraîchissement reste dans le trousseau, sous la clé IMAP du compte |
| 14 | `0014_oauth_google.sql` | `accounts.oauth_provider` accepte aussi `google` : la colonne est remplacée par une colonne de même nom à la nouvelle contrainte `CHECK` (ajout, recopie, `DROP COLUMN`, `RENAME COLUMN`), sans reconstruire `accounts` ni toucher à ses dépendances en cascade |
| 15 | `0015_outbox_interruptions.sql` | `outbox.failure` accepte aussi `encryption_interrupted` (connexion coupée ou bloquée avant le chiffrement, à plusieurs reprises) et `authentication_interrupted` (connexion coupée pendant l'authentification) : une contrainte de table cite la colonne, la table `outbox` est donc reconstruite (copie des lignes, numérotation `sqlite_sequence` conservée, index `outbox_by_account` recréé) ; aucune table ne la référence |
| 16 | `0016_outbox_unresponsive.sql` | `outbox.failure` accepte aussi `unresponsive` (serveur joint puis muet, ou connexion coupée, une fois le chiffrement en place) et `microsoft_smtp_disabled` (refus `535 5.7.139`) : table `outbox` reconstruite comme en version 15 (lignes, numérotation et index conservés) |
| 17 | `0017_oauth_subject.sql` | `accounts.oauth_subject` : identifiant stable du compte chez son fournisseur OAuth (`oid` de Microsoft), qui le reconnaît aux reconnexions même si son adresse principale change ; `NULL` pour un compte à mot de passe ou Google, ou un compte Microsoft qui n'en a pas encore (enregistré à sa première reconnexion réussie, jamais remplacé ensuite) ; `CHECK` : compte Microsoft seulement, 1 à 128 caractères ASCII visibles, comme le domaine ; ce n'est pas un secret |
| 18 | `0018_contacts_invisible.sql` | schéma inchangé ; étape Rust jouée dans la même transaction (`M::up_with_hook`, `clean_invisible_contacts`) : les caractères invisibles désormais refusés à la saisie sont retirés des textes des fiches existantes selon la règle du domaine (`clean_invisible`), qui sinon deviendraient illisibles (séparateur de ligne remplacé par une espace, ou un saut de ligne dans les notes) ; champ vidé : `NULL`, téléphone sans numéro supprimé ; une adresse qui en contient est supprimée, jamais transformée (ce serait une autre boîte ; les clés `address_key` restantes sont inchangées), la première restante devient préférée ; fiche sans adresse restante supprimée (et retirée de ses listes) ; `sort_key` et `search_text` recalculés ; une fiche invalide pour une autre raison, ou illisible (type ou encodage d'une colonne), est laissée telle quelle sans bloquer l'ouverture ; de même, comme en version 19, pour une fiche dont une adresse gardée n'a pas pour clé `address_key` stockée celle que calcule le domaine (clé modifiée à la main : sa réécriture pourrait heurter l'unicité des clés ; étape corrigée après publication, par dérogation, fichier SQL inchangé) ; journal : nombres de fiches réécrites, supprimées, illisibles et invalides seulement ; écritures en paramètres liés |
| 19 | `0019_contact_names.sql` | schéma inchangé ; étape Rust jouée dans la même transaction (`M::up_with_hook`, `drop_names_imitating_address`), sur le modèle de la version 18 : le nom affiché, le prénom, le nom ou le surnom d'une fiche qui imite une adresse, désormais refusé à la saisie, est retiré selon la règle du domaine (`clean_names_imitating_address`) ; `sort_key` et `search_text` recalculés ; une fiche qui contient aussi des caractères invisibles (laissée telle quelle en version 18, car validée avec ce nom) en est nettoyée du même coup, selon `clean_invisible` ; une fiche invalide pour une autre raison, illisible (type ou encodage d'une colonne, fiche marquée illisible à la relecture) ou dont une clé `address_key` stockée n'est pas celle de son adresse (sa réécriture heurterait l'unicité des clés), est laissée telle quelle sans bloquer l'ouverture ; journal : nombres seulement ; écritures en paramètres liés |
| 20 | `0020_sender_auth.sql` | `messages.sender_auth` (`pass`, `fail` ou `unknown`, `CHECK`) : verdict du domaine (`SenderAuth`) sur l'expéditeur, tiré à la réception de l'en-tête `Authentication-Results` posé par le serveur du compte ; recopié avec les copies provisoires d'un déplacement ; les messages déjà enregistrés valent `unknown` (sans pastille ni avertissement), sans rattrapage depuis les corps stockés |
| 21 | `0021_labels.sql` | étiquettes : `labels` (mot-clé par compte, couleur locale `#rrggbb` en minuscules par `CHECK`, `hidden` pour une étiquette supprimée de la liste sans être retirée des messages), `label_rewrites` (réécritures en attente d'un mot-clé sur tous les messages du serveur : renommage, ou retrait si `to_keyword` est `NULL` ; essais comptés), `mailboxes.permanent_flags` (`PERMANENTFLAGS` du dernier `SELECT` en écriture : `NULL` inconnu, `*` tout mot-clé, `=` suivi des mots-clés permis) ; `pending_ops` reconstruite pour les opérations de mot-clé (`flag = 'keyword'` et colonne `keyword`, exigée par `CHECK`), lignes, index et numérotation `sqlite_sequence` conservés (les copies provisoires désignent leur opération par son identifiant) |
| 22 | `0022_contact_dismissals.sql` | `contact_duplicate_dismissals` : paires de contacts passées dans la recherche des doublons (`low_id < high_id` par `CHECK`, clé primaire sur la paire), `batch` numéro du groupe passé (un appel de `dismiss`), `rowid` de la plus ancienne à la plus récente ; oubliées quand une fiche disparaît (clés étrangères `ON DELETE CASCADE`) ou change (déclencheur `contact_duplicate_dismissals_forget` sur `UPDATE` de `contacts`, que toute écriture d'une fiche réécrit : modification, import, fusion) ; une paire retenue de nouveau est réinsérée (`INSERT OR REPLACE`, la plus récente) ; au-delà de `MAX_DISMISSED_PAIRS` (10 000), les plus anciens groupes sont supprimés, chacun en entier (jamais un groupe en partie), dans la même transaction ; paire d'une fiche inconnue ignorée ; écritures en paramètres liés |
| 23 | `0023_contact_full_name_search.sql` | schéma inchangé ; étape Rust jouée dans la même transaction (`M::up_with_hook`, `index_full_names`) : `search_text` recalculé par le domaine pour y ajouter le nom complet composé « prénom nom » (« alice mar » trouve Alice Martin), sans quoi les fiches enregistrées seraient tenues pour corrompues à la relecture ; seule cette colonne est réécrite, et seulement pour une fiche dont `sort_key` et `search_text` stockés sont ceux que calculait le domaine avant (ou déjà ceux d'aujourd'hui), pour la fiche relue ou telle qu'enregistrée avec un nom imitant une adresse (`ContactDraft::full_name_reindex`) ; une fiche illisible (type ou encodage d'une colonne), invalide, aux clés de tri ou de recherche retouchées, ou dont une clé `address_key` stockée n'est pas celle de son adresse, est laissée telle quelle, ni réparée ni masquée (toujours écartée à la relecture), sans bloquer l'ouverture ; parcours des fiches par lots commun aux étapes 18, 19 et 23 (`for_each_stored_contact`) ; journal : nombres seulement ; écritures en paramètres liés |
| 24 | `0024_carddav.sql` | synchronisation CardDAV (ADR-014) : `contacts.modified_at`, date de la dernière écriture d'une fiche posée par déclencheurs (création, et modification de ses champs : toute écriture réécrit la ligne de `contacts`), fiches existantes datées de la migration ; `carddav_account` (compte unique : serveur, identifiant, carnet ; jamais le mot de passe), `carddav_links` (ressources du carnet distant : ETag, contact relié, date de suppression de son contact posée par déclencheur avant suppression puis `ON DELETE SET NULL`, fiche locale et fiche distante à la dernière synchronisation ; un contact n'est relié qu'à une ressource), `carddav_sync` (dernière synchronisation : date, bilan ou code d'échec) |

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
- Dossier renommé ou déplacé sur le serveur (`MailStore::rename_mailboxes`) :
  dans **une seule transaction**, la ligne de `mailboxes` et celles de ses
  sous-dossiers changent de `name` (identifiant, état de synchronisation et
  messages gardés ; nouveaux noms calculés par `MailboxName::relocated`),
  une ligne périmée qui portait déjà un des nouveaux noms est supprimée
  avec ses messages (les opérations en attente qui la visaient sont
  abandonnées, effet local inversé), et les noms des opérations en attente
  du compte (`pending_ops.mailbox`, `target`) suivent ; une ligne de
  `pending_ops` illisible est laissée telle quelle. Les boîtes renommées passent d'abord par un nom provisoire
  (caractère nul, impossible pour une vraie boîte) : aucun conflit
  d'unicité `(account_id, name)` pendant l'opération. La synchronisation
  suivante vérifie l'UIDVALIDITY au lieu de tout télécharger à nouveau.
- Écritures du rejeu sur `pending_ops` (`complete`, `revert`, `claim`, et
  la fusion de `record_all`) : filtrées par identifiant **et** par compte.
- Nom affiché d'un compte (`accounts.display_name`) : relu par
  `DisplayName::from_stored`, avec les mêmes règles qu'à la saisie sauf le
  refus des noms qui imitent une adresse. Un compte enregistré avant ce
  refus avec un nom contenant « @ », « < » ou « > » s'ouvre donc tel
  quel, sans migration ni réécriture ; ce nom reste écarté à l'envoi
  (`Recipient::new`), comme avant.
- Noms d'une fiche contact : relus par `ContactDraft::from_stored`, avec
  les mêmes règles qu'à la saisie, sauf qu'un nom qui imite une adresse est
  retiré au lieu d'être refusé. La migration 19 a retiré ces noms des
  fiches enregistrées ; la relecture tolérante garde lisibles les fiches
  qu'une règle élargie plus tard viendrait à viser. Pour une telle fiche,
  `sort_key` et `search_text` stockés doivent être ceux que le domaine
  calcule pour la fiche relue, ou pour la fiche telle qu'enregistrée, avec
  ce nom (`ContactDraft::keys_as_stored`) ; d'autres valeurs la rendent
  invalide (écartée seule), même avec un tel nom. Le curseur de la liste
  reprend après la clé de tri **stockée** de la dernière fiche, celle de
  l'ordre SQL : une fiche rangée sous son nom enregistré ne fait ni sauter
  ni répéter de fiches.
- Carnet d'adresses : une adresse (à la casse près) n'appartient qu'à un
  contact (`contact_emails.address_key UNIQUE`) ; ordre et filtre de la
  liste comparent `sort_key` et `search_text` octet par octet, sans le
  `lower()` de SQLite (ASCII seul) : SQLite et le faux en mémoire trient
  et filtrent à l'identique. Le carnet n'appartient à aucun compte : la
  suppression d'un compte n'y touche pas.
- Fiches relues : chaque fiche est revalidée par le domaine
  (`ContactDraft::new`, `ContactId::new`) ; clé d'adresse, `sort_key` et
  `search_text` stockées doivent être celles que calcule le domaine, et
  une valeur d'un type inattendu rend la fiche invalide elle aussi.
  Changer le calcul de `sort_key` ou de `search_text` exige donc une
  migration qui les recalcule. Un identifiant de contact tient sur
  32 bits (`MAX_CONTACT_ID`, ceux de l'interface) : la séquence épuisée,
  la création est refusée (transaction annulée), jamais un identifiant
  réattribué. Une fiche invalide est écartée de la liste, des fiches
  cherchées par adresse (noms du courrier, suggestions), de l'export et
  de la recherche des doublons, et journalisée une fois par processus
  (son identifiant seul). La pagination la dépasse (la reprise d'un lot
  relit la clé de tri en SQL, dans une transaction) : les pages restent
  pleines, mais une page lit au plus dix lots de lignes, au-delà la liste
  s'arrête. Lue seule (`get`) ou à la fusion, elle reste une erreur.
  Aucune migration : rien n'est réécrit, la fiche redevient visible si
  elle est réparée.
- Synchronisation CardDAV : la date de modification d'une fiche
  (`contacts.modified_at`) et la date de suppression d'un contact relié
  (`carddav_links.deleted_at`) sont posées par des **déclencheurs**, quel
  que soit le chemin d'écriture (saisie, import, fusion, synchronisation) :
  elles départagent un conflit (la dernière modification gagne). Changer le
  compte pour un autre carnet ou un autre serveur oublie les ressources
  connues et la dernière synchronisation ; retirer le compte les oublie
  aussi, contacts gardés.

### Lectures d'écran (CQRS léger)

Les écritures passent par le domaine ; les lectures d'écran passent par des
ports de requête taillés pour l'écran, paginés par curseur
(`FolderTreeQuery`, `ConversationQuery`, `MessageListQuery`, `SearchIndex`,
`ContactQuery`, `ContactDirectory`), implémentés en SQL dans `crates/adapters/sqlite/src/`.
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
