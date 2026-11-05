# Candidature SignPath Foundation (ADR-006)

Dossier préparé pour le formulaire <https://signpath.org/apply>. À envoyer
par le mainteneur, une fois une première version publiée (SignPath demande
un projet déjà distribué).

## Conditions de SignPath Foundation, et où elles sont remplies

| Condition | Norelly |
|---|---|
| Licence OSI, sans partie propriétaire | `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`) ; dépendances permissives vérifiées par `cargo deny` (ADR-009) |
| Pas de logiciel malveillant ni de comportement indésirable | aucune télémétrie ; accès réseau décrits dans `docs/CODE_SIGNING.md` |
| Projet maintenu, avec des versions publiées | versions GitHub construites par `release.yml` |
| Build automatisé et vérifiable | GitHub Actions, depuis un tag, actions figées par SHA, sans cache |
| Politique de signature publique (rôles, confidentialité) | `docs/CODE_SIGNING.md`, à lier depuis le README |
| Authentification forte des membres | 2FA obligatoire sur le compte GitHub du mainteneur et sur SignPath |

## Réponses préparées

- **Nom du projet** : Norelly
- **Dépôt** : https://github.com/gbourquet/norelly
- **Description** : client mail IMAP/SMTP de bureau pour Windows et Linux,
  rapide, pilotable au clavier, configuration presque automatique, sans
  télémétrie. Rust (Tauri 2) et Svelte.
- **Artefacts à signer** : installateur NSIS par utilisateur
  (`Norelly_<version>_x64-setup.exe`) et `norelly.exe`.
- **Système de build** : GitHub Actions, job `windows` de
  `.github/workflows/release.yml` (runner `windows-2025`).
- **Fréquence** : quelques versions par an.
- **Mainteneur et approbateur** : Guillaume Bourquet.

## Après acceptation

1. Créer le projet SignPath (organisation, projet `norelly`, politique de
   signature `release-signing` avec approbation manuelle) et le connecteur
   GitHub (jeton `SIGNPATH_API_TOKEN` en secret du dépôt).
2. Dans `release.yml`, job `windows` : téléverser l'installateur non signé
   comme artefact, puis l'action officielle `signpath/github-action-submit-signing-request`
   (figée par SHA) avec `wait-for-completion: true` ; publier l'installateur
   signé.
3. La signature Authenticode change l'installateur : le signer **avant** la
   signature minisign de la mise à jour (`tauri signer sign`), puis régénérer
   `latest.json`.
4. Retirer la mention « non signé » de `docs/CODE_SIGNING.md` et du README.
