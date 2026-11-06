# Commandes : souris, clavier, palette, accessibilité

[← Fonctionnalités](README.md)

## But

Que toute action soit faisable à la souris par un élément visible et
compréhensible, sans connaître aucun raccourci ; que le clavier serve
d'accélérateur, de façon cohérente et facile à découvrir ; et que
l'interface soit utilisable avec un lecteur d'écran.

## Comportement attendu

- **Aucune action n'est réservée au clavier** : chacune a un équivalent
  visible à la souris (bouton libellé, menu « Plus… », menu d'un compte,
  menu contextuel, case à cocher).
- **Chaque action est une commande**, avec un libellé, un groupe et,
  souvent, un raccourci. Une même commande est accessible par son
  raccourci, la **palette** (`Ctrl+K`), l'**aide** (`?`), les barres
  d'actions, le menu « Plus… », les boutons au survol et le menu
  contextuel : elle fait partout la même chose.
- **Infobulles** : chaque bouton affiche son libellé et son raccourci,
  sous la forme « Archiver — E » (le libellé seul s'il n'y a pas de
  raccourci). Les menus affichent le raccourci à droite de chaque entrée.
- **Bouton « ? »**, toujours visible en haut du panneau de navigation : il
  ouvre l'aide des raccourcis, d'où un bouton ouvre aussi la palette.
- **Palette** : recherche par libellé, groupe et mots-clés ; seules les
  commandes possibles à cet instant sont proposées ; Entrée exécute.
- **Aide** (`?` ou bouton « ? ») : toutes les commandes qui ont un
  raccourci, par groupe, puis les touches de la fenêtre de rédaction
  (envoi, fermeture, mise en forme).
- Chaque fenêtre a un bouton **Fermer**, en plus d'Échap et du clic à
  l'extérieur ; à l'ouverture, le focus va au premier champ, pas à ce
  bouton.
- Raccourcis à une touche, à la Gmail : inactifs pendant la saisie dans un
  champ (seuls ceux avec Ctrl, Alt ou Méta y fonctionnent).
- Les raccourcis fonctionnent aussi quand le focus est dans un message
  affiché.
- **Accessibilité** :
  - focus visible partout, ordre de tabulation logique ;
  - listes parcourues aux flèches et annoncées par les lecteurs d'écran ;
  - nom lisible par un lecteur d'écran pour chaque bouton et chaque champ ;
  - contrastes conformes au niveau AA des WCAG, en thème clair comme en
    thème sombre ;
  - séparateurs de panneaux réglables au clavier (flèches, Début, Fin).

## Règles

- Un raccourci ne déclenche jamais deux commandes.
- `Maj` compte pour les lettres (`Maj+I` n'est pas `i`), pas pour les
  caractères qui l'exigent déjà sur le clavier (`?`).
- Chaque commande figure dans l'aide-mémoire de la documentation
  utilisateur, en français et en anglais.

## Liste des commandes

La liste est reprise, avec les mêmes libellés, dans
[l'aide-mémoire utilisateur](../utilisateur/fr/shortcuts.md).

| Groupe | Commandes (raccourci) |
|---|---|
| Général | Palette de commandes (`Ctrl+K`), Aide des raccourcis clavier (`?`), Paramètres… (`Ctrl+,`) |
| Lecture | Conversation suivante (`j`), Conversation précédente (`k`), Ouvrir la conversation (`Entrée`), Retour à la liste (`Échap`), Rechercher dans les messages (`/`), Boîte de réception unifiée, Préférer le texte brut (ou le HTML) |
| Actions | Archiver (`e`), Supprimer (`#`), Suivre (ou ne plus suivre) (`s`), Marquer comme lu (`Maj+I`), Marquer comme non lu (`Maj+U`), Déplacer vers… (`v`), Sélectionner la conversation (`x`), Tout désélectionner, Annuler la dernière action (`z`) |
| Rédaction | Nouveau message (`c`), Répondre (`r`), Répondre à tous (`a`), Transférer (`f`), Brouillons…, Boîte d'envoi |
| Comptes | Ajouter un compte…, Choisir le dossier d'archives…, Synchroniser le compte sélectionné, Supprimer le compte sélectionné… |

Touches propres à un élément de l'interface : `Ctrl+Entrée` (envoyer,
fenêtre de rédaction) ; `Ctrl+B`, `Ctrl+I`, `Ctrl+Maj+S`, `Ctrl+Maj+8`,
`Ctrl+Maj+7`, `Ctrl+Maj+B` (gras, italique, barré, listes, citation,
dans l'éditeur) ; flèches, Entrée, virgule, Tab (champ des
destinataires) ; flèches, Début, Fin (séparateurs) ; Espace (sélection
d'une ligne de liste).

## Cas limites

- Commande impossible à cet instant (par exemple « Répondre » sans
  conversation ouverte) : son raccourci ne fait rien, elle n'apparaît pas
  dans la palette.
- Répondre, répondre à tous ou transférer depuis la liste (menu
  contextuel, raccourci) avec une seule conversation visée : elle s'ouvre,
  puis la fenêtre de rédaction. Avec des conversations cochées : indisponible.
- Une commande indisponible reste visible dans les barres et les menus,
  grisée.

## Messages affichés

| Situation | Texte |
|---|---|
| Palette | Rechercher une commande…, Aucune commande |
| Aide | Raccourcis clavier, Dans la fenêtre de rédaction, Envoyer le message, Fermer en gardant le brouillon, Toutes les commandes sont aussi accessibles par la palette :, Palette de commandes |
| Infobulle | {label} — {keys} |
| Fenêtres | Fermer |
| Noms des touches | Maj, Échap, Espace, Entrée |
