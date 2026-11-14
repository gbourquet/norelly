-- Version 14 : comptes connectés par OAuth chez Google (« Se connecter
-- avec Google »), en plus de Microsoft.
--
-- Une contrainte CHECK ne se modifie pas : la colonne est remplacée par une
-- colonne de même nom qui accepte les deux fournisseurs, valeurs recopiées.
-- La table `accounts` n'est pas reconstruite (ses dépendances en cascade
-- restent intactes).

ALTER TABLE accounts ADD COLUMN oauth_provider_v14 TEXT
    CHECK (oauth_provider_v14 IS NULL OR oauth_provider_v14 IN ('microsoft', 'google'));

UPDATE accounts SET oauth_provider_v14 = oauth_provider;

ALTER TABLE accounts DROP COLUMN oauth_provider;
ALTER TABLE accounts RENAME COLUMN oauth_provider_v14 TO oauth_provider;
