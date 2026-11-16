# Dossiers : créer, renommer, déplacer, supprimer

[← Fonctionnalités](README.md)

## But

Ranger ses messages dans ses propres dossiers, sans passer par le site de
son fournisseur : créer un dossier, au premier niveau ou dans un dossier
existant, le renommer, le ranger ailleurs dans l'arborescence ou le
supprimer, depuis l'application.

## Comportement attendu

### Ordre dans le panneau de gauche

- Chaque dossier est suivi de ses sous-dossiers, en retrait, rangés par
  nom : un dossier rangé dans la boîte de réception ou dans la corbeille
  s'affiche juste sous elle.
- Les dossiers spéciaux viennent d'abord, toujours dans le même ordre
  (boîte de réception, brouillons, envoyés, archives, indésirables,
  corbeille…), puis les autres dossiers par nom.
- Un dossier spécial que le fournisseur range dans un autre dossier (les
  messages envoyés de Gmail, par exemple) garde sa place parmi les
  dossiers spéciaux, sans retrait : il ne semble pas rangé sous le
  dossier qui le précède.
- Un dossier dont le dossier parent n'est pas annoncé par le fournisseur
  s'affiche un cran sous son plus proche dossier parent présent (ou au
  premier niveau s'il n'en a aucun), avec le chemin qui manque dans son
  nom (« Perso/Banque » sous la boîte de réception, par exemple), dans le
  panneau comme dans les emplacements proposés ; son nom complet chez le
  fournisseur s'affiche en info-bulle.
- Les dossiers spéciaux portent un nom dans la langue de l'application
  (Boîte de réception, Brouillons, Envoyés, Archives, Indésirables,
  Corbeille, Tous les messages, Messages suivis), quel que soit le nom
  choisi par le fournisseur. Ce nom d'origine s'affiche en info-bulle
  quand il diffère. Le même nom traduit est repris partout : titre de la
  liste, « Déplacer vers… », emplacements proposés, messages de
  confirmation ; « Déplacer vers… » trouve un dossier spécial par l'un ou
  l'autre nom. Les autres dossiers gardent le nom du serveur.

### Créer un dossier

- Accès, qui font tous la même chose :
  - menu **⋯** d'un compte, dans le panneau de gauche : **Nouveau
    dossier…** ;
  - palette de commandes (« Nouveau dossier… »).
- Pour créer directement dans un dossier, **Nouveau sous-dossier…** dans
  le menu de ce dossier (**⋯** ou clic droit, voir plus bas), ou dans la
  palette de commandes pour le dossier affiché : le dossier est déjà
  choisi comme emplacement.
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

### Créer un dossier en déplaçant des messages

- Dans la liste **Déplacer vers…**, la dernière entrée, **Nouveau
  dossier…**, est toujours proposée, même quand la recherche ne trouve
  aucun dossier.
- Elle ouvre la même fenêtre que la création d'un dossier, dans le compte
  des messages à déplacer. Le nom déjà tapé dans la recherche est repris
  comme nom du dossier, sauf s'il désigne un dossier déjà proposé. Le bouton devient **Créer et déplacer**.
- Le dossier est créé, puis les messages visés y sont déplacés, en un seul
  geste. Le déplacement s'annule comme tout autre déplacement ; le dossier
  créé reste.
- Si le dossier est créé mais que l'application ne le voit pas encore
  (connexion perdue juste après), rien n'est déplacé et un message invite
  à le choisir dans **Déplacer vers…**.

### Menu d'un dossier

- Chaque dossier du panneau de gauche a un menu, ouvert par le bouton
  **⋯** (qui apparaît au survol, et reste visible sur le dossier affiché)
  ou par un **clic droit** sur le dossier (au doigt : un appui long sans
  bouger) : les deux proposent les mêmes entrées.
- Un dossier ordinaire propose **Nouveau sous-dossier…**, **Renommer le
  dossier…**, **Déplacer le dossier…** et **Supprimer le dossier…**.
- La boîte de réception et les dossiers spéciaux ne proposent que
  **Nouveau sous-dossier…**.
- Les dossiers virtuels (tous les messages, messages suivis) n'ont pas de
  menu : rien ne peut y être rangé.

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

### Déplacer un dossier

- Accès, qui font tous la même chose :
  - menu **⋯** d'un dossier, dans le panneau de gauche : **Déplacer le
    dossier…** ;
  - palette de commandes (« Déplacer le dossier… »), pour le dossier
    affiché ;
  - **glisser-déposer** dans le panneau de gauche (voir plus bas).
- Une fenêtre rappelle le nom du dossier et demande son **nouvel
  emplacement** : « Au premier niveau » ou l'un des dossiers du compte,
  présentés dans l'ordre et avec le retrait de l'arborescence. Le dossier
  lui-même et ses sous-dossiers ne sont pas proposés.
- L'emplacement actuel est présélectionné ; **Déplacer** reste grisé tant
  qu'il n'a pas changé. Pendant le déplacement, le bouton indique
  « Déplacement… » et la fenêtre ne peut pas être fermée.
- Le dossier garde son nom. Ses sous-dossiers et ses messages le suivent.
- Le dossier est déplacé **sur le serveur**. Il apparaît aussitôt à sa
  nouvelle place, avec ses messages ; s'il était affiché, il le reste.
- En cas de refus, la fenêtre reste ouverte avec le choix et l'explication.

Exemple : avec deux dossiers « Bla » et « Bli » au premier niveau,
déplacer « Bli » dans « Bla » en fait un sous-dossier de « Bla ».

**Glisser-déposer**, à la souris ou au doigt, sans fenêtre :

- saisir un dossier dans le panneau de gauche et le lâcher **sur un autre
  dossier** pour l'y ranger, ou **sur la ligne du compte** (nom et
  adresse) pour le remettre au premier niveau ;
- seuls les dossiers ordinaires se saisissent : ni la boîte de
  réception, ni les dossiers spéciaux ;
- le glisser ne commence qu'après un petit déplacement bouton enfoncé :
  un simple clic affiche toujours le dossier ; au doigt, après un appui
  long sans bouger (un doigt qui bouge tout de suite fait défiler) ;
- près du haut ou du bas du panneau, celui-ci défile pour atteindre les
  dossiers hors de vue ;
- pendant le glisser, le nom du dossier suit le pointeur, le dossier
  saisi est estompé et la cible survolée est encadrée ; ne sont pas des
  cibles : le dossier lui-même, ses sous-dossiers, les dossiers virtuels,
  les dossiers d'un autre compte, et l'endroit où il est déjà ;
- relâcher ailleurs, ou appuyer sur **Échap**, annule sans rien changer ;
- le déplacement suit les mêmes règles que par la fenêtre ; un refus est
  annoncé par un message en bas de l'écran.

### Supprimer un dossier

- Accès, qui font tous la même chose :
  - menu **⋯** d'un dossier, dans le panneau de gauche : **Supprimer le
    dossier…** ;
  - palette de commandes (« Supprimer le dossier… »), pour le dossier
    affiché.
- Une fenêtre demande confirmation, en rappelant le nom du dossier : tous
  les messages qu'il contient seront **supprimés définitivement, sans passer
  par la corbeille**. **Annuler** ne supprime rien. La fenêtre n'annonce
  jamais un dossier « vide » ni un nombre de messages : ceux connus de
  l'application peuvent être incomplets (dossier pas encore synchronisé).
- Le dossier disparaît aussitôt du panneau de gauche et de la liste
  « Déplacer vers… ». S'il était affiché, la boîte de réception du compte
  s'affiche à sa place.

## Règles

- Les espaces au début et à la fin du nom sont retirés.
- Le nom est refusé s'il est vide, s'il dépasse **100 caractères**, ou
  s'il contient `*`, `%` ou un caractère invisible.
- Le nom est aussi refusé s'il serait interprété par certains serveurs au
  lieu d'être pris tel quel : s'il commence par `~` ou `#`, s'il vaut `.`
  ou `..` (y compris entre deux `/` ou `\`), ou s'il finit par `}`.
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
- Un dossier déplacé suit les règles d'emplacement d'un nouveau dossier.
  Il ne peut aller ni dans lui-même ni dans un de ses sous-dossiers, et
  pas là où un dossier porte déjà son nom ; dans les deux cas, majuscules
  et minuscules sont confondues, certains serveurs ne les distinguant pas.
  Un dossier dont le nom serait interprété par certains serveurs (voir
  plus haut), créé par une autre application, ne se déplace pas : le
  renommer d'abord.
- La **boîte de réception** et les **dossiers spéciaux** (envoyés,
  brouillons, corbeille, indésirables, archives, dossiers virtuels) ne se
  renomment, ne se déplacent ni ne se suppriment : leur menu ne propose
  que **Nouveau sous-dossier…** (aucun menu pour les dossiers virtuels).
  Ils restent proposés comme emplacement, sauf les dossiers virtuels.
- Un dossier qui contient un **dossier spécial**, à quelque profondeur que
  ce soit (par exemple un dossier de fournisseur qui regroupe envoyés et
  corbeille), ne se renomme ni ne se déplace : le dossier spécial le
  suivrait.
- Un dossier qui contient des **sous-dossiers** ne peut pas être supprimé :
  il faut d'abord supprimer ses sous-dossiers. Juste avant la suppression,
  ses sous-dossiers sont recherchés à nouveau sur le serveur, y compris
  ceux que l'application n'affiche pas (nom illisible ou refusé, compte
  aux très nombreux dossiers) : s'il en a un, rien n'est supprimé.

## Cas limites

- **Hors ligne** : la création, le renommage, le déplacement ou la
  suppression attend la reconnexion au plus 30 secondes, puis échoue avec
  un message ; rien n'est mis en attente, et rien ne sera fait plus tard à
  l'insu de l'utilisateur.
- **Serveur sans sous-dossiers** : un emplacement autre que le premier
  niveau est refusé.
- **Refus du serveur** (droits, quota, nom interdit) : le message cite la
  réponse du serveur.
- Connexion perdue juste après la création, le renommage, le déplacement
  ou la suppression : l'opération est bien annoncée comme faite ; la liste des
  dossiers est mise à jour à la reconnexion.
- Dossier **modifié entre-temps** par une autre application (renommé,
  supprimé, sous-dossier ajouté) : la vérification se fait d'après la liste
  présente sur le serveur, et l'opération est refusée si elle n'a plus de
  sens.
- Un dossier renommé ou déplacé garde ses messages déjà présents sur
  l'ordinateur, sous-dossiers compris : rien n'est téléchargé à nouveau,
  seuls les changements faits entre-temps sur le serveur sont récupérés.
- Un message rangé dans un dossier (ou sorti d'un dossier) juste avant que
  ce dossier soit renommé ou déplacé, alors que l'action n'est pas encore
  faite sur le serveur (délai d'annulation, hors ligne), arrive bien dans
  le dossier à sa nouvelle place : l'action suit le dossier, ses
  sous-dossiers compris. Une action faite pendant le renommage ou le
  déplacement suit aussi, y compris si le compte est arrêté juste après.

## Messages affichés

| Situation | Texte |
|---|---|
| Menu, palette | Nouveau dossier…, Nouveau sous-dossier… |
| Déplacer vers… | Nouveau dossier… ; fenêtre : Créer et déplacer |
| Dossier créé, messages non déplacés | Dossier « {folder} » créé. Choisissez-le dans « Déplacer vers… » pour y déplacer les messages. |
| Fenêtre | Nouveau dossier, Nom du dossier, Emplacement, Au premier niveau, Créer, Création…, Annuler |
| Dossier créé | Dossier « {folder} » créé. |
| Nom vide | Donnez un nom au dossier. |
| Nom trop long | Ce nom de dossier est trop long. |
| Caractère interdit | Le nom ne peut contenir ni « * », ni « % », ni caractère invisible. |
| Nom interprété par certains serveurs | Le nom ne peut ni commencer par « ~ » ou « # », ni valoir « . » ou « .. », ni finir par « } » : certains serveurs l'interpréteraient. |
| Séparateur dans le nom | Le nom ne peut pas contenir le séparateur de dossiers du serveur (« / ») |
| Nom déjà pris | Un dossier de ce nom existe déjà à cet endroit. |
| Pas de sous-dossiers | Ce serveur n'accepte pas de sous-dossiers : choisissez « Au premier niveau ». |
| Refus, hors ligne | Le dossier n'a pas pu être créé sur le serveur. (« réponse du serveur ») |
| Menu d'un dossier | Actions du dossier {folder} : Nouveau sous-dossier…, Renommer le dossier…, Déplacer le dossier…, Supprimer le dossier… |
| Fenêtre de renommage | Renommer le dossier, Nouveau nom, Renommer, Renommage…, Annuler |
| Dossier renommé | Dossier renommé en « {folder} ». |
| Glisser un dossier | Nom du dossier |
| Fenêtre de déplacement | Déplacer le dossier ; « Où ranger le dossier « {folder} » ? Ses sous-dossiers et ses messages le suivent. » ; Nouvel emplacement, Au premier niveau, Déplacer, Déplacement…, Annuler |
| Dossier déplacé | Dossier « {folder} » déplacé. |
| Dans lui-même | Un dossier ne peut pas être rangé dans lui-même ni dans un de ses sous-dossiers. |
| Nom du dossier déplacé interprété par certains serveurs | Certains serveurs interpréteraient le nom de ce dossier (il commence par « ~ » ou « # », vaut « . » ou « .. », ou finit par « } ») : renommez-le d'abord, puis déplacez-le. |
| Refus, hors ligne (déplacement) | Le dossier n'a pas pu être déplacé sur le serveur. (« réponse du serveur ») |
| Fenêtre de suppression | Supprimer le dossier ; « Le dossier « {folder} » et tous les messages qu'il contient seront supprimés définitivement du serveur, sans passer par la corbeille. » ; « Cette action ne peut pas être annulée. » ; Supprimer le dossier, Suppression…, Annuler |
| Dossier supprimé | Dossier « {folder} » supprimé. |
| Dossier protégé | La boîte de réception et les dossiers spéciaux (envoyés, brouillons, corbeille…) ne peuvent être ni renommés, ni déplacés, ni supprimés. |
| Sous-dossiers | Ce dossier contient des sous-dossiers : supprimez-les d'abord. |
| Contient un dossier spécial | Ce dossier contient un dossier spécial (envoyés, brouillons, corbeille…) : il ne peut être ni renommé ni déplacé. |
| Refus, hors ligne (renommage) | Le dossier n'a pas pu être renommé sur le serveur. (« réponse du serveur ») |
| Refus, hors ligne (suppression) | Le dossier n'a pas pu être supprimé sur le serveur. (« réponse du serveur ») |

## Raccourcis

| Commande | Raccourci |
|---|---|
| Nouveau dossier… | palette |
| Nouveau sous-dossier… | palette |
| Renommer le dossier… | palette |
| Déplacer le dossier… | palette |
| Supprimer le dossier… | palette |
