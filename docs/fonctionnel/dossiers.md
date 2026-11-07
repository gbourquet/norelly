# Dossiers : créer, renommer, supprimer

[← Fonctionnalités](README.md)

## But

Ranger ses messages dans ses propres dossiers, sans passer par le site de
son fournisseur : créer un dossier, au premier niveau ou dans un dossier
existant, le renommer ou le supprimer, depuis l'application.

## Comportement attendu

### Créer un dossier

- Accès, qui font tous la même chose :
  - menu **⋯** d'un compte, dans le panneau de gauche : **Nouveau
    dossier…** ;
  - palette de commandes (« Nouveau dossier… »).
- Une fenêtre demande le **nom du dossier** et son **emplacement** : « Au
  premier niveau » (par défaut) ou dans l'un des dossiers du compte,
  présentés dans l'ordre et avec le retrait de l'arborescence.
- **Créer** reste grisé tant que le nom est vide. Pendant la création, le
  bouton indique « Création… » et la fenêtre ne peut pas être fermée.
- Le dossier est créé **sur le serveur**, donc visible aussi dans les
  autres applications et sur le site du fournisseur. Il apparaît aussitôt
  dans le panneau de gauche et dans la liste « Déplacer vers… ».
- En cas de refus, la fenêtre reste ouverte avec la saisie et l'explication,
  pour corriger et réessayer.

### Renommer un dossier

- Accès, qui font tous la même chose :
  - menu **⋯** d'un dossier, dans le panneau de gauche : **Renommer le
    dossier…** ; ce bouton apparaît au survol du dossier, et reste visible
    sur le dossier affiché ;
  - palette de commandes (« Renommer le dossier… »), pour le dossier
    affiché.
- Une fenêtre propose le **nom actuel**, à modifier. **Renommer** reste
  grisé tant que le nom est vide ou inchangé. Pendant le renommage, le
  bouton indique « Renommage… » et la fenêtre ne peut pas être fermée.
- Le dossier garde sa place : seul son nom change. Ses sous-dossiers et
  ses messages le suivent.
- Le dossier est renommé **sur le serveur**. Il apparaît aussitôt sous son
  nouveau nom, avec ses messages ; s'il était affiché, il le reste.
- En cas de refus, la fenêtre reste ouverte avec la saisie et l'explication.

### Supprimer un dossier

- Accès, qui font tous la même chose :
  - menu **⋯** d'un dossier, dans le panneau de gauche : **Supprimer le
    dossier…** ;
  - palette de commandes (« Supprimer le dossier… »), pour le dossier
    affiché.
- Une fenêtre demande confirmation, en rappelant le nom du dossier et le
  nombre de messages qu'il contient, qui seront **supprimés définitivement,
  sans passer par la corbeille**. **Annuler** ne supprime rien.
- Le dossier disparaît aussitôt du panneau de gauche et de la liste
  « Déplacer vers… ». S'il était affiché, la boîte de réception du compte
  s'affiche à sa place.

## Règles

- Les espaces au début et à la fin du nom sont retirés.
- Le nom est refusé s'il est vide, s'il dépasse **100 caractères**, ou
  s'il contient `*`, `%` ou un caractère invisible.
- Un nom ne crée qu'**un seul niveau** : le caractère qui sépare les
  niveaux sur le serveur (souvent `/` ou `.`) est refusé dans le nom ; pour
  un sous-dossier, choisir l'emplacement.
- Un dossier **du même nom au même endroit** est refusé, majuscules et
  minuscules confondues (certains serveurs ne les distinguent pas).
- Le nom est vérifié d'après la liste des dossiers **présente sur le
  serveur** au moment de la création, y compris ceux créés entre-temps par
  une autre application.
- Sur les serveurs qui rangent tous les dossiers sous la boîte de
  réception, un dossier « au premier niveau » y est rangé aussi, comme les
  autres.
- Les dossiers virtuels du serveur (tous les messages, messages suivis) ne
  sont pas proposés comme emplacement.
- Le nouveau nom d'un dossier renommé suit les mêmes règles qu'un nouveau
  dossier ; seul un changement de majuscules et minuscules de son propre nom
  est accepté.
- La **boîte de réception** et les **dossiers spéciaux** (envoyés,
  brouillons, corbeille, indésirables, archives, dossiers virtuels) ne se
  renomment ni ne se suppriment : leur menu **⋯** n'est pas proposé.
- Un dossier qui contient des **sous-dossiers** ne peut pas être supprimé :
  il faut d'abord supprimer ses sous-dossiers.

## Cas limites

- **Hors ligne** : la création, le renommage ou la suppression attend la
  reconnexion au plus 30 secondes, puis échoue avec un message ; rien n'est
  mis en attente, et rien ne sera fait plus tard à l'insu de l'utilisateur.
- **Serveur sans sous-dossiers** : un emplacement autre que le premier
  niveau est refusé.
- **Refus du serveur** (droits, quota, nom interdit) : le message cite la
  réponse du serveur.
- Connexion perdue juste après la création, le renommage ou la
  suppression : l'opération est bien annoncée comme faite ; la liste des
  dossiers est mise à jour à la reconnexion.
- Dossier **modifié entre-temps** par une autre application (renommé,
  supprimé, sous-dossier ajouté) : la vérification se fait d'après la liste
  présente sur le serveur, et l'opération est refusée si elle n'a plus de
  sens.
- Les messages d'un dossier renommé sont téléchargés à nouveau sous son
  nouveau nom.

## Messages affichés

| Situation | Texte |
|---|---|
| Menu, palette | Nouveau dossier… |
| Fenêtre | Nouveau dossier, Nom du dossier, Emplacement, Au premier niveau, Créer, Création…, Annuler |
| Dossier créé | Dossier « {folder} » créé. |
| Nom vide | Donnez un nom au dossier. |
| Nom trop long | Ce nom de dossier est trop long. |
| Caractère interdit | Le nom ne peut contenir ni « * », ni « % », ni caractère invisible. |
| Séparateur dans le nom | Le nom ne peut pas contenir le séparateur de dossiers du serveur (« / ») |
| Nom déjà pris | Un dossier de ce nom existe déjà à cet endroit. |
| Pas de sous-dossiers | Ce serveur n'accepte pas de sous-dossiers : choisissez « Au premier niveau ». |
| Refus, hors ligne | Le dossier n'a pas pu être créé sur le serveur. (« réponse du serveur ») |
| Menu d'un dossier | Actions du dossier {folder} : Renommer le dossier…, Supprimer le dossier… |
| Fenêtre de renommage | Renommer le dossier, Nouveau nom, Renommer, Renommage…, Annuler |
| Dossier renommé | Dossier renommé en « {folder} ». |
| Fenêtre de suppression | Supprimer le dossier ; « Le dossier « {folder} » et tous ses messages ({count}) seront supprimés définitivement du serveur, sans passer par la corbeille. » (ou « Le dossier « {folder} » est vide : il sera supprimé du serveur. ») ; « Cette action ne peut pas être annulée. » ; Supprimer le dossier, Suppression…, Annuler |
| Dossier supprimé | Dossier « {folder} » supprimé. |
| Dossier protégé | La boîte de réception et les dossiers spéciaux (envoyés, brouillons, corbeille…) ne peuvent être ni renommés ni supprimés. |
| Sous-dossiers | Ce dossier contient des sous-dossiers : supprimez-les d'abord. |
| Refus, hors ligne (renommage) | Le dossier n'a pas pu être renommé sur le serveur. (« réponse du serveur ») |
| Refus, hors ligne (suppression) | Le dossier n'a pas pu être supprimé sur le serveur. (« réponse du serveur ») |

## Raccourcis

| Commande | Raccourci |
|---|---|
| Nouveau dossier… | palette |
| Renommer le dossier… | palette |
| Supprimer le dossier… | palette |
