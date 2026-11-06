# Notifications des nouveaux messages

[← Fonctionnalités](README.md)

## But

Prévenir de l'arrivée d'un message quand Norelly n'est pas au premier plan,
sans bombarder l'utilisateur ni exposer le contenu si on ne le souhaite
pas.

## Comportement attendu

- Une notification du système s'affiche quand de nouveaux messages non lus
  arrivent dans une boîte de réception **et que la fenêtre de Norelly n'est
  pas au premier plan**.
- Un seul message : « Nouveau message », avec l'expéditeur et le sujet.
  Plusieurs : « N nouveaux messages », avec le plus récent.
- Option de confidentialité : ni expéditeur ni sujet, seulement le nombre et
  l'adresse du compte.
- Désactivables dans les paramètres.

## Règles

- Seuls les messages **non lus** comptent : un message déjà lu sur un autre
  appareil n'est pas signalé.
- Rien à la **première synchronisation** d'un compte : son contenu existant
  n'est pas « nouveau ».
- Les messages arrivés ensemble sont **regroupés** en une seule
  notification : leur nombre et, au plus, le plus récent.
- Avec l'option de confidentialité, l'expéditeur et le sujet ne sont jamais
  transmis au système.
- Le sujet et l'expéditeur s'affichent tels quels : aucune mise en forme
  qu'ils contiendraient n'est interprétée.

## Cas limites

- Fenêtre au premier plan : pas de notification.
- Sujet ou expéditeur contenant du balisage (`<b>`, `&amp;`) : affiché
  littéralement.

## Messages affichés

| Situation | Texte |
|---|---|
| Un message | Nouveau message |
| Plusieurs messages | {count} nouveaux messages |
| Le plus récent | Dernier : {from} — {subject} |
| Paramètre | Notifier les nouveaux messages |
| Paramètre de confidentialité | Masquer l'expéditeur et le sujet dans les notifications |

## Raccourcis

Aucun (réglages dans les [paramètres](parametres.md)).
