-- Version 22 : groupes de doublons écartés du carnet (ADR-012).
--
-- contact_duplicate_dismissals : paires de contacts que l'utilisateur a
-- passées dans la recherche de doublons (norelly_domain::contact_duplicates,
-- Dismissal), du plus petit identifiant au plus grand. `batch` numérote le
-- groupe passé (un appel) ; le rowid ordonne les paires de la plus ancienne
-- à la plus récente. Au-delà de la borne (MAX_DISMISSED_PAIRS), les plus
-- anciens groupes sont supprimés, chacun en entier.
--
-- Une paire est oubliée dès que l'une de ses fiches change ou disparaît :
-- - suppression : clés étrangères ON DELETE CASCADE ;
-- - modification : déclencheur sur contacts. Toute écriture d'une fiche
--   (modification, import, fusion) réécrit sa ligne de contacts
--   (norelly_sqlite::contacts::write), adresses et téléphones compris.

CREATE TABLE contact_duplicate_dismissals (
    low_id  INTEGER NOT NULL REFERENCES contacts (id) ON DELETE CASCADE,
    high_id INTEGER NOT NULL REFERENCES contacts (id) ON DELETE CASCADE,
    batch   INTEGER NOT NULL CHECK (batch > 0),
    PRIMARY KEY (low_id, high_id),
    CHECK (low_id < high_id)
);

CREATE INDEX contact_duplicate_dismissals_by_high ON contact_duplicate_dismissals (high_id);
CREATE INDEX contact_duplicate_dismissals_by_batch ON contact_duplicate_dismissals (batch);

CREATE TRIGGER contact_duplicate_dismissals_forget AFTER UPDATE ON contacts
BEGIN
    DELETE FROM contact_duplicate_dismissals
    WHERE low_id = OLD.id OR high_id = OLD.id;
END;
