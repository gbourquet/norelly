# Clavier : registre de commandes, palette, raccourcis, accessibilité

[← Fonctionnalités](README.md)

## But

Que toute action soit accessible au clavier, de façon cohérente et
découvrable, et que l'interface soit utilisable avec un lecteur d'écran.

## Comportement attendu

- **Registre de commandes central** : chaque action est une commande
  (identifiant, libellé, groupe, raccourci facultatif, condition
  d'activation). Il alimente les raccourcis, la **palette** (`Ctrl+K`),
  l'**aide** (`?`), les barres d'outils, les boutons au survol et le menu
  contextuel (ADR-005).
- **Palette** : recherche par libellé, groupe et mots-clés ; seules les
  commandes actives dans le contexte sont proposées ; Entrée exécute.
- **Aide** (`?`) : toutes les commandes qui ont un raccourci, par groupe.
- Raccourcis à une touche, à la Gmail : inactifs dans un champ de saisie
  (seuls ceux avec Ctrl, Alt ou Méta y fonctionnent).
- Les touches pressées dans le cadre d'un message sont relayées au
  registre.
- Accessibilité : focus visible partout, listes en `listbox`/`option` avec
  `aria-activedescendant`, nom accessible pour chaque bouton et champ,
  contrastes WCAG AA dans les deux thèmes, séparateurs de panneaux
  manipulables au clavier (flèches, Début, Fin).

## Règles métier

| Règle | Où |
|---|---|
| Un identifiant ou un raccourci déjà pris est refusé à l'enregistrement : un raccourci ne déclenche jamais deux commandes (`Ctrl+K` et `mod+k` sont le même) | `app/src/lib/commands/registry.ts` |
| `Maj` ignoré pour les caractères qui en dépendent (`?`), pas pour les lettres (`Maj+I` ≠ `i`) | `app/src/lib/commands/shortcut.ts` |
| Le registre ne prend aucune décision de sécurité : une commande appelle l'interface ou une commande IPC revalidée en Rust | `registry.ts` |
| Messages `postMessage` de l'agent validés (source, schéma, taille) | `app/src/lib/agent.ts`, ADR-004 |
| Chaque commande figure dans l'aide-mémoire de la documentation utilisateur, dans les deux langues | `app/src/docs.test.ts` (via `scripts/check-docs.sh`) |

## Liste des commandes

La liste exacte est dans `registerAppCommands`
(`app/src/lib/app.svelte.ts`) ; elle est reprise, avec ses libellés, dans
[l'aide-mémoire utilisateur](../utilisateur/fr/shortcuts.md).

| Groupe | Commandes (raccourci) |
|---|---|
| Général | `palette.open` (`Ctrl+K`), `help.shortcuts` (`?`), `settings.open` (`Ctrl+,`) |
| Lecture | `conversation.next` (`j`), `conversation.previous` (`k`), `conversation.open` (`Entrée`), `conversation.back` (`Échap`), `search.open` (`/`), `mailbox.unified`, `reading.plain-text` |
| Actions | `action.archive` (`e`), `action.delete` (`#`), `action.flag` (`s`), `action.mark-read` (`Maj+I`), `action.mark-unread` (`Maj+U`), `action.move` (`v`), `action.toggle-mark` (`x`), `action.undo` (`z`) |
| Rédaction | `compose.new` (`c`), `compose.reply` (`r`), `compose.reply-all` (`a`), `compose.forward` (`f`), `drafts.open`, `compose.outbox` |
| Comptes | `account.add`, `account.archive-folder`, `account.sync`, `account.remove` |

Hors registre, propres à un composant : `Ctrl+Entrée` (envoyer, éditeur) ;
flèches, Entrée, virgule, Tab (champ des destinataires) ; flèches, Début,
Fin (séparateurs) ; Espace (sélection d'une ligne de liste).

## Cas limites

- Commande inactive dans le contexte (par exemple « Répondre » sans
  conversation ouverte) : son raccourci ne fait rien, elle n'apparaît pas
  dans la palette.
- **Écrire, répondre et transférer n'ont pas de bouton** : seulement leur
  raccourci et la palette (constat de la rédaction de la documentation,
  phase 11).

## Messages affichés

| Clé | Texte (fr) |
|---|---|
| `palette_placeholder`, `palette_empty` | Rechercher une commande…, Aucune commande |
| `help_title`, `help_palette_hint` | Raccourcis clavier, Toutes les commandes sont aussi accessibles par la palette : |
| `key_shift`, `key_escape`, `key_space` | Maj, Échap, Espace |
