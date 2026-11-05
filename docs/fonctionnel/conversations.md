# Liste des conversations et boîte unifiée

[← Fonctionnalités](README.md)

## But

Présenter les messages d'un dossier, ou de toutes les boîtes de réception,
regroupés par conversation, instantanément même sur 50 000 messages.

## Comportement attendu

- La liste du panneau central montre une ligne par **conversation** :
  expéditeurs (trois au plus), nombre de messages, date du plus récent,
  sujet, indicateurs (pièce jointe, suivi), non lus en gras.
- Tri par date décroissante du message le plus récent.
- **Boîte de réception unifiée** : les conversations des boîtes de réception
  de tous les comptes, mêlées par date.
- Ouvrir une conversation affiche ses messages empilés, du plus ancien au
  plus récent ; un clic sur l'en-tête d'un message le replie ou le déplie.
- La liste est **virtualisée** : seules les lignes visibles sont rendues ;
  les pages sont demandées à Rust au fil du défilement.
- La largeur des panneaux se règle (souris ou clavier) et est mémorisée.

## Règles métier

| Règle | Où |
|---|---|
| Regroupement JWZ : `References`, puis `In-Reply-To`, puis sujet en dernier recours | `crates/domain/src/conversation.rs` (`thread`) |
| Version incrémentale (`assign`) : un message rejoint toute conversation dont il connaît un identifiant ; fusion si besoin ; converge vers JWZ quel que soit l'ordre d'arrivée (propriété testée) | `conversation.rs` |
| Regroupement par sujet seulement pour une **réponse** (préfixe `Re:`, `Fwd:`… ou références sans correspondance), dans une fenêtre de 31 jours | `conversation.rs` (`SUBJECT_WINDOW_SECS`) |
| Préfixes reconnus : `Re`, `RE`, `Fwd`, `Fw`, `TR`, `AW`, `WG`, `SV`, `VS`, `Antw`, `RIF`, `R`, `RE :`… (liste fixe) | `conversation.rs` (`strip_prefix`) |
| Une conversation n'est jamais scindée quand un message qui la reliait disparaît | ADR-003, `crates/adapters/sqlite/src/conversations.rs` |
| Pages de 500 au plus ; 500 messages au plus par conversation affichée | `crates/domain/src/ports.rs` (`MAX_PAGE_SIZE`, `MAX_CONVERSATION_MESSAGES`) |
| Lecture paginée par curseur (CQRS léger, ADR-008) | ports `ConversationQuery`, `MessageListQuery` |

## Cas limites

- Deux messages « Bonjour » sans rapport : jamais réunis (pas une réponse).
- Une réponse arrivée avant le message d'origine : rattachée à l'arrivée de
  l'original (fusion).
- Dans la boîte unifiée, les actions visent chaque conversation dans son
  compte ; « déplacer » exige des conversations d'un seul compte.
- Mesures (bench, 50 000 messages) : première page ≈ 0,3 ms ; changement de
  dossier de bout en bout 13 à 16 ms (PLAN, phase 9).

## Messages affichés

| Clé | Texte (fr) |
|---|---|
| `list_empty` | Aucune conversation. |
| `mailbox_unified_inbox` | Boîte de réception unifiée |
| `common_no_subject` | (sans objet) |
| `common_unknown_sender` | (expéditeur inconnu) |
| `list_marked` | {count} sélectionnée(s) |

## Raccourcis

| Commande | Raccourci |
|---|---|
| `conversation.next` (Conversation suivante) | `j` |
| `conversation.previous` (Conversation précédente) | `k` |
| `conversation.open` (Ouvrir la conversation) | `Entrée` |
| `conversation.back` (Retour à la liste) | `Échap` |
| `mailbox.unified` (Boîte de réception unifiée) | palette |
