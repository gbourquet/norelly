# Backlog

Sujets ouverts : décisions à prendre, fonctionnalités, corrections,
durcissement et tâches humaines. On retire un élément quand il est traité (lien vers la PR), ou
quand le mainteneur l'abandonne (avec la raison).

## À trancher par le mainteneur

- **Notifications par `notify-rust`** : dérogation à consigner dans
  [DECISIONS.md](DECISIONS.md). `tauri-plugin-notification` tire zbus 5,
  dont `zbus_names` et `zvariant` sont signalés non maintenus
  (RUSTSEC-2026-0370, -0371) et font échouer `cargo deny` ; remplacé par
  `notify-rust` sans zbus (fonctionnalité `d`, libdbus du système), appelé
  depuis Rust seulement.
- **Seuil du démarrage** (porté de 200 à 400 ms le 2026-11-04) : démarrage
  mesuré à 293-307 ms sous Xvfb, WebKitGTK sans accélération graphique ; les
  appels IPC prennent moins de 10 ms, le temps est passé dans la webview
  (chargement du script ≈ 65 ms, fil principal occupé ≈ 67 ms après le
  montage, ≈ 100 ms entre `list_conversations` et l'affichage). Pistes :
  profiler le premier rendu (Svelte, liste virtuelle, styles), mesurer sur
  une machine avec GPU, ou revoir ce que mesure le « démarrage ».
- **Captures des écrans d'installation** (installateur NSIS, SmartScreen,
  logithèques Linux) : ils appartiennent au système et ne sont pas produits
  par les e2e ; `docs/utilisateur/*/install.md` les décrit en texte.
  Captures manuelles, ou texte seul ?
- **Captures recopiées par les e2e** : `pnpm -C app e2e` réécrit les
  captures suivies de `docs/utilisateur/*/captures`, donc lancer les e2e
  modifie le dépôt. Ajouter une variable d'environnement pour ne pas les
  recopier ? (`app/e2e/run.sh`)

## Interface et fonctionnalités

- **Ouvrir un message ne le marque pas comme lu** : corps lus en
  `BODY.PEEK`, aucun marquage automatique ; la ligne reste en gras et le
  compteur ne baisse pas. Marquer lu à l'ouverture (éventuellement après un
  délai réglable), par une action du domaine. (`app/src/lib/mail.svelte.ts`,
  `app/src-tauri/src/reading.rs`)
- **Noms de dossiers non traduits** : « Drafts », « Sent », « Trash »
  viennent du serveur alors que la boîte de réception est traduite. Libellé
  traduit d'après le rôle SPECIAL-USE, nom du serveur en info-bulle.
  (`Sidebar.svelte`, `folderLabel`)
- **Étiquettes** : le domaine a un type `Keyword` (mots-clés IMAP), mais
  aucun cas d'usage, commande IPC ni interface ne permet d'étiqueter un
  message. À faire comme une fonctionnalité complète (domaine, IMAP
  `STORE` des mots-clés, port, IPC), puis l'ajouter au menu « Plus… » et
  au menu contextuel. (`crates/domain/src/mailbox.rs`, `visible-actions.ts`)
- **Cache local perdu au renommage ou au déplacement d'un dossier** : la
  liste relue oublie l'ancien nom avec ses messages, qui sont téléchargés à
  nouveau sous le nouveau nom (coûteux pour un gros dossier). Renommer les boîtes locales
  (et leurs sous-boîtes) dans le stockage, puis laisser la synchronisation
  vérifier l'UIDVALIDITY. (`MailStore::replace_mailboxes`,
  `crates/application/src/account_actor.rs`)
- **Noms de dossier particuliers** (revue de sécurité, info) : un nom
  commençant par `~` ou `#`, ou les segments `.` / `..`, sont envoyés tels
  quels ; certains serveurs (UW-IMAP) les interprètent (répertoire
  personnel, espaces de noms). Sans vecteur tiers (l'utilisateur agit sur
  son propre compte) ; les refuser dans `folder_label` en défense en
  profondeur, et pour le segment conservé d'un dossier déplacé (il vient du
  serveur et ne passe pas par `folder_label`). Idem si le serveur n'annonce aucun séparateur.
  (`crates/domain/src/mailbox.rs`)
- **Menu d'un dossier dans le panneau de gauche** : le « ⋯ » d'un dossier
  ordinaire propose renommer et supprimer ; manquent le clic droit, un menu
  sur les dossiers spéciaux (sans entrée aujourd'hui) et « Nouveau
  sous-dossier… », qui y présélectionnerait l'emplacement. (`Sidebar.svelte`)
- **Sous-dossiers des dossiers spéciaux mal placés dans le panneau de
  gauche** : la liste est triée par rôle puis par nom (`FolderTreeQuery::tree`),
  pas dans l'ordre de l'arborescence. Un dossier rangé dans la boîte de
  réception (ou dans la corbeille) s'affiche, indenté, après le dernier
  dossier spécial : on le croirait dans la corbeille. Plus visible depuis
  qu'on peut déplacer un dossier dans la boîte de réception. Trier en
  arbre (chaque dossier suivi de ses sous-dossiers, rôles spéciaux d'abord
  au premier niveau). (`crates/adapters/sqlite/src/mail.rs`, `Sidebar.svelte`)
- **« Nouveau dossier… » depuis « Déplacer vers… »** : le sélecteur ne
  propose que les dossiers existants ; créer puis déplacer en un geste.
  (`FolderPicker.svelte`)
- **Glisser-déposer, suites** : seules les lignes de la liste des
  conversations et les dossiers du panneau de gauche se glissent (pas les
  résultats de recherche ni le message ouvert) ; pas de défilement automatique du panneau de gauche quand le
  pointeur approche de son bord (dossiers hors de vue inaccessibles par ce
  geste) ; rien au toucher. (`app/src/lib/drag-move.svelte.ts`,
  `SearchPanel.svelte`, `Sidebar.svelte`)
- **Imprimer un message** : n'existe pas. À prévoir dans « Plus… » ;
  impression du rendu assaini, depuis Rust ou la webview, à étudier avec la
  revue de sécurité (rendu HTML). (`visible-actions.ts`)
- **« Répondre à tous » replié dans « Plus… » à 1 280 px** : les cinq
  boutons libellés ne tiennent pas dans le volet de lecture avec les
  largeurs par défaut des panneaux. Pistes : volet de liste plus étroit par
  défaut, ou barre sur deux rangées au-delà d'une certaine largeur.
  (`ActionBar.svelte`, `--pane-list`)
- **Libellé `add_password_hint` faux** (« … jamais dans les fichiers de
  Norelly ») si l'utilisateur accepte le fichier de secrets non chiffré.
  Reformuler, ou n'afficher qu'avec un trousseau. (`app/messages/{fr,en}.json`,
  `AddAccountDialog.svelte`)

## Bogues

- **Certificat autosigné mal qualifié** : présenté comme « Le certificat a
  été refusé. » (`add_issue_other`) au lieu de « n'est pas signé par une
  autorité reconnue ». Hypothèse : `openssl req -x509` pose `CA:TRUE`,
  webpki répond `CaUsedAsEndEntity`, que `certificate_issue` range dans
  `Other` ; cas courant des serveurs personnels. Test rouge avec un
  certificat autosigné `CA:TRUE`, puis correspondance vers `UnknownIssuer`
  (ou un problème dédié). (`crates/adapters/tls/src/lib.rs`)

## Sécurité et chaîne d'approvisionnement

- Oracle du fuzzing de `sanitize_html` limité à `<script` : l'enrichir
  (`on*`, `javascript:` en attribut, `<iframe>`, `<base>`, `<meta>`,
  `<form>`).
- Chaîne Rust `stable` et outils d'`install-action` flottants dans la CI.
- Pas d'attestation de provenance ni de `SHA256SUMS` signé pour les
  versions publiées.
- Paquets apt de l'image e2e non figés.
- `fuzz/Cargo.lock` hors `cargo deny`.
- Flathub demandera une source `git` (tag et commit) au lieu de `type: dir`.
- `--filesystem=xdg-download` (Flatpak) à revoir si le sélecteur de
  fichiers passe par le portail.
- Installateur Windows non signé (Authenticode) tant que SignPath n'a pas
  accepté le projet ([candidature](signpath-candidature.md)).
- Permission Tauri `core:default` réductible.
- **Dossier modifié malgré un échec affiché** (revue de sécurité, basse) :
  le délai de 30 s de l'interface peut expirer pendant un `RENAME` ou un
  `DELETE` déjà commencé par l'acteur ; l'utilisateur voit « impossible »
  alors que l'opération aboutit. Code « résultat inconnu » puis relecture de
  la liste, ou délai limité à la prise en charge par l'acteur.
  (`app/src-tauri/src/mail.rs`, `crates/application/src/account_actor.rs`)
- **Dossiers spéciaux emportés par un parent renommé ou déplacé** (revue
  de sécurité, basse) : seul le dossier visé est protégé ; un dossier
  ordinaire qui contient un dossier spécial (`Perso/Envoyés` en `\Sent`),
  ou un conteneur `\Noselect` comme `[Gmail]`, part avec lui en `RENAME`.
  Refuser dans le domaine un renommage ou un déplacement dont un
  descendant a un rôle autre que `Normal`. (`crates/domain/src/mailbox.rs`,
  `moved_folder_name`, `renamed_folder_name`)
- **Contrôle « dans lui-même » sensible à la casse** (revue de sécurité,
  basse) : `is_same_or_under` compare les préfixes à la casse près et avec
  le seul séparateur du dossier déplacé ; sur un serveur insensible à la
  casse (`Projets` et `projets/2026`, `inbox/…`), la boucle n'est pas vue
  et le `RENAME` part (Dovecot et Cyrus refusent). Comparer sans la casse,
  comme pour les doublons, et avec le séparateur du parent aussi.
  (`crates/domain/src/mailbox.rs`)
- **Sous-dossiers ignorés au contrôle avant suppression** (revue de
  sécurité, basse) : un sous-dossier au nom indécodable ou refusé par
  `MailboxName::parse`, ou au-delà de `MAX_MAILBOXES`, n'est pas vu par
  `deletable_folder` ; le parent part en `DELETE` (effet selon le serveur).
  `LIST` dédié `<parent><délim>%` avant `DELETE`, ou noms écartés comptés
  comme « sous-dossier possible ». (`crates/domain/src/mailbox.rs`,
  `crates/adapters/imap/src/session.rs`)
- **Nom de dossier finissant par `{n}`** (revue de sécurité, basse) :
  cité par le serveur à la fin d'une ligne d'état, il est pris pour
  l'annonce d'un littéral ; la commande attend jusqu'au délai et la session
  est rouverte. Refuser `{` final (ou `\{\d+\}$`) dans `folder_label`, et
  dans le segment conservé d'un dossier déplacé.
  (`crates/domain/src/mailbox.rs`, `crates/adapters/imap/src/connection.rs`)

**Règle permanente** : à chaque montée de version de Tauri, rejouer le test
e2e qui vérifie qu'aucun accès à l'IPC n'est possible depuis l'iframe
`mailview://` (la capability ne distingue pas l'iframe de lecture, et
`tauri::test` n'enregistre pas les protocoles).

## Tâches humaines

- Lecture de `docs/utilisateur/fr/` (ou `en/`) par une personne peu à l'aise
  avec un ordinateur, idéalement en lui demandant d'installer Norelly et
  d'ajouter un compte de test sans aide ; noter où elle hésite.
- Relecture par le mainteneur de `docs/technique/`, `CONTRIBUTING.md` et
  `docs/utilisateur/{fr,en}/` (`docs/fonctionnel/` relu le 2026-11-05).
- Première exécution de [MANUAL-TESTS.md](MANUAL-TESTS.md) avec des comptes
  de test dédiés, puis à chaque version.
