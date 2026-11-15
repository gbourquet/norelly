# Envoi : boîte d'envoi, nouvelles tentatives, envoi différé

[← Fonctionnalités](README.md)

## But

Qu'un message envoyé parte, même hors ligne, sans jamais arriver en double
chez les destinataires, et pouvoir programmer un envoi.

## Comportement attendu

- « Envoyer » place le message dans la **boîte d'envoi** du compte, qui
  survit à la fermeture de l'application ; la fenêtre de rédaction se
  ferme.
- Le message est envoyé, puis une copie est rangée dans « Envoyés » (sauf
  chez Gmail, qui le fait lui-même).
- Hors ligne, le message attend et part au retour du réseau.
- En cas d'échec définitif, un bandeau le signale une fois ; la **boîte
  d'envoi** (menu « ⋯ » du compte, ou palette) montre chaque message et son état, avec **Réessayer**
  et **Annuler l'envoi**.
- **Envoyer plus tard** : date et heure choisies ; le message reste dans la
  boîte d'envoi jusque-là, avec **Modifier** (retour dans la fenêtre de
  rédaction) et **Annuler**.

## Règles

États d'un message de la boîte d'envoi :

```text
En attente ──▶ Envoi en cours ──▶ Envoyé
     ▲               │
     └── échec temporaire (serveur injoignable, refus temporaire)
                     └── échec définitif (refus, identifiants, trop d'essais) ──▶ Échec
```

- **Serveur injoignable** : l'envoi attend le retour du réseau, sans que
  l'essai compte.
- **Plusieurs connexions par essai** : quand la connexion échoue pour une
  raison passagère (réseau instable, serveur joint puis muet, connexion
  coupée avant l'envoi du message), l'application se reconnecte aussitôt,
  à quelques secondes d'intervalle, avant de reporter l'essai : un réseau
  instable ne retarde pas le message d'un passage à l'autre. Jamais une
  fois le message transmis, ni après des identifiants refusés ou restés
  sans réponse. Quand le serveur reste injoignable ou muet, les autres
  messages du compte attendent le passage suivant.
- **Refus temporaire** du serveur : nouvel essai, avec une attente
  croissante (30 secondes à 1 heure), **8 essais** au plus.
- **Le serveur ne répond pas** (joint, puis muet ou connexion coupée,
  avant l'envoi du message) : rien n'est parti ; nouveaux essais comme
  pour un refus temporaire, puis un échec qui le dit.
- **Connexion coupée ou bloquée avant d'être chiffrée** : rien n'est parti ;
  nouveaux essais comme pour un refus temporaire, puis un échec qui le dit
  et signale que le réseau utilisé peut en être la cause.
- **Connexion coupée pendant la vérification des identifiants** : échec
  immédiat, sans nouvel essai automatique (certains serveurs bloquent le
  compte après plusieurs essais) ; l'utilisateur relance. Après des
  identifiants refusés ou une telle coupure, les autres messages du compte
  attendent un quart d'heure, sauf si l'utilisateur envoie un message ou
  clique sur « Réessayer ».
- **Refus définitif**, identifiants refusés, connexion non sûre : échec
  immédiat.
- **Envoi refusé par Microsoft pour les applications** (compte connecté
  avec Microsoft qui n'autorise pas l'envoi depuis un logiciel de
  messagerie) : échec
  immédiat, avec le conseil d'envoyer depuis le site de Microsoft ou de
  contacter son support. Les autres messages du compte attendent comme
  après des identifiants refusés.
- **Envoi incertain** : si l'application s'arrête pendant un envoi, ou si
  la connexion se coupe avant la réponse du serveur, le message est peut-être
  parti. Il n'est **jamais renvoyé automatiquement** : l'utilisateur décide.
- **Envoi différé** : un an au plus ; le brouillon reste attaché au
  message ; « Modifier » n'est possible que tant que le message attend.
- **Annuler** : seulement si le message n'est pas en cours d'envoi.
- **Connexion au serveur d'envoi** : toujours chiffrée (TLS dès la
  connexion, ou STARTTLS exigé), sauf consentement explicite à une
  connexion non chiffrée (voir [comptes](comptes.md)).

## Cas limites

- Message en cours d'envoi : « Réessayer », « Annuler » et « Modifier »
  refusés.
- Envoi programmé à plus d'un an : refusé.
- Échec « peut-être parti » : vérifier « Envoyés » ou demander au
  destinataire avant de réessayer.

## Messages affichés

| Situation | Texte |
|---|---|
| Boîte d'envoi | Boîte d'envoi, Aucun message en attente. |
| États | En attente, Envoi en cours, Envoyé |
| Envoi programmé | Programmé pour le {date} |
| Échec | Envoi impossible : « {subject} » ({reason}). |
| Raison de l'échec | serveur injoignable, refus temporaire, refusé par le serveur, identifiants refusés, connexion non sûre, envoi interrompu, peut-être parti, trop d'essais, connexion coupée avant le chiffrement, peut-être par le réseau utilisé, connexion coupée pendant la vérification des identifiants, le serveur ne répond pas, Microsoft n'autorise pas l'envoi depuis les applications pour ce compte. Envoyez depuis outlook.com, ou contactez le support Microsoft. |
| Boutons | Réessayer, Annuler l'envoi, Modifier |
| Envoi différé | Envoyer plus tard, Programmer, Envoi programmé. |
| Date trop lointaine | L'envoi ne peut pas être programmé à plus d'un an. |
| Message occupé | Ce message est en cours d'envoi : réessayez dans un instant. |

## Raccourcis

| Commande | Raccourci |
|---|---|
| Boîte d'envoi | palette |
