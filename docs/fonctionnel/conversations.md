# Liste des conversations et boîte unifiée

[← Fonctionnalités](README.md)

## But

Présenter les messages d'un dossier, ou de toutes les boîtes de réception,
regroupés par conversation, sans attente même sur des dizaines de milliers
de messages.

## Comportement attendu

- La liste du panneau central montre une ligne par **conversation** :
  expéditeurs (trois au plus), nombre de messages, date du plus récent,
  sujet, indicateurs (pièce jointe, suivi), non lus en gras.
- Tri par date décroissante du message le plus récent.
- **Boîte de réception unifiée** : les conversations des boîtes de réception
  de tous les comptes, mêlées par date.
- Ouvrir une conversation affiche ses messages empilés, du plus ancien au
  plus récent ; un clic sur l'en-tête d'un message le replie ou le déplie.
- Le défilement reste fluide quelle que soit la taille du dossier : la
  liste se complète au fil du défilement.
- La largeur des panneaux se règle (souris ou clavier) et est mémorisée.

## Règles

- **Regroupement** : un message rejoint la conversation du message auquel
  il répond, d'après les références que portent les messages ; le sujet ne
  sert qu'en dernier recours.
- Le regroupement est le même quel que soit l'ordre d'arrivée des
  messages : deux conversations se réunissent quand arrive le message qui
  les relie.
- **Regroupement par sujet** : seulement pour une **réponse** (sujet
  commençant par `Re:`, `Fwd:`… ou réponse dont l'original est inconnu),
  et dans une fenêtre de 31 jours.
- Préfixes de réponse reconnus, dans plusieurs langues : `Re`, `RE`, `Fwd`,
  `Fw`, `TR`, `AW`, `WG`, `SV`, `VS`, `Antw`, `RIF`, `R`, `RE :`…
- Une conversation n'est jamais scindée quand un message qui la reliait
  disparaît.
- Une conversation affiche au plus ses 500 messages.

## Cas limites

- Deux messages « Bonjour » sans rapport : jamais réunis (ce ne sont pas
  des réponses).
- Une réponse arrivée avant le message d'origine : rattachée à l'arrivée de
  l'original.
- Dans la boîte unifiée, les actions visent chaque conversation dans son
  compte ; « déplacer » exige des conversations d'un seul compte.

## Messages affichés

| Situation | Texte |
|---|---|
| Dossier vide | Aucune conversation. |
| Boîte unifiée | Boîte de réception unifiée |
| Message sans sujet | (sans objet) |
| Expéditeur absent | (expéditeur inconnu) |
| Sélection | {count} sélectionnée(s) |

## Raccourcis

| Commande | Raccourci |
|---|---|
| Conversation suivante | `j` |
| Conversation précédente | `k` |
| Ouvrir la conversation | `Entrée` |
| Retour à la liste | `Échap` |
| Boîte de réception unifiée | palette |
