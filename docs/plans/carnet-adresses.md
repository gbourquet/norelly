# Plan de l'epic : carnet d'adresses

## Contexte

Norelly n'a pas de carnet d'adresses : les seules « adresses connues » sont
calculées à la volée par `ContactQuery::suggest`
(`crates/domain/src/ports.rs:1257`, implémentation SQL
`crates/adapters/sqlite/src/compose.rs:748`) à partir des expéditeurs reçus
et des destinataires des messages envoyés. L'utilisateur ne peut ni
enregistrer un contact, ni corriger un nom, ni importer le carnet de son
ancien client (Gmail, Outlook, Thunderbird), ni écrire à un groupe. Le nom
affiché pour un expéditeur est celui de l'en-tête `From`, que l'expéditeur
choisit.

Décisions prises avec le mainteneur :
- carnet **local** (SQLite), modèle compatible vCard ; **CardDAV plus tard**
  (backlog) ;
- **un carnet unique** pour tous les comptes ;
- formats : import vCard 3.0/4.0, CSV Gmail/Outlook ; export vCard et CSV ;
  aligné sur Thunderbird, donc aussi **LDIF** (import/export) et CSV
  Thunderbird ;
- le plan est versionné dans le dépôt : `docs/plans/carnet-adresses.md`
  (ce fichier), livré dans la PR du lot 0.

Référence fonctionnelle (Thunderbird / Gmail / Outlook) : fiche contact
(nom, prénom, surnom, plusieurs adresses dont une préférée, téléphones,
organisation, notes), recherche et tri, listes de diffusion, « Ajouter au
carnet » depuis un message, « Écrire à », favoris, fusion des doublons,
import vCard/CSV/LDIF, export vCard/CSV/LDIF, adresses collectées
automatiquement (déjà là : `ContactQuery`).

## Découpage en lots (une branche et une PR par lot, toujours vers `master`)

Chaque lot suit TDD (rouge → vert → refactor), passe
`./scripts/check.sh full`, met à jour fiche fonctionnelle, traçabilité,
guides fr/en, backlog, et lance `security-reviewer` quand il touche une zone
sensible (SQL, IPC, fichiers, rendu).

### Lot 0 — `docs/carnet-adresses-plan` : plan et ADR
- `docs/plans/carnet-adresses.md` (ce plan).
- `docs/DECISIONS.md` : **ADR-012 Carnet d'adresses** (local, unique,
  modèle vCard, codecs dans un adaptateur, CardDAV différé, dépendance
  `csv`) — acceptée le 2026-11-07.
- Linear, epic « Carnet d'adresses : import, export et nouvelles
  fonctions » : « Synchronisation CardDAV », « Photos de contact ».

### Lot 1 — `domaine/contacts` : modèle, port, stockage
- `crates/domain/src/contacts.rs` (nouveau module, déclaré dans `lib.rs`) :
  - `ContactId`, `Contact { display_name, given_name, family_name,
    nickname, emails: Vec<ContactEmail{address: EmailAddress, label,
    preferred}>, phones, organization, notes, favorite }` ;
  - constructeur validant : au moins une adresse, une seule préférée,
    adresses distinctes sans la casse, longueurs bornées (réutiliser
    `header_text` / `MAX_ADDRESS_CHARS` de `mailbox.rs`), pas de caractères
    de contrôle ;
  - `ContactDraft` (saisie ou import, non encore enregistrée) ;
  - règle `display_name()` : nom affiché, sinon prénom + nom, sinon surnom,
    sinon adresse ;
  - normalisation de la clé d'adresse (minuscule, réutilise `EmailAddress`).
- `crates/domain/src/ports.rs` :
  - `ContactBook` (écriture) : `create`, `update`, `delete`, `delete_many`,
    erreurs `DuplicateAddress`, `NotFound` ;
  - `ContactDirectory` (lecture d'écran) : `page(cursor, filter)`
    (pagination par curseur, tri par nom, filtre texte, favoris),
    `get(id)`, `by_addresses(&[address])` (pour le nom d'expéditeur).
- Migration `crates/adapters/sqlite/migrations/0011_contacts.sql` :
  `contacts`, `contact_emails (contact_id, address, address_key UNIQUE,
  label, preferred)`, `contact_phones`, index sur `address_key` et sur le nom
  (tri). Déclarée à la fin de `migrations()` (`sqlite/src/lib.rs:38`),
  fixture ajoutée à `FIXTURES`, documentée dans `docs/technique/donnees.md`.
- `crates/adapters/sqlite/src/contacts.rs` : implémentation (paramètres
  liés, échappement LIKE comme dans `compose.rs:748`).
- `crates/test-support` : faux `ContactBook`/`ContactDirectory` +
  `contacts_contracts.rs`, joué contre le faux et SQLite.
- `crates/application/src/contacts.rs` : cas d'usage (créer, modifier,
  supprimer, lister), sans règle propre.

### Lot 2 — `ui/carnet-adresses` : ajouter, modifier, supprimer
- IPC (`app/src-tauri/src/contacts.rs`, DTO dans `ipc.rs`) :
  `list_contacts`, `get_contact`, `create_contact`, `update_contact`,
  `delete_contacts` ; revalidation par les constructeurs du domaine ; codes
  `ErrorCodeDto` (`contact_duplicate_address`, `contact_invalid`,
  `contact_not_found`) + `app/src/lib/errors.ts`. Enregistrement aux trois
  endroits (`lib.rs` `collect_commands!`, `ipc-commands.rs`,
  `permissions/main-window.toml`), `./scripts/gen-bindings.sh`, enveloppes
  dans `app/src/lib/ipc/api.ts`. Câblage dans `state.rs`.
- Interface :
  - entrée **« Contacts »** dans le pied du panneau de gauche
    (`Sidebar.svelte:281`, à côté de Paramètres/Boîte d'envoi) ; ouvre une
    vue Contacts dans `ThreePaneLayout` : liste virtuelle
    (`virtual-list.svelte.ts`) avec recherche, fiche à droite ;
  - `ContactDialog.svelte` (ajout/modification, plusieurs adresses,
    « préférée ») sur le modèle de `NewFolderDialog`/`Modal.svelte` ;
    état dans `app/src/lib/contacts.svelte.ts` (testé en TDD) ;
  - suppression avec confirmation (`DeleteContactDialog`, comme
    `DeleteFolderDialog`), sélection multiple ;
  - commandes du registre (`registerAppCommands`, `app.svelte.ts`) :
    `contacts.open`, `contacts.new`, `contacts.edit`, `contacts.delete`,
    `contacts.write` (« Écrire à » → rédaction préremplie), `contacts.favorite` ;
    chacune visible à la souris (`visible-actions.ts` : barre de la vue,
    « Plus… », menu contextuel d'une ligne) ; lignes dans
    `docs/utilisateur/{fr,en}/shortcuts.md` ; clés i18n fr/en.
- e2e + captures vérifiées (clair/sombre, survol, menus).
- Docs : `docs/fonctionnel/contacts.md` (nouvelle fiche),
  `docs/utilisateur/{fr,en}/contacts.md`.

### Lot 3 — `ui/nom-expediteur-carnet` : nom du carnet et « Ajouter au carnet »
- Domaine : règle `sender_label(header: &Correspondent, contact:
  Option<&Contact>)` — le nom du carnet l'emporte si l'adresse correspond
  exactement (clé normalisée) ; sinon le nom de l'en-tête, avec la règle
  existante qui écarte un nom imitant une adresse.
- Lectures d'écran : liste des conversations, messages d'une conversation
  et résultats de recherche ; le DTO indique le contact du carnet
  (pastille discrète, pas de texte Rust).
  *Réalisé* (dérogation validée par le mainteneur, consignée dans
  l'ADR-012) : plutôt qu'une jointure SQL (qui aurait recopié en SQL la
  normalisation de `AddressKey`), les commandes cherchent en une requête
  les contacts des adresses de la page par `ContactDirectory::by_addresses`
  (index unique sur `address_key`, cas d'usage `NameSenders`) ; le DTO
  porte `contact: Option<u32>` (identifiant pour « Voir le contact »).
- Bancs 50k (`cargo bench -p norelly-sqlite -- --test`) : pas de
  régression de la liste ni de la recherche.
- « Ajouter au carnet » / « Voir le contact » : menu sur l'expéditeur
  et les destinataires dans `ConversationView.svelte`, menu contextuel de
  la liste ; pré-remplit `ContactDialog`.
- Revue de sécurité : usurpation (le nom de confiance s'affiche pour une
  adresse `From` falsifiée) → consigner le constat ; piste backlog :
  n'afficher la pastille que si l'authentification (DKIM/DMARC) est connue.

### Lot 4 — `ui/suggestions-carnet` : autocomplétion
- Suggestions : contacts du carnet d'abord (nom du carnet, favoris en
  tête), puis adresses collectées, dédoublonnées par adresse. Écart validé
  (ADR-012) : `ContactQuery::suggest` inchangé, carnet lu par
  `ContactDirectory::page`, fusion par une règle du domaine ; contrat du
  carnet étendu (recherche sur toutes les adresses d'une fiche).
- Nom de fiche imitant une adresse écarté à l'affichage (courrier,
  suggestions).
- Saisie d'un destinataire lue en Rust (`parse_recipient`, validé par le
  mainteneur) : la règle `imitates_address` quitte l'interface.
- `RecipientInput.svelte` : distinguer visuellement contact / collecté.

### Lot 5 — `domaine/listes-diffusion` : listes de diffusion
- Domaine : `ContactList { name, description, members }` ; membres =
  contacts (adresse préférée ou une adresse choisie) ; nom unique sans la
  casse, liste non vide pour l'envoi, taille bornée ; règle
  `expand(list) -> Vec<Correspondent>` dédoublonnée.
  *Réalisé* (écart validé par le mainteneur, consigné dans l'ADR-012) :
  membres = contacts seulement, toujours écrits à leur adresse préférée
  du moment (pas d'adresse choisie) ; Cci enregistré avec la liste ;
  section « Listes » en 3e filtre de la vue Contacts ; listes proposées
  par `suggest_contact_lists`.
- Migration `0012_contact_lists.sql` (`contact_lists`,
  `contact_list_members` avec `ON DELETE CASCADE`) ; port, SQLite, faux,
  contrat.
- IPC + UI : listes dans la vue Contacts (section « Listes »), dialogue
  de création/modification (choix des membres avec recherche), ajout d'un
  contact à une liste depuis sa fiche ou le menu contextuel.
- Rédaction : la liste apparaît dans les suggestions ; la choisir insère
  ses membres en puces (comme Thunderbird à l'envoi) ; option « mettre en
  Cci » proposée dans le dialogue (protège les adresses des membres).

### Lot 6 — `import/contacts` : import vCard, CSV, LDIF
- Nouvel adaptateur `crates/adapters/contacts-io` (traduit des formats,
  sans décider) implémentant un port `ContactCodec` :
  - vCard 3.0 / 4.0 (RFC 2426 / 6350) écrit à la main : dépliage des
    lignes, échappements, `CHARSET`/`QUOTED-PRINTABLE` (vCard 2.1 exportés
    par Outlook/Android), `FN`, `N`, `NICKNAME`, `EMAIL;TYPE=…;PREF`, `TEL`,
    `ORG`, `NOTE`, plusieurs cartes par fichier, propriétés inconnues
    ignorées ;
  - CSV Google Contacts, Outlook, Thunderbird (détection par l'en-tête,
    séparateur `,`/`;`/tab, BOM UTF-8, encodage Windows-1252 de repli) via
    la crate `csv` (MIT/Unlicense) — à justifier dans la PR ;
  - LDIF (RFC 2849, export Thunderbird), y compris valeurs base64 ;
  - limites : taille du fichier, nombre de cartes, longueur des champs ;
    cible de fuzzing ajoutée (`fuzz/`).
- Domaine : `plan_import(existing, drafts, policy)` → à créer / à
  fusionner / ignorés (doublons par adresse) / rejetés (avec raison) ;
  politique « ignorer les doublons » ou « compléter les contacts
  existants ».
- IPC : `preview_contact_import` (ouvre le sélecteur via
  `tauri-plugin-dialog`, comme `compose.rs:678`, lit le fichier en Rust,
  renvoie un aperçu chiffré), `apply_contact_import` (transaction unique).
- UI : bouton « Importer… » dans la vue Contacts et « Plus… » ; dialogue
  en deux temps (aperçu : N nouveaux, N doublons, N illisibles → confirmer).
- Jeux d'essai réels anonymisés dans `crates/adapters/contacts-io/tests/data`
  (exports Gmail, Outlook, Thunderbird, Apple).
  *Réalisé* (écarts validés par le mainteneur, consignés dans l'ADR-012) :
  NFC par un port `TextNormalizer` (`icu_normalizer` dans `contacts-io`) ;
  nom imitant une adresse retiré, fiche gardée ; caractères invisibles
  refusés à l'import seulement ; aperçu chiffré pour les deux politiques,
  choix dans la fenêtre ; « Importer… » dans l'en-tête de la vue (repli
  dans « Plus… ») et dans le carnet vide. Jeux d'essai reconstitués dans
  la structure des exports, données fictives (écart validé après coup :
  le remplacement par de vrais exports anonymisés est au backlog).

### Lot 7 — `export/contacts` : export vCard, CSV, LDIF
- `ContactCodec::encode` : vCard 4.0 (et 3.0 au choix pour les vieux
  clients), CSV (colonnes Google, lisibles par Outlook), LDIF.
- IPC `export_contacts(ids|all, format)` avec sélecteur d'enregistrement ;
  UI : « Exporter… » (tout ou la sélection), choix du format.
- Test aller-retour : export puis import = même carnet.
  *Réalisé* (choix validés par le mainteneur, consignés dans l'ADR-012 ;
  colonne `Name`, règle du `FN` vCard et « Importer… » replié à
  1 280 px validés après coup) : CSV aux colonnes de Google plus `Name` ; règles de
  sortie dans le domaine (`export_card`), adresse préférée en tête,
  étiquettes vCard en minuscules, LDIF sans étiquette d'adresse ; favori
  et listes non exportés ; cellules CSV neutralisées (téléphones
  compris), apostrophe retirée à l'import ; « Exporter… » dans l'en-tête
  et la barre des contacts cochés.

### Lot 8 — `ui/contacts-doublons` : fusion des doublons
- Domaine : détection (même adresse dans deux contacts impossible par
  contrainte ; doublons = même nom normalisé ou adresses qui se
  recoupent après import en mode « ignorer ») et règle de fusion.
- UI : « Rechercher les doublons… » dans « Plus… » de la vue Contacts.
  *Réalisé* (règles validées par le mainteneur avant le code, consignées
  dans l'ADR-012) : deux contacts ne partagent jamais une adresse, le
  second critère est donc le téléphone (même nom en NFC sans casse ni
  ordre des mots, ou même numéro d'au moins 6 chiffres, par
  transitivité) ; fusion dans la fiche choisie (la plus complète par
  défaut), champs vides complétés, adresses et téléphones réunis, notes
  bout à bout, listes reportées, en une transaction
  (`ContactBook::merge`) ; fenêtre qui présente les groupes un par un
  (« Fusionner », « Passer ») ; groupe passé non mémorisé (backlog).

## Fichiers critiques
- `crates/domain/src/ports.rs`, `crates/domain/src/contacts.rs` (nouveau),
  `crates/domain/src/mailbox.rs` (`Correspondent`, `header_text`)
- `crates/adapters/sqlite/src/{lib.rs,contacts.rs,conversations.rs,search.rs,compose.rs}`,
  `migrations/0011_contacts.sql`, `0012_contact_lists.sql`
- `crates/adapters/contacts-io/` (nouveau), `deny.toml` (aucune exception attendue)
- `app/src-tauri/src/{ipc.rs,contacts.rs,state.rs,lib.rs}`,
  `ipc-commands.rs`, `permissions/main-window.toml`
- `app/src/components/{Sidebar,RecipientInput,ConversationView,ComposeDialog}.svelte`,
  nouveaux `Contacts*.svelte`, `ContactDialog.svelte`
- `app/src/lib/{app.svelte.ts,visible-actions.ts,compose.svelte.ts,errors.ts}`,
  `app/messages/{fr,en}.json`

## Vérification (par lot)
- Tests Rust : `cargo nextest run -p norelly-domain contacts`,
  `-p norelly-sqlite contacts`, contrats dans `test-support`.
- Front : `pnpm -C app test`, `pnpm -C app check`, `pnpm -C app lint`.
- `./scripts/check.sh full` (couverture, SQL, i18n, docs, deny).
- e2e : `pnpm -C app e2e` — scénarios : créer/modifier/supprimer un
  contact ; nom du carnet affiché dans la liste et le message ; importer
  un `.vcf` et un CSV Gmail ; créer une liste et l'utiliser dans un
  message envoyé vers Mailpit ; exporter puis réimporter.
- Lots 3–4 : bancs 50k sans régression.
- Manuel : `pnpm -C app tauri dev`, capture vérifiée de chaque écran
  (clair, sombre, survol, menus, libellés non tronqués à 1 280 px).
