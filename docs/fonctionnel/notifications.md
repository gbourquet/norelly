# Notifications des nouveaux messages

[← Fonctionnalités](README.md)

## But

Prévenir de l'arrivée d'un message quand Norelly n'est pas au premier plan,
sans bombarder l'utilisateur ni exposer le contenu si on ne le souhaite
pas.

## Comportement attendu

- Une notification native du système s'affiche quand de nouveaux messages
  non lus arrivent dans une boîte de réception **et que la fenêtre n'a pas
  le focus**.
- Un seul message : « Nouveau message », avec l'expéditeur et le sujet.
  Plusieurs : « N nouveaux messages », avec le plus récent.
- Option de confidentialité : ni expéditeur ni sujet, seulement le nombre et
  l'adresse du compte.
- Désactivables dans les paramètres.

## Règles métier

| Règle | Où |
|---|---|
| Seuls les messages **non lus** comptent (un message lu ailleurs n'est pas signalé) | `crates/domain/src/notify.rs` |
| Rien à la **première synchronisation** d'une boîte | `notify.rs` |
| Arrivées d'une synchronisation **regroupées** : un nombre et, au plus, le message le plus récent | `notify.rs` |
| Confidentialité : contenu retiré **en Rust** (`NewMail::without_content`) avant d'atteindre l'interface | `notify.rs`, `app/src-tauri/src/settings.rs` (`new_mail_event`) |
| Texte composé par l'interface dans la langue de l'utilisateur, revalidé par la commande `show_notification` (balisage freedesktop neutralisé) | ADR-011, `app/src-tauri/src/settings.rs` |
| Affichage par `notify-rust` (sans zbus) depuis Rust seulement | PLAN phase 9 (dérogation) |

## Cas limites

- Fenêtre au premier plan : pas de notification.
- Sujet ou expéditeur contenant du balisage (`<b>`, `&amp;`) : affiché
  littéralement (constat haut de la revue de la phase 9, corrigé).
- Affichage réel à vérifier à la main (`docs/MANUAL-TESTS.md`).

## Messages affichés

| Clé | Texte (fr) |
|---|---|
| `notify_new_one` | Nouveau message |
| `notify_new_many` | {count} nouveaux messages |
| `notify_latest` | Dernier : {from} — {subject} |
| `settings_notifications` | Notifier les nouveaux messages |
| `settings_privacy` | Masquer l'expéditeur et le sujet dans les notifications |

## Raccourcis

Aucun (réglages dans les [paramètres](parametres.md)).
