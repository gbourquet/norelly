# Recherche plein texte

[← Fonctionnalités](README.md)

## But

Retrouver un message en quelques frappes, parmi des dizaines de milliers,
hors ligne, avec quelques filtres simples.

## Comportement attendu

- `/` ou la loupe de l'en-tête de la liste remplace la liste par le panneau
  de recherche ; la recherche se fait **pendant la frappe**.
- Résultats triés par pertinence (sinon par date), avec le sujet,
  l'expéditeur et un extrait où les termes trouvés sont **surlignés**.
- `j`/`k` parcourent les résultats ; Entrée ouvre la conversation **dans le
  dossier du message** (les actions s'y appliquent) ; Échap revient aux
  résultats, puis ferme la recherche.
- Portée : tous les comptes (ou un compte, un dossier, selon l'appelant).
- Fonctionne **hors ligne** : l'index est local.

## Règles métier

Syntaxe (`crates/domain/src/search.rs`, `parse_query`) :

| Syntaxe | Effet |
|---|---|
| mots libres | cherchés partout (sujet, expéditeur, destinataires, texte), **en préfixe** à partir de deux lettres (`fact` trouve « facture ») |
| `"phrase exacte"` | expression ; guillemet final facultatif |
| `from:`, `to:` (À et Cc), `subject:` | sur un champ, suivis d'un mot ou d'une phrase |
| `has:attachment`, `is:unread`, `is:read`, `is:flagged` | filtres |
| `before:AAAA-MM-JJ`, `after:AAAA-MM-JJ` | `before` exclut le jour, `after` l'inclut ; début du jour selon le fuseau de l'utilisateur |
| `-clause` | exclusion |

| Règle | Où |
|---|---|
| Clauses combinées par « et » ; opérateur inconnu ou valeur invalide (`is:bleu`, `before:hier`) cherchés comme du texte : une faute de frappe ne fait jamais échouer | `search.rs` |
| Requête bornée : 512 caractères, 16 clauses | `search.rs` (`MAX_QUERY_CHARS`, `MAX_CLAUSES`) |
| Casse et diacritiques ignorés (pliage aligné sur `unicode61 remove_diacritics 2`) ; surlignage en segments de texte, jamais en HTML | `search.rs` |
| Chaque clause devient une chaîne FTS5 entre guillemets, en paramètre lié : aucun mot de l'utilisateur n'est lu comme un opérateur | `crates/adapters/sqlite/src/search.rs` |
| Classement BM25 pondéré (sujet 10, expéditeur 5, destinataires 3, texte 1) ; 1 000 résultats au plus ; doublons de `\All` (Gmail) écartés | `search.rs` (sqlite), ADR-003 |
| Texte indexé : partie texte, sinon HTML réassaini réduit au texte, pièces jointes exclues, 100 000 caractères au plus | `crates/adapters/render/src/search_text.rs` |
| Une recherche est interrompue au-delà de 2 s | `crates/adapters/sqlite/src/search.rs` |

## Cas limites

- Pas de racinisation ni de tolérance aux fautes (ADR-003) : « cheval » ne
  trouve pas « chevaux ».
- Requête trop longue ou trop de critères : `err_invalid_input` avec le
  champ `field_query`.
- Requêtes hostiles (`OR`, `NEAR`, `col:`, `^`, `*`, parenthèses…) :
  cherchées comme du texte (tests de contrat).
- Mesure (bench, 50 000 messages de 300 mots) : 0,3 à 89 ms selon la
  requête, seuil 100 ms.

## Messages affichés

| Clé | Texte (fr) |
|---|---|
| `search_placeholder` | Rechercher… (from:, subject:, has:attachment, "phrase") |
| `search_none` | Aucun message trouvé. |
| `search_close` | Fermer la recherche |

## Raccourcis

| Commande | Raccourci |
|---|---|
| `search.open` (Rechercher dans les messages) | `/` |
| `conversation.next`, `conversation.previous` (dans les résultats) | `j`, `k` |
| `conversation.open`, `conversation.back` | `Entrée`, `Échap` |
