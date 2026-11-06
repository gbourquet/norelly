# Brouillons

[← Fonctionnalités](README.md)

## But

Ne jamais perdre ce qui est en cours d'écriture, retrouver ses brouillons
et reprendre ceux commencés sur un autre logiciel de messagerie.

## Comportement attendu

- Le brouillon est enregistré **sur l'ordinateur à chaque pause de
  frappe** ; une copie est déposée sur le serveur, dans le dossier
  « Brouillons ».
- Fermer la fenêtre de rédaction (Échap, croix) garde le brouillon.
- « Brouillons… » (palette) liste les brouillons du compte sélectionné :
  - les **brouillons écrits dans Norelly** (sujet, destinataires, date) ;
  - sous « Dans le dossier Brouillons du serveur », les brouillons écrits
    avec **un autre logiciel** (sans les copies de ceux de Norelly).
- Choisir un brouillon le rouvre ; un brouillon d'un autre logiciel est
  repris tel quel (destinataires, sujet, texte, conversation).
- À l'envoi, le brouillon passe dans la boîte d'envoi ; sa copie sur le
  serveur est retirée une fois le message parti.
- « Supprimer le brouillon » le supprime de l'ordinateur et du serveur.

## Règles

- La copie sur le serveur n'est remplacée que si le brouillon a changé, et
  **au plus toutes les 30 secondes** ; elle remplace la précédente.
- Si le serveur ne permet pas de remplacer une copie de façon sûre, les
  brouillons restent sur l'ordinateur seulement.
- Norelly ne supprime une ancienne copie que s'il est certain qu'il s'agit
  bien de ce brouillon.
- Copie refusée par le serveur : nouvel essai une heure plus tard ; les
  autres brouillons continuent d'être copiés.
- Un brouillon d'un autre logiciel, répondant à un message, reste rattaché
  à sa conversation.

## Cas limites

- Dossier « Brouillons » disparu ou refusé par le serveur : les anciennes
  copies qui restaient à supprimer sont oubliées.
- Brouillon programmé pour un envoi différé : il reste attaché au message
  dans la boîte d'envoi et revient dans la fenêtre de rédaction par
  « Modifier » (voir [envoi](envoi.md)).

## Messages affichés

| Situation | Texte |
|---|---|
| Titre de la liste | Brouillons |
| Aucun brouillon | Aucun brouillon enregistré. |
| Destinataires | À : {recipients} |
| Brouillons d'autres logiciels | Dans le dossier Brouillons du serveur |
| Abandon | Supprimer le brouillon |

## Raccourcis

| Commande | Raccourci |
|---|---|
| Brouillons… | palette |
