# Synchronisation

[← Fonctionnalités](README.md)

## But

Tenir une copie locale fidèle de chaque compte (dossiers, messages, états
lu, suivi…), à jour en quelques secondes, en économisant le réseau, et qui
redevient identique au serveur après toute interruption.

## Comportement attendu

- Au démarrage de l'application (ou à l'ajout du compte), chaque compte
  synchronise la liste de ses dossiers, puis chaque dossier : la **boîte
  de réception d'abord**, les dossiers virtuels qui répètent des messages
  déjà présents ailleurs (« Tous les messages », « Suivis » chez Gmail) en
  dernier.
- Pendant une synchronisation complète (au démarrage, toutes les 10
  minutes, à la demande, après une reconnexion), une ligne
  « Synchronisation… » s'affiche sous le compte, avec le nombre de
  dossiers traités sur le total ; elle disparaît à la fin, ou si la
  connexion est perdue.
- Ce qui arrive s'affiche **au fil de l'eau** : la liste des dossiers dès
  qu'elle est connue, les messages d'un dossier dès que leurs en-têtes
  sont reçus, sans attendre leur contenu ni les autres dossiers. Le
  premier tour d'un gros compte peut être long ; on voit qu'il avance.
  L'affichage est rafraîchi au plus une fois par seconde, pour rester
  fluide même avec des milliers de dossiers.
- Une action faite hors ligne et pas encore envoyée garde son effet à
  l'écran pendant la synchronisation (un message marqué lu le reste).
- Les nouveaux messages de la **boîte de réception** arrivent en quelques
  secondes, sans action de l'utilisateur : le serveur les signale dès leur
  arrivée.
- Les autres dossiers sont resynchronisés **toutes les 10 minutes**, et à
  la demande (entrée « Synchroniser » du menu « ⋯ » du compte, ou commande
  « Synchroniser le compte sélectionné »).
- Les en-têtes et le texte de tous les messages sont téléchargés ; les
  pièces jointes seulement sous un **seuil** (2 Mio par défaut, réglable) :
  au-delà, elles sont téléchargées à l'ouverture.
- Hors ligne, une icône (Wi-Fi barré) s'affiche à côté du compte, avec la
  raison au survol ; la reconnexion est automatique.
- Les dossiers affichent leur rôle (icône) et leur nombre de non lus.

## Règles

- **Rôle d'un dossier** (réception, envoyés, brouillons, corbeille,
  indésirables, archives, tous les messages) : celui que déclare le
  serveur, sinon déduit du nom du dossier.
- Après chaque synchronisation, la copie locale est **identique au
  serveur** ; synchroniser deux fois de suite ne change rien.
- Seul ce qui a changé depuis la dernière fois est téléchargé, quand le
  serveur le permet ; un dossier qui n'a pas bougé ne coûte presque rien.
- **Seuil de téléchargement des pièces jointes** : entre 64 Kio et 64 Mio.
- Messages à la structure démesurée (imbrication, nombre de parties) :
  traités dans des limites fixes, pour qu'un message piégé ne bloque pas
  l'application.
- **Reconnexion** : nouveaux essais de plus en plus espacés, d'une seconde
  à cinq minutes, puis retour au rythme normal dès que la connexion
  revient.
- Tout texte venu du serveur (noms de dossiers, raisons d'erreur) est
  nettoyé avant affichage ; le nombre et la longueur des noms de dossiers
  sont bornés.

## Cas limites

- **Dossier réinitialisé par le serveur** (il arrive qu'un serveur
  renumérote tous les messages d'un dossier) : le dossier est entièrement
  resynchronisé ; les actions en attente sur ce dossier sont abandonnées
  (voir [actions](actions.md)).
- **Coupure en pleine synchronisation** : ce qui a été reçu est gardé, la
  synchronisation reprend où elle en était à la reconnexion.
- **Serveur aux fonctions limitées** : la synchronisation compare tout le
  dossier ; plus lente, mais le résultat est le même.
- Message dont une pièce jointe dépasse le seuil : la pièce est téléchargée
  quand l'utilisateur l'ouvre.

## Messages affichés

| Situation | Texte |
|---|---|
| Synchronisation en cours | Synchronisation… {done} / {total} (au survol et pour les lecteurs d'écran : Synchronisation de {address} : {done} dossiers sur {total}) |
| Compte injoignable | Hors ligne |
| Survol de l'icône | Hors ligne : {reason} |
| Raison | serveur injoignable, le serveur ne répond pas, identifiants refusés, connexion refusée par sécurité, réponse du serveur non conforme… |
| Contenu pas encore téléchargé | Le contenu de ce message n'est pas encore disponible. |

## Raccourcis

| Commande | Raccourci |
|---|---|
| Synchroniser le compte sélectionné | palette |
