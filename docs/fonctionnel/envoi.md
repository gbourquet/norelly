# Envoi : file d'envoi, nouvelles tentatives, envoi différé

[← Fonctionnalités](README.md)

## But

Qu'un message envoyé parte, même hors ligne, sans jamais le dupliquer chez
les destinataires, et pouvoir programmer un envoi.

## Comportement attendu

- « Envoyer » place le message dans la **file d'envoi** du compte, durable ;
  l'éditeur se ferme (`compose_queued`).
- Le travailleur de rédaction du compte envoie en SMTP, puis dépose la
  copie dans « Envoyés » (sauf Gmail, qui le fait lui-même).
- Hors ligne, le message attend et part au retour du réseau.
- En cas d'échec définitif, un bandeau le signale une fois ; la **boîte
  d'envoi** (palette) montre chaque message et son état, avec **Réessayer**
  et **Annuler l'envoi**.
- **Envoyer plus tard** : date et heure choisies ; le message reste dans la
  file jusque-là, avec **Modifier** (retour dans l'éditeur) et **Annuler**.

## Règles métier

États (`crates/domain/src/compose/outbox.rs`) :

```text
Queued ──start──▶ Sending ──succeeded──▶ Sent
  ▲                  │
  └──failed (temporaire, injoignable)
                     └──failed (refus, authentification, trop d'essais)──▶ Failed
```

| Règle | Où |
|---|---|
| Serveur injoignable : essai **non compté**, envoi différé | `outbox.rs` |
| Échec temporaire (SMTP 4xx, coupure) : nouvel essai avec attente croissante (30 s à 1 h), **8 essais** au plus | `outbox.rs` (`MAX_SEND_ATTEMPTS`, `RETRY_BASE_SECS`, `RETRY_MAX_SECS`) |
| Refus définitif (5xx), authentification, sécurité : échec immédiat | `outbox.rs` |
| Envoi interrompu par l'arrêt de l'application (`Sending` au démarrage) ou transmission interrompue sans réponse : **incertain**, jamais renvoyé automatiquement ; l'utilisateur décide | `outbox.rs` (`SendFailure::Uncertain`), `crates/application/src/compose_worker.rs` (`RecoverOutbox`) |
| Envoi différé : un an au plus ; brouillon gardé avec le message ; repris d'un bloc seulement s'il est encore en attente | `outbox.rs` (`MAX_SCHEDULE_SECS`), migration 10 |
| Annuler : seulement si le message n'est pas en cours d'envoi | `app/src-tauri/src/compose.rs` (`cancel_send`) |
| Connexion SMTP : TLS implicite (465) ou STARTTLS exigé (587), TLS négocié par `norelly-tls` | ADR-002, `crates/adapters/smtp` |
| Copie dans Envoyés : APPEND, sauf serveurs qui la font eux-mêmes (Gmail, `X-GM-EXT-1`) | `crates/application/src/compose.rs` (`FileSentCopies`) |

## Cas limites

- Message en cours d'envoi : « Réessayer », « Annuler » ou « Modifier »
  refusés (`err_outbox_busy`).
- Envoi programmé à plus d'un an : `err_schedule_too_far`.
- Échec « peut-être parti » : vérifier Envoyés ou le destinataire avant de
  réessayer.

## Messages affichés

| Clé | Texte (fr) |
|---|---|
| `outbox_title`, `outbox_empty` | Boîte d'envoi, Aucun message en attente. |
| `outbox_state_queued`, `outbox_state_sending`, `outbox_state_sent` | En attente, Envoi en cours, Envoyé |
| `outbox_state_scheduled` | Programmé pour le {date} |
| `compose_send_failed` | Envoi impossible : « {subject} » ({reason}). |
| `send_failure_*` | serveur injoignable, refus temporaire, refusé par le serveur, identifiants refusés, connexion non sûre, envoi interrompu, peut-être parti, trop d'essais |
| `outbox_retry`, `outbox_cancel`, `outbox_edit` | Réessayer, Annuler l'envoi, Modifier |
| `compose_later`, `compose_schedule`, `compose_scheduled` | Envoyer plus tard, Programmer, Envoi programmé. |
| `err_schedule_too_far` | L'envoi ne peut pas être programmé à plus d'un an. |
| `err_outbox_busy` | Ce message est en cours d'envoi : réessayez dans un instant. |

## Raccourcis

| Commande | Raccourci |
|---|---|
| `compose.outbox` (Boîte d'envoi) | palette |
