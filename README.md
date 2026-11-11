# Norelly

Client mail IMAP/SMTP desktop pour Windows et Linux : rapide, sobre,
utilisable entièrement à la souris (et plus vite encore au clavier),
configuration quasi automatique. Aucune télémétrie.

> Projet en pré-alpha.

## Développement

Prérequis : Rust ≥ 1.90, Node ≥ 22 et pnpm 10, Docker (tests d'intégration),
dépendances système Tauri (Linux : `webkit2gtk-4.1`, `libsoup-3.0`),
`cargo install --locked cargo-nextest cargo-deny cargo-llvm-cov` et les
outils LLVM (`rustup component add llvm-tools-preview`, ou `llvm-cov` et
`llvm-profdata` du système, de même version majeure que `rustc`) pour la
couverture vérifiée par `./scripts/check.sh`.

```sh
pnpm -C app install
pnpm -C app tauri dev     # lancer l'application
./scripts/check.sh        # vérifications rapides
./scripts/check.sh full   # tout, y compris les tests d'intégration (Docker)
```

## Documentation

- [Guide d'utilisation](docs/utilisateur/fr/README.md) (en français) et
  [user guide](docs/utilisateur/en/README.md) (in English) ;
- [documentation fonctionnelle](docs/fonctionnel/README.md) : ce que fait
  chaque fonctionnalité, ses règles et ses messages ;
- [documentation technique](docs/technique/README.md) et
  [CONTRIBUTING.md](CONTRIBUTING.md) : architecture, données, sécurité,
  tests, méthode, premiers pas ;
- [DECISIONS.md](docs/DECISIONS.md) (choix d'architecture).

## Distribution

- Windows : installateur NSIS (versions GitHub), winget ; mises à jour
  signées (minisign). Politique de signature : `docs/CODE_SIGNING.md`.
- Linux : Flathub (`packaging/flatpak`), paquets .deb et .rpm.

## Licence

Au choix : [MIT](LICENSE-MIT) ou [Apache-2.0](LICENSE-APACHE).
