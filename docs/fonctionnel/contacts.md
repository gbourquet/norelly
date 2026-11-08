# Contacts : le carnet d'adresses

[← Fonctionnalités](README.md)

## But

Garder les personnes à qui l'on écrit souvent dans un carnet à soi :
enregistrer un contact avec son nom, ses adresses et ses téléphones, le
corriger, le marquer comme favori, lui écrire en un clic et le supprimer ;
voir ses contacts sous le nom qu'on leur a donné dans le courrier,
les retrouver en premier en écrivant un message, et ajouter au carnet une
personne rencontrée dans un message ; regrouper des contacts dans des
listes de diffusion pour leur écrire en une fois.
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

- Accès : bouton **Nouveau contact** en haut de la liste, palette de
  commandes ou raccourci. Depuis le
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

### Nom des contacts dans le courrier

- Dans la liste des conversations, les messages ouverts (expéditeur et
  destinataires) et les résultats de recherche, une personne dont
  l'adresse est dans le carnet s'affiche sous le **nom du carnet**, quel
  que soit le nom choisi par l'expéditeur. L'adresse est reconnue sans
  tenir compte des majuscules ; une autre adresse de la même fiche nomme
  aussi le contact. Une fiche sans nom laisse l'adresse affichée.
- Dans un message ouvert, une petite **pastille** à côté du nom signale
  une personne du carnet (« Dans vos contacts » au survol).
- Une personne absente du carnet garde le nom donné par le message, sauf
  si ce nom ressemble à une adresse (il contient « @ », « < » ou « > ») :
  seule l'adresse réelle s'affiche alors, pour qu'un message ne puisse pas
  se faire passer pour une autre adresse. La même règle vaut pour le nom
  d'une fiche : un contact nommé comme une adresse s'affiche sous son
  adresse réelle.
- Après un ajout, une modification ou une suppression dans le carnet, les
  noms se mettent à jour au retour dans le courrier.

### Retrouver ses contacts en écrivant

- En tapant un destinataire (À, Cc, Cci), les suggestions montrent
  **d'abord les contacts du carnet**, favoris en tête, sous le nom du
  carnet et marqués « Dans vos contacts » ; viennent ensuite les adresses
  déjà utilisées dans le courrier du compte.
- La recherche porte sur le nom, le prénom, le surnom, l'organisation et
  toutes les adresses d'une fiche, sans tenir compte des majuscules. Un
  contact trouvé par son nom propose toutes ses adresses, la préférée
  d'abord ; trouvé par une adresse, seulement celle-ci.
- Une adresse n'est proposée qu'une fois : présente dans le carnet et
  dans le courrier, elle apparaît sous le nom du carnet.
- La liste reste courte ; un carnet momentanément illisible laisse les
  seules adresses déjà utilisées.

### Ajouter au carnet, voir le contact

- Dans un message ouvert, un clic sur le **nom** de l'expéditeur ou d'un
  destinataire ouvre son menu :
  - **Ajouter au carnet d'adresses** pour une personne absente du carnet :
    la fenêtre d'un nouveau contact s'ouvre, préremplie avec son nom et son
    adresse, sans quitter le courrier ;
  - **Voir le contact** pour une personne du carnet : le carnet s'ouvre
    sur sa fiche.
- La même entrée est proposée pour le dernier expéditeur d'une
  conversation dans le menu du **clic droit** sur une ligne de la liste,
  et dans **Plus…** du message ouvert. Palette de commandes : elle vise
  l'expéditeur du dernier message de la conversation ouverte.

### Listes de diffusion

- Une liste regroupe des **contacts du carnet** sous un nom
  (« Famille », « Club de lecture »), avec une description facultative.
  Elle ne garde pas d'adresses : chaque membre reçoit le message à son
  adresse **préférée du moment**. Changer l'adresse préférée d'un contact
  vaut donc pour toutes ses listes.
- Accès : bouton **Listes** à côté de **Tous** et **Favoris** dans la vue
  Contacts ; la colonne montre alors les listes (nom, nombre de membres,
  mention « Cci »), triées par nom, avec la même recherche (sur le nom).
  La liste choisie s'affiche à droite : description, envoi en copie
  cachée, membres avec l'adresse à laquelle chacun sera écrit.
- **Nouvelle liste** (bouton en haut de la vue, palette de commandes)
  ouvre une fenêtre : nom, description, case **Envoyer aux membres en
  copie cachée (Cci)**, et membres choisis en cherchant un contact par son
  nom ou son adresse ; un contact déjà membre est signalé, un membre se
  retire d'un clic. Une liste peut être enregistrée vide.
- Deux listes ne peuvent pas porter le même nom, majuscules comprises
  (« Famille » et « FAMILLE ») : la fenêtre reste ouverte et l'explique.
  Une liste compte au plus autant de membres qu'un message peut avoir de
  destinataires.
- Sur la liste ouverte : **Écrire à la liste**, **Modifier la liste**,
  **Supprimer la liste** (avec confirmation), aussi dans le menu du clic
  droit sur une ligne. Supprimer une liste garde ses contacts ; supprimer
  un contact le retire de toutes ses listes.
- **Ajouter à une liste…** (dans **Plus…** de la fiche d'un contact, le
  menu du clic droit sur un contact et la barre des contacts cochés)
  ouvre le choix de la liste ; les contacts qui en font déjà partie ne
  sont pas ajoutés deux fois.
- En écrivant un message, les listes dont le nom contient la saisie sont
  proposées en tête des suggestions, avec leur nombre de membres et la
  mention « Cci » le cas échéant ; une liste vide n'est pas proposée.
  Choisie, la liste est remplacée par une pastille par membre (chacun une
  fois, même s'il figurait déjà parmi les destinataires), comme si chaque
  adresse avait été saisie.
- Une liste « en Cci » range toujours ses membres en **copie cachée**,
  même choisie dans le champ À : aucun membre ne voit l'adresse des
  autres. Les pastilles se modifient ensuite comme les autres
  destinataires.

### Importer des contacts

- But : reprendre en une fois le carnet d'un ancien logiciel (Gmail,
  Outlook, Thunderbird, Contacts d'Apple, téléphone Android).
- **Importer…** en haut de la vue Contacts (dans **Plus…** si la colonne
  est trop étroite), le bouton du carnet vide, ou la palette de
  commandes, ouvre le sélecteur de fichiers du système. Fichiers lus :
  vCard (`.vcf`, toutes versions courantes, y compris les anciens exports
  d'Outlook et d'Android), CSV de Google Contacts, d'Outlook (anglais ou
  français, virgules ou points-virgules) et de Thunderbird, LDIF de
  Thunderbird. Le format est reconnu au contenu, pas au nom du fichier ;
  les accents sont gardés quel que soit l'encodage de l'export.
- Rien n'est écrit avant confirmation. Une fenêtre résume d'abord ce que
  l'import va faire : le fichier et son format, le nombre de nouveaux
  contacts, de fiches déjà dans le carnet, de fiches impossibles à
  importer. **Voir les fiches impossibles à importer** donne, pour
  chacune, son rang dans le fichier, son nom et la raison (illisible,
  sans adresse électronique, adresse invalide, champ trop long ou avec un
  caractère interdit, trop d'adresses, liste de diffusion).
- Une fiche est **déjà dans le carnet** si l'une de ses adresses y est,
  majuscules comprises ; deux fiches du fichier qui partagent une adresse
  n'en font qu'une. Quand il y en a, la fenêtre propose : **Les laisser
  tels quels** (par défaut) ou **Les compléter avec le fichier** : les
  champs vides du contact sont remplis, les adresses et téléphones qui
  lui manquent ajoutés ; rien n'est effacé ni remplacé, l'adresse
  préférée et le favori restent. Les chiffres suivent le choix.
- Un nom (nom affiché, prénom, nom, surnom) qui ressemble à une adresse
  électronique n'est pas repris : l'adresse s'affiche à sa place. La
  fenêtre le signale.
- **Importer** écrit tout en une fois : soit tout l'import réussit, soit
  rien ne change. Un message annonce ensuite le nombre de contacts
  ajoutés et complétés. Un contact ajouté entre l'aperçu et l'import est
  traité comme un doublon.
- Fichiers refusés en entier, avec un message clair : format inconnu (le
  message invite à exporter en vCard ou CSV), fichier trop gros (20 Mo)
  ou trop de contacts (25 000).

## Sécurité

- Tout ce qui est saisi est affiché comme du **texte**, jamais interprété.
- Les fiches sont revalidées par l'application avant tout enregistrement,
  quelle que soit la saisie de l'écran.
- Le carnet est stocké dans la base locale de l'application, avec les
  messages ; il n'est envoyé à aucun serveur.
- Un fichier importé est lu par l'application elle-même, taille limitée
  avant lecture ; l'écran ne reçoit que le résumé. Il ne fait charger
  aucune ressource extérieure (photo, lien). Les caractères invisibles ou
  trompeurs d'une fiche importée la font refuser.
- Le nom du carnet s'affiche d'après l'adresse annoncée par le message,
  que l'expéditeur peut falsifier : il ne prouve pas que le message vient
  bien de ce contact.

## Limites actuelles

- Les notifications de nouveaux messages gardent le nom donné par le
  message.
- Un nom de fiche qui ressemble à une adresse est accepté à la saisie
  (il est seulement écarté à l'affichage) ; l'import ne le reprend pas.
- Pas encore d'export (vCard, CSV, LDIF), ni de recherche des doublons
  déjà présents ; les listes ne s'importent ni ne s'exportent. Photos,
  adresses postales et anniversaires d'un fichier importé sont ignorés.
