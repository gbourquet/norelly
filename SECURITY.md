# Politique de sécurité / Security policy

## Signaler une vulnérabilité

**Ne publiez pas de vulnérabilité dans une issue publique.** Utilisez le
signalement privé de GitHub : onglet **Security** du dépôt, puis **Report a
vulnerability**. Décrivez le problème, la version (`Paramètres` ou nom de
l'installateur), le système et, si possible, un message de test qui le
reproduit (jamais un vrai message privé, jamais un mot de passe).

Exemples de problèmes de sécurité : un message qui exécute du code ou
atteint l'application, une image ou une requête réseau émise malgré le
blocage, une connexion acceptée sans chiffrement ou avec un certificat
invalide, un mot de passe visible dans un fichier ou un journal.

Le mainteneur publie un correctif, puis un avis une fois la version
corrigée disponible.

## Versions prises en charge

Seule la dernière version publiée reçoit des correctifs de sécurité.

## Report a vulnerability

**Do not publish vulnerabilities in public issues.** Use GitHub's private
reporting: the repository's **Security** tab, then **Report a
vulnerability**. Only the latest release receives security fixes.

Modèle de menace et défenses : [docs/technique/securite.md](docs/technique/securite.md).
