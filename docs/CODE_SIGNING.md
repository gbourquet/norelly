# Politique de signature du code

Les installateurs Windows de Norelly seront signés gratuitement par
[SignPath.io](https://about.signpath.io), avec un certificat fourni par la
[SignPath Foundation](https://signpath.org) (ADR-006), dès que le projet aura
été accepté. D'ici là, l'installateur NSIS n'est pas signé (Authenticode) ;
seules les mises à jour automatiques sont vérifiées par leur signature
minisign.

## Ce qui est signé

- L'installateur NSIS (`Norelly_<version>_x64-setup.exe`) et l'exécutable
  qu'il contient, construits par le workflow public
  [`release.yml`](../.github/workflows/release.yml) à partir d'un tag `v*` du
  dépôt <https://github.com/gbourquet/norelly>.
- Rien d'autre : aucun binaire construit hors de GitHub Actions, aucun
  composant tiers resigné.

## Rôles

| Rôle | Personnes |
|---|---|
| Auteurs (commits) | contributeurs du dépôt ; chaque modification passe par une PR |
| Relecteurs | Guillaume Bourquet (mainteneur), relecture obligatoire de chaque PR vers `master` (branche protégée) |
| Approbateurs de signature | Guillaume Bourquet |

Chaque demande de signature est approuvée manuellement dans SignPath par un
approbateur, après vérification que l'artefact provient du workflow de
publication du tag attendu.

## Engagements

- Le code source de chaque version signée est public, sous licence
  `MIT OR Apache-2.0`.
- Les dépendances sont contrôlées par `cargo deny` (licences, avis de
  sécurité, sources) et `pnpm audit` ; les actions GitHub sont figées par
  SHA de commit.
- Un compte d'approbateur compromis ou une signature indue est signalé
  publiquement (avis de sécurité GitHub) et le certificat est révoqué via
  SignPath.

## Confidentialité

Norelly n'envoie aucune donnée à ses auteurs : aucune télémétrie, aucun
rapport d'erreur automatique. Le seul accès réseau hors des serveurs mail de
l'utilisateur est la recherche de mises à jour (installateur Windows
seulement), faite uniquement si l'utilisateur l'a acceptée, vers
`github.com`, sans aucun identifiant ; la recherche des réglages d'un compte
interroge le fournisseur de l'adresse et la base publique de Thunderbird
(ISPDB).
