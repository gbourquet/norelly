# Relectures de la documentation

Retours sur les trois documentations (phase 11) : relecture du mainteneur,
lecture de la documentation utilisateur par une personne non technique, et
constats faits en la rédigeant. Chaque retour reçoit une suite : corrigé
(commit), reporté (phase), ou sans suite (raison).

## Constats de la rédaction (2026-11-05)

Relevés en écrivant la documentation et en produisant les captures ; la
documentation décrit le comportement **actuel**. À trancher par le
mainteneur.

| # | Constat | Où | Suite proposée |
|---|---|---|---|
| R1 | **Aucun bouton** pour écrire, répondre, répondre à tous ou transférer : seulement `c`, `r`, `a`, `f` et la palette. Bloquant pour un public non technique, qui n'est pas guidé vers `Ctrl+K`. | `app/src/lib/visible-actions.ts`, `ConversationList.svelte`, `ConversationView.svelte` | Ajouter « Nouveau message » en tête de liste et « Répondre », « Répondre à tous », « Transférer » dans la barre d'outils de lecture (commandes existantes du registre) |
| R2 | **Ouvrir un message ne le marque pas comme lu** (corps lus en `BODY.PEEK`, aucun marquage automatique) : la ligne reste en gras, le compteur ne baisse pas. | `app/src/lib/mail.svelte.ts`, `app/src-tauri/src/reading.rs` | Marquer lu à l'ouverture (éventuellement après un délai, réglable), par une action du domaine |
| R3 | Fenêtre de rédaction en français : le pied (Joindre, Supprimer le brouillon, Envoyer plus tard, date, Programmer, Envoyer) déborde, le bouton **Envoyer** sort du cadre (capture `docs/utilisateur/fr/captures/compose.png`). | `app/src/components/ComposeDialog.svelte` | Pied sur deux lignes ou envoi différé replié |
| R4 | Certificat **autosigné** des e2e présenté comme « Le certificat a été refusé. » (`add_issue_other`) au lieu de « n'est pas signé par une autorité reconnue (certificat autosigné ?) » (capture `certificate.png`). Hypothèse à vérifier : `openssl req -x509` pose `CA:TRUE`, webpki répond `CaUsedAsEndEntity`, que `certificate_issue` range dans `Other` ; cas courant des serveurs personnels. | `crates/adapters/tls/src/lib.rs` (`certificate_issue`) | Test rouge avec un certificat autosigné `CA:TRUE`, puis correspondance vers `UnknownIssuer` (ou un problème dédié) |
| R5 | La question des mises à jour n'existe que dans l'installateur Windows (module `updater`) : elle n'est pas capturée par les e2e (build Linux). La documentation la décrit en texte. | `app/e2e/captures/captures.e2e.mjs` | Sans suite (conforme à ADR-006) |
| R6 | Les écrans d'installation (installateur NSIS, SmartScreen, logithèques Linux) appartiennent au système : ils ne sont pas produits par le scénario e2e. La documentation utilisateur les décrit pas à pas, avec la capture du premier écran de Norelly. | `docs/utilisateur/*/install.md` | **À valider par le mainteneur** (le PLAN demande « avec captures d'écran ») : captures manuelles des installateurs, ou texte seul |
| R7 | Les noms de dossiers viennent du serveur (« Drafts », « Sent », « Trash » chez Dovecot), non traduits, alors que la boîte de réception l'est. | `app/src/components/Sidebar.svelte` (`folderLabel`) | Libellé traduit d'après le rôle SPECIAL-USE, nom du serveur en info-bulle |
| R8 | Les e2e étaient cassés depuis le renommage du binaire en `norelly` (phase 10, `mainBinaryName`) : `in-container.sh` lançait `norelly-app`. Corrigé dans cette phase. | `app/e2e/in-container.sh` | Corrigé |
| R9 | Le libellé `add_password_hint` (« … jamais dans les fichiers de Norelly ») est faux si l'utilisateur accepte le fichier de secrets non chiffré (revue de la phase 11). | `app/messages/{fr,en}.json`, `AddAccountDialog.svelte` | Reformuler, ou n'afficher qu'avec un trousseau |
| R10 | `pnpm -C app e2e` réécrit les captures suivies de `docs/utilisateur/*/captures` : lancer les e2e modifie le dépôt (revue de la phase 11). | `app/e2e/run.sh` | Variable d'environnement pour ne pas recopier les captures, si le mainteneur le souhaite |

## Relecture du mainteneur

À faire : relire `docs/fonctionnel/`, `docs/technique/`, `CONTRIBUTING.md`,
`docs/utilisateur/{fr,en}/` ; consigner ici les retours (page, passage,
remarque) et leur suite.

| Date | Page | Retour | Suite |
|---|---|---|---|
| | | | |

## Lecture par une personne non technique

À faire : faire lire `docs/utilisateur/fr/` (ou `en/`) à une personne peu à
l'aise avec un ordinateur, idéalement en lui demandant d'installer Norelly
et d'ajouter un compte de test en suivant le guide, sans aide. Noter où elle
hésite, ce qu'elle ne comprend pas, ce qui manque.

| Date | Lecteur (profil, sans nom) | Page | Difficulté rencontrée | Suite |
|---|---|---|---|---|
| | | | | |
