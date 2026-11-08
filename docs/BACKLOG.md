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

- **Carnet : découvert au lot 5** :
  - l'expansion d'une liste relit ses membres un par un
    (`ContactDirectory::get`, 100 lectures au plus) : un port qui rend des
    fiches par identifiants en une requête (`crates/application/src/contact_lists.rs`) ;
  - les listes proposées à la saisie relisent une page entière de listes
    (membres compris) à chaque frappe, puis écartent les vides : requête
    dédiée (comptage des membres, `LIMIT`) si les listes se multiplient ;
  - plusieurs listes insérées dans un message peuvent dépasser la limite
    de destinataires : le refus n'arrive qu'à l'envoi ; prévenir dès
    l'insertion des pastilles (`app/src/lib/compose.svelte.ts`) ;
  - import et export des listes (vCard `KIND:group`, LDIF
    `groupOfNames`) à prévoir avec les lots 6 et 7.
- **Carnet : découvert au lot 7** :
  - le favori n'est porté par aucun format exporté : l'écrire (vCard
    `CATEGORIES`, colonne `Labels` de Google « * starred ») et le relire
    à l'import si le besoin se confirme ;
  - le CSV exporté n'a pas de colonne `Labels` : Google range les contacts
    importés dans un libellé propre à l'import, sans « myContacts » ;
  - « Exporter… » de l'en-tête reste actif sur un carnet vide (le refus
    arrive après la fenêtre du format) : le désactiver quand la liste est
    vide ;
  - un export de tout le carnet lit toutes les fiches en une requête et
    les garde en mémoire (25 000 au plus) avant d'écrire : lecture par
    pages si le carnet grandit ;
  - le nom du fichier proposé (« contacts.vcf ») n'est pas traduit.
  - à 1 280 px, « Importer… » et « Exporter… » sont tous deux rangés dans
    « Plus… » de l'en-tête de la vue Contacts (choix du mainteneur) : à
    revoir si la colonne s'élargit ou si l'en-tête est réorganisé.
- **Carnet : constats de la revue de sécurité du lot 7** (infos ; le
  constat bas, les droits du fichier exporté, est corrigé dans la PR, avec
  le caret vCard et les signes pleine chasse du CSV) :
  - un arrêt brutal entre l'écriture et le renommage laisse le temporaire
    `.contacts.vcf.<pid>-<n>.tmp` (tout le carnet, droits 0600) à côté de
    la cible (`app/src-tauri/src/contact_export.rs`, `write_atomic`) ;
  - l'écriture atomique derrière le portail de documents Flatpak
    (temporaire voisin dans `/run/user/<uid>/doc/…`) n'est pas testée :
    essai manuel à faire sous Flatpak ;
  - `save_attachment` crée ses fichiers avec les droits par défaut : à
    aligner sur l'export (0600) si le besoin se confirme.
  - neutralisation CSV : les petites formes (U+FE62, U+FE63, U+FE66,
    U+FE6B) et le signe moins U+2212 ne sont pas traités ; aucun tableur
    courant ne les évalue, à ajouter si l'un le fait.
- **Carnet : découvert au lot 6** :
  - remplacer les jeux d'essai reconstitués
    (`crates/adapters/contacts-io/tests/data`) par de vrais exports
    anonymisés de Gmail, Outlook, Thunderbird et Apple, comme le prévoyait
    le plan ;
  - les étiquettes importées gardent le mot du fichier (`work`, `home`,
    `cell` des vCard, « Travail » d'un export Google français) : les
    traduire à l'affichage, et lire les étiquettes `X-ABLabel` d'Apple
    (`item2.X-ABLabel:bureau`), aujourd'hui ignorées ;
  - les en-têtes français d'Outlook et de Thunderbird sont reconnus
    d'après leur forme connue, sans export réel pour les vérifier ; en
    ajouter un jeu d'essai, et d'autres langues au besoin ;
  - un aperçu abandonné (fenêtre fermée) garde ses fiches en mémoire
    jusqu'à l'aperçu suivant (20 Mio de fichier au plus) : les libérer
    à la fermeture (commande dédiée) ;
  - anniversaires, adresses postales, titres et sites des fiches importées
    sont ignorés : à ajouter au modèle si le besoin se confirme ;
- **Carnet : constats de la revue de sécurité du lot 6** (infos ; le
  constat moyen et les deux bas sont corrigés dans la PR) :
  - le plan est calculé sur des fiches lues hors de la transaction : une
    modification faite entre la lecture et l'import d'un contact à
    compléter serait écrasée. Relire ces contacts dans la transaction, ou
    comparer une version avant d'écrire
    (`crates/application/src/contact_import.rs`, `sqlite/src/contacts.rs`) ;
  - étendre l'import piégé (`hostile_message_sql.rs`) aux étiquettes
    d'adresse et de téléphone (colonnes Google) et à une partie locale
    avec `'` ;
  - les caractères invisibles des parties locales sont refusés à l'import
    seulement : même durcissement à faire à la saisie (élément du lot 1).
- **Carnet : constats de la revue de sécurité du lot 5** (infos) :
  - une seule liste corrompue en base rend toute la section « Listes »
    illisible et vide les suggestions de listes : n'écarter que la liste
    fautive, journaliser son identifiant (`crates/adapters/sqlite/src/contact_lists.rs`, `load`) ;
  - unicité du nom sans normalisation Unicode (NFC) : « É » précomposé et
    « E » suivi d'un accent combinant font deux listes au rendu
    identique ; à traiter avant l'import de listes (lots 6 et 7), qui
    devra aussi appliquer bornes et revalidation (`ContactListDraft::name_key`) ;
  - étendre le message piégé de bout en bout aux listes quand l'import
    alimentera ces tables.
- **Carnet : découvert au lot 4** :
  - le filtre du carnet cherche dans chaque champ séparément : « alice
    mar » ne trouve pas Alice Martin (prénom et nom dans deux champs), ni
    dans la liste, ni dans les suggestions. Chercher aussi dans le nom
    affiché composé (`ContactDraft::search_text`, colonne de recherche
    SQLite, migration) ;
  - les suggestions relisent chaque fiche trouvée (`ContactDirectory::get`,
    au plus deux fois la limite demandée) faute d'une lecture de fiches
    complètes filtrées : à mesurer avec les bancs 50k si le carnet grossit
    (import, lot 6), et au besoin un port qui rend des `Contact`.
- **Carnet : constats de la revue de sécurité du lot 4** (infos) :
  - `Recipient` garde des champs publics : la règle « nom imitant une
    adresse écarté » ne tient que par `Recipient::new` et `Recipient::parse`
    (aucun contournement aujourd'hui) ; rendre les champs privés avec
    accesseurs (`crates/domain/src/compose/draft.rs`) ;
  - une fiche illisible écarte tout le carnet des suggestions et journalise
    un avertissement à chaque frappe : n'écarter que la fiche, journaliser
    une fois (rejoint « une fiche relue invalide » du lot 1) ;
  - `Recipient::new` s'applique aussi au nom d'expéditeur du compte : un
    nom d'affichage de compte contenant « @ » part sans nom ; refuser ce
    nom dès la saisie du compte, avec un message clair.
- **Carnet : constats de la revue de sécurité du lot 1** (bas) :
  - caractères invisibles encore admis dans les champs d'une fiche
    (U+2028/2029, U+206A–206F, U+FFF9–FFFB, étiquettes U+E0000–E007F,
    sélecteurs de variante, remplisseurs Hangul, U+2800) : refusés à
    l'import depuis le lot 6 (`contact_import::INVISIBLE`) ; reste à les
    refuser à la saisie, avec l'assainissement des fiches existantes
    (`crates/domain/src/contacts.rs`, `connection::is_format_char`) ;
  - une fiche relue invalide fait échouer toute la page : écarter et
    signaler la ligne, ou assainir par migration à chaque durcissement de
    règle (`crates/adapters/sqlite/src/contacts.rs`). Depuis le lot 3, le
    courrier n'en souffre pas (noms des en-têtes gardés, `NameSenders`),
    mais les noms du carnet disparaissent de toute la page ;
  - nom imitant une adresse (`ceo@bank.example`) : écarté à l'affichage
    pour les en-têtes, les fiches (courrier, suggestions) et les
    destinataires (`Recipient::new`, saisie lue en Rust), retiré à
    l'import (lot 6) ; reste à le refuser à la saisie d'une fiche, avec
    l'assainissement des fiches existantes ;
  - plafond global du nombre de fiches : l'import en borne 25 000 par
    fichier (lot 6), sans plafond du carnet ; à fixer avec des bancs du
    carnet (liste, suggestions, import de 25 000 fiches) ;
  - un lien `tel:` éventuel exigera un jeu de caractères restreint décidé
    en Rust.
- **Carnet : constats de la revue de sécurité du lot 2** (bas et infos) :
  - une fiche d'identifiant supérieur à `u32::MAX` fait échouer toute la
    page de la liste (`id32` dans `app/src-tauri/src/contacts.rs`) :
    écarter et journaliser la ligne, ou garantir des identifiants sur
    32 bits côté stockage (rejoint « une fiche relue invalide ») ;
  - taille de page bornée seulement dans l'adaptateur SQLite
    (`MAX_PAGE_SIZE`) : borner dans le domaine (type `PageSize` ou plafond
    du cas d'usage) et l'ajouter à la suite de contrat ;
  - cases cochées non plafonnées dans l'interface : au-delà de 1 000, la
    suppression échoue sur un « valeur refusée » générique ; « Écrire au
    contact » relit une à une les fiches hors de la liste affichée :
    message clair et borne côté interface (`app/src/lib/contacts.svelte.ts`).
- **Carnet : constats de la revue de sécurité du lot 3** :
  - (moyen) le nom du carnet et la pastille « Dans vos contacts »
    s'affichent pour une adresse `From` falsifiable : l'application ne lit
    ni `Authentication-Results`, ni DKIM, ni DMARC. Piste : lire en Rust
    `Authentication-Results` du seul `authserv-id` du serveur du compte,
    règle du domaine `SenderAuth { Pass, Fail, Unknown }` (DMARC pass, ou
    DKIM pass aligné sur le domaine du `From`) ; pastille seulement sur
    `Pass`, avertissement « Expéditeur non vérifié » sur `Fail`. Garder le
    libellé « Dans vos contacts », jamais « vérifié »
    (`crates/domain/src/sender.rs`, `PersonMenu.svelte`) ;
  - (bas) `imitates_address` couvre `@ < >` et leurs sosies courants
    (pleine largeur, petites formes, chevrons), pas les formes textuelles
    (`support(at)banque.fr`) ni un nom qui ressemble à un domaine : piste,
    afficher aussi l'adresse dans la liste quand le nom d'en-tête contient
    un domaine ; appliquer la règle au nom prérempli par « Ajouter au
    carnet » (les noms des fiches sont écartés à l'affichage depuis le
    lot 4).
  - les notifications de nouveaux messages gardent le nom de l'en-tête
    (filtré s'il imite une adresse) : les nommer d'après le carnet.
- **Synchronisation CardDAV du carnet** (après l'epic carnet) : adaptateur
  WebDAV sur `tls`, synchronisation bidirectionnelle, conflits, sans
  reprise du schéma (modèle déjà calqué sur vCard, ADR-012).
- **Photos de contact** : `PHOTO` des vCard ignorée à l'import ; une image
  venue d'un fichier tiers passe par le même contrôle que les pièces
  jointes et le rendu (revue de sécurité) ; initiales en attendant.
- **Doublons du carnet : groupes écartés et critères** (lot 8) : un groupe
  « passé » est reproposé à chaque recherche ; retenir les paires écartées
  (migration, effacées quand une fiche change ou disparaît). Critères
  laissés de côté : noms sans les accents (tables Unicode hors du
  domaine), équivalences d'adresses propres à un fournisseur (Gmail :
  points, `googlemail.com`). Revue de sécurité (basse, traitée en
  partie : l'aperçu montre le nom et l'adresse préférée gardés) : pour un
  groupe rapproché par le seul téléphone, une fiche importée hostile très
  remplie reste proposée comme fiche gardée ; pistes : proposer la plus
  ancienne, ne rien cocher d'office, ne pas réunir le favori d'une fiche
  importée.
- **Captures e2e en sombre et au survol** : `pnpm -C app e2e` ne capture
  qu'en clair (WebKitWebDriver n'émule ni `prefers-color-scheme` ni le
  survol) ; lancer les captures avec `GTK_THEME=Adwaita:dark` et des
  déplacements de souris, pour que la vérification du plan (clair,
  sombre, survol, menus) se fasse dans l'application réelle.

- **Délai du marquage comme lu dans la fenêtre des réglages** : le réglage
  existe de bout en bout (`Settings::mark_seen_delay_secs`, 5 s par défaut,
  0 à 60 s, dans `SettingsDto`) mais aucun contrôle ne permet encore de le
  modifier. Ajouter le choix (immédiat, quelques secondes…) à
  `SettingsDialog.svelte`, libellés dans les deux catalogues, fiche
  fonctionnelle et guides. Une option « jamais » demanderait de changer le
  type du réglage (domaine, DTO) ; elle compte pour la vie privée (revue
  de sécurité, basse) : l'état lu est visible des autres personnes qui
  partagent la boîte, et le marquage s'active à la mise à jour sans choix
  de l'utilisateur. (`crates/domain/src/settings.rs`)
- **Exemplaire forgé marqué lu à l'ouverture** (revue de sécurité,
  basse) : `opened_seen_targets` étend le marquage aux membres de même
  `Message-ID` relus au moment du marquage ; un message forgé avec le
  `Message-ID` d'un message affiché, arrivé pendant le délai, est marqué
  lu, puis remplace l'affichage (dédoublonnage « le plus récent
  l'emporte » de `messages_query`). Ne garder que les exemplaires présents
  à l'ouverture (repère transmis par l'interface), préférer l'exemplaire
  le plus ancien ou hors Indésirables au dédoublonnage, et fixer le cas
  par un test. (`crates/domain/src/actions.rs`,
  `crates/adapters/sqlite/src/conversations.rs`)
- **Action en attente du marquage automatique sans limite** (revue de
  sécurité, info) : `#autoSeenInFlight` couvre aussi le rafraîchissement
  qui suit ; n'attendre que l'appel `markOpenedSeen`, ou borner l'attente.
  (`app/src/lib/mail.svelte.ts`)
- **Marquage comme lu fenêtre cachée** (revue de sécurité, info) : le
  délai court même fenêtre réduite ou masquée ; le suspendre tant que
  `document.visibilityState` n'est pas `visible`. (`app/src/lib/mail.svelte.ts`)
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
- **Retrait d'un dossier spécial rangé dans un autre** : il reste en tête,
  parmi les dossiers spéciaux (`folder_display_order`), mais son retrait
  suit toujours la profondeur de son nom (`depth`, `list_folders`) :
  `[Gmail]/Sent Mail` ou `INBOX/Sent` s'affichent en retrait sous un
  dossier qui n'est pas leur parent. Même défaut pour un dossier dont le
  parent n'est pas listé (revue de sécurité, basse) : `INBOX/Perso/Banque`
  sans `INBOX/Perso` semble rangé sous le dossier qui le précède, et les
  fenêtres de choix n'affichent que « Banque ». Profondeur affichée
  calculée par le domaine avec l'ordre (0 pour un dossier remonté au
  premier niveau, ancêtre + 1 pour un orphelin, libellé avec le chemin
  intermédiaire), en gardant la profondeur du nom pour `parentName` et
  `isSameOrUnder`. (`app/src-tauri/src/reading.rs`, `Sidebar.svelte`,
  `folder-edit.svelte.ts`)
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
- **Nom d'hôte non signalé pour un certificat autosigné** (revue de
  sécurité, info) : webpki refuse la chaîne (autorité inconnue, ou
  `CA:TRUE` en feuille) avant de comparer le nom ; un certificat autosigné
  au mauvais nom n'est présenté que comme « autorité inconnue ». Comparer
  aussi le nom pour l'afficher en plus de la raison.
  (`crates/adapters/tls/src/lib.rs`)
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
- **Action enfilée pendant le renommage d'un dossier** (revue de sécurité,
  basse) : les actions en attente suivent un dossier renommé ou déplacé
  (`FollowFolder`), mais une action enregistrée par l'interface entre le
  `RENAME` et la relecture de la liste garde l'ancien nom, tout comme
  celles d'une réécriture interrompue par l'arrêt de l'acteur ; elles sont
  abandonnées au rejeu (état local rétabli, aucun autre message touché).
  Rappeler `FollowFolder` après `sync_folders`, ou renommer la boîte locale
  dans la même transaction. (`crates/application/src/account_actor.rs`,
  `change_folder`)
- **`PendingOpStore::relocate` sans filtre de compte** (revue de sécurité,
  info) : `WHERE id = ?1` seulement, comme `complete`, `revert`, `cancel` ;
  les identifiants viennent aujourd'hui de `pending(account)`. Ajouter le
  compte au port et `AND account_id = ?`, avec au contrat : opérations de
  l'autre compte intactes, déplacement avec `target = None`.
  (`crates/adapters/sqlite/src/actions.rs`, `actions_contracts.rs`)
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
