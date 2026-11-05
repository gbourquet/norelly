# Synchronisation

[← Fonctionnalités](README.md)

## But

Tenir une copie locale fidèle de chaque compte (dossiers, messages,
drapeaux), à jour en quelques secondes, en économisant le réseau, et qui
converge après toute interruption.

## Comportement attendu

- Au démarrage de l'application (ou à l'ajout du compte), chaque compte
  synchronise l'arbre de ses dossiers puis chaque dossier.
- Les nouveaux messages de la **boîte de réception** arrivent en quelques
  secondes sans action de l'utilisateur (IMAP IDLE).
- Les autres dossiers sont resynchronisés **toutes les 10 minutes**, et à
  la demande (commande « Synchroniser le compte sélectionné »).
- Les en-têtes et le texte de tous les messages sont téléchargés ; les
  pièces jointes seulement sous un **seuil** (2 Mio par défaut, réglable) :
  au-delà, elles sont téléchargées à l'ouverture.
- Hors ligne, une icône (Wi-Fi barré) s'affiche à côté du compte, avec la
  raison au survol ; la reconnexion est automatique.
- Les dossiers affichent leur rôle (icône) et leur nombre de non lus.

## Règles métier

| Règle | Où |
|---|---|
| Rôle d'un dossier : attribut SPECIAL-USE (`\Sent`, `\Drafts`, `\Trash`, `\Junk`, `\Archive`, `\All`), sinon heuristique de nom | `crates/domain/src/mailbox.rs` |
| Stratégie par dossier : complète (première fois ou UIDVALIDITY changée) ; QRESYNC (`VANISHED`) ; CONDSTORE (drapeaux depuis `HIGHESTMODSEQ`) ; sans extension (comparaison des UID et drapeaux) ; rien si `HIGHESTMODSEQ` et `UIDNEXT` n'ont pas bougé | `crates/domain/src/sync.rs` (`plan`) |
| Propriétés : convergence (local = serveur), idempotence, application double sans effet (tests `proptest`) | `sync.rs`, `sync/tests.rs` |
| Rapatriement : en-têtes et texte pour tous ; pièces jointes sous le seuil ; structure MIME bornée (profondeur 16, 256 parties) | `crates/domain/src/fetch_policy.rs` |
| Seuil de rapatriement borné entre 64 Kio et 64 Mio | `crates/domain/src/settings.rs` |
| IDLE renouvelé toutes les 25 minutes (avant la limite des 29 minutes de la RFC 2177) | `sync.rs` (`IDLE_RENEWAL_SECS`) |
| Reconnexion : attente exponentielle de 1 s à 5 min, avec gigue, remise à zéro après un succès | `sync.rs` (`Backoff`) |
| Tout texte venu du serveur est assaini (contrôles retirés, longueur bornée) ; noms de boîte ≤ 1024 caractères, 10 000 boîtes au plus | `mailbox.rs` |

Orchestration : acteur de compte à deux connexions (commandes ; IDLE sur la
boîte de réception), `SyncAccount` boîte par boîte, enregistrement par lots
pour une reprise propre (`crates/application/src/account_actor.rs`,
`sync.rs`).

## Cas limites

- **UIDVALIDITY changée** : la boîte est resynchronisée entièrement ; les
  actions en attente sur cette boîte sont abandonnées (voir
  [actions](actions.md)).
- **Coupure en pleine synchronisation** : chaque lot intermédiaire est
  enregistré sans `HIGHESTMODSEQ` ; la reprise compare les UID. Vérifié
  par Toxiproxy (coupure, latence, connexion à moitié fermée).
- **`VANISHED` d'UID inconnus** localement : ignorés ; dépliage des plages
  borné.
- **Serveur sans CONDSTORE ni QRESYNC** : comparaison complète, plus
  coûteuse mais correcte.
- **COMPRESS=DEFLATE** : non activé (ADR-002) ; sans effet fonctionnel.
- Message dont le corps est partiel (pièce au-dessus du seuil) : la partie
  est téléchargée à la demande par la connexion de commandes.

## Messages affichés

| Clé | Texte (fr) |
|---|---|
| `common_offline` | Hors ligne |
| `sidebar_offline_reason` | Hors ligne : {reason} |
| `failure_*` | raison (serveur injoignable, identifiants refusés, connexion refusée par sécurité, réponse du serveur non conforme…) |
| `message_notice_not_available` | Le contenu de ce message n'est pas encore disponible. |

## Raccourcis

| Commande | Raccourci |
|---|---|
| `account.sync` (Synchroniser le compte sélectionné) | palette |
