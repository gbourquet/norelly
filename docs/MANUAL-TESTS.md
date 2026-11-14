# Tests manuels sur de vrais fournisseurs

Les tests automatisés tournent contre des serveurs de test en conteneur
(Dovecot, Mailpit), jamais contre un vrai serveur (voir `CLAUDE.md`). Cette
checklist couvre ce qu'ils ne peuvent pas vérifier : le comportement des
grands fournisseurs (Gmail, Fastmail, Outlook.com), l'autodécouverte réelle,
les notifications et le trousseau du système.

À rejouer avant chaque version publiée, avec des comptes de test dédiés
(jamais un compte personnel). Consigner le résultat dans le tableau en fin
de document : date, version (`git describe`), système, testeur, résultat.

## Préparation

| Fournisseur | Prérequis |
|---|---|
| Gmail | Validation en deux étapes activée, **mot de passe d'application** créé (Compte Google › Sécurité › Mots de passe des applications) ; IMAP actif (Paramètres › Transfert et POP/IMAP). « Se connecter avec Google » : avertissement « Google n'a pas validé cette application » passé par « Paramètres avancés » ; chaque nouveau compte consomme une des places limitées de l'application non validée : utiliser un compte réel, toujours le même. |
| Fastmail | **Mot de passe d'application** avec accès IMAP et SMTP (Paramètres › Confidentialité et sécurité). |
| Outlook.com | Validation en deux étapes activée, **mot de passe d'application** créé (compte Microsoft › Sécurité › Options de sécurité avancées). IMAP par mot de passe peut être refusé par Microsoft : noter le message obtenu. |

Pour chaque fournisseur, partir d'un profil Norelly vierge (dossier de
données supprimé) et d'une boîte contenant au moins : un message texte, un
message HTML avec images distantes, un message avec une pièce jointe PDF,
une conversation de trois messages.

## Checklist (à cocher pour chaque fournisseur)

### Premier lancement et compte
- [ ] La question « rechercher automatiquement les mises à jour » apparaît, « Oui » présélectionné ; la réponse est gardée au redémarrage.
- [ ] Ajout du compte par l'adresse et le mot de passe d'application : serveurs trouvés automatiquement (source affichée), connexion testée, compte ajouté.
- [ ] Le mot de passe est rangé dans le trousseau du système (GNOME Keyring, KWallet, Gestionnaire d'identification Windows), pas dans un fichier.
- [ ] Mauvais mot de passe : message « identifiants refusés » compréhensible, avec la mention du mot de passe d'application.

### Synchronisation et lecture
- [ ] Dossiers affichés avec leurs rôles (Boîte de réception, Envoyés, Brouillons, Corbeille, Indésirables, Archives / Tous les messages chez Gmail).
- [ ] Boîte unifiée : les conversations des comptes apparaissent, la plus récente en premier.
- [ ] Message HTML : affiché, images distantes bloquées, bandeau proposé ; « Afficher les images de cet expéditeur » fonctionne et est retenu.
- [ ] Pièce jointe : ouverture et enregistrement ; un fichier exécutable est proposé à l'enregistrement seulement.
- [ ] Un message envoyé depuis un autre appareil arrive en quelques secondes (IDLE), sans action.

### Notifications
- [ ] Fenêtre en arrière-plan : un nouveau message produit une notification du système (expéditeur en titre, sujet en texte).
- [ ] Plusieurs messages d'un coup : une seule notification regroupée (« N nouveaux messages », le plus récent).
- [ ] Option « Masquer l'expéditeur et le sujet » : la notification ne montre que le nombre et l'adresse du compte.
- [ ] Notifications désactivées : aucune notification.
- [ ] Fenêtre au premier plan : pas de notification.
- [ ] Aucune notification à la première synchronisation d'un compte.

### Actions
- [ ] Marquer lu / non lu, suivre : reflété sur le webmail du fournisseur.
- [ ] Archiver (`e`) : message retiré de la boîte de réception, présent dans Archives (Gmail : libellé « Boîte de réception » retiré).
- [ ] Supprimer (`#`) : message dans la Corbeille du fournisseur.
- [ ] Déplacer vers un dossier : reflété sur le webmail.
- [ ] Annuler dans les 5 secondes : rien n'est envoyé au serveur.
- [ ] Action hors ligne (réseau coupé) : effet visible tout de suite, rejouée à la reconnexion.

### Rédaction
- [ ] Nouveau message (`c`) avec Cc, Cci et pièce jointe : reçu correctement ; le Cci n'apparaît pas chez les destinataires.
- [ ] Répondre (`r`), répondre à tous (`a`), transférer (`f`) : destinataires, sujet `Re:`/`Fwd:` et citation corrects.
- [ ] Copie dans Envoyés : présente une seule fois (Gmail la dépose lui-même).
- [ ] Brouillon : copié dans le dossier Brouillons du fournisseur, remplacé à chaque modification, supprimé après envoi.
- [ ] Envoi hors ligne : message mis en file, envoyé à la reconnexion.

### Recherche
- [ ] `/` puis un mot du corps d'un message : trouvé, surligné, ouvert par Entrée.
- [ ] `from:`, `subject:`, `has:attachment`, `is:unread`, `before:`/`after:` donnent les résultats attendus.
- [ ] Réseau coupé : la recherche fonctionne toujours.
- [ ] Gmail : un message présent dans « Tous les messages » et dans la boîte de réception n'apparaît qu'une fois.

### Paramètres et langue
- [ ] `Ctrl+,` ouvre les paramètres ; chaque réglage est gardé au redémarrage.
- [ ] Langue « English » : l'interface et les notifications sont en anglais au redémarrage ; « Celle du système » revient à la langue du système.
- [ ] Navigation entièrement au clavier : chaque action est atteignable sans souris (raccourcis, palette `Ctrl+K`), le focus est toujours visible.
- [ ] Entièrement à la souris, sans aucun raccourci : écrire, répondre, répondre à tous, transférer, archiver, supprimer, déplacer, marquer lu / non lu, suivre, cocher plusieurs messages puis agir dessus, rechercher, ouvrir les brouillons, la boîte d'envoi, les paramètres et l'aide, synchroniser, ajouter puis supprimer un compte, fermer chaque fenêtre.
- [ ] Chaque bouton affiche une infobulle « Libellé — Touche » ; le clic droit sur un message ouvre le menu des actions, raccourcis à droite.

## Résultats

| Date | Version | Système | Fournisseur | Testeur | Résultat (cases en échec, remarques) |
|---|---|---|---|---|---|
| | | | Gmail | | |
| | | | Fastmail | | |
| | | | Outlook.com | | |
