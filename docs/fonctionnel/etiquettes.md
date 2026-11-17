# Étiquettes

[← Fonctionnalités](README.md)

## But

Classer un message sous un ou plusieurs thèmes (« Travail », « Factures »,
« À faire »…) sans le déplacer, retrouver d'un coup d'œil ces thèmes dans la
liste et dans le message ouvert, et retrouver tous les messages d'un thème
d'un clic. Les étiquettes sont gardées sur le serveur : un autre logiciel de
messagerie qui les connaît (Thunderbird, par exemple) les voit aussi.

## Comportement attendu

- **Étiqueter…** est une action sur la cible, comme les autres actions
  (voir [actions](actions.md)) : dans **Plus…** du message ouvert, dans le
  menu du clic droit d'une ligne, dans **Plus…** de la barre de sélection,
  à la palette et au clavier (`l`).
- La fenêtre **Étiqueter** liste les étiquettes du compte, chacune avec sa
  pastille :
  - une étiquette est **cochée** quand tous les messages visés la portent ;
  - cocher une étiquette la **pose** tout de suite, décocher la **retire** ;
    un bandeau confirme, avec **Annuler** pendant 5 secondes ;
  - **Nouvelle étiquette** : un nom, puis **Créer et poser** ; l'étiquette
    est ajoutée à la liste du compte et posée d'un coup ;
  - **Terminé** ferme la fenêtre.
- **Portée** : la même que pour Lu et Suivi. Sur une conversation,
  l'étiquette vise tous ses messages du dossier affiché ; dans la boîte
  unifiée, ceux du type de dossier affiché.
- **Pastilles** :
  - dans la **liste**, après le sujet : deux pastilles au plus, puis
    « +N » (le nom des autres en infobulle) ;
  - dans le **message ouvert**, sous le sujet : toutes les étiquettes des
    messages de la conversation ;
  - une pastille montre le nom de l'étiquette sur sa couleur, ou sur un
    fond neutre sans couleur ; le texte reste lisible quelle que soit la
    couleur, en thème clair comme en thème sombre ;
  - un **clic sur une pastille** lance la recherche des messages de cette
    étiquette (`étiquette:"nom"`).
- **Réglages**, section **Étiquettes**, par compte :
  - la liste réunit les étiquettes créées dans Norelly et celles déjà
    présentes sur les messages du serveur ;
  - **Ajouter** une étiquette ;
  - **Couleur** : sélecteur de couleur ; **Sans couleur** revient au fond
    neutre. La couleur reste sur cet ordinateur ;
  - **Renommer** : le nouveau nom s'affiche tout de suite ; tous les
    messages du serveur qui portent l'étiquette sont mis à jour en
    arrière-plan, la couleur suit ;
  - **Supprimer** : retire l'étiquette de la liste ; elle n'est plus
    affichée sur aucun message. Avec **La retirer aussi de tous les
    messages**, elle est en plus retirée de tous les messages du serveur, en
    arrière-plan. Créer de nouveau une étiquette du même nom la fait
    réapparaître.
- **Recherche** : `étiquette:nom` (ou `label:nom`, accent facultatif), ou
  `étiquette:"nom avec espaces"`, trouve les messages qui portent
  l'étiquette ; `-étiquette:nom` les exclut. Majuscules et accents du nom
  n'ont pas d'importance.

## Règles

- **Nom** : non vide, sans caractère de contrôle ni caractère invisible,
  borné en longueur ; espaces de début et de fin retirés. Deux étiquettes
  d'un même compte ne peuvent pas porter le même nom, aux majuscules et aux
  accents près.
- Les noms des marques techniques (voir plus bas : « Junk »,
  « Forwarded »…) sont réservés : refusés à la création et au renommage.
- **Tous les caractères** sont permis dans un nom (accents, espaces,
  ponctuation, autres écritures) : il est enregistré sur le serveur sous
  une forme que tout logiciel de messagerie accepte, et relu à l'identique.
- **Étiquettes de Thunderbird** : ses cinq étiquettes d'origine
  s'affichent sous leur nom, traduit : Important, Travail, Personnel,
  À faire, Plus tard.
- **Marques techniques** posées par les serveurs et les logiciels de
  messagerie (indésirable, transféré, accusé de lecture envoyé…) ne sont
  jamais affichées comme des étiquettes.
- Une étiquette posée par un autre logiciel sous une forme que Norelly ne
  sait pas relire s'affiche telle quelle.
- **Serveur qui refuse les étiquettes** dans un dossier : **Étiqueter…**
  reste visible mais désactivé, et son infobulle explique pourquoi. Si le
  serveur n'accepte que certaines étiquettes, les autres sont désactivées
  dans la fenêtre, avec une explication, et la création d'une nouvelle
  étiquette est désactivée. Une étiquette déjà posée peut toujours être
  retirée. Aucune étiquette n'est gardée seulement sur l'ordinateur.
- **Hors ligne**, poser ou retirer une étiquette suit les règles des
  autres actions : effet immédiat, envoi au serveur à la reconnexion.
- **Renommer, ou retirer de tous les messages**, survit à une fermeture de
  l'application ou à une coupure : le travail reprend à la connexion
  suivante, sans jamais laisser un message sans aucune des deux étiquettes
  (ancienne ou nouvelle). Il se fait par lots, sans bloquer l'interface,
  même sur une très grande boîte. Un dossier qui refuse le nouveau nom est
  laissé tel quel, et l'affichage de ce dossier est remis à l'état du
  serveur.
- Une étiquette posée ou retirée juste avant un renommage suit le nouveau
  nom.

## Cas limites

- Sélection de conversations de plusieurs comptes : la fenêtre réunit les
  étiquettes des comptes ; une nouvelle étiquette est créée sur chacun.
- Nom déjà pris (aux majuscules et accents près) : refusé à la création et
  au renommage ; dans la fenêtre **Étiqueter**, l'étiquette existante est
  posée.
- Renommer sous le même nom, ou en ne changeant que des majuscules sans
  accent (« Projet » en « PROJET ») : rien ne change, la plupart des
  serveurs ne distinguant pas ces deux noms.
- Une étiquette renommée sur un autre ordinateur apparaît sous son nouveau
  nom à la synchronisation suivante ; l'ancien nom disparaît de la liste
  quand plus aucun message ne le porte.

## Messages affichés

| Situation | Texte |
|---|---|
| Action | Étiqueter… |
| Fenêtre | Étiqueter ; Cochez une étiquette pour la poser sur la sélection, décochez-la pour la retirer. |
| Fenêtre, aucune étiquette | Aucune étiquette pour l'instant : créez-en une ci-dessous. |
| Création | Nouvelle étiquette, Créer et poser, Terminé |
| Pose, retrait | Étiquette « {label} » posée., Étiquette « {label} » retirée. |
| Serveur qui refuse | Ce serveur ne garde pas les étiquettes dans ce dossier. ; Ce serveur n'accepte pas cette étiquette dans ce dossier. ; Ce serveur n'accepte pas de nouvelle étiquette dans ce dossier. |
| Pastille | Rechercher les messages étiquetés « {label} » ; +{count} ; Autres étiquettes : {labels} |
| Réglages | Étiquettes, Compte, Aucune étiquette sur ce compte., Ajouter, Couleur de « {label} », Sans couleur, Renommer, Supprimer |
| Suppression | Supprimer « {label} » de la liste ?, La retirer aussi de tous les messages (sur le serveur) |
| Renommage | Étiquette renommée. Les messages du serveur suivent en arrière-plan. |
| Trop de changements en attente | Des modifications d'étiquettes attendent encore le serveur : réessayez une fois qu'elles sont faites. |
| Nom refusé | Donnez un nom à l'étiquette, sans caractère invisible. ; Ce nom d'étiquette est trop long. ; Une étiquette porte déjà ce nom. ; Ce nom est réservé à un usage technique : choisissez-en un autre. |

## Raccourcis

| Commande | Raccourci |
|---|---|
| Étiqueter… | `l` |
