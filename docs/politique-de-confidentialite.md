# Politique de confidentialité

*[English version](privacy-policy.md)*

Norelly est un client de messagerie qui s'exécute localement sur votre
appareil. Cette politique décrit les données que Norelly traite et la
manière dont elles sont traitées. Elle couvre l'application Norelly et son
site, norelly.fr.

## Données stockées sur votre appareil uniquement

- Vos messages, pièces jointes et carnets d'adresses sont stockés
  localement (base de données et fichiers sur votre machine).
- Vos mots de passe et jetons de session (OAuth) sont conservés dans le
  trousseau sécurisé de votre système d'exploitation. Si votre système
  n'a pas de trousseau, Norelly vous propose de les garder dans un fichier
  sur votre appareil, lisible par vous seul et non chiffré ; il ne le fait
  qu'avec votre accord.
- Aucune donnée n'est transmise aux développeurs de Norelly : pas de
  télémétrie, pas de statistiques d'usage, pas de publicité.

## Connexions à vos fournisseurs de messagerie

Norelly se connecte directement aux serveurs IMAP et SMTP de vos
fournisseurs de messagerie, ainsi qu'à leurs services d'authentification
OAuth (Microsoft, Google) lorsque vous le demandez. Ces connexions sont
chiffrées (TLS) et n'ont lieu qu'entre votre appareil et ces serveurs. Si
un serveur ne propose pas le chiffrement, Norelly refuse de s'y connecter,
sauf si vous l'autorisez vous-même expressément pour ce compte.

## Autres connexions

Norelly ne se connecte à rien d'autre, sauf dans les cas suivants, tous
liés à une action ou à un choix de votre part :

- **Ajout d'un compte** : pour trouver les réglages de votre messagerie,
  Norelly interroge le site de votre fournisseur, puis, s'il ne répond
  pas, la base de configurations de Mozilla (autoconfig.thunderbird.net),
  à qui seul le nom de domaine de votre adresse est transmis (par exemple
  « exemple.fr »), jamais l'adresse complète. Il consulte aussi les
  annonces DNS de ce domaine et essaie les noms de serveurs habituels de
  votre fournisseur (par exemple « imap.exemple.fr »).
- **Carnet d'adresses** : si vous le reliez à un serveur de contacts,
  Norelly s'y connecte, avec l'adresse et le mot de passe que vous avez
  saisis.
- **Images d'un message** : les images hébergées ailleurs sont bloquées
  par défaut. Si vous les autorisez pour un expéditeur, elles sont
  chargées depuis le site qui les héberge, sans cookies.
- **Mises à jour** (installateur Windows) : si vous l'acceptez, Norelly
  vérifie auprès de GitHub, où il est publié, si une nouvelle version
  existe.

## Comptes Google et Microsoft

Quand vous choisissez « Se connecter avec Google » ou « Se connecter avec
Microsoft », Norelly vous demande, sur la page de Google ou de Microsoft,
les seuls accès dont il a besoin pour faire ce qu'un client de messagerie
fait :

- **Google** : l'accès à votre messagerie Gmail, pour recevoir, afficher,
  ranger, supprimer et envoyer vos messages, et votre adresse
  électronique, pour vérifier que le compte choisi est bien celui que vous
  ajoutez.
- **Microsoft** : la réception (IMAP) et l'envoi (SMTP) des messages de
  votre boîte Outlook.com (ou Hotmail, Live), le maintien de la connexion
  sans vous redemander votre mot de passe, et votre adresse électronique,
  pour vérifier que le compte choisi est bien celui que vous ajoutez.
  Microsoft transmet aussi votre nom avec cette autorisation ; Norelly ne
  s'en sert pas.

Norelly ne demande l'accès ni à vos contacts, ni à votre agenda, ni à vos
fichiers chez Google ou Microsoft.

Ces données servent uniquement à vous montrer, ranger, supprimer et
envoyer vos messages dans Norelly. Elles restent sur votre appareil :

- elles ne sont ni vendues, ni transmises à des tiers, ni envoyées aux
  développeurs de Norelly ;
- elles ne servent jamais à de la publicité, ni à évaluer votre
  solvabilité ;
- personne ne les lit à votre place : aucune personne n'a accès à vos
  messages, sauf si vous choisissez vous-même de les lui envoyer (par
  exemple dans un signalement de problème) ;
- elles ne servent pas à entraîner ou à améliorer un modèle
  d'intelligence artificielle.

L'utilisation par Norelly des informations reçues des API Google, et leur
transfert à toute autre application, respectent la
[politique relative aux données utilisateur des services d'API Google](https://developers.google.com/terms/api-services-user-data-policy)
(*Google API Services User Data Policy*), y compris les exigences
d'utilisation limitée (*Limited Use*).

Vous pouvez retirer cet accès à tout moment : supprimez le compte dans
Norelly, qui efface alors l'autorisation de votre appareil, ou retirez
l'accès de Norelly dans les réglages de sécurité de votre compte Google ou
Microsoft.

## Site norelly.fr

Le site norelly.fr ne dépose aucun cookie et ne contient aucun traceur :
ni statistiques de visite, ni publicité, ni contenu chargé depuis un autre
site.

Le serveur du site ne tient aucun journal des visites. Il ne garde qu'un
journal des erreurs techniques (par exemple une demande impossible à
traiter), qui peut contenir l'adresse IP de l'ordinateur concerné et la
page demandée. Ce journal sert seulement à corriger le fonctionnement du
site, n'est communiqué à personne et est effacé automatiquement au plus
tard au bout de 11 jours.

## Suppression des données

Supprimer un compte dans l'application efface de votre appareil ses
messages, ses réglages et ses mots de passe ou autorisations enregistrés.

Désinstaller Norelly peut laisser en place son dossier de données. Pour
tout effacer, supprimez ce dossier après la désinstallation :

- sous Windows : `%APPDATA%\io.github.gbourquet.Norelly` ;
- sous Linux : `~/.local/share/io.github.gbourquet.Norelly`.

Les mots de passe rangés dans le trousseau du système sont effacés quand
vous supprimez le compte dans Norelly ; supprimez les comptes avant de
désinstaller.

## Éditeur et contact

Norelly est un logiciel libre développé par Guillaume Bourquet —
contact : contact@norelly.fr.
Code source : https://github.com/gbourquet/norelly

*Dernière mise à jour : 25 novembre 2026.*
