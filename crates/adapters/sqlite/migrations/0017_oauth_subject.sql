-- Version 17 : identifiant stable d'un compte chez son fournisseur OAuth
-- (`oid` chez Microsoft), qui le reconnaît aux reconnexions même si son
-- adresse principale change. NULL : compte à mot de passe ou Google, ou
-- compte Microsoft dont aucun identifiant n'est encore enregistré (il l'est
-- à sa prochaine reconnexion réussie). Ce n'est pas un secret.

-- Mêmes règles que le domaine : compte Microsoft seulement, 1 à 128
-- caractères ASCII visibles (de « ! » à « ~ »).
ALTER TABLE accounts ADD COLUMN oauth_subject TEXT
    CHECK (oauth_subject IS NULL OR (
        oauth_provider IS 'microsoft'
        AND length(oauth_subject) BETWEEN 1 AND 128
        AND oauth_subject NOT GLOB '*[^!-~]*'
    ));
