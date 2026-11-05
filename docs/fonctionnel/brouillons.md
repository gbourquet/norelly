# Brouillons

[← Fonctionnalités](README.md)

## But

Ne jamais perdre ce qui est en cours d'écriture, retrouver ses brouillons
et reprendre ceux commencés sur un autre client.

## Comportement attendu

- Le brouillon est enregistré **localement à chaque pause de frappe** ; une
  copie est déposée sur le serveur, dans le dossier « Brouillons ».
- Fermer l'éditeur (Échap, croix) garde le brouillon.
- « Brouillons… » (palette) liste les brouillons du compte sélectionné :
  - les **brouillons locaux** (sujet, destinataires, date) ;
  - sous « Dans le dossier Brouillons du serveur », les brouillons écrits
    par **un autre client** (hors copies des brouillons locaux).
- Choisir un brouillon le rouvre dans l'éditeur ; un brouillon d'un autre
  client est repris tel quel (destinataires, sujet, corps, fil).
- À l'envoi, le brouillon passe dans la file d'envoi ; sa copie sur le
  serveur est retirée une fois le message parti.
- « Supprimer le brouillon » le supprime localement et sur le serveur.

## Règles métier

| Règle | Où |
|---|---|
| Chaque enregistrement local incrémente la révision (`DraftSync::edited`) | `crates/domain/src/compose/draft.rs` |
| Copie sur le serveur seulement si la révision a changé, **au plus toutes les 30 s** (`REMOTE_SAVE_INTERVAL_SECS`) ; elle remplace l'ancienne | `draft.rs` (`remote_save_due`) |
| Sans UIDPLUS, les brouillons restent locaux (pas de remplacement fiable) | `crates/adapters/imap/src/session.rs`, PLAN phase 7 |
| Une ancienne copie n'est supprimée que si la boîte a la même UIDVALIDITY et que le message porte `\Draft` | ADR-002, PLAN phase 7 |
| Copie refusée par le serveur (`NO`) ou impossible à construire : journalisée, nouvel essai une heure plus tard ; les brouillons suivants continuent | `draft.rs` (`remote_refused`, `REMOTE_REFUSED_RETRY_SECS`) |
| Brouillon d'un autre client repris avec son fil (`resumed_threading`) | `crates/domain/src/compose.rs`, `crates/application/src/compose.rs` (`PrepareReply::resume`) |

## Cas limites

- Dossier « Brouillons » disparu ou refusé : les copies distantes en
  attente de suppression sont oubliées, la boîte enregistrée avec chaque
  copie (migration 8).
- Brouillon programmé pour un envoi différé : il reste attaché au message
  dans la file et revient dans l'éditeur par « Modifier » (voir
  [envoi](envoi.md)).

## Messages affichés

| Clé | Texte (fr) |
|---|---|
| `drafts_title` | Brouillons |
| `drafts_empty` | Aucun brouillon enregistré. |
| `drafts_to` | À : {recipients} |
| `drafts_server` | Dans le dossier Brouillons du serveur |
| `compose_discard` | Supprimer le brouillon |

## Raccourcis

| Commande | Raccourci |
|---|---|
| `drafts.open` (Brouillons…) | palette |
