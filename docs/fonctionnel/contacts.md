# Contacts : le carnet d'adresses

[← Fonctionnalités](README.md)

## But

Garder les personnes à qui l'on écrit souvent dans un carnet à soi :
enregistrer un contact avec son nom, ses adresses et ses téléphones, le
corriger, le marquer comme favori, lui écrire en un clic et le supprimer.
Le carnet est **unique** pour tous les comptes et reste **sur
l'ordinateur** : rien n'est envoyé à un serveur.

## Comportement attendu

### Ouvrir les contacts

- Accès, qui font tous la même chose :
  - **Contacts**, en bas du panneau de gauche, au-dessus d'« Ajouter un
    compte » et des paramètres ;
  - palette de commandes (« Ouvrir les contacts ») ;
  - raccourci (voir l'aide des raccourcis).
- La liste des contacts remplace celle des conversations ; la fiche du
  contact choisi s'affiche à droite. « Contacts » reste mis en évidence
  dans le panneau de gauche tant que le carnet est affiché.
- Un clic sur la boîte de réception unifiée ou sur un dossier ramène au
  courrier. Tant que le carnet est affiché, les actions sur les messages
  (archiver, supprimer, naviguer…) ne visent rien.
- La liste est triée par nom. Chaque ligne montre le nom, l'adresse
  préférée, l'organisation éventuelle et une étoile pour un favori. Elle
  reste fluide quel que soit le nombre de contacts.
- Un champ **Rechercher** filtre la liste au fil de la frappe, sur le nom,
  le prénom, le surnom, l'organisation et les adresses, sans tenir compte
  des majuscules. Les boutons **Tous** / **Favoris** limitent la liste aux
  favoris.
- Carnet vide : un message invite à cliquer sur **Nouveau contact**.
  Aucun résultat : « Aucun contact ne correspond. ».

### Ajouter un contact

- Accès : bouton **Nouveau contact** en haut de la liste, bouton du même
  nom sur la fiche vide, palette de commandes ou raccourci. Depuis le
  courrier, le raccourci et la palette ouvrent d'abord le carnet.
- Une fenêtre demande **Prénom**, **Nom**, **Nom affiché** (facultatif),
  **Surnom**, une ou plusieurs **adresses électroniques**, des
  **téléphones**, l'**Organisation**, des **Notes** et **Favori**.
- Chaque adresse a une étiquette facultative (« travail », « maison ») et
  l'une d'elles est **préférée** : c'est celle utilisée pour écrire au
  contact. **Ajouter une adresse** et **Ajouter un téléphone** ajoutent une
  ligne ; la corbeille d'une ligne la retire. La dernière adresse ne peut
  pas être retirée ; retirer l'adresse préférée rend la première préférée.
- Les lignes laissées vides ne sont pas enregistrées. **Enregistrer** reste
  grisé tant qu'aucune adresse n'est saisie ; pendant l'enregistrement, il
  indique « Enregistrement… » et la fenêtre ne peut pas être fermée.
- Nom affiché d'un contact : le **nom affiché** s'il est saisi, sinon
  prénom et nom, sinon le surnom, sinon l'adresse préférée.
- Refus, fenêtre gardée ouverte avec la saisie et l'explication :
  - aucune adresse, ou adresse mal formée (citée) ;
  - même adresse deux fois dans la fiche ;
  - adresse qui appartient déjà à **un autre contact**, sans tenir compte
    des majuscules (citée) : une adresse n'appartient qu'à un contact ;
  - texte trop long ou caractère invisible dans un champ (le champ est
    nommé).
- Contact enregistré : la fenêtre se ferme, un avis « Contact … ajouté. »
  s'affiche, la liste est relue et la fiche du contact s'ouvre.

### Modifier un contact

- Accès : **Modifier le contact** sur la fiche ouverte, dans le menu du
  clic droit d'une ligne, palette ou raccourci.
- La même fenêtre s'ouvre, préremplie ; mêmes règles et mêmes refus. Un
  contact supprimé entre-temps est signalé (« Ce contact n'existe plus. »).

### Favoris

- **Ajouter aux favoris** / **Retirer des favoris** : dans « Plus… » de la
  fiche, dans le menu du clic droit d'une ligne, palette ou raccourci ; la
  case **Favori** de la fenêtre fait de même. Un avis confirme le
  changement ; l'étoile apparaît ou disparaît dans la liste et sur la fiche.

### Écrire à un contact

- **Écrire au contact** : sur la fiche, dans le menu du clic droit, dans la
  barre des contacts cochés, palette ou raccourci.
- Un nouveau message s'ouvre depuis le compte sélectionné dans le panneau
  de gauche, avec le nom et l'adresse préférée de chaque contact visé dans
  **À**, chacun une seule fois. Sans compte, l'action est grisée.

### Sélectionner plusieurs contacts

- La case d'une ligne la coche sans ouvrir la fiche. Dès qu'un contact est
  coché, une barre indique le nombre de contacts sélectionnés et propose
  **Écrire au contact**, **Supprimer le contact** et **Désélectionner les
  contacts**.
- Les actions visent le contact du menu du clic droit s'il est ouvert,
  sinon les contacts cochés, sinon le contact affiché. **Modifier** et le
  favori ne visent qu'un contact à la fois.

### Supprimer des contacts

- Accès : **Supprimer le contact** sur la fiche, dans le menu du clic
  droit, dans la barre des contacts cochés, palette ou raccourci.
- Une fenêtre demande confirmation, avec le nom du contact (ou le nombre
  de contacts), et précise que les messages ne sont pas touchés et que la
  suppression est définitive.
- Après la suppression : avis « N contact(s) supprimé(s). », liste relue ;
  la fiche affichée se ferme si elle a été supprimée.

## Sécurité

- Tout ce qui est saisi est affiché comme du **texte**, jamais interprété.
- Les fiches sont revalidées par l'application avant tout enregistrement,
  quelle que soit la saisie de l'écran.
- Le carnet est stocké dans la base locale de l'application, avec les
  messages ; il n'est envoyé à aucun serveur.

## Limites actuelles

- Les noms du carnet ne remplacent pas encore les noms affichés pour les
  expéditeurs, et les suggestions de destinataires ne proposent pas encore
  les contacts du carnet.
- Pas encore de listes de diffusion, d'import ni d'export (vCard, CSV,
  LDIF), ni de recherche des doublons.
