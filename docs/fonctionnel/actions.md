# Actions sur les messages, hors ligne et annulation

[← Fonctionnalités](README.md)

## But

Ranger vite (archiver, supprimer, déplacer, suivre, marquer lu ou non lu),
avec un effet visible immédiatement, même hors ligne, et pouvoir se
rattraper.

## Comportement attendu

- Les actions portent sur la **cible** : les conversations cochées (`x`),
  sinon la conversation ouverte, sinon celle pointée dans la liste. Une
  action sur une conversation vise tous ses messages du dossier affiché.
- Accès : raccourcis, palette, barre d'outils de la vue de lecture, boutons
  au survol d'une ligne, menu contextuel (clic droit) ; tous font la même
  chose.
- **Effet immédiat** : le résultat est visible tout de suite (état changé,
  conversation retirée de la liste, message déjà présent dans le dossier de
  destination), puis l'action est envoyée au serveur.
- Un bandeau confirme l'action, avec **Annuler** pendant **5 secondes**
  (aussi `z`). Passé ce délai, l'action part au serveur.
- **Hors ligne**, les actions attendent et sont envoyées à la reconnexion,
  dans l'ordre, même si l'application a été fermée entre-temps.
- « Suivre » est une bascule : si toutes les cibles sont suivies, le suivi
  est retiré, sinon il est ajouté.

## Règles

- **Supprimer** = déplacer vers la Corbeille ; **depuis la Corbeille**,
  suppression définitive.
- **Archiver** = déplacer vers le dossier d'archives du compte, sinon vers
  le dossier choisi par l'utilisateur (« Choisir le dossier
  d'archives… »).
- **Annulation** possible pendant 5 secondes ; au-delà, « trop tard ».
- **Conflit avec le serveur** à l'envoi d'une action (message disparu,
  dossier de destination supprimé, dossier réinitialisé) : l'action est
  abandonnée, l'affichage revient à l'état d'avant, puis la
  synchronisation suivante montre l'état du serveur.
- Une suppression ne fait jamais disparaître un message qu'un autre client
  a marqué pour suppression sans le supprimer.
- Un changement d'état (lu, suivi) ne remplace pas un changement fait
  entre-temps par un autre client, quand le serveur permet de le
  détecter.
- **Bornes** : 1 000 messages par action ; 5 essais d'envoi au serveur ;
  10 000 actions en attente par compte.
- **Message déplacé** : visible tout de suite dans le dossier de
  destination ; retiré si l'action est annulée ou abandonnée ; remplacé par
  le vrai message à la synchronisation suivante, sans trou ni doublon.

## Cas limites

- Compte sans Corbeille : suppression refusée.
- Compte sans dossier d'archives et sans dossier choisi : l'application
  invite à en choisir un (« Choisir le dossier d'archives… »).
- Déplacer vers le dossier courant, ou vers un dossier qui ne peut pas
  contenir de messages : refusé.
- Déplacer des conversations de plusieurs comptes : refusé, un dossier
  n'appartenant qu'à un compte.
- Trop d'actions en attente : refusé jusqu'à la reconnexion.

## Messages affichés

| Situation | Texte |
|---|---|
| Archivage, suppression | Archivé., Supprimé. |
| Déplacement | Déplacé vers « {folder} ». |
| Suivi | Suivi activé., Suivi retiré. |
| Lu, non lu | Marqué comme lu., Marqué comme non lu. |
| Annulation | Annuler, Action annulée. |
| Annulation tardive | Trop tard pour annuler : l'action a déjà été envoyée au serveur. |
| Rien à faire | Rien à changer. |
| Corbeille, archives absentes | Aucun dossier Corbeille sur ce compte., Aucun dossier d'archives sur ce compte : choisissez-en un … |
| Trop d'actions en attente | Trop d'actions attendent le serveur sur ce compte : réessayez une fois reconnecté. |
| Dossier d'archives choisi | Dossier d'archives : « {folder} ». |

## Raccourcis

| Commande | Raccourci |
|---|---|
| Archiver | `e` |
| Supprimer | `#` |
| Suivre (ou ne plus suivre) | `s` |
| Marquer comme lu | `Maj+I` |
| Marquer comme non lu | `Maj+U` |
| Déplacer vers… | `v` |
| Sélectionner la conversation | `x` |
| Annuler la dernière action | `z` |
| Choisir le dossier d'archives… | palette |
