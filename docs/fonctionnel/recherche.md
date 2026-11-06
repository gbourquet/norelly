# Recherche plein texte

[← Fonctionnalités](README.md)

## But

Retrouver un message en quelques frappes, parmi des dizaines de milliers,
hors ligne, avec quelques filtres simples.

## Comportement attendu

- Le bouton **Rechercher** de l'en-tête de la liste (ou `/`) remplace la
  liste par le panneau de recherche ; la recherche se fait **pendant la frappe**.
- Résultats triés par pertinence (sinon par date), avec le sujet,
  l'expéditeur et un extrait où les termes trouvés sont **surlignés**.
- `j`/`k` parcourent les résultats ; Entrée ouvre la conversation **dans le
  dossier du message** (les actions s'y appliquent) ; Échap revient aux
  résultats, puis ferme la recherche.
- Portée : tous les comptes.
- Fonctionne **hors ligne** : la recherche porte sur la copie locale.

## Règles

Syntaxe :

| Syntaxe | Effet |
|---|---|
| mots libres | cherchés partout (sujet, expéditeur, destinataires, texte), **en début de mot** à partir de deux lettres (`fact` trouve « facture ») |
| `"phrase exacte"` | expression ; guillemet final facultatif |
| `from:`, `to:` (À et Cc), `subject:` | sur un champ, suivis d'un mot ou d'une phrase |
| `has:attachment`, `is:unread`, `is:read`, `is:flagged` | filtres : avec pièce jointe, non lu, lu, suivi |
| `before:AAAA-MM-JJ`, `after:AAAA-MM-JJ` | `before` exclut le jour, `after` l'inclut ; la journée commence à minuit, heure de l'utilisateur |
| `-critère` | exclusion |

- Les critères se cumulent (tous doivent être vérifiés).
- **Une faute de frappe ne fait jamais échouer la recherche** : un filtre
  inconnu ou une valeur invalide (`is:bleu`, `before:hier`) sont cherchés
  comme du texte.
- Requête de 512 caractères et 16 critères au plus.
- Majuscules et accents ignorés : `ecole` trouve « École ».
- Pertinence : un mot trouvé dans le sujet compte plus que dans
  l'expéditeur, puis les destinataires, puis le texte.
- 1 000 résultats au plus ; un même message présent dans plusieurs
  dossiers (Gmail) n'apparaît qu'une fois.
- Texte cherché : le texte du message (ou, à défaut, sa version mise en
  forme réduite au texte), sans les pièces jointes, jusqu'à 100 000
  caractères par message.
- Une recherche trop longue est interrompue au-delà de 2 secondes.

## Cas limites

- Pas de recherche des formes voisines d'un mot ni de tolérance aux fautes
  : « cheval » ne trouve pas « chevaux ».
- Requête trop longue ou trop de critères : refusée, avec un message sur le
  champ de recherche.
- Mots ou symboles qu'un moteur de recherche pourrait lire comme des
  opérateurs (`OR`, `NEAR`, `*`, parenthèses…) : cherchés comme du texte.

## Messages affichés

| Situation | Texte |
|---|---|
| Champ de recherche | Rechercher… (from:, subject:, has:attachment, "phrase") |
| Aucun résultat | Aucun message trouvé. |
| Fermeture | Fermer la recherche |

## Raccourcis

| Commande | Raccourci |
|---|---|
| Rechercher dans les messages | `/` |
| Résultat suivant, précédent | `j`, `k` |
| Ouvrir, revenir | `Entrée`, `Échap` |
