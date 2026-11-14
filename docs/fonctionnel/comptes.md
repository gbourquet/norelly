# Comptes : ajout, configuration, retrait

[← Fonctionnalités](README.md)

## But

Ajouter une adresse électronique avec le moins de saisie possible (adresse
et mot de passe), sans jamais affaiblir la sécurité de la connexion pour y
parvenir ; retirer un compte proprement.

## Comportement attendu

### Ajout

Depuis « Ajouter un compte », toujours visible en bas du panneau de
navigation, ou depuis l'accueil quand il n'y a aucun compte.

1. L'utilisateur saisit l'**adresse**, le **mot de passe** et,
   facultativement, un **nom affiché**, puis **Continuer**.
2. Norelly cherche les serveurs (progression affichée, source par source).
3. Si les serveurs de réception (IMAP) et d'envoi (SMTP) sont trouvés par
   une source sûre, Norelly **teste la connexion** (chiffrement,
   identifiants) aux deux serveurs, puis enregistre le compte : le mot de
   passe va dans le trousseau du système. La synchronisation démarre
   aussitôt.
4. Sinon, ou en cas d'échec, l'écran de **saisie manuelle** s'affiche,
   prérempli avec ce qui a été trouvé : serveur, port, sécurité (TLS,
   STARTTLS, aucune) et utilisateur, pour la réception et pour l'envoi.
5. Le test peut s'arrêter sur un écran de décision :
   - **certificat non reconnu** : raison, serveur, empreinte du
     certificat ; « Faire confiance à ce serveur » si le problème le
     permet ;
   - **connexion non chiffrée** : consentement explicite pour ce serveur et
     ce port ;
   - **aucun trousseau** sur le système (certaines installations Linux) :
     consentement pour un fichier de mots de passe non chiffré ;
   - **serveur situé hors du domaine** de l'adresse : case à cocher de
     confirmation.

### Se connecter avec Microsoft

Pour une adresse Outlook.com, Hotmail, Live ou MSN, le formulaire ne
demande pas de mot de passe : il propose **Se connecter avec Microsoft**.

1. Le navigateur du système s'ouvre sur la page de Microsoft ; l'utilisateur
   s'y connecte et accepte l'accès de Norelly.
2. De retour dans Norelly, la réception et l'envoi sont testés, puis le
   compte est enregistré et la synchronisation démarre.
3. Si l'autorisation cesse de fonctionner (mot de passe Microsoft changé,
   accès retiré), le compte affiche « Reconnexion nécessaire » et un
   bouton **Se reconnecter**, visible à côté du compte et présent dans son
   menu « ⋯ ». La réception reprend après la reconnexion.

### Se connecter avec Google

Pour une adresse Gmail (`gmail.com`), le formulaire propose **Se connecter
avec Google**, sur le même parcours que Microsoft, sans mot de passe.

1. Avant d'ouvrir le navigateur, l'écran explique l'avertissement que
   Google affiche tant qu'il n'a pas validé Norelly (« Google n'a pas
   validé cette application ») et le chemin pour le passer : « Paramètres
   avancés », puis « Accéder à Norelly (non sécurisé) ». L'explication est
   rappelée pendant l'attente du navigateur.
2. L'utilisateur se connecte chez Google et accepte l'accès de Norelly ; la
   réception et l'envoi sont testés, puis le compte est enregistré.
3. Le **mot de passe d'application** reste possible : le bouton **Utiliser
   un mot de passe d'application** affiche le champ du mot de passe, et
   **Se connecter avec Google plutôt** y revient.
4. La reconnexion d'un compte Google se fait comme pour Microsoft
   (« Se reconnecter »).

### Retrait

Depuis le menu « ⋯ » du compte (panneau de gauche, entrée « Supprimer ce
compte… ») ou la commande « Supprimer le compte sélectionné… », après
confirmation : le compte cesse
de se synchroniser, son mot de passe est effacé du trousseau, ses données
locales (messages, actions et envois en attente, brouillons, certificats
acceptés) sont supprimées. **Rien n'est supprimé sur le serveur.**

## Règles

- **Adresse** : vérifiée strictement. Les caractères accentués sont admis
  avant l'arobase ; les adresses entre guillemets et les adresses IP à la
  place du domaine sont refusées.
- **Recherche des serveurs**, dans l'ordre, jusqu'à trouver la réception et
  l'envoi :
  1. configuration publiée par le fournisseur (en HTTPS) ;
  2. base de configurations de Mozilla (en HTTPS ; seul le domaine de
     l'adresse est transmis) ;
  3. annonces DNS du domaine ;
  4. essai des noms usuels (`imap.`, `smtp.`, `mail.`, le domaine lui-même)
     sur les ports usuels ;
  5. à défaut, saisie manuelle.
- **Sécurité préférée** : TLS dès la connexion, puis STARTTLS, puis aucune.
- Aucune configuration n'est lue sans chiffrement. Un serveur annoncé hors
  du domaine de l'adresse demande une confirmation.
- **Certificat** : accepté s'il est reconnu par le système, ou si
  l'utilisateur lui a fait confiance (serveur et empreinte précis). Sinon,
  la connexion est refusée et la raison affichée. Une empreinte différente
  de celle acceptée est signalée comme un changement.
- **Connexion non chiffrée** : refusée sans consentement explicite, serveur
  par serveur. Si le serveur annonce STARTTLS, ou si l'utilisateur l'a
  choisi, le chiffrement est obligatoire : jamais de repli en clair.
- **Identifiants** : envoyés uniquement sur une connexion chiffrée, sauf
  consentement explicite à une connexion non chiffrée.
- **Mot de passe** : il est conservé dans le trousseau du système (Windows :
  Gestionnaire d'identification ; Linux : GNOME Keyring, KWallet…), jamais
  dans les données de l'application, ni affiché, ni écrit dans les
  journaux.
- **Sans trousseau** : les mots de passe peuvent être gardés dans un fichier
  non chiffré, lisible par le seul utilisateur, et seulement après son
  consentement. Le trousseau reste prioritaire. Dans ce cas, la mention
  « conservé dans le trousseau du système » n'apparaît pas à l'ajout :
  l'écran n'annonce que des garanties vraies.

## Cas limites

- Adresse déjà enregistrée : refus.
- Serveurs trouvés en partie (réception sans envoi, par exemple) : saisie
  manuelle préremplie.
- Certificat non acceptable (révoqué, signature invalide) : pas de bouton
  de confiance.
- Changement d'empreinte d'un serveur déjà accepté : avertissement renforcé
  et case « J'ai vérifié auprès de l'administrateur » obligatoire.
- Annulation pendant le test : retour à la saisie.
- Gmail passe par « Se connecter avec Google » ou par un mot de passe
  d'application, jamais par le mot de passe habituel. Outlook.com et
  Hotmail ne passent que par « Se connecter avec Microsoft ». Une adresse
  d'entreprise ou d'école gérée par Google ne se reconnaît pas à son nom :
  elle passe par le mot de passe.
- Connexion chez Google refusée, abandonnée ou restée sans réponse dans le
  navigateur, à l'ajout d'un compte : Google peut bloquer les nouveaux
  comptes tant qu'il n'a pas validé Norelly (nombre d'utilisateurs limité),
  sans toujours le dire à Norelly. L'écran revient au formulaire, l'explique
  et bascule sur le mot de passe d'application, avec ses étapes ; rien
  n'est enregistré.
- Accès aux messages décoché dans la page de Google : le compte ne pourrait
  ni recevoir ni envoyer. Il n'est pas ajouté (ni reconnecté), et l'écran
  demande de recommencer en cochant toutes les autorisations.
- Compte choisi chez Google dont l'adresse n'est pas vérifiée par Google,
  ou connexion qui n'a pas été faite pour Norelly : refusée, comme un autre
  compte.
- Version de Norelly qui ne peut pas se connecter à Google (version non
  officielle) : « Se connecter avec Google » n'est pas proposé, une adresse
  Gmail passe par le mot de passe d'application. Un compte Google déjà
  enregistré n'envoie plus rien à Google : il affiche que cette version ne
  peut pas se connecter à ce fournisseur, sans nouveaux essais ni
  reconnexion.
- Adresse modifiée juste avant « Se connecter avec … » : Norelly vérifie le
  fournisseur de l'adresse finale avant d'ouvrir le navigateur.
- Connexion chez Microsoft refusée ou abandonnée dans le navigateur : retour
  au formulaire, avec une explication ; rien n'est enregistré.
- Compte choisi dans la page de Microsoft différent de l'adresse saisie
  (erreur de compte, ou retour de connexion substitué par un autre
  programme) : refusé, à l'ajout comme à la reconnexion. Rien n'est
  enregistré, l'autorisation précédente d'un compte reconnecté reste en
  place, et l'écran demande de choisir le compte de l'adresse.
- Autorisation de rester connecté refusée dans la page de Microsoft : le
  compte cesserait de fonctionner au bout d'une heure. Il n'est pas
  ajouté (ni reconnecté), et l'écran demande d'accepter toutes les
  autorisations demandées.
- Envoi désactivé par Microsoft sur la boîte, alors que la réception
  fonctionne : le compte n'est pas ajouté et l'écran l'explique.
- Connexion Microsoft acceptée mais boîte inaccessible en réception (refus
  parfois passager chez Microsoft) : Norelly réessaie plusieurs fois ; si
  le refus persiste, il demande de vérifier que l'accès IMAP est activé
  dans Outlook.com, sans parler de mot de passe.
- Réseau instable, ou serveur joint mais qui ne répond pas ou coupe la
  connexion : Norelly réessaie seul, sur une nouvelle connexion, avant
  d'annoncer un échec. Un refus du serveur (identifiants, sécurité) ne se
  réessaie pas.
- Échec de la connexion après le passage chez Microsoft (serveur muet,
  réseau instable) : un nouvel essai pour la même adresse, dans les dix
  minutes, réutilise cette connexion sans rouvrir le navigateur. Elle n'est
  gardée qu'en mémoire, jamais enregistrée, et ne sert qu'une fois.
- Compte Microsoft : réception vérifiée, mais serveur d'envoi de Microsoft
  qui ne répond pas ou coupe la connexion : le compte est ajouté quand
  même, et un message dit que l'envoi n'a pas pu être vérifié ; il le sera
  au premier message. Un refus ou une protection empêchent toujours
  l'ajout. Pour un compte ajouté avec un mot de passe, l'envoi doit être
  vérifié : un port ou un chiffrement faux fait lui aussi taire le serveur.
- Serveur qui ne répond pas : le message ne met en cause ni le nom ni le
  port, il invite à vérifier la connexion à Internet puis à réessayer.
- L'autorisation Microsoft n'est jamais envoyée à un autre serveur que ceux
  de Microsoft, ni sur une connexion non chiffrée. De même, l'autorisation
  Google ne part que vers les serveurs de Gmail.
- Trousseau du système verrouillé ou indisponible au moment de renouveler
  l'autorisation Microsoft : le compte indique un problème de trousseau, et
  non des identifiants refusés ; rien n'est envoyé au serveur, et Norelly
  réessaie plus tard. Un compte lent chez Microsoft ou dont le trousseau
  tarde à répondre ne retarde pas les autres comptes.
- Microsoft injoignable, trop lent ou en panne au moment de renouveler
  l'autorisation : le compte indique que la connexion au fournisseur est
  impossible, et non des identifiants refusés ; Norelly réessaie plus tard,
  et le renouvellement ne dure jamais plus d'une demi-minute.
- Compte supprimé : les autorisations gardées en mémoire sont oubliées
  avec lui ; elles ne servent plus et ne sont plus enregistrées.
- Trousseau qui refuse d'enregistrer l'autorisation renouvelée : le compte
  continue de fonctionner tant que Norelly est ouvert, et l'enregistrement
  est retenté au renouvellement suivant.
- Ordinateur sorti de veille : une autorisation expirée pendant la veille
  est renouvelée avant la connexion suivante ; une autorisation refusée par
  le serveur est renouvelée une fois avant d'annoncer un échec.
- Compte Google refusé (compte dont les serveurs de réception et d'envoi
  sont tous deux ceux de Google, pour une adresse Gmail comme pour une
  adresse d'entreprise) : Norelly explique
  comment créer un mot de passe d'application, en trois étapes, avec un
  bouton qui ouvre la page de Google. Le champ du mot de passe est vidé
  pour accueillir le nouveau code ; l'adresse, le nom et les serveurs
  saisis sont gardés. Un refus d'une autre nature (méthode de connexion
  impossible) n'affiche pas cette aide. L'aide disparaît dès qu'un des
  serveurs est modifié.
- Adresse Gmail dont l'un des serveurs saisis à la main n'est pas celui de
  Google, connexion non chiffrée, ou certificat accepté malgré l'alerte :
  un refus affiche le message habituel « identifiants refusés », sans
  l'aide au mot de passe d'application. Ce mot de passe, qui contourne la
  validation en deux étapes, partirait vers les deux serveurs : il n'est
  conseillé que si les deux sont ceux de Google, joints de façon sûre.
- Serveur limité à d'anciennes versions de TLS (1.0, 1.1) : connexion
  refusée par sécurité.

## Messages affichés

| Situation | Texte |
|---|---|
| Recherche des serveurs | Recherche des paramètres de {address}… |
| Test de connexion | Test de la connexion à {imap} et {smtp}… |
| Serveurs trouvés en partie | Les serveurs n'ont pas tous été trouvés automatiquement : vérifiez ou complétez les paramètres. |
| Serveur hors du domaine | Serveur trouvé par le DNS en dehors du domaine de votre adresse : {hosts}. … |
| Certificat refusé | Certificat non reconnu, avec la raison : autorité inconnue, expiré, pas encore valide, nom différent, révoqué, signature invalide, illisible, autre ; un certificat autosigné, cas courant d'un serveur personnel, est toujours présenté comme « autorité inconnue » |
| Certificat changé | ce serveur présente un certificat différent de celui que vous aviez accepté. … |
| Connexion non chiffrée | La connexion à {host}:{port} ne serait pas chiffrée : … |
| Aucun trousseau | Aucun trousseau du système … ne répond. … fichier non chiffré … |
| Identifiants refusés | {prefix}identifiants refusés. … (un mot de passe d'application est parfois nécessaire). |
| Compte Google refusé (serveurs de Google seulement) | {prefix}identifiants refusés : un compte Google demande un mot de passe d'application. Puis « Créer un mot de passe d'application Google » : validation en deux étapes, création du mot de passe (« Ouvrir la page Google »), collage à la place du mot de passe habituel. |
| Serveur injoignable | {prefix}serveur injoignable. Vérifiez le nom et le port. |
| Serveur qui ne répond pas | {prefix}le serveur ne répond pas. Vérifiez votre connexion à Internet, puis réessayez. |
| Adresse Microsoft | Outlook.com et Hotmail n'acceptent pas de mot de passe ici : connectez-vous chez Microsoft dans votre navigateur. Norelly ne voit jamais votre mot de passe. |
| Attente du navigateur | Connectez-vous dans la fenêtre de votre navigateur, puis revenez ici. |
| Nouvel essai sans navigateur | Connexion à votre messagerie Microsoft… (Google : « Connexion à votre messagerie Gmail… ».) |
| Adresse Gmail | Connectez-vous chez Google dans votre navigateur : Norelly ne voit jamais votre mot de passe. Google affichera d'abord « Google n'a pas validé cette application » : cliquez sur « Paramètres avancés », puis sur « Accéder à Norelly (non sécurisé) ». Norelly n'a simplement pas encore été examinée par Google. |
| Attente du navigateur (Google) | Connectez-vous dans la fenêtre de votre navigateur, puis revenez ici. Si Google affiche « Google n'a pas validé cette application », cliquez sur « Paramètres avancés », puis sur « Accéder à Norelly (non sécurisé) ». |
| Connexion Google refusée ou abandonnée | La connexion avec Google n'a pas abouti. Si Google indique que Norelly a atteint son nombre maximal d'utilisateurs (100, tant que Google n'a pas validé l'application), ou si vous préférez ne pas passer par cette page, utilisez un mot de passe d'application : les étapes sont ci-dessous. |
| Envoi non vérifié à l'ajout | Compte ajouté. L'envoi n'a pas pu être vérifié, le serveur ne répondait pas : Norelly réessaiera à votre premier message. |
| Connexion annulée | La connexion a été annulée dans le navigateur. |
| Autre compte choisi | Le compte choisi dans le navigateur n'est pas celui de cette adresse. Réessayez en choisissant le compte de {address}. (Reconnexion : « autre compte choisi dans le navigateur ».) |
| Boîte inaccessible (Microsoft) | {prefix}votre compte Microsoft est bien connecté, mais Outlook.com refuse l'accès à la boîte. Vérifiez que l'accès IMAP est activé dans Outlook.com (Paramètres, Courrier, Transfert et IMAP), puis réessayez dans quelques minutes. |
| Fournisseur injoignable | Connexion au fournisseur impossible. Vérifiez votre connexion à Internet, puis réessayez. (Compte déjà ajouté : « connexion au fournisseur impossible ».) |
| Rester connecté refusé | Norelly n'a pas reçu l'autorisation de rester connecté à votre compte. Réessayez en acceptant toutes les autorisations demandées dans la page de connexion. (Reconnexion : « autorisation de rester connecté refusée ».) |
| Accès aux messages refusé | Norelly n'a pas reçu l'autorisation d'accéder à vos messages. Réessayez en cochant toutes les autorisations demandées dans la page de connexion. (Compte déjà ajouté : « accès aux messages refusé ».) |
| Fournisseur absent de cette version | Cette version de Norelly ne peut pas se connecter à ce fournisseur. Installez la version publiée de Norelly ou, pour Gmail, utilisez un mot de passe d'application. (Compte déjà ajouté : « connexion à ce fournisseur indisponible dans cette version ».) |
| Envoi désactivé (Microsoft) | {prefix}l'envoi de messages est désactivé sur ce compte Microsoft. Activez-le dans les réglages de votre compte Outlook.com, puis réessayez. |
| Reconnexion nécessaire | Bouton « Se reconnecter » à côté du compte ; infobulle « Reconnexion nécessaire : {address} » ; « Compte reconnecté. » après succès. |
| Refus de sécurité | {prefix}connexion refusée par sécurité. |
| Compte existant | Un compte existe déjà pour cette adresse. |
| Retrait | … son mot de passe est effacé du trousseau et ses données locales sont supprimées. Les messages restent sur le serveur. |

`{prefix}` vaut « Serveur de réception (IMAP) : » ou « Serveur d'envoi
(SMTP) : ».

## Raccourcis

| Commande | Raccourci |
|---|---|
| Ajouter un compte… | palette |
| Supprimer le compte sélectionné… | palette |
