# Lecture sécurisée

[← Fonctionnalités](README.md)

## But

Afficher fidèlement un message, y compris sa mise en forme (HTML), sans
qu'il puisse exécuter de code, joindre le réseau, pister la lecture ou agir
sur l'application ; ouvrir liens et pièces jointes sans danger.

## Comportement attendu

- Le corps du message est affiché dans un cadre isolé du reste de
  l'application, à la hauteur de son contenu. Les raccourcis clavier
  restent actifs quand le focus est dans le message.
- **Images distantes bloquées** par défaut, avec un bandeau et le nombre
  d'images ; « Afficher les images de cet expéditeur » les autorise pour
  cet expéditeur, durablement. Les images contenues dans le message
  lui-même s'affichent.
- **Liens** : un clic ouvre le navigateur du système, seulement pour les
  liens web (`http`, `https`) et les adresses électroniques (`mailto`).
- **Pièces jointes** : nom et taille ; « Ouvrir » (application du système)
  et « Enregistrer… » (choix du dossier). Une pièce non encore téléchargée
  l'est à la demande.
- **Texte brut** : bouton par message, et préférence globale.
- Une conversation non lue **devient lue quand elle reste ouverte quelques
  secondes** : la parcourir rapidement ne change rien. Seuls les messages
  affichés à l'ouverture deviennent lus : une réponse arrivée pendant la
  lecture reste non lue. Ce marquage ne
  remplace pas le message de la dernière action ni sa possibilité
  d'annulation. Elle redevient non lue par le menu « Plus… » de la barre
  d'actions, le bouton au survol de la ligne ou `Maj+U`.

## Règles

- **Le message ne peut rien exécuter** : aucun script du message ne
  s'exécute, et le message n'a aucun accès à l'application ni aux données.
- **Aucune requête réseau à la lecture** sans autorisation : ni image
  distante, ni feuille de style, ni police externe.
- **Images distantes autorisées par expéditeur**, d'après son adresse. Une
  adresse d'expéditeur peut être falsifiée : l'autorisation protège du
  pistage, pas de l'usurpation.
- Images distantes autorisées : chargées sans cookies, sans indiquer la
  page d'origine, avec une identité de navigateur neutre, en taille et en
  types limités ; seules des adresses publiques sont jointes (jamais le
  réseau local).
- **Liens** : seuls les liens `http`, `https` et `mailto` s'ouvrent, et
  pas au-delà de 2 048 caractères.
- **Nom d'une pièce jointe** : nettoyé avant enregistrement (pas de
  chemin, pas de nom réservé par Windows, pas de caractère invisible ni de
  caractère qui inverse le sens d'écriture, ce qui démasque un `exe.pdf`
  déguisé), 200 octets au plus.
- **Ouverture directe** réservée aux types sans danger (documents sans
  macros, images, texte, sons, vidéos) ; tout le reste s'enregistre
  seulement.
- **Enregistrement** : jamais d'écrasement d'un fichier existant.

## Cas limites

- Message trop imbriqué ou trop long pour être affiché en HTML : notice et
  affichage simplifié.
- Corps pas encore téléchargé : notice.
- Expéditeur sans adresse valide : autorisation des images impossible.
- Lien `javascript:`, `file:`, `data:`… : refusé.
- Pièce exécutable ou à macros : « Ouvrir » refusé ; « Enregistrer… »
  reste possible.
- Une image distante ne peut viser qu'une adresse présente dans le
  message affiché, jamais une adresse arbitraire.

## Messages affichés

| Situation | Texte |
|---|---|
| Images bloquées | Images distantes bloquées ({count}) pour protéger votre vie privée. |
| Bouton d'autorisation | Afficher les images de cet expéditeur |
| Bascule texte brut | Afficher en texte brut / Afficher le HTML |
| Pièce jointe | Ouvrir, Enregistrer… |
| Pièce enregistrée | Enregistré sous « {path} ». |
| Lien refusé | Lien refusé : seuls les liens http, https et mailto s'ouvrent. |
| Pièce dangereuse | Type de fichier exécutable : enregistrez-le plutôt que de l'ouvrir. |
| Message illisible | Message illisible : {error} |

## Raccourcis

| Commande | Raccourci |
|---|---|
| Préférer le texte brut (ou le HTML) | palette |
