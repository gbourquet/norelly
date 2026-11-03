# Norelly

Client mail IMAP/SMTP desktop pour Windows et Linux : rapide, sobre,
pilotable au clavier, configuration quasi automatique. Aucune télémétrie.

> Projet en cours de développement (pré-alpha). Voir `docs/PLAN.md`.

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

Documentation : `docs/DECISIONS.md` (choix d'architecture), `docs/PLAN.md`
(phases), `docs/REVIEWS.md` (revues sécurité).

## Licence

Au choix : [MIT](LICENSE-MIT) ou [Apache-2.0](LICENSE-APACHE).
