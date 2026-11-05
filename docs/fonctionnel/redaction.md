# Rédaction : nouveau message, réponse, transfert

[← Fonctionnalités](README.md)

## But

Écrire, répondre et transférer avec une mise en forme simple et des pièces
jointes, en produisant des messages corrects pour tous les clients
(fils de discussion, sujets, citations), sans que l'éditeur devienne une
surface d'attaque.

## Comportement attendu

- **Nouveau message** (`c`) : depuis le compte sélectionné.
- **Répondre** (`r`), **Répondre à tous** (`a`), **Transférer** (`f`) :
  depuis le dernier message de la conversation ouverte. Destinataires,
  sujet et citation sont préremplis ; le transfert reprend les pièces
  jointes d'origine.
- Champs : À, Cc et Cci (affichés à la demande), Objet, corps.
  Destinataires en jetons, avec **autocomplétion** depuis les adresses
  connues (flèches, Entrée, virgule ou Tab pour valider).
- Éditeur (Tiptap) : gras, italique, barré, lien, listes à puces et
  numérotées, citation.
- **Pièces jointes** : bouton « Joindre des fichiers » (sélecteur ouvert par
  Rust) ou glisser-déposer ; taille affichée ; retrait par la croix.
- **Ctrl+Entrée** envoie ; **Échap** ferme en gardant le brouillon
  ([brouillons](brouillons.md)) ; « Supprimer le brouillon » l'abandonne.
- Envoi immédiat, ou « Envoyer plus tard » ([envoi](envoi.md)).

## Règles métier

| Règle | Où |
|---|---|
| Destinataires de « répondre » : `Reply-To` prioritaire, sinon `From` ; « répondre à tous » ajoute À et Cc ; ses propres adresses exclues, doublons retirés, adresses invalides ignorées ; répondre à son propre message vise ses destinataires | `crates/domain/src/compose.rs` (`reply_recipients`) |
| Sujet : toujours `Re:` / `Fwd:`, sans empilement (préfixes étrangers reconnus puis retirés : `RE :`, `AW:`, `TR:`, `SV:`, `Antw:`…) | `compose.rs` (`reply_subject`, `forward_subject`), ADR-011 |
| `In-Reply-To` et `References` (bornées à 20 : racine et plus récents) | `compose.rs` (`reply_threading`, `MAX_REFERENCES`) |
| Citation : en-tête « Le … a écrit : » construit par l'interface dans la langue de l'utilisateur, inséré tel quel par le domaine | `compose.rs` (`quote`), ADR-011 |
| Validation avant envoi : au moins un destinataire, 100 au plus ; sujet sans caractère de contrôle, 998 caractères au plus ; corps ≤ 5 Mio ; pièces jointes ≤ 25 Mio au total et 100 au plus | `crates/domain/src/compose/draft.rs` (`check_sendable`) |
| HTML de l'éditeur réassaini en Rust (liste blanche minimale : p, br, gras, italique, barré, liens http/https/mailto, listes, citations) ; version texte dérivée du HTML assaini ; envoi en `multipart/alternative` | `crates/adapters/render/src/compose.rs`, ADR-010 |
| `Bcc` seulement dans les copies locales, jamais dans les octets transmis ; `Message-ID` aléatoire de 128 bits, sans nom de machine | `render/src/compose.rs` |
| Type MIME des pièces jointes recalculé en Rust d'après le nom ; taille réelle revérifiée ; chemins des fichiers jamais transmis par l'interface (jeton de dépôt) | `crates/application/src/compose.rs` (`QueueMessage`) |
| Transfert : pièces relues depuis le stockage local, message entier téléchargé si le corps est partiel ; mêmes bornes que l'envoi | `compose.rs` (`PrepareReply`), port `RemoteBodyFetcher` |

## Cas limites

- HTML saisi littéralement dans l'éditeur (`<b>…</b>`) : envoyé comme du
  texte, jamais interprété (test e2e `compose.e2e.mjs`).
- Imbrication au-delà de 64 niveaux : réduite au texte.
- Pièces jointes du transfert au-delà de 100 ou de 25 Mio : refus.
- Brouillon relu depuis la base : revalidé (`check_storable`).

## Messages affichés

| Clé | Texte (fr) |
|---|---|
| `compose_title_new`, `compose_title_reply`, `compose_title_forward` | Nouveau message, Répondre, Transférer |
| `compose_attribution` | Le {date}, {name} a écrit : |
| `compose_forward_intro` | Message transféré de {name}, le {date} : |
| `err_no_recipient` | Ajoutez au moins un destinataire. |
| `err_too_many_recipients` | Trop de destinataires. |
| `err_invalid_subject` | Le sujet contient un caractère interdit ou est trop long. |
| `err_body_too_large` | Le message est trop long. |
| `err_attachments_too_large` | Les pièces jointes dépassent 25 Mio. |
| `err_too_many_attachments` | Trop de pièces jointes (100 au plus). |
| `compose_queued` | Message en cours d'envoi. |

## Raccourcis

| Commande | Raccourci |
|---|---|
| `compose.new` (Nouveau message) | `c` |
| `compose.reply` (Répondre) | `r` |
| `compose.reply-all` (Répondre à tous) | `a` |
| `compose.forward` (Transférer) | `f` |
| Envoyer (dans l'éditeur, hors registre) | `Ctrl+Entrée` |
