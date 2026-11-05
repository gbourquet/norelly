# Glossaire

[← Fonctionnalités](README.md)

Termes de Norelly, dans l'ordre alphabétique. Entre parenthèses : le nom
dans le code quand il diffère.

**Action** (`MessageAction`) : opération de rangement demandée par
l'utilisateur (archiver, supprimer, déplacer, suivre, lu, non lu). Voir
[actions](actions.md).

**Agent** : seul script autorisé dans le cadre d'un message
(`crates/adapters/render/src/agent.js`) ; il transmet la hauteur, les clics
sur les liens et les touches à l'interface par `postMessage`.

**Application optimiste** : l'effet d'une action est appliqué localement
tout de suite, avant la confirmation du serveur, et défait en cas de
conflit.

**Archiver** : déplacer vers le dossier d'archives (`\Archive`, ou le
dossier de repli choisi).

**Autodécouverte** : recherche automatique des serveurs IMAP et SMTP à
partir de l'adresse. Voir [comptes](comptes.md).

**Blob** : message brut (`.eml`) compressé (`.zst`), stocké hors de la base,
par compte et par empreinte de contenu.

**Boîte (aux lettres)** (`Mailbox`) : dossier IMAP. **Rôle** : sa fonction
(réception, envoyés, brouillons, corbeille, indésirables, archives, tous les
messages), lue dans les attributs SPECIAL-USE ou devinée d'après le nom.

**Boîte d'envoi** : vue de la **file d'envoi** d'un compte.

**Boîte (de réception) unifiée** : liste des conversations des boîtes de
réception de tous les comptes.

**Brouillon** (`Draft`) : message en cours d'écriture, enregistré
localement et copié dans le dossier « Brouillons » du serveur. Voir
[brouillons](brouillons.md).

**Cible** : conversations visées par une action (cochées, ouverte, ou
pointée).

**Commande** : action de l'interface inscrite au registre de commandes,
avec un identifiant (`action.archive`), un libellé et éventuellement un
raccourci. Voir [clavier](clavier.md).

**Compte** (`Account`) : une adresse électronique, son nom affiché et ses
serveurs IMAP et SMTP. Le mot de passe n'en fait pas partie.

**Conversation** : ensemble des messages qui se répondent, regroupés par
l'algorithme JWZ. Voir [conversations](conversations.md).

**Copie provisoire** : copie locale d'un message déplacé, visible dans le
dossier de destination avant la confirmation du serveur.

**Empreinte** (`CertificateFingerprint`) : SHA-256 du certificat d'un
serveur, affichée pour décider de lui faire confiance.

**Envoi différé** : envoi programmé à une date et une heure.

**Envoi incertain** (`SendFailure::Uncertain`) : envoi interrompu au moment
de la transmission ; peut-être parti, jamais renvoyé automatiquement.

**Exception de certificat** : confiance accordée par l'utilisateur à un
certificat non reconnu, pour un hôte et une empreinte précis.

**File d'actions** (`pending_ops`) : opérations en attente d'envoi au
serveur, durables, rejouées à la reconnexion.

**File d'envoi** (`outbox`) : messages en attente d'envoi SMTP, durables,
avec leur état (en attente, en cours, envoyé, échec, programmé). Voir
[envoi](envoi.md).

**Hors ligne** : état d'un compte dont le serveur ne répond pas ; l'icône
Wi-Fi barré s'affiche à côté du compte.

**IDLE** : extension IMAP par laquelle le serveur signale aussitôt les
changements de la boîte de réception.

**Mot de passe d'application** : mot de passe dédié à un programme, créé
sur le site du fournisseur (Gmail, Outlook.com, Fastmail).

**Palette (de commandes)** : fenêtre ouverte par `Ctrl+K` pour chercher et
exécuter n'importe quelle commande.

**Pièce jointe différée** : pièce au-dessus du seuil de rapatriement,
téléchargée seulement à l'ouverture.

**Port** (architecture) : trait du domaine implémenté par un adaptateur.
Voir [la documentation technique](../technique/architecture.md).

**Rapatriement** (`fetch_policy`) : ce qui est téléchargé pendant la
synchronisation ; les pièces jointes au-dessus d'un **seuil** sont
différées.

**Rejeu** : envoi au serveur des opérations de la file d'actions.

**Suivre** : poser le drapeau `\Flagged` (« étoile », « drapeau »).

**Synchronisation** : rapprochement de la copie locale et du serveur. Voir
[synchronisation](synchronisation.md).

**Trousseau** (`SecretStore`) : coffre-fort à mots de passe du système
(Gestionnaire d'identification Windows, Secret Service sous Linux : GNOME
Keyring, KWallet).

**UID, UIDVALIDITY, MODSEQ** : identifiants IMAP d'un message dans une
boîte, de la « génération » de la boîte et de la version de ses drapeaux.
Un changement d'UIDVALIDITY oblige à tout resynchroniser.
