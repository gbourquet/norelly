# Sécurité

[← Documentation technique](README.md)

Un client de messagerie lit des données écrites par n'importe qui et parle
à des serveurs à travers des réseaux non fiables. La sécurité est donc le
premier critère de conception : **toute décision de sécurité est prise en
Rust**, l'interface ne fait que présenter (ADR-001, ADR-008).

## Modèle de menace

| Adversaire | Capacité supposée | Objectifs à contrer |
|---|---|---|
| **Expéditeur malveillant** | Envoie un message arbitraire : HTML, CSS, MIME malformé, en-têtes hostiles, noms de fichiers piégés, pièces jointes exécutables | Exécuter du code, atteindre l'IPC, pister la lecture, sonder le réseau local, faire planter ou ralentir l'application, injecter du SQL ou de la syntaxe de recherche, usurper l'interface |
| **Réseau hostile** (Wi-Fi public, FAI) | Intercepte, modifie, rejoue le trafic ; falsifie le DNS | Lire le mot de passe ou les messages (STARTTLS retiré, certificat substitué, injection avant la poignée de main), rediriger l'autodécouverte |
| **Serveur hostile ou défaillant** | Réponses IMAP/SMTP arbitraires, tronquées, énormes | Plantage, boucle, consommation mémoire, corruption de l'état local |
| **Autoconfiguration falsifiée** | Réponse XML ou SRV choisie par l'attaquant | Envoyer le mot de passe à un serveur tiers |
| **Page ou programme local** | Envoie des requêtes à la redirection OAuth locale ; lit l'URL d'autorisation | Faire accepter un code forgé (CSRF), annuler la connexion, voler le code d'autorisation |
| **Chaîne d'approvisionnement** | Dépendance ou action CI compromise, mise à jour substituée | Exécuter du code chez l'utilisateur |

Hors périmètre : un attaquant qui contrôle déjà la session de l'utilisateur
(lecture du dossier de données, du trousseau déverrouillé). La base locale
n'est pas chiffrée (ADR-003) : on s'appuie sur le chiffrement du disque.

## Défenses en place

### Transport (ADR-002)

- TLS par `rustls` (1.2 minimum), vérificateur de la plateforme ; exception
  par **hôte + empreinte SHA-256**, décidée par l'utilisateur ; empreinte
  changée signalée (`crates/domain/src/security.rs`,
  `crates/adapters/tls`).
- STARTTLS **obligatoire** s'il est demandé ou annoncé : aucun repli en
  clair ; données reçues après la réponse à `STARTTLS` et avant la poignée
  de main rejetées ; capacités relues après TLS
  (`crates/adapters/imap/src/connection.rs`, `crates/adapters/smtp`).
- Clair refusé sans consentement explicite par serveur.
- Pilote d'E/S IMAP maison : lignes et littéraux bornés, délais, aucune
  panique sur réponse malformée.
- Délais : chaque lecture ou écriture de l'ouverture d'une connexion IMAP
  ou SMTP (salutation, `STARTTLS`, authentification) a son propre délai,
  plus court que celui d'une session ouverte, et l'ouverture entière a une
  durée totale bornée (un serveur qui envoie un octet de temps en temps ne
  bloque pas le compte) ; le test de connexion a des délais courts
  (poignée de main TLS comprise) et une durée totale plafonnée par
  serveur ; la synchronisation et l'envoi gardent une poignée de main
  large pour les liaisons dégradées (`app/src-tauri/src/state.rs`,
  `connection_test`). Un délai plus court ne change aucune vérification :
  un essai abandonné ferme sa connexion, et le suivant repart de zéro (TLS,
  certificat, STARTTLS) ; les octets écrits et lus par le pilote IMAP sont
  effacés même quand l'essai est abandonné (`Zeroizing`).
- L'authentification par mot de passe attend au moins
  `PASSWORD_AUTH_TIMEOUT`, plus que les retards qu'appliquent les serveurs
  à un refus : un mot de passe refusé lentement est lu comme un refus, non
  renvoyé à un nouvel essai.
- Les identifiants ne sont jamais rejoués aussitôt : une coupure (fermeture,
  réinitialisation, `* BYE`) ou un silence une fois `LOGIN`, `AUTHENTICATE`
  ou `AUTH` envoyé donne `AuthError::Interrupted`, que le test de connexion
  ne réessaie pas (un serveur qui bloque un compte après N échecs le
  bloquerait sinon plus vite) ; seuls les échecs qui précèdent l'envoi des
  identifiants sont réessayés (`ConnectionPhase`, domaine). La file
  d'envoi en fait un échec définitif (`SendFailure::AuthenticationInterrupted`)
  et, après un refus ou une interruption, n'envoie plus rien pour ce
  compte pendant `CREDENTIALS_HOLD_SECS` (15 min) sauf action de
  l'utilisateur (`credentials_hold`) : les messages suivants ne
  présentent pas les mêmes identifiants coup sur coup. Le travailleur de
  rédaction applique la même attente à sa connexion IMAP (copie des
  brouillons enregistrés toutes les deux secondes, dépôt dans
  « Envoyés ») : `connection_hold`. L'acteur de
  synchronisation attend au moins `CREDENTIALS_RETRY_MS` (15 min) avant de
  se reconnecter après un mot de passe refusé ou interrompu
  (`Backoff::next_delay_after`, `replay_risks_lockout`).
- Une coupure ou un silence avant la fin du chiffrement (échange en clair
  précédant STARTTLS, borné en entier par le délai d'ouverture, ou poignée
  de main : `ClosedBeforeEncryption`, `Timeout(BeforeEncryption)`,
  `Timeout(TlsHandshake)`) ne laisse jamais ajouter un compte OAuth avec
  « envoi non vérifié » : c'est ce que produirait un réseau hostile ; à
  l'envoi, son échec répété est nommé (`SendFailure::EncryptionInterrupted`).
  Un réseau perdu (hôte injoignable) n'est pas requalifié.
- Niveau `TRACE` supprimé à la compilation : `imap-next` y journaliserait
  les mots de passe ; test `credentials_never_appear_in_logs_or_debug`.
- Secrets : type `Secret` sans `Display`, `Debug` masqué, non sérialisable,
  effacé (`zeroize`) ; stockés dans le trousseau du système.

### Connexion OAuth (ADR-013)

- Jeton d'accès porteur : il n'est envoyé qu'aux serveurs IMAP et SMTP de
  son fournisseur, en TLS ou STARTTLS, jamais en clair, même consenti
  (`crates/domain/src/oauth.rs`, `token_destination_allowed`, vérifié par
  les adaptateurs IMAP et SMTP avant toute authentification) ; un compte
  OAuth n'utilise que XOAUTH2.
- Code d'autorisation protégé par PKCE S256 ; `state` aléatoire
  (128 bits) vérifié avant tout le reste ; retour forgé ou illisible
  ignoré sans annuler la connexion ; redirection `localhost` écoutée sur
  `127.0.0.1` et `::1` au même port (aucun autre programme ne peut
  occuper l'une des deux ; `127.0.0.1` seul sans IPv6, ADR-013), port
  éphémère, requête bornée en taille (8 Kio) et en durée (5 s pour la
  requête entière), une tâche par connexion, 32 au plus à la fois (les
  suivantes attendent dans la file du système), erreur d'acceptation
  passagère : un client local lent ou un afflux de connexions ne bloque
  pas l'attente (`crates/adapters/oauth`). Fenêtre d'ajout fermée ou
  annulée : l'attente s'arrête et l'écoute est refermée
  (`cancel_oauth_sign_in`, `TokenBroker::cancel_sign_in`), la connexion
  gardée d'un essai échoué est oubliée ; une annulation arrivée pendant
  le test des serveurs empêche l'enregistrement du compte et la garde de
  la connexion (contrôle sous le verrou de `pending`). Requête du
  navigateur (code) et corps de la réponse des jetons lus dans des
  tampons réservés d'emblée et effacés (`SecretBytes`), jetons lus
  directement en `Secret` ; les morceaux reçus du client HTTP ne sont pas
  effacés. Client HTTP en clair réservé aux tests (`#[cfg(test)]`).
  Limite connue : sous Windows, l'écoute n'est pas exclusive
  (`SO_EXCLUSIVEADDRUSE` exigerait `socket2` en dépendance directe) ; un
  code intercepté reste inutilisable sans le vérificateur PKCE.
- Compte vérifié : la connexion interactive demande aussi `openid email
  profile` (Google : `openid email`) ; l'`id_token` reçu du point d'accès des jetons (HTTPS vérifié,
  en échange du code et du vérificateur PKCE, donc signature non
  vérifiée) doit porter l'adresse du compte en `preferred_username` ou
  `email`, sinon les jetons sont refusés avant tout test, stockage ou
  remplacement, à l'ajout comme à la reconnexion
  (`crates/domain/src/oauth.rs`, `verify_sign_in`, `verify_identity`). Un
  code d'un autre compte injecté dans la redirection locale est ainsi
  refusé. Google : `iss` de Google (`https://accounts.google.com` ou
  `accounts.google.com`), `aud` égal à l'identifiant client et
  `email_verified` exigés (`check_identity_claims`) ; Microsoft : `aud`
  égal à l'identifiant client, `tid` égal au locataire des comptes
  personnels (`OAuthConfig::tenant`, `MICROSOFT_CONSUMERS_TENANT`) et
  `iss` émis par ce locataire
  (`https://login.microsoftonline.com/<tid>/v2.0`), comme le demande
  OpenID Connect Core (section 3.1.3.7). La suite de contrat du client
  OAuth (`oauth_client_contract`) exige qu'une connexion interactive
  rapporte une identité acceptée par `verify_sign_in`, contre le faux et
  contre l'adaptateur réel.
- Compte Microsoft reconnu à son identifiant stable : `oid` de l'`id_token`
  (`OAuthSubject`, ASCII visible, 128 caractères au plus ; illisible =
  absent), et non `sub`, propre à chaque identifiant client chez
  Microsoft. Enregistré à l'ajout, ou à la première reconnexion réussie
  d'un compte qui n'en a pas, seulement si `preferred_username` est
  l'adresse (`subject_to_record`) : l'`email` seul, plus facile à
  modifier, ne lie pas le compte pour toujours. Ensuite, la reconnexion
  exige ce même `oid` (`verify_reconnection`), sans comparer l'adresse, que
  l'utilisateur peut changer chez Microsoft ; `oid` absent : refus
  (`TokenError::Protocol`). Un identifiant enregistré n'est jamais
  remplacé (`record_oauth_subject`, mise à jour conditionnelle, faite avant
  le rangement des jetons). Limites connues : l'`oid` n'est unique qu'à
  l'intérieur d'un locataire et il est stocké sans `tid` ; c'est sûr tant
  que `check_identity_claims` impose le seul locataire des comptes
  personnels, et à revoir si Norelly s'ouvre aux comptes professionnels.
  Un ajout Microsoft sans `oid` reste accepté et le compte reste reconnu à
  son adresse (pas de blocage si Microsoft omettait la revendication).
  Google n'est pas concerné (`OAuthProvider::has_stable_subject`) : son
  adresse est vérifiée (`email_verified`) et ne change pas.
  Connexion Google sans le scope de messagerie (consentement granulaire) :
  refusée (`require_mail_access`). Version compilée sans les identifiants
  d'un fournisseur : rien ne lui est envoyé, ni jeton de rafraîchissement
  ni ouverture du navigateur (`TokenBroker`, `AuthError::ProviderNotConfigured`,
  refus définitif sans nouvel essai). Le
  rafraîchissement redemande les seuls scopes de messagerie chez
  Microsoft ; Google garde les scopes accordés (`openid email` compris),
  le jeton ne partant de toute façon que vers les serveurs de Gmail.
- Ajout d'un compte OAuth : l'interface n'envoie pas le fournisseur ; le
  cas d'usage le déduit de l'adresse (`OAuthProvider::for_address`), et
  une adresse sans fournisseur est refusée avant toute connexion.
- Compte supprimé pendant une reconnexion OAuth : la reconnexion ne tient
  le compte (`AccountLocks`, partagé avec `remove_account`) qu'au retour
  du navigateur, le relit, et ne range le jeton qu'ensuite ; supprimé
  entre-temps, elle rapporte `NotFound`, sans écriture au trousseau ni
  relance de la synchronisation. La garde (`Reconnected::held`) couvre
  la relance de la synchronisation ; une suppression demandée ensuite
  l'attend puis l'arrête. Un emplacement de jetons oublié
  (`TokenCache::discard`) reste marqué, sans jeton : défense en
  profondeur contre une écriture tardive.
- Identifiant client public embarqué (application de bureau). Microsoft :
  sans secret. Google : identifiant et `client_secret` du client
  « application de bureau » fournis à la compilation (secrets de la CI,
  jamais dans le dépôt ; ADR-013), masqués dans `Debug` ; le secret n'est
  pas confidentiel (même valeur
  dans chaque copie de l'application, toléré par Google pour les
  applications installées) et ne protège rien : le code d'autorisation
  reste protégé par PKCE et `state`, et le jeton n'est présenté qu'aux
  serveurs de Gmail. Jeton de rafraîchissement dans le trousseau seulement ; jeton
  d'accès en mémoire ; aucun jeton dans les journaux (fichier `norelly.log`
  du dossier des données, borné, niveau INFO au plus, TRACE supprimé à la
  compilation), les erreurs ni
  `Debug` ; le corps des réponses du fournisseur n'est jamais recopié, et
  un code d'erreur n'est repris que s'il est court et imprimable.
- Point d'accès des jetons en HTTPS, vérificateur de la plateforme, sans
  redirection ; rafraîchissement borné à 30 s, un verrou par compte.
- Ajout de compte échoué après la connexion chez le fournisseur : jetons
  gardés en mémoire seulement (jamais dans le trousseau ni sur disque), un
  seul ajout à la fois, rendus une seule fois et pour la même adresse et le
  même fournisseur, dix minutes au plus.

### Autodécouverte

HTTPS uniquement ; seul le domaine est transmis à l'ISPDB ; serveur SRV hors
du domaine de l'adresse soumis à confirmation ; XML hostile sans panique
(fuzzé) (`crates/domain/src/discovery.rs`, `crates/adapters/discovery`).

### Rendu des messages (ADR-004, « A+ »)

- HTML assaini par liste blanche (`ammonia`), CSS assaini (échappements
  résolus, `@import`, `@custom-media`, `@page` et `url()` externes
  retirés ; requêtes de média alignées sur l'écran : `print` et les
  caractéristiques qui changent sur le papier (`update`, `device-*`,
  résolution, couleurs, `prefers-color-scheme`) jamais vraies, `screen`
  toujours vraie ; `print-color-adjust` imposé par Norelly, fonds imprimés
  comme à l'écran : un mail ne peut rien montrer seulement à
  l'impression) ; suite de 56 vecteurs
  hostiles (`crates/adapters/render/tests/hostile_html.rs`).
- Cadre `sandbox="allow-scripts"` sans `allow-same-origin` (origine opaque),
  servi par `mailview://` avec une CSP stricte ; seul l'agent s'exécute
  (empreinte SHA-256 vérifiée).
- Messages `postMessage` de l'agent validés par l'interface (source, type,
  taille, schéma).
- Images distantes bloquées par défaut ; proxy `remote-img://` sans cookies
  ni `Referer`, adresses publiques seulement, taille et type bornés,
  autorisation par expéditeur revérifiée en Rust à chaque image.
- Liens : `http`, `https`, `mailto` seulement, ouverts par Rust dans le
  navigateur du système ; navigation de la webview bloquée
  (`navigation_guard`).
- Pièces jointes : nom assaini par le domaine, ouverture directe réservée à
  une liste blanche d'extensions, écriture par création exclusive dans le
  dossier choisi.
- Impression : `window.print()` de la fenêtre principale, lancé par
  l'interface seulement (l'agent ne relaie aucune demande d'impression,
  test `agent_cannot_print`) ; aucune commande IPC, aucune capability, cadre
  inchangé (sans `allow-modals`), aucune requête de plus. La feuille
  `@media print` masque tout sauf la conversation ouverte
  (`data-print-root`), y compris les fenêtres, menus et notifications
  affichés par-dessus.
- Interface : jamais de `{@html}`, d'`innerHTML` ni d'`eval`.

### Expéditeur et carnet d'adresses

L'adresse `From` est falsifiable : le nom du carnet s'affiche d'après elle,
mais la pastille « Dans vos contacts » n'est accordée qu'à un expéditeur
authentifié par le serveur du compte (`crates/domain/src/sender_auth.rs`,
décision du mainteneur du 2026-11-16) :

- seul le **premier** `Authentication-Results` du message compte (le
  dernier posé, par le serveur de remise) ; il doit venir d'un
  `authserv-id` du domaine de l'hôte IMAP du compte (sans sa première
  étiquette, jamais un deuxième niveau de pays comme `co.uk` ou `asso.fr`),
  ou d'un alias constant du fournisseur (`mx.google.com` pour Gmail). Un en-tête du
  dessus étranger, sans `authserv-id` (forme de Microsoft 365), trop long ou
  illisible ne laisse jamais décider un en-tête plus bas : verdict
  `Unknown` ;
- `Pass` : un unique résultat DMARC `pass` dont `header.from` est le domaine
  du `From`, ou, sans résultat DMARC, une signature DKIM valide d'un
  domaine aligné ; `Fail` : un DMARC `fail` sur ce domaine (il l'emporte
  sur tout `pass` injecté) ; plusieurs champs `From`, `From` invalide ou
  domaine internationalisé non convertible : `Unknown` ;
- l'interface n'affiche que l'état reçu (`CorrespondentDto.contact_badge`,
  `auth`) ; les messages enregistrés avant la migration 20 valent
  `Unknown`.

Risques résiduels, acceptés avec la décision du mainteneur :

- un message qui n'est jamais passé par un serveur posant
  `Authentication-Results` (ajouté par `APPEND` par un autre client ou un
  outil d'import, hébergeur sans vérification DKIM ni DMARC, envoi interne
  non vérifié) est jugé sur un en-tête que l'expéditeur a pu écrire avec
  l'`authserv-id` attendu ; la RFC 8601 (§ 5) impose au serveur de retirer
  ces en-têtes, ce que tous ne font pas ;
- un serveur de confiance qui recopie sans échappement une donnée de
  l'expéditeur dans son en-tête peut y laisser injecter un résultat ; un
  `fail` l'emporte et plusieurs résultats DMARC ne valent rien, mais un
  résultat DKIM injecté reste possible sans résultat DMARC ;
- les comptes Microsoft 365 et Outlook.com, dont l'en-tête n'a pas
  d'`authserv-id`, restent en `Unknown` (ni pastille, ni avertissement).

### Données

- SQL : paramètres liés uniquement (`scripts/check-sql.sh`) ; requêtes de
  recherche traduites en chaînes FTS5 citées, en paramètre lié ; tests
  `hostile_message_sql.rs` avec des charges dans chaque champ stocké.
- Tout texte venu du serveur est assaini et borné à l'entrée du domaine
  (`mailbox.rs`, `conversation.rs`).
- Bornes partout : profondeur MIME, nombre de parties, tailles, nombre de
  clauses, de destinataires, de pièces jointes, d'opérations en attente,
  durée d'une recherche (2 s).
- Suppression d'un dossier : refusée s'il a un sous-dossier, d'après la
  liste du compte puis un `LIST "<parent><délim>%"` relu juste avant
  `DELETE`, où tout nom illisible ou refusé compte
  (`RemoteMailboxSession::count_subfolders`, `no_listed_subfolder`).
  Limite propre à IMAP, sans commande atomique : un sous-dossier créé par
  un autre client entre ce `LIST` et `DELETE` n'est pas vu.
- Renommage et déplacement d'un dossier : refusés s'il contient un dossier
  spécial ou s'il irait sous lui-même, à la casse près et selon le
  séparateur de l'un ou de l'autre (`relocatable`, `is_same_or_under`).

### Rédaction

HTML de l'éditeur réassaini en Rust (ADR-010) ; `Bcc` jamais transmis ;
`Message-ID` aléatoire sans nom de machine ; chemins de fichiers jamais
reçus de l'interface ; types MIME recalculés.

### Application Tauri

- Liste de contrôle d'accès : les commandes ne sont accordées qu'à la
  fenêtre `main`, depuis l'origine de l'application (`ipc-commands.rs`,
  `permissions/main-window.toml`, test `acl_tests`).
- Aucune permission de plugin accordée à l'interface : dialogues et mises à
  jour appelés depuis Rust.
- Toute entrée IPC revalidée par les constructeurs du domaine.
- `unsafe` interdit dans tout le workspace.

### Chaîne d'approvisionnement et distribution

- `cargo deny` : licences (ADR-009) et avis RUSTSEC ; exceptions commentées
  et validées par le mainteneur dans `deny.toml`.
- Versions figées côté front ; actions GitHub et images Docker figées par
  empreinte.
- Mises à jour (NSIS) signées **minisign**, signature faite dans un
  environnement de publication séparé de la compilation ; délais bornés
  (`docs/CODE_SIGNING.md`).
- Flatpak : autorisations minimales.
- Caches de la CI enregistrés depuis `master` seulement, jamais utilisés
  par `release.yml`. Cache de build de l'e2e réduit à `target/` et aux
  archives `.crate` (aucune configuration de cargo, refusée aussi par
  `app/e2e/run.sh`) ; image de l'e2e renouvelée chaque semaine et
  acceptée seulement sous son tag attendu. Risque résiduel accepté : un
  `target/` empoisonné depuis `master` (build script compromis) servirait
  jusqu'à la semaine suivante, dans un job sans secret.

## Fichiers sensibles

Signalés au mainteneur dans chaque PR qui les touche (ADR-008) :

- `crates/domain/src/security.rs`, `discovery.rs`, `reading.rs`,
  `credentials.rs`, `oauth.rs`, `auth.rs`, `sender_auth.rs` ;
- `crates/adapters/oauth/src/lib.rs`, `crates/application/src/oauth.rs` ;
- `crates/adapters/tls/src/lib.rs`, `crates/adapters/imap/src/connection.rs`,
  `crates/adapters/smtp/src/stream.rs` ;
- `crates/adapters/render/src/sanitize.rs`, `css.rs`, `compose.rs`,
  `agent.js` ;
- `crates/adapters/contacts-io/src/` (lecture et écriture de fichiers
  externes), `crates/domain/src/contact_import.rs`, `contact_export.rs` ;
- `app/src-tauri/src/protocols.rs`, `remote_images.rs`, `reading.rs`
  (pièces jointes), `contact_import.rs` (fichier importé),
  `contact_export.rs` (fichier exporté), `ipc.rs`,
  `capabilities/`, `permissions/` ;
- `app/src/lib/agent.ts` ;
- infrastructure : `deny.toml`, `.github/workflows/*.yml`,
  `packaging/flatpak/*.yml`, `scripts/check.sh`.

## Revues

Toute modification d'une zone sensible passe par une revue de l'agent
`security-reviewer` (`.claude/agents/security-reviewer.md`) ; les constats
critiques et hauts sont corrigés avant la PR, les autres reportés dans le
suivi des sujets ouverts, avec leur gravité.

Signaler une vulnérabilité : voir [SECURITY.md](../../SECURITY.md)
(signalement privé GitHub), jamais dans une issue publique.
