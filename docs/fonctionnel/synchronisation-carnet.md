# Synchroniser le carnet avec un serveur de contacts

[← Fonctionnalités](README.md)

## But

Retrouver ses contacts sur ses autres appareils (téléphone, autre
ordinateur, webmail) : Norelly tient le carnet d'adresses à jour avec
celui d'un serveur de contacts (CardDAV : Nextcloud, Fastmail, Synology,
Baïkal…), dans les deux sens. Un contact ajouté, modifié ou supprimé d'un
côté l'est aussi de l'autre.

## Comportement attendu

### Relier un carnet distant

- Accès, qui font tous la même chose :
  - **Synchroniser avec un serveur…** dans le menu **Plus…** de l'en-tête
    de la vue Contacts ;
  - palette de commandes (« Synchroniser avec un serveur… »).
- La fenêtre **Synchroniser le carnet** demande :
  - l'**adresse du serveur** : le nom du serveur suffit (par exemple
    `nextcloud.exemple.fr`), l'adresse complète du carnet est acceptée
    aussi ;
  - l'**identifiant** ;
  - le **mot de passe d'application**, créé dans les réglages de sécurité
    du fournisseur.
- **Rechercher les carnets** : Norelly trouve les carnets du compte sur le
  serveur et les propose ; le premier est choisi d'office, un autre se
  choisit d'un clic. Modifier l'adresse ou l'identifiant efface la liste :
  il faut rechercher de nouveau.
- **Synchroniser ce carnet** relie le carnet choisi et lance aussitôt la
  première synchronisation. Le mot de passe est gardé dans le trousseau du
  système, jamais ailleurs.

### Une fois relié

- La fenêtre montre le carnet relié (nom, identifiant, serveur) et l'état
  de la dernière synchronisation : date, fiches reçues et envoyées, fiches
  laissées de côté, ou la raison d'un échec.
- **Synchroniser maintenant** relance une synchronisation ; un message
  confirme le nombre de fiches reçues et envoyées, et la liste des contacts
  se met à jour.
- Le carnet est aussi synchronisé à chaque ouverture de l'application.
- **Déconnecter** oublie le serveur et le mot de passe ; les contacts
  restent dans Norelly, et rien n'est supprimé sur le serveur.

## Règles

- Un seul carnet distant, relié au carnet unique de Norelly, indépendant
  des comptes de messagerie.
- Connexion chiffrée seulement, et certificat du serveur valable : sinon
  rien n'est envoyé (pas d'exception possible, à la différence d'un compte
  de messagerie). Le mot de passe n'est envoyé qu'au serveur saisi, jamais
  à une autre adresse qu'il indiquerait.
- Modifiée d'un seul côté, une fiche suit de l'autre (création,
  modification, suppression).
- Modifiée des deux côtés depuis la dernière synchronisation : la
  modification **la plus récente** gagne. Une fiche supprimée ici puis
  modifiée sur le serveur plus tard revient ; un contact modifié ici après
  la suppression de sa fiche sur le serveur y est recréé.
- Première synchronisation : une fiche du serveur qui porte l'adresse d'un
  contact déjà dans Norelly lui est reliée, sans doublon ; les autres
  contacts partent sur le serveur, les autres fiches arrivent dans Norelly.
- Une fiche modifiée dans Norelly ne change sur le serveur que ce que
  Norelly connaît (noms, organisation, adresses, téléphones, notes) : la
  photo, l'adresse postale ou l'anniversaire saisis ailleurs sont gardés.
- Les favoris et les listes de diffusion restent propres à Norelly.
- Une fiche du serveur que Norelly ne peut pas reprendre (sans adresse
  électronique, adresse déjà portée par un autre contact) est laissée de
  côté et comptée ; elle n'est jamais supprimée du serveur, et elle est
  revue si elle change.
- Une fiche changée sur le serveur pendant la synchronisation n'est pas
  écrasée : elle est revue à la synchronisation suivante.
- Par prudence, si le serveur ne renvoie qu'une liste incomplète ou vide,
  ou si plus de la moitié des contacts reliés disparaissaient d'un coup,
  rien n'est supprimé dans Norelly : ces suppressions attendent. Pour
  vider tout le carnet, supprimez les contacts dans Norelly.
- Un très grand carnet (photos nombreuses) se reprend en plusieurs
  synchronisations.
- Le mot de passe n'est envoyé au serveur que lorsqu'il le demande.

## Cas limites

- Identifiant ou mot de passe refusés, serveur injoignable, certificat non
  valable, aucun carnet à cette adresse : la recherche s'arrête avec un
  message, la saisie est gardée.
- Échec d'une synchronisation : rien n'est perdu, la raison est affichée
  dans la fenêtre et la synchronisation suivante reprend où il faut.
- Carnet distant trop grand (au-delà de la taille d'un import) : refusé.
- Une seule synchronisation à la fois.
- Serveur dont le carnet est rangé sous une autre adresse que celle
  saisie (iCloud, par exemple) : pas encore pris en charge.
- Google Contacts : pas encore pris en charge (il demande une connexion
  Google).

## Messages affichés

- « Identifiant ou mot de passe refusé par le serveur de contacts. »
- « Serveur de contacts injoignable. Vérifiez son adresse et la connexion
  à Internet. »
- « Le certificat du serveur de contacts n'est pas valable : rien ne lui a
  été envoyé. »
- « Aucun carnet d'adresses trouvé à cette adresse. Essayez l'adresse
  complète du carnet, donnée par votre fournisseur. »
- « Carnet synchronisé : 2 fiche(s) reçue(s), 1 envoyée(s). »
- « Relié au carnet « Contacts » de alice sur https://dav.exemple.fr/. »

## Raccourcis

« Synchroniser avec un serveur… » : menu **Plus…** de la vue Contacts et
palette, sans raccourci.
