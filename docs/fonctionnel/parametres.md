# Paramètres, langue et mises à jour

[← Fonctionnalités](README.md)

## But

Régler les quelques préférences utiles sans écran complexe, et décider en
connaissance de cause de la recherche de mises à jour.

## Comportement attendu

Dialogue **Paramètres** (`Ctrl+,`) :

| Réglage | Valeurs | Effet |
|---|---|---|
| Langue | Celle du système (défaut), Français, English | au prochain démarrage |
| Rechercher automatiquement les mises à jour | oui / non | installateur Windows seulement |
| Notifier les nouveaux messages | oui (défaut) / non | immédiat |
| Masquer l'expéditeur et le sujet dans les notifications | non (défaut) / oui | immédiat |
| Afficher les messages en texte brut | non (défaut) / oui | immédiat |
| Télécharger d'avance les pièces jointes jusqu'à | en Mio (défaut 2) | au prochain démarrage |

**Mises à jour (installateur NSIS, Windows)** : au premier lancement, la
question « Norelly peut rechercher automatiquement ses mises à jour. Le
souhaitez-vous ? » est posée, **Oui** ayant le focus. Fermer la question
sans répondre ne vaut pas « Oui ». Si une version est disponible, un
bandeau propose « Installer et redémarrer » ou « Plus tard ». Avec Flathub,
.deb ou .rpm, le module est absent : ni question ni bandeau.

**Langue** : langue du système par défaut (français si la langue préférée
est le français, anglais pour toute autre langue).

## Règles métier

| Règle | Où |
|---|---|
| Recherche de mises à jour : demandée au premier lancement, « oui » présélectionné (`UpdateCheck::PROPOSED`), modifiable | `crates/domain/src/settings.rs`, ADR-006 |
| Vérification au plus une fois par jour, sans identifiant envoyé ; mise à jour signée (minisign) vérifiée avant installation | `settings.rs` (`UPDATE_CHECK_INTERVAL_SECS`), `app/src-tauri/src/updates.rs` |
| Module de mise à jour compilé seulement avec la fonctionnalité `updater` (NSIS) | ADR-006, `app/src-tauri/Cargo.toml` |
| Seuil de rapatriement entre 64 Kio et 64 Mio ; valeur illisible ou hors bornes remplacée par la valeur par défaut | `settings.rs`, port `SettingsStore` |
| Langue : choix de la locale parmi les préférences du système, anglais en repli ; dates, heures, tailles par `Intl` avec la locale active | `app/src/lib/i18n.ts`, ADR-011 |

## Cas limites

- Paramètres corrompus en base : valeurs par défaut, sans erreur.
- Échec d'installation d'une mise à jour : `err_update_failed`.

## Messages affichés

| Clé | Texte (fr) |
|---|---|
| `settings_title` | Paramètres |
| `settings_restart_note` | La langue et le seuil de téléchargement s'appliquent au prochain démarrage. |
| `update_question` | Norelly peut rechercher automatiquement ses mises à jour. Le souhaitez-vous ? |
| `update_available` | La version {version} de Norelly est disponible. |
| `update_install`, `update_later` | Installer et redémarrer, Plus tard |
| `err_update_failed` | Mise à jour impossible. |

## Raccourcis

| Commande | Raccourci |
|---|---|
| `settings.open` (Paramètres…) | `Ctrl+,` |
