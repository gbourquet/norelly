# Contribuer à Norelly

Merci de votre intérêt ! Ce fichier résume l'essentiel ; le détail est dans
[la documentation technique](docs/technique/README.md).

## Avant de commencer

- Lisez [l'architecture](docs/technique/architecture.md) et
  [DECISIONS.md](docs/DECISIONS.md) : les décisions d'architecture font foi.
  Une dérogation se discute (issue) **avant** d'être codée.
- Une question, un bogue : ouvrez une issue. **Une vulnérabilité** : voir
  [SECURITY.md](SECURITY.md) (signalement privé), jamais une issue
  publique.

## Installer, lancer, tester

```sh
pnpm -C app install
pnpm -C app tauri dev      # lancer l'application
./scripts/check.sh         # vérifications rapides
./scripts/check.sh full    # tout, avec Docker (intégration)
pnpm -C app e2e            # end-to-end et captures de la documentation (Docker)
```

Prérequis et pas à pas (ajouter une commande IPC, une migration) :
[premiers pas](docs/technique/premiers-pas.md).

## Règles

1. **TDD obligatoire** : test rouge (pour la bonne raison), code minimal,
   refactor. Correction de bug = d'abord un test qui le reproduit. Voir
   [méthode](docs/technique/methode.md).
2. **Ne jamais ramollir, désactiver ou contourner un test**, ni affaiblir
   `scripts/check.sh`, `deny.toml` ou un seuil de couverture.
3. **Sécurité en Rust** : l'interface ne fait que présenter. Pas
   d'`unwrap`/`expect` sur des données externes, pas d'`unsafe`, SQL
   paramétré, jamais de `{@html}` ni d'`innerHTML`.
4. **Domaine pur** : `crates/domain` sans E/S, dépendances limitées à
   `thiserror` et `serde`.
5. **i18n** : toute chaîne d'interface dans `app/messages/fr.json` **et**
   `en.json` ; Rust n'envoie que des codes.
6. **Documentation à jour** : une nouvelle commande du registre figure dans
   l'aide-mémoire `docs/utilisateur/{fr,en}/shortcuts.md` ; une nouvelle
   page utilisateur existe dans les deux langues ; une fonctionnalité
   modifiée met à jour sa fiche dans `docs/fonctionnel/` (comportement
   seulement, sans vocabulaire technique) et sa correspondance avec le code
   dans `docs/technique/tracabilite.md`. Les captures
   d'écran sont régénérées par `pnpm -C app e2e`, jamais retouchées à la
   main. `./scripts/check-docs.sh` vérifie liens, pages, captures,
   aide-mémoire et l'absence de vocabulaire technique dans
   `docs/fonctionnel/`.
7. **Langue** : code, commentaires, documentation et commits en français ;
   identifiants en anglais.

## Proposer une modification

1. Créez une branche `<zone>/<sujet>` (par exemple `imap/idle-timeout`).
   **Ne poussez jamais sur `master`.**
2. Commits atomiques, message `<zone>: <résumé>` (par exemple
   `imap: rejette STARTTLS absent`).
3. `./scripts/check.sh full` doit passer.
4. Poussez la branche et ouvrez une PR vers `master` (`gh pr create`) :
   décrivez le comportement, les tests ajoutés et les fichiers sensibles
   touchés ([liste](docs/technique/securite.md#fichiers-sensibles)).
5. Seul le mainteneur relit et merge.

## Licence

En contribuant, vous acceptez que votre contribution soit publiée sous
`MIT OR Apache-2.0`, comme le reste du projet.
