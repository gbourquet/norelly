# Lecture sécurisée

[← Fonctionnalités](README.md)

## But

Afficher fidèlement un message, y compris son HTML, sans qu'il puisse
exécuter de code, joindre le réseau, pister la lecture ou atteindre
l'application ; ouvrir liens et pièces jointes sans danger.

## Comportement attendu

- Le corps HTML est affiché dans un cadre isolé (`mailview://`), à la
  hauteur de son contenu. Les raccourcis clavier restent actifs quand le
  focus est dans le message.
- **Images distantes bloquées** par défaut, avec un bandeau et le nombre
  d'images ; « Afficher les images de cet expéditeur » les autorise pour
  cet expéditeur, durablement. Les images intégrées (`cid:`) s'affichent.
- **Liens** : un clic ouvre le navigateur du système, seulement pour
  `http`, `https` et `mailto`.
- **Pièces jointes** : nom et taille ; « Ouvrir » (application du système)
  et « Enregistrer… » (sélecteur de dossier). Une pièce non encore
  téléchargée l'est à la demande.
- **Texte brut** : bouton par message, et préférence globale.
- Ouvrir un message **ne le marque pas comme lu** : les corps sont lus sans
  poser `\Seen` (`BODY.PEEK`), et l'interface ne marque rien
  automatiquement. L'utilisateur marque lu avec `Maj+I` ou le bouton de la
  barre d'actions (constat de la phase 11, à trancher par le mainteneur).

## Règles métier

| Règle | Où |
|---|---|
| Liens ouverts : schémas `http`, `https`, `mailto` uniquement, 2048 caractères au plus | `crates/domain/src/reading.rs` (`MAX_LINK_LEN`) |
| Images distantes : autorisation par adresse `From` (falsifiable : protège du pistage, pas de l'usurpation) | `reading.rs`, ADR-004 |
| Nom de fichier assaini : séparateurs, `..`, noms réservés Windows, caractères de contrôle et de sens d'écriture (`exe.pdf` déguisé), 200 octets au plus | `reading.rs` (`safe_file_name`) |
| Ouverture directe réservée à une liste blanche d'extensions (documents sans macros, images matricielles, texte, sons, vidéos) ; tout le reste s'enregistre seulement | `reading.rs` (`OPENABLE_EXTENSIONS`, `is_unsafe_to_open`) |
| Enregistrement : création exclusive dans le dossier choisi, jamais d'écrasement ni de lien suivi | `app/src-tauri/src/reading.rs`, ADR-004 |
| HTML assaini par liste blanche (`ammonia`), CSS assaini (`@import`, `url()` externes retirés), `cid:` → `mailpart://`, images distantes → espace réservé ou `remote-img://` | `crates/adapters/render/src/sanitize.rs`, `css.rs` |
| CSP du document : `default-src 'none'`, seul script autorisé : l'agent (empreinte SHA-256) ; cadre `sandbox="allow-scripts"` sans `allow-same-origin` | ADR-004, `app/src-tauri/src/protocols.rs` |
| Proxy d'images : sans cookies ni `Referer`, User-Agent neutre, taille et types bornés, adresses publiques seulement, autorisation revérifiée à chaque image | `app/src-tauri/src/remote_images.rs` |

## Cas limites

- Message trop imbriqué ou trop long pour le HTML : notice et repli
  (`message_notice_too_complex`, `message_notice_too_long`).
- Corps pas encore téléchargé : notice `message_notice_not_available`.
- Expéditeur sans adresse valide : autorisation des images impossible
  (`err_no_valid_sender`).
- Lien `javascript:`, `file:`, `data:`… : refusé (`err_link_refused`).
- Pièce exécutable ou à macros : « Ouvrir » refusé
  (`err_executable_attachment`) ; « Enregistrer… » reste possible.
- Une image distante ne peut viser qu'une adresse établie par le dernier
  rendu (index), jamais une adresse arbitraire, ni le réseau local.
- Tests e2e de sécurité (WebKitGTK) : zéro requête vers un serveur canari,
  aucun script du mail exécuté, IPC inaccessible depuis `mailview://`,
  `javascript:` jamais ouvert (`app/e2e/specs/reading-security.e2e.mjs`).

## Messages affichés

| Clé | Texte (fr) |
|---|---|
| `message_images_blocked` | Images distantes bloquées ({count}) pour protéger votre vie privée. |
| `message_show_images` | Afficher les images de cet expéditeur |
| `message_show_text` / `message_show_html` | Afficher en texte brut / Afficher le HTML |
| `message_open`, `message_save` | Ouvrir, Enregistrer… |
| `message_saved_as` | Enregistré sous « {path} ». |
| `err_link_refused` | Lien refusé : seuls les liens http, https et mailto s'ouvrent. |
| `err_executable_attachment` | Type de fichier exécutable : enregistrez-le plutôt que de l'ouvrir. |
| `message_unreadable` | Message illisible : {error} |

## Raccourcis

| Commande | Raccourci |
|---|---|
| `reading.plain-text` (Préférer le texte brut (ou le HTML)) | palette |

Les touches pressées dans le cadre du message sont relayées par l'agent
(`postMessage`) au registre de commandes.
