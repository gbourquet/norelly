# Clavier : commandes, palette, raccourcis, accessibilité

[← Fonctionnalités](README.md)

## But

Que toute action soit accessible au clavier, de façon cohérente et facile à
découvrir, et que l'interface soit utilisable avec un lecteur d'écran.

## Comportement attendu

- **Chaque action est une commande**, avec un libellé, un groupe et,
  souvent, un raccourci. Une même commande est accessible par son
  raccourci, la **palette** (`Ctrl+K`), l'**aide** (`?`), les barres
  d'outils, les boutons au survol et le menu contextuel : elle fait partout
  la même chose.
- **Palette** : recherche par libellé, groupe et mots-clés ; seules les
  commandes possibles à cet instant sont proposées ; Entrée exécute.
- **Aide** (`?`) : toutes les commandes qui ont un raccourci, par groupe.
- Raccourcis à une touche, à la Gmail : inactifs pendant la saisie dans un
  champ (seuls ceux avec Ctrl, Alt ou Méta y fonctionnent).
- Les raccourcis fonctionnent aussi quand le focus est dans un message
  affiché.
- **Accessibilité** :
  - focus visible partout ;
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
| Actions | Archiver (`e`), Supprimer (`#`), Suivre (ou ne plus suivre) (`s`), Marquer comme lu (`Maj+I`), Marquer comme non lu (`Maj+U`), Déplacer vers… (`v`), Sélectionner la conversation (`x`), Annuler la dernière action (`z`) |
| Rédaction | Nouveau message (`c`), Répondre (`r`), Répondre à tous (`a`), Transférer (`f`), Brouillons…, Boîte d'envoi |
| Comptes | Ajouter un compte…, Choisir le dossier d'archives…, Synchroniser le compte sélectionné, Supprimer le compte sélectionné… |

Touches propres à un élément de l'interface : `Ctrl+Entrée` (envoyer,
fenêtre de rédaction) ; flèches, Entrée, virgule, Tab (champ des
destinataires) ; flèches, Début, Fin (séparateurs) ; Espace (sélection
d'une ligne de liste).

## Cas limites

- Commande impossible à cet instant (par exemple « Répondre » sans
  conversation ouverte) : son raccourci ne fait rien, elle n'apparaît pas
  dans la palette.
- **Écrire, répondre et transférer n'ont pas de bouton** : seulement leur
  raccourci et la palette.

## Messages affichés

| Situation | Texte |
|---|---|
| Palette | Rechercher une commande…, Aucune commande |
| Aide | Raccourcis clavier, Toutes les commandes sont aussi accessibles par la palette : |
| Noms des touches | Maj, Échap, Espace |
