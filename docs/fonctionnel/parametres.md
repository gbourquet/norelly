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

**Mises à jour (installateur Windows)** : au premier lancement, la question
« Norelly peut rechercher automatiquement ses mises à jour. Le
souhaitez-vous ? » est posée, **Oui** étant proposé par défaut. Fermer la
question sans répondre ne vaut pas « Oui ». Si une version est disponible,
un bandeau propose « Installer et redémarrer » ou « Plus tard ». Installé
depuis Flathub ou par un paquet .deb ou .rpm, Norelly est mis à jour par le
système : ni question ni bandeau.

**Langue** : langue du système par défaut (français si la langue préférée
est le français, anglais pour toute autre langue). Dates, heures, tailles
et nombres suivent la langue choisie.

## Règles

- **Recherche de mises à jour** : proposée au premier lancement,
  modifiable à tout moment ; au plus une fois par jour, sans aucun
  identifiant de l'utilisateur ni de l'ordinateur.
- **Mise à jour installée seulement si elle est authentique** : sa
  signature est vérifiée avant l'installation.
- **Seuil de téléchargement des pièces jointes** : entre 64 Kio et
  64 Mio ; une valeur hors de ces bornes est remplacée par la valeur par
  défaut.

## Cas limites

- Paramètres illisibles : valeurs par défaut, sans erreur.
- Échec d'installation d'une mise à jour : message d'erreur.

## Messages affichés

| Situation | Texte |
|---|---|
| Titre | Paramètres |
| Réglages appliqués au redémarrage | La langue et le seuil de téléchargement s'appliquent au prochain démarrage. |
| Question au premier lancement | Norelly peut rechercher automatiquement ses mises à jour. Le souhaitez-vous ? |
| Version disponible | La version {version} de Norelly est disponible. |
| Boutons | Installer et redémarrer, Plus tard |
| Échec | Mise à jour impossible. |

## Raccourcis

| Commande | Raccourci |
|---|---|
| Paramètres… | `Ctrl+,` |
