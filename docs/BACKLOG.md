# Backlog

Sujets ouverts à la fin du développement initial (phases 1 à 11), repris de
l'ancien plan, des relectures de la documentation et des revues de
sécurité. On retire un élément quand il est traité (lien vers la PR), ou
quand le mainteneur l'abandonne (avec la raison).

## À trancher par le mainteneur

- **Notifications par `notify-rust`** : dérogation à consigner dans
  [DECISIONS.md](DECISIONS.md). `tauri-plugin-notification` tire zbus 5,
  dont `zbus_names` et `zvariant` sont signalés non maintenus
  (RUSTSEC-2026-0370, -0371) et font échouer `cargo deny` ; remplacé par
  `notify-rust` sans zbus (fonctionnalité `d`, libdbus du système), appelé
  depuis Rust seulement.
- **Seuil du démarrage** (porté de 200 à 400 ms le 2026-11-04) : démarrage
  mesuré à 293-307 ms sous Xvfb, WebKitGTK sans accélération graphique ; les
  appels IPC prennent moins de 10 ms, le temps est passé dans la webview
  (chargement du script ≈ 65 ms, fil principal occupé ≈ 67 ms après le
  montage, ≈ 100 ms entre `list_conversations` et l'affichage). Pistes :
  profiler le premier rendu (Svelte, liste virtuelle, styles), mesurer sur
  une machine avec GPU, ou revoir ce que mesure le « démarrage ».
- **Captures des écrans d'installation** (installateur NSIS, SmartScreen,
  logithèques Linux) : ils appartiennent au système et ne sont pas produits
  par les e2e ; `docs/utilisateur/*/install.md` les décrit en texte.
  Captures manuelles, ou texte seul ?
- **Captures recopiées par les e2e** : `pnpm -C app e2e` réécrit les
  captures suivies de `docs/utilisateur/*/captures`, donc lancer les e2e
  modifie le dépôt. Ajouter une variable d'environnement pour ne pas les
  recopier ? (`app/e2e/run.sh`)

## Interface et fonctionnalités

- **Boutons pour écrire et répondre** : aucun bouton pour écrire, répondre,
  répondre à tous ou transférer, seulement `c`, `r`, `a`, `f` et la palette ;
  bloquant pour un public non technique. Ajouter « Nouveau message » en tête
  de liste et « Répondre », « Répondre à tous », « Transférer » dans la barre
  de lecture (commandes existantes du registre). (`app/src/lib/visible-actions.ts`,
  `ConversationList.svelte`, `ConversationView.svelte`)
- **Ouvrir un message ne le marque pas comme lu** : corps lus en
  `BODY.PEEK`, aucun marquage automatique ; la ligne reste en gras et le
  compteur ne baisse pas. Marquer lu à l'ouverture (éventuellement après un
  délai réglable), par une action du domaine. (`app/src/lib/mail.svelte.ts`,
  `app/src-tauri/src/reading.rs`)
- **Pied de l'éditeur qui déborde en français** : le bouton « Envoyer » sort
  du cadre (capture `docs/utilisateur/fr/captures/compose.png`). Pied sur
  deux lignes, ou envoi différé replié. (`ComposeDialog.svelte`)
- **Noms de dossiers non traduits** : « Drafts », « Sent », « Trash »
  viennent du serveur alors que la boîte de réception est traduite. Libellé
  traduit d'après le rôle SPECIAL-USE, nom du serveur en info-bulle.
  (`Sidebar.svelte`, `folderLabel`)
- **Libellé `add_password_hint` faux** (« … jamais dans les fichiers de
  Norelly ») si l'utilisateur accepte le fichier de secrets non chiffré.
  Reformuler, ou n'afficher qu'avec un trousseau. (`app/messages/{fr,en}.json`,
  `AddAccountDialog.svelte`)

## Bogues

- **Certificat autosigné mal qualifié** : présenté comme « Le certificat a
  été refusé. » (`add_issue_other`) au lieu de « n'est pas signé par une
  autorité reconnue ». Hypothèse : `openssl req -x509` pose `CA:TRUE`,
  webpki répond `CaUsedAsEndEntity`, que `certificate_issue` range dans
  `Other` ; cas courant des serveurs personnels. Test rouge avec un
  certificat autosigné `CA:TRUE`, puis correspondance vers `UnknownIssuer`
  (ou un problème dédié). (`crates/adapters/tls/src/lib.rs`)

## Sécurité et chaîne d'approvisionnement

- Oracle du fuzzing de `sanitize_html` limité à `<script` : l'enrichir
  (`on*`, `javascript:` en attribut, `<iframe>`, `<base>`, `<meta>`,
  `<form>`).
- Chaîne Rust `stable` et outils d'`install-action` flottants dans la CI.
- Pas d'attestation de provenance ni de `SHA256SUMS` signé pour les
  versions publiées.
- Paquets apt de l'image e2e non figés.
- `fuzz/Cargo.lock` hors `cargo deny`.
- Flathub demandera une source `git` (tag et commit) au lieu de `type: dir`.
- `--filesystem=xdg-download` (Flatpak) à revoir si le sélecteur de
  fichiers passe par le portail.
- Installateur Windows non signé (Authenticode) tant que SignPath n'a pas
  accepté le projet ([candidature](signpath-candidature.md)).
- Permission Tauri `core:default` réductible.

**Règle permanente** : à chaque montée de version de Tauri, rejouer le test
e2e qui vérifie qu'aucun accès à l'IPC n'est possible depuis l'iframe
`mailview://` (la capability ne distingue pas l'iframe de lecture, et
`tauri::test` n'enregistre pas les protocoles).

## Tâches humaines

- Lecture de `docs/utilisateur/fr/` (ou `en/`) par une personne peu à l'aise
  avec un ordinateur, idéalement en lui demandant d'installer Norelly et
  d'ajouter un compte de test sans aide ; noter où elle hésite.
- Relecture par le mainteneur de `docs/technique/`, `CONTRIBUTING.md` et
  `docs/utilisateur/{fr,en}/` (`docs/fonctionnel/` relu le 2026-11-05).
- Première exécution de [MANUAL-TESTS.md](MANUAL-TESTS.md) avec des comptes
  de test dédiés, puis à chaque version.
