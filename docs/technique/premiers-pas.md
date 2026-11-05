# Premiers pas

[← Documentation technique](README.md)

## 1. Installer

Prérequis :

- **Rust** ≥ 1.90 (via rustup de préférence), avec `rustfmt`, `clippy` et
  `llvm-tools-preview` (`rustup component add llvm-tools-preview`) ; sans
  rustup, `llvm-cov` et `llvm-profdata` du système, de même version majeure
  que `rustc` ;
- outils cargo : `cargo install --locked cargo-nextest cargo-deny cargo-llvm-cov` ;
- **Node** ≥ 22 et **pnpm** 10 ;
- **Docker** (tests d'intégration et e2e) ;
- dépendances système de Tauri. Linux (Debian, Ubuntu) :
  `libwebkit2gtk-4.1-dev libsoup-3.0-dev libayatana-appindicator3-dev
  librsvg2-dev libdbus-1-dev patchelf`, plus `desktop-file-utils` et
  `appstream` pour `check-packaging.sh`. Windows : WebView2 (fourni par
  Windows 10/11) et les outils de compilation MSVC.

Puis :

```sh
git clone git@github.com:gbourquet/norelly.git
cd norelly
pnpm -C app install
```

## 2. Lancer

```sh
pnpm -C app tauri dev
```

L'application démarre avec un profil réel (dossier de données de
l'utilisateur). Pour essayer sans compte personnel, lancez Dovecot et
Mailpit en local comme le font les e2e, ou utilisez un compte de test
dédié (jamais un compte personnel dans les tests).

## 3. Tester

| Quoi | Commande |
|---|---|
| Vérifications rapides (sans Docker) | `./scripts/check.sh` |
| Tout, avec Docker | `./scripts/check.sh full` |
| Tests Rust | `cargo nextest run --workspace` |
| Tests d'intégration | `cargo nextest run --workspace --profile integration` |
| Tests front | `pnpm -C app test` |
| End-to-end et captures de la documentation | `pnpm -C app e2e` |
| Documentation (liens, pages, captures, aide-mémoire) | `./scripts/check-docs.sh` |
| Bancs (liste et recherche, 50k) | `cargo bench -p norelly-sqlite -- --test` |
| Format | `cargo fmt --all`, `pnpm -C app format` |

Avant toute PR : `./scripts/check.sh full` doit passer.

## 4. Ajouter une commande IPC, pas à pas

Exemple : une commande `mark_all_read(account, mailbox)`.

1. **Domaine d'abord (si une règle est en jeu).** Si la commande porte une
   règle métier, elle vit dans `crates/domain` (test rouge, puis code), et
   le cas d'usage dans `crates/application`. La commande Tauri ne fait que
   revalider, appeler et traduire les erreurs.
2. **Test rouge de la commande** dans `app/src-tauri/src/command_tests.rs`,
   avec l'application factice :

   ```rust
   #[tokio::test]
   async fn mark_all_read_revalidates_and_marks() {
       let t = test_app().await;
       // entrée invalide : code d'erreur, jamais de texte
       assert_eq!(
           code(mark_all_read(t.state(), 0, 1).await),
           (ErrorCodeDto::InvalidInput, Some(FieldDto::Account))
       );
       // cas nominal…
   }
   ```

   Lancer `cargo nextest run -p norelly-app mark_all_read` : il doit
   échouer sur une assertion (écrire d'abord une version vide de la
   commande qui compile, si besoin).
3. **Types** dans `app/src-tauri/src/ipc.rs` : DTO `#[derive(Serialize,
   Deserialize, Type)]`, nouveau code d'erreur dans `ErrorCodeDto` ou champ
   dans `FieldDto` si nécessaire. Aucun texte pour l'utilisateur.
4. **La commande**, dans le module concerné (`reading.rs`, `actions.rs`…) :

   ```rust
   #[tauri::command]
   #[specta::specta]
   pub async fn mark_all_read(
       state: State<'_, AppState>,
       account: u32,
       mailbox: u32,
   ) -> Result<(), IpcError> {
       let account = AccountId::new(i64::from(account))
           .map_err(|_| IpcError::invalid(FieldDto::Account))?;
       // … revalidation par le domaine, appel du cas d'usage
       Ok(())
   }
   ```

5. **Enregistrement**, à trois endroits (le test `acl_tests` échoue si l'un
   manque) :
   - `app/src-tauri/src/lib.rs`, dans `collect_commands![…]` ;
   - `app/src-tauri/ipc-commands.rs` (`APP_COMMANDS`) ;
   - `app/src-tauri/permissions/main-window.toml` (`allow-mark-all-read`).
6. **Bindings** : `./scripts/gen-bindings.sh`, puis commiter
   `app/src/lib/ipc/bindings.ts` (le test `bindings_are_up_to_date` échoue
   sinon).
7. **Enveloppe TypeScript** dans `app/src/lib/ipc/api.ts` :

   ```ts
   markAllRead: (account: number, mailbox: number): Promise<void> =>
     unwrap(commands.markAllRead(account, mailbox)).then(() => undefined),
   ```

8. **Interface** : test Vitest rouge (état ou composant), puis le code.
   Nouvelle action utilisateur = nouvelle **commande du registre**
   (`registerAppCommands`, `app/src/lib/app.svelte.ts`) avec libellé et
   mots-clés dans `app/messages/fr.json` **et** `en.json`, et une ligne dans
   `docs/utilisateur/{fr,en}/shortcuts.md` (vérifié par
   `app/src/docs.test.ts`). Nouveau code d'erreur : son texte dans
   `app/src/lib/errors.ts` et les catalogues.
9. **Vérifier** : `./scripts/check.sh`.

## 5. Ajouter une migration SQLite, pas à pas

Exemple : une colonne `mailboxes.color`.

1. **Test rouge** dans `crates/adapters/sqlite` : le comportement attendu
   (lecture et écriture de la couleur par le port concerné), et, si des
   données existantes doivent être transformées, un test de montée de
   version qui part de la version précédente avec des données.
2. **Nouveau fichier** `crates/adapters/sqlite/migrations/0011_mailbox_color.sql`,
   commenté en tête (version, raison, phase) :

   ```sql
   -- Version 11 : couleur des dossiers (phase N).
   ALTER TABLE mailboxes ADD COLUMN color TEXT;
   ```

   **Ne jamais modifier une migration déjà publiée** : toujours en ajouter
   une.
3. **Déclarer** la migration à la fin de `migrations()` dans
   `crates/adapters/sqlite/src/lib.rs` :

   ```rust
   M::up(include_str!("../migrations/0011_mailbox_color.sql")),
   ```

4. **Code** : requêtes avec paramètres liés uniquement (`?1`, `:nom`) ; une
   constante interpolée porte `-- sql-const` (vérifié par
   `scripts/check-sql.sh`).
5. **Vérifier** : `migrations_are_valid` et
   `upgrade_from_every_previous_version_keeps_data` passent
   (`cargo nextest run -p norelly-sqlite`), puis `./scripts/check.sh`.
6. **Documenter** la migration dans [donnees.md](donnees.md#migrations).

## 6. Proposer la modification

Voir [CONTRIBUTING.md](../../CONTRIBUTING.md) : branche `<zone>/<sujet>`,
commits atomiques `<zone>: <résumé>`, PR vers `master` ; seul le mainteneur
merge.
