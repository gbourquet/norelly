-- Version 13 : comptes connectés par OAuth (« Se connecter avec
-- Microsoft »). NULL : compte à mot de passe. Les jetons restent dans le
-- trousseau du système, jamais en base.

ALTER TABLE accounts ADD COLUMN oauth_provider TEXT
    CHECK (oauth_provider IS NULL OR oauth_provider IN ('microsoft'));
