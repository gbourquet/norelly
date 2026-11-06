# Rédaction : nouveau message, réponse, transfert

[← Fonctionnalités](README.md)

## But

Écrire, répondre et transférer avec une mise en forme simple et des pièces
jointes, en produisant des messages corrects pour tous les logiciels de
messagerie (conversations, sujets, citations), sans que ce qui est saisi
puisse devenir dangereux.

## Comportement attendu

- **Nouveau message** : bouton toujours visible en haut du panneau de
  navigation (ou `c`), depuis le compte sélectionné.
- **Répondre**, **Répondre à tous**, **Transférer** : boutons de la barre
  d'actions du message ouvert, entrées du menu contextuel d'une ligne (ou
  `r`, `a`, `f`), depuis le dernier message de la conversation. Depuis la
  liste, la conversation s'ouvre d'abord. Destinataires,
  sujet et citation sont préremplis ; le transfert reprend les pièces
  jointes d'origine.
- Champs : À, Cc et Cci (affichés à la demande), Objet, corps.
  Destinataires en étiquettes, avec **suggestions** parmi les adresses
  connues (flèches, Entrée, virgule ou Tab pour valider).
- La fenêtre se ferme par son bouton **Fermer**, Échap ou un clic à
  l'extérieur, en gardant le brouillon.
- Mise en forme : gras, italique, barré, lien, listes à puces et
  numérotées, citation.
- **Pièces jointes** : bouton « Joindre des fichiers » ou glisser-déposer ;
  taille affichée ; retrait par la croix.
- **Ctrl+Entrée** envoie ; **Échap** ferme en gardant le brouillon
  ([brouillons](brouillons.md)) ; « Supprimer le brouillon » l'abandonne.
- Envoi immédiat, ou « Envoyer plus tard » ([envoi](envoi.md)).

## Règles

- **Destinataires d'une réponse** : l'adresse de réponse indiquée par
  l'expéditeur si elle existe, sinon l'expéditeur. « Répondre à tous »
  ajoute les destinataires (À et Cc) du message. Ses propres adresses sont
  exclues, les doublons retirés, les adresses invalides ignorées. Répondre
  à son propre message s'adresse à ses destinataires.
- **Sujet** : toujours `Re:` (réponse) ou `Fwd:` (transfert), sans
  empilement : les préfixes déjà présents, y compris ceux d'autres langues
  (`RE :`, `AW:`, `TR:`, `SV:`, `Antw:`…), sont reconnus puis remplacés.
- La réponse est rattachée à la **conversation** d'origine chez tous les
  destinataires.
- **Citation** : introduite par « Le {date}, {name} a écrit : », dans la
  langue de l'utilisateur.
- **Avant l'envoi** :
  - au moins un destinataire, 100 au plus ;
  - sujet sans caractère invisible, 998 caractères au plus ;
  - texte de 5 Mio au plus ;
  - pièces jointes : 100 au plus, 25 Mio au total.
- **Mise en forme envoyée** : uniquement celle que propose l'éditeur (gras,
  italique, barré, liens web et `mailto`, listes, citations) ; une version
  en texte simple accompagne toujours la version mise en forme.
- Les destinataires en **Cci** ne sont jamais visibles des autres
  destinataires. L'identifiant technique du message ne révèle rien de
  l'ordinateur de l'expéditeur.
- **Pièces jointes** : le type et la taille sont déterminés par
  l'application d'après le fichier lui-même.
- **Transfert** : les pièces jointes d'origine sont reprises, y compris
  celles pas encore téléchargées ; mêmes limites qu'à l'envoi.

## Cas limites

- Balisage saisi littéralement (`<b>…</b>`) : envoyé comme du texte, jamais
  interprété.
- Mise en forme imbriquée sur plus de 64 niveaux : réduite au texte.
- Transfert de plus de 100 pièces jointes ou de plus de 25 Mio : refusé.

## Messages affichés

| Situation | Texte |
|---|---|
| Titre de la fenêtre | Nouveau message, Répondre, Transférer |
| Citation | Le {date}, {name} a écrit : |
| Transfert | Message transféré de {name}, le {date} : |
| Aucun destinataire | Ajoutez au moins un destinataire. |
| Trop de destinataires | Trop de destinataires. |
| Sujet invalide | Le sujet contient un caractère interdit ou est trop long. |
| Texte trop long | Le message est trop long. |
| Pièces trop lourdes | Les pièces jointes dépassent 25 Mio. |
| Trop de pièces | Trop de pièces jointes (100 au plus). |
| Message confié à la boîte d'envoi | Message en cours d'envoi. |

## Raccourcis

| Commande | Raccourci |
|---|---|
| Nouveau message | `c` |
| Répondre | `r` |
| Répondre à tous | `a` |
| Transférer | `f` |
| Envoyer (dans la fenêtre de rédaction) | `Ctrl+Entrée` |
