# Documentation fonctionnelle de Norelly

Ce dossier décrit **ce que fait Norelly**, du point de vue de la personne
qui l'utilise : comportements attendus, règles, cas limites et messages
affichés. Il s'adresse au mainteneur, aux contributeurs, aux testeurs et à
quiconque doit décider si un comportement est un défaut.

- Le **comment** (architecture, code, tests) est dans
  [la documentation technique](../technique/README.md), qui relie aussi
  chaque fiche au code qui la met en œuvre
  ([traçabilité](../technique/tracabilite.md)).
- Le **mode d'emploi** pour le grand public est dans
  [la documentation utilisateur](../utilisateur/fr/README.md).

Si l'application ne se comporte pas comme le décrit une fiche, l'écart est
à signaler : soit la fiche, soit l'application est à corriger.

## Présentation du produit

### À qui s'adresse Norelly

Norelly est un client de messagerie **IMAP/SMTP** de bureau, pour
**Windows et Linux**. Il vise :

- les particuliers qui ont deux ou trois adresses (personnelle,
  professionnelle, associative) chez des fournisseurs différents et
  veulent les lire au même endroit ;
- les personnes attentives à leur vie privée, qui ne veulent ni pistage à
  la lecture ni télémétrie ;
- le grand public habitué à Gmail ou Outlook, qui fait tout à la souris,
  ne connaît aucun raccourci et ne lit pas de documentation : chaque action
  se comprend en regardant l'écran ;
- les utilisateurs qui préfèrent le clavier (raccourcis à la Gmail, palette
  de commandes), pour qui les raccourcis sont des accélérateurs ;
- les personnes qui gardent beaucoup de messages : plusieurs comptes de
  plusieurs dizaines de milliers de messages chacun.

### Objectifs

- **Une application performante** : elle s'ouvre vite et reste fluide,
  même avec des boîtes volumineuses (changer de dossier, faire défiler la
  liste, chercher).
- **Une application sûre** : un message reçu ne peut ni exécuter de code,
  ni pister sa lecture, ni affaiblir la sécurité de la connexion.
- **Une application qui fonctionne hors ligne** : lire, chercher, ranger et
  écrire restent possibles sans réseau.
- **Une application utilisable entièrement à la souris**, par des boutons
  et des menus visibles et libellés, **et rapide au clavier** ; accessible.
- **Une application sobre** : légère, sans superflu.

### Ce que fait Norelly

- ajoute un compte à partir de l'adresse et du mot de passe seuls, en
  trouvant les serveurs automatiquement ;
- synchronise les dossiers et garde une copie locale des messages,
  consultable et cherchable hors ligne ;
- regroupe les messages en conversations, y compris dans une boîte de
  réception unifiée de tous les comptes ;
- affiche le HTML des messages de façon isolée, sans requête réseau non
  autorisée ;
- range, supprime, archive, marque, y compris hors ligne (actions envoyées
  au serveur au retour du réseau), avec annulation ;
- rédige, répond, transfère, avec mise en forme simple et pièces jointes ;
  enregistre les brouillons ; envoie tout de suite, plus tard ou au retour
  du réseau ;
- recherche en plein texte dans tous les messages ;
- notifie les nouveaux messages ;
- en français et en anglais.

### Ce que Norelly ne fait pas (aujourd'hui)

- **Pas de POP3, ni d'Exchange, ni de JMAP** : IMAP et SMTP uniquement.
- **Connexion OAuth2 (« authentification moderne ») chez Microsoft
  seulement** (Outlook.com, Hotmail) ; Gmail passe par un mot de passe
  d'application.
- **Pas de chiffrement de bout en bout** (OpenPGP, S/MIME), ni de signature
  des messages.
- **Pas de chiffrement des données locales par Norelly** : elles sont
  protégées par le chiffrement du disque, s'il est activé sur l'ordinateur.
- **Pas d'agenda ni de filtres** ; le carnet d'adresses est local (pas de
  synchronisation CardDAV) et les suggestions de destinataires viennent des
  adresses déjà vues.
- **Pas de macOS** ni de version mobile.
- **Pas de télémétrie**, ni de rapport de plantage automatique.

### Principes

1. **Sécurité d'abord.** Tout ce qui vient du réseau (messages, réponses
   des serveurs, recherche automatique des paramètres) est traité comme
   hostile. Aucune baisse de sécurité silencieuse : pas de connexion non
   chiffrée sans consentement, pas de chiffrement annoncé puis ignoré, pas
   de certificat accepté sans décision explicite de l'utilisateur.
2. **Vie privée.** Aucune requête au moment de la lecture d'un message sans
   autorisation (images distantes bloquées par défaut) ; aucune
   télémétrie ; la recherche de mises à jour est proposée, jamais imposée.
3. **Hors ligne.** Ce qui est affiché vient de la copie locale. Lire,
   chercher, ranger et écrire fonctionnent sans réseau ; les actions et les
   envois attendent le retour du réseau, sans se perdre, même si
   l'application est fermée entre-temps.
4. **Souris d'abord, clavier en accélérateur.** Aucune action n'est
   réservée au clavier : chacune a un élément visible et libellé (bouton,
   menu « Plus… », menu d'un compte, menu contextuel, case à cocher). Toute
   action est aussi une commande, accessible par la palette et, pour les
   plus courantes, par un raccourci, rappelé dans l'infobulle du bouton.
5. **Sobriété.** Application légère, qui ne charge que ce qu'elle affiche,
   sans fonctions superflues.

## Fonctionnalités

| Fonctionnalité | Fiche |
|---|---|
| Ajouter, configurer et retirer un compte (recherche des serveurs, certificats, trousseau) | [comptes.md](comptes.md) |
| Synchroniser les dossiers et les messages, rester à jour, reconnexion | [synchronisation.md](synchronisation.md) |
| Afficher la liste des conversations, boîte unifiée | [conversations.md](conversations.md) |
| Lire un message en sécurité : HTML isolé, images, liens, pièces jointes, texte brut | [lecture.md](lecture.md) |
| Agir sur les messages : archiver, supprimer, déplacer, suivre, lu/non lu, annuler, hors ligne | [actions.md](actions.md) |
| Créer un dossier (au premier niveau ou dans un dossier existant), le renommer, le déplacer, le supprimer | [dossiers.md](dossiers.md) |
| Rédiger : nouveau message, répondre, répondre à tous, transférer, pièces jointes | [redaction.md](redaction.md) |
| Brouillons : enregistrement, reprise, brouillons d'autres clients | [brouillons.md](brouillons.md) |
| Envoyer : boîte d'envoi, nouvelles tentatives, envoi différé | [envoi.md](envoi.md) |
| Rechercher en plein texte | [recherche.md](recherche.md) |
| Carnet d'adresses : ajouter, modifier, supprimer un contact, favoris, écrire à un contact | [contacts.md](contacts.md) |
| Notifier les nouveaux messages | [notifications.md](notifications.md) |
| Paramètres, langue et mises à jour | [parametres.md](parametres.md) |
| Commandes : souris, clavier, palette, accessibilité | [clavier.md](clavier.md) |

Vocabulaire : voir le [glossaire](glossaire.md).

## Lire une fiche

Chaque fiche suit le même plan :

- **But** : le besoin couvert ;
- **Comportement attendu** : ce que voit et fait l'utilisateur ;
- **Règles** : les règles que l'application applique ;
- **Cas limites** : bornes, conflits, erreurs ;
- **Messages affichés** : textes affichés à l'utilisateur, en français ;
- **Raccourcis** : commandes et touches associées.
