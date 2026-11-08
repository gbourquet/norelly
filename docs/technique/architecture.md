# Architecture

[← Documentation technique](README.md)

Norelly suit une **architecture hexagonale dosée** (ADR-008) : un domaine
pur entouré de ports, des adaptateurs qui les implémentent, des cas d'usage
qui les orchestrent et un point d'assemblage Tauri. Les schémas ci-dessous
sont en Mermaid (rendus par GitHub et la plupart des éditeurs).

## Vue d'ensemble

```mermaid
flowchart LR
  subgraph UI["app/src : interface Svelte 5 (présentation seulement)"]
    reg["Registre de commandes"]
    views["Vues : liste, lecture, rédaction, recherche"]
  end
  subgraph Shell["app/src-tauri : adaptateur entrant + assemblage"]
    ipc["Commandes IPC (ipc.rs, *.rs)"]
    proto["Protocoles mailview:// mailpart:// remote-img://"]
    state["AppState : assemblage des adaptateurs"]
  end
  subgraph App["crates/application : cas d'usage async"]
    uc["SyncAccount, PerformAction, QueueMessage, ReadMessage…"]
    actor["Acteur de compte, travailleur de rédaction"]
  end
  subgraph Domain["crates/domain : règles pures + ports (traits)"]
    rules["sync, actions, conversation, compose, search, security, discovery…"]
    ports["ports.rs"]
  end
  subgraph Adapters["crates/adapters/*"]
    imap & smtp & tls & sqlite & blobs & keyring & discovery & render
  end
  UI -- "invoke (bindings TS générés)" --> ipc
  UI -- "iframe sandbox" --> proto
  ipc --> uc
  proto --> uc
  uc --> rules
  uc --> ports
  Adapters -. implémentent .-> ports
  state --> Adapters
```

Règle de dépendance : `domain` ne dépend de rien d'autre que `thiserror`
et `serde` (vérifié par `scripts/check-domain-deps.sh`) ; `application`
dépend du domaine ; les adaptateurs dépendent du domaine ; seul
`app/src-tauri` connaît tout le monde.

## Les couches

### Domaine (`crates/domain`)

Règles métier pures, **sans aucune E/S**, et ports. C'est la zone de
relecture prioritaire du mainteneur.

| Module | Contenu |
|---|---|
| `address`, `account`, `credentials`, `endpoint` | Valeurs validées : adresse, compte, secret masqué, point de connexion |
| `security`, `capabilities`, `auth`, `connection` | Confiance TLS et exceptions, consentement au clair, STARTTLS, choix SASL, erreurs structurées |
| `discovery` | Stratégie d'autodécouverte |
| `mailbox`, `sync`, `fetch_policy` | Boîtes, UID, drapeaux ; rapprochement de synchronisation ; rapatriement ; reconnexion (`Backoff`) |
| `conversation` | Fils JWZ, version incrémentale, boîte unifiée |
| `reading` | Liens, images distantes, noms et ouverture des pièces jointes |
| `actions` | Sémantique des actions, application optimiste, file durable, conflits |
| `compose` (`draft`, `outbox`) | Réponses, sujets, fils, citation ; brouillon ; file d'envoi |
| `search` | Syntaxe des requêtes, surlignage |
| `contacts` | Fiche du carnet d'adresses validée, clé d'adresse, nom affiché, filtre et ordre de la liste (ADR-012) |
| `contact_import` | Plan d'import d'un carnet : noms en NFC (port `TextNormalizer`), nom imitant une adresse retiré, caractères invisibles refusés, doublons par `AddressKey`, rejets motivés, politique « ignorer » ou « compléter » |
| `notify`, `settings` | Notifications à émettre ; paramètres |
| `ports` | Tous les traits (≈ 27) : `RemoteMailboxConnector`, `RemoteMailboxSession`, `MailSender`, `SecretStore`, `MailStore`, `BlobStore`, `PendingOpStore`, `DraftStore`, `OutboxStore`, `SearchIndex`, carnet (`ContactBook`), requêtes d'écran (`FolderTreeQuery`, `ConversationQuery`, `MessageListQuery`, `ContactDirectory`)… |

Les ports asynchrones renvoient `impl Future + Send` (dispatch statique dans
les chemins chauds) ; `dyn` n'est utilisé qu'au point d'assemblage.

### Cas d'usage (`crates/application`)

Orchestration async des ports, sans règle métier propre :

| Module | Rôle |
|---|---|
| `accounts`, `discover_settings`, `test_connection` | `AddAccount`, `RemoveAccount`, `DiscoverSettings`, `TestConnection` |
| `account_actor` | Un acteur par compte : connexion de commandes + connexion IDLE, événements `SyncEvent` |
| `sync` | `SyncAccount`, boîte par boîte, enregistrement par lots |
| `reading` | `ReadMessage`, `allow_remote_images`, `DownloadPart` |
| `actions` | `PerformAction`, `UndoActions`, `ReplayActions` |
| `compose`, `compose_worker` | `SaveDraft`, `QueueMessage`, `PrepareReply`, `DeliverOutbox`, `RecoverOutbox`, `FileSentCopies`, `SyncDrafts` ; un travailleur par compte |
| `contacts` | `CreateContact`, `UpdateContact`, `DeleteContacts`, `ListContacts`, `GetContact` |

### Adaptateurs (`crates/adapters/*`)

| Crate | Implémente | Notes sécurité |
|---|---|---|
| `tls` | `ClientConfig` rustls : vérificateur de la plateforme + exception par empreinte | fichier sensible |
| `imap` | `RemoteMailboxConnector`, `RemoteMailboxSession` : pilote d'E/S maison au-dessus d'`imap-next` (lignes bornées, délais, rejet des octets reçus après `STARTTLS`) | fichier sensible (`connection.rs`) |
| `smtp` | `MailSender` : TLS par `norelly-tls`, puis `lettre` | STARTTLS exigé |
| `sqlite` | `MailStore`, `AccountRepository`, requêtes d'écran, `PendingOpStore`, `DraftStore`, `OutboxStore`, `SearchIndex`, `SettingsStore`, `ContactBook`, `ContactDirectory`… | SQL paramétré uniquement (`check-sql.sh`) |
| `blobs` | `BlobStore` : `.eml.zst` par compte et empreinte, écriture atomique | — |
| `keyring` | `SecretStore` : Secret Service, Credential Manager ; fichier 0600 en dérogation | — |
| `discovery` | `AutoconfigClient`, `SrvResolver`, `PortProber` | HTTPS seulement |
| `render` | Rendu MIME → vue, assainissement HTML et CSS, agent, construction MIME sortante, extraction du texte indexé | fichiers sensibles (`sanitize.rs`, `css.rs`, `agent.js`) |
| `contacts-io` | `ContactCodec` (lecture vCard, CSV Google/Outlook/Thunderbird, LDIF), `TextNormalizer` (NFC) | entrées bornées, fuzzé (`contact_import`) ; fichier lu par le shell, jamais d'URL suivie |

### Point d'assemblage et adaptateur entrant (`app/src-tauri`)

- `state.rs` : `AppState` ouvre la base, les blobs, le trousseau, démarre les
  acteurs et travailleurs ; c'est le seul endroit qui choisit les
  implémentations.
- `ipc.rs` : **contrat IPC** (DTO), sans dépendance de `specta` dans le
  domaine ; toute entrée est revalidée par les constructeurs du domaine.
- `commands.rs`, `reading.rs`, `actions.rs`, `compose.rs`, `search.rs`,
  `settings.rs`, `updates.rs` : commandes `#[tauri::command]`.
- `protocols.rs`, `remote_images.rs` : protocoles personnalisés (voir
  [données](donnees.md#protocoles-personnalisés)).
- `mail.rs` : relais des événements des acteurs vers l'interface.
- `ipc-commands.rs`, `permissions/main-window.toml`,
  `capabilities/default.json` : liste de contrôle d'accès (seule la
  fenêtre `main`, depuis l'origine de l'application, peut appeler les
  commandes).

### Interface (`app/src`)

Présentation seulement, Svelte 5 en runes. **Aucune décision de sécurité.**

- `lib/app.svelte.ts` : état global et **registre de commandes**
  (`registerAppCommands`), qui alimente raccourcis, palette, aide, barres
  d'actions, menus « Plus… », menus des comptes et menu contextuel ;
- `lib/visible-actions.ts` : actions proposées à la souris dans chaque
  zone (barre du message ouvert et « Plus… », survol d'une ligne, menu
  contextuel, barre de sélection), priorité de repli dans « Plus… »
  (`fitByPriority`) ; `lib/commands/describe.ts` (libellé, infobulle
  « Libellé — Touches », disponibilité), `lib/commands/watch.svelte.ts`
  (relecture du registre par les composants), `lib/fixed-shortcuts.ts`
  (touches de la fenêtre de rédaction) ;
- `lib/mail.svelte.ts`, `compose.svelte.ts`, `search.svelte.ts`,
  `settings.svelte.ts`, `updates.svelte.ts` : états par domaine d'écran ;
- `lib/ipc/bindings.ts` (généré, commité) et `lib/ipc/api.ts` (enveloppe :
  `Result` → exception `IpcFailure`) ;
- `lib/errors.ts` : traduction des codes d'erreur IPC ;
- `lib/agent.ts` : validation des messages `postMessage` de l'agent ;
- `components/*.svelte` ; `styles/tokens.css`.

## Contrat IPC

```mermaid
sequenceDiagram
  participant V as Vue Svelte
  participant A as api.ts
  participant B as bindings.ts (généré)
  participant C as Commande Rust
  participant D as Domaine
  V->>A: api.performAction(...)
  A->>B: commands.performAction(...)
  B->>C: invoke("perform_action", DTO)
  C->>D: revalidation (constructeurs du domaine)
  D-->>C: valeur valide ou erreur
  C-->>B: Result<DTO, IpcError{code, field, detail}>
  B-->>A: {status: "ok" | "error"}
  A-->>V: valeur, ou IpcFailure
  V->>V: errorText(code) → texte du catalogue (fr/en)
```

- Types définis en Rust (`app/src-tauri/src/ipc.rs`), exportés en
  TypeScript par `tauri-specta` dans `app/src/lib/ipc/bindings.ts` (commité).
  Le test `bindings_are_up_to_date` échoue s'ils sont périmés ;
  `./scripts/gen-bindings.sh` les régénère.
- **Aucun texte destiné à l'utilisateur ne vient de Rust** (ADR-011) : les
  erreurs sont des codes (`ErrorCodeDto`) avec un champ (`FieldDto`) et un
  `detail` technique brut facultatif, affiché tel quel.
- Événements Rust → interface : `SyncEventDto`, `ComposeEventDto`,
  `UpdateEventDto`.
- Listes paginées côté Rust (curseur) : jamais de transfert en masse
  (ADR-001).
- Les chemins de fichiers ne traversent jamais l'IPC depuis l'interface :
  les sélecteurs sont ouverts par Rust, un dépôt de fichier est désigné par
  un jeton.

## Flux principaux

### Synchronisation

```mermaid
sequenceDiagram
  participant Act as Acteur de compte
  participant S as SyncAccount
  participant Dom as domain::sync
  participant I as Adaptateur IMAP
  participant DB as SQLite (MailStore)
  participant UI as Interface
  Act->>I: connexion de commandes (TLS, auth)
  Act->>S: synchroniser le compte
  loop pour chaque boîte
    S->>DB: état local (UIDVALIDITY, HIGHESTMODSEQ, UIDNEXT)
    S->>Dom: select_request(état)
    S->>I: SELECT (CONDSTORE / QRESYNC)
    S->>Dom: plan(réponse) → SyncPlan
    S->>I: requêtes du plan (FETCH drapeaux, UID SEARCH…)
    S->>Dom: reconcile(rapport) → SyncOutcome
    S->>DB: enregistrement par lots (+ conversations, index FTS par déclencheurs)
    S->>I: corps selon fetch_policy (plus récents d'abord)
    S-->>UI: SyncEvent::FolderChanged
  end
  Note over Act,I: connexion IDLE sur INBOX : changement → sync de INBOX,<br/>renouvelée toutes les 25 min ; reconnexion avec Backoff
```

### Lecture sécurisée

```mermaid
sequenceDiagram
  participant UI as Interface
  participant F as iframe sandbox="allow-scripts"<br/>(origine opaque)
  participant P as Protocole mailview://
  participant R as ReadMessage + render
  participant Px as remote-img:// (proxy)
  UI->>F: src = mailview://localhost/<id>
  F->>P: GET
  P->>R: corps (blobs) + préférences + autorisation images
  R->>R: mail-parser → meilleure alternative → ammonia + CSS assaini
  R-->>P: HTML assaini, cid: → mailpart://, images distantes → espace réservé ou remote-img://
  P-->>F: HTML + CSP (default-src 'none', script = agent par empreinte)
  F->>UI: postMessage (hauteur, lien cliqué, touche) via l'agent
  UI->>UI: validation (source, schéma, taille)
  UI->>P: open_external(lien) : Rust vérifie http/https/mailto
  F->>Px: image autorisée (index du dernier rendu)
  Px->>Px: expéditeur autorisé ? adresse publique ? taille et type bornés
```

### Actions hors ligne

```mermaid
sequenceDiagram
  participant UI as Interface
  participant PA as PerformAction
  participant Dom as domain::actions
  participant Q as pending_ops (SQLite)
  participant Act as Acteur (ReplayActions)
  participant I as IMAP
  UI->>PA: archiver (cibles)
  PA->>Dom: resolve(action, dossier) → Effect
  PA->>Q: PendingOp (avec état d'annulation)
  PA->>Q: local_effect (optimiste) + copie provisoire
  PA-->>UI: notice « Archivé. » + Annuler (5 s)
  alt Annuler dans les 5 s
    UI->>PA: undo → revert_effect
  else délai passé, serveur joignable
    Act->>Q: opérations dues
    Act->>Dom: decide(op, état serveur)
    alt pas de conflit
      Act->>I: MOVE, ou COPY + STORE \Deleted + UID EXPUNGE, ou EXPUNGE sûr
    else conflit (UIDVALIDITY, message ou dossier disparu)
      Act->>Q: abandon + revert_effect
    end
    Act->>Act: resynchroniser les boîtes touchées
  end
```

### Envoi

```mermaid
sequenceDiagram
  participant UI as Interface
  participant Qm as QueueMessage
  participant O as outbox (SQLite)
  participant W as Travailleur de rédaction
  participant Rd as render (MIME)
  participant S as SMTP
  participant I as IMAP
  UI->>Qm: send_draft(brouillon)
  Qm->>Qm: check_sendable, tailles réelles, types MIME recalculés
  Qm->>O: Queued (+ brouillon gardé si programmé)
  W->>O: prochain message dû
  W->>Rd: HTML réassaini + texte → multipart/alternative (Bcc hors octets)
  W->>S: connexion (TLS/STARTTLS par norelly-tls), envoi
  alt succès
    W->>O: Sent
    W->>I: APPEND dans Envoyés (sauf Gmail), retrait de la copie du brouillon
  else temporaire / injoignable
    W->>O: Queued, attente croissante (injoignable : non compté)
  else refus, auth, trop d'essais
    W->>O: Failed → l'interface le signale
  end
  Note over W,O: au démarrage, Sending → Failed(Uncertain), jamais renvoyé seul
```

## Pour aller plus loin

- Choix et alternatives écartées : [DECISIONS.md](../DECISIONS.md).
- Règles par fonctionnalité : [documentation fonctionnelle](../fonctionnel/README.md), reliée au code par [la traçabilité](tracabilite.md).
- Données et protocoles : [donnees.md](donnees.md).
