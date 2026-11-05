# Documentation fonctionnelle de Norelly

Ce dossier décrit **ce que fait Norelly**, du point de vue de la personne
qui l'utilise : comportements attendus, règles métier, cas limites et
messages affichés. Il s'adresse au mainteneur, aux contributeurs, aux
testeurs et à quiconque doit décider si un comportement est un défaut.

- Le **comment** (architecture, code, tests) est dans
  [la documentation technique](../technique/README.md).
- Le **mode d'emploi** pour le grand public est dans
  [la documentation utilisateur](../utilisateur/fr/README.md).
- Les **choix** et leurs raisons sont dans [DECISIONS.md](../DECISIONS.md).

En cas de désaccord entre une fiche et le code, **le domaine fait foi**
(`crates/domain`, cité dans chaque fiche) : la fiche est à corriger, ou
l'écart est un défaut à signaler.

## Présentation du produit

### À qui s'adresse Norelly

Norelly est un client de messagerie **IMAP/SMTP** de bureau, pour
**Windows et Linux**. Il vise :

- les particuliers qui ont deux ou trois adresses (personnelle,
  professionnelle, associative) chez des fournisseurs différents et
  veulent les lire au même endroit ;
- les personnes attentives à leur vie privée, qui ne veulent ni pistage à
  la lecture ni télémétrie ;
- les utilisateurs qui préfèrent le clavier (raccourcis à la Gmail, palette
  de commandes) ;
- des boîtes volumineuses : de l'ordre de **50 000 messages** par compte,
  sur 2 ou 3 comptes, avec une interface qui reste fluide (cible : moins de
  200 ms pour changer de dossier).

### Ce que fait Norelly

- ajoute un compte à partir de l'adresse et du mot de passe seuls, en
  trouvant les serveurs automatiquement ;
- synchronise les dossiers et garde une copie locale des messages,
  consultable et cherchable hors ligne ;
- regroupe les messages en conversations, y compris dans une boîte de
  réception unifiée de tous les comptes ;
- affiche le HTML des messages de façon isolée, sans requête réseau non
  autorisée ;
- range, supprime, archive, marque, y compris hors ligne (actions rejouées
  au retour du réseau), avec annulation ;
- rédige, répond, transfère, avec mise en forme simple et pièces jointes ;
  enregistre les brouillons ; envoie tout de suite, plus tard ou au retour
  du réseau ;
- recherche en plein texte dans tous les messages ;
- notifie les nouveaux messages ;
- en français et en anglais.

### Ce que Norelly ne fait pas (aujourd'hui)

- **Pas de POP3, ni d'Exchange (EWS), ni de JMAP** : IMAP et SMTP
  uniquement.
- **Pas de connexion OAuth2** (« authentification moderne ») : les
  fournisseurs qui l'exigent (Outlook.com, de plus en plus) ne sont pas
  utilisables ; Gmail l'est par mot de passe d'application. Le mécanisme
  `XOAUTH2` est prévu dans le domaine mais renvoie `Unsupported`.
- **Pas de chiffrement de bout en bout** (OpenPGP, S/MIME), ni de signature
  des messages.
- **Pas de chiffrement de la base locale** : on s'appuie sur le chiffrement
  du disque du système (ADR-003).
- **Pas de carnet d'adresses, d'agenda ni de filtres** côté client ; les
  suggestions de destinataires viennent des adresses déjà vues.
- **Pas de macOS** ni de version mobile.
- **Pas de télémétrie**, ni de rapport de plantage automatique.

### Principes

1. **Sécurité d'abord, décidée en Rust.** Tout ce qui vient du réseau
   (messages, réponses des serveurs, autoconfiguration, DNS) est traité
   comme hostile. L'interface n'est qu'une présentation : aucune décision
   de sécurité n'y est prise (ADR-001, ADR-008). Pas de repli silencieux :
   pas de clair sans consentement, pas de STARTTLS ignoré, pas de
   certificat accepté sans décision explicite.
2. **Vie privée.** Aucune requête au moment de la lecture d'un message sans
   autorisation (images distantes bloquées par défaut, ADR-004) ; aucune
   télémétrie ; la recherche de mises à jour est demandée, jamais imposée
   (ADR-006).
3. **Hors ligne.** La copie locale fait foi pour l'affichage. Lire,
   chercher, ranger et écrire fonctionnent sans réseau ; les écritures
   attendent dans des files durables (actions, envois) et sont rejouées.
4. **Clavier.** Toute action est une commande d'un registre unique, avec
   raccourci et entrée dans la palette (ADR-005) ; la souris n'est pas
   indispensable, le clavier non plus pour les actions de rangement.
5. **Sobriété.** Faible empreinte (Tauri, webview du système), listes
   paginées, rien de superflu.

## Fonctionnalités

| Fonctionnalité | Fiche |
|---|---|
| Ajouter, configurer et retirer un compte (autodécouverte, certificats, trousseau) | [comptes.md](comptes.md) |
| Synchroniser les dossiers et les messages, rester à jour (IDLE), reconnexion | [synchronisation.md](synchronisation.md) |
| Afficher la liste des conversations, boîte unifiée | [conversations.md](conversations.md) |
| Lire un message en sécurité : HTML isolé, images, liens, pièces jointes, texte brut | [lecture.md](lecture.md) |
| Agir sur les messages : archiver, supprimer, déplacer, suivre, lu/non lu, annuler, hors ligne | [actions.md](actions.md) |
| Rédiger : nouveau message, répondre, répondre à tous, transférer, pièces jointes | [redaction.md](redaction.md) |
| Brouillons : enregistrement, reprise, brouillons d'autres clients | [brouillons.md](brouillons.md) |
| Envoyer : file d'envoi, nouvelles tentatives, envoi différé | [envoi.md](envoi.md) |
| Rechercher en plein texte | [recherche.md](recherche.md) |
| Notifier les nouveaux messages | [notifications.md](notifications.md) |
| Paramètres, langue et mises à jour | [parametres.md](parametres.md) |
| Clavier : registre de commandes, palette, raccourcis, accessibilité | [clavier.md](clavier.md) |

Vocabulaire : voir le [glossaire](glossaire.md).

## Lire une fiche

Chaque fiche suit le même plan :

- **But** : le besoin couvert ;
- **Comportement attendu** : ce que voit et fait l'utilisateur ;
- **Règles métier** : les règles, avec le module du domaine qui les porte
  (`crates/domain/src/…`) ;
- **Cas limites** : bornes, conflits, erreurs ;
- **Messages affichés** : clés des catalogues `app/messages/{fr,en}.json`
  (ADR-011) et texte français ;
- **Raccourcis** : commandes du registre (`app/src/lib/app.svelte.ts`).
