# Dossiers : créer un dossier

[← Fonctionnalités](README.md)

## But

Ranger ses messages dans ses propres dossiers, sans passer par le site de
son fournisseur : créer un dossier, au premier niveau ou dans un dossier
existant, depuis l'application.

## Comportement attendu

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

## Cas limites

- **Hors ligne** : la création attend la reconnexion au plus 30 secondes,
  puis échoue avec un message ; rien n'est mis en attente.
- **Serveur sans sous-dossiers** : un emplacement autre que le premier
  niveau est refusé.
- **Refus du serveur** (droits, quota, nom interdit) : le message cite la
  réponse du serveur.
- Connexion perdue juste après la création : le dossier est bien annoncé
  comme créé ; il apparaît dans la liste à la reconnexion.

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

## Raccourcis

| Commande | Raccourci |
|---|---|
| Nouveau dossier… | palette |
