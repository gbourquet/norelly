-- Version 25 : titre, anniversaire, adresses postales et sites web des
-- fiches du carnet (ADR-012, modèle calqué sur vCard).
--
-- Les lignes relues sont revalidées par le domaine
-- (norelly_domain::contacts::ContactDraft::new), comme le reste de la
-- fiche.
--
-- - contacts.title : titre ou fonction (TITLE), NULL si absent ; il entre
--   dans search_text (calculé par le domaine). Les fiches enregistrées
--   avant n'en ont pas : leur search_text ne change pas.
-- - contacts.birthday : anniversaire sous la forme du domaine
--   (Birthday::iso : « 1985-04-12 », ou « --04-12 » sans année), NULL si
--   absent.
-- - contact_postal_addresses : adresses postales (ADR) dans l'ordre de la
--   fiche ; un élément vide est NULL, la rue peut tenir sur plusieurs
--   lignes.
-- - contact_websites : sites web (URL) dans l'ordre de la fiche, adresse
--   http ou https telle que l'accepte le domaine (website_url).

ALTER TABLE contacts ADD COLUMN title TEXT;
ALTER TABLE contacts ADD COLUMN birthday TEXT;

CREATE TABLE contact_postal_addresses (
    contact_id  INTEGER NOT NULL REFERENCES contacts (id) ON DELETE CASCADE,
    position    INTEGER NOT NULL CHECK (position >= 0),
    street      TEXT,
    locality    TEXT,
    region      TEXT,
    postal_code TEXT,
    country     TEXT,
    label       TEXT,
    PRIMARY KEY (contact_id, position)
) WITHOUT ROWID;

CREATE TABLE contact_websites (
    contact_id INTEGER NOT NULL REFERENCES contacts (id) ON DELETE CASCADE,
    position   INTEGER NOT NULL CHECK (position >= 0),
    url        TEXT    NOT NULL,
    label      TEXT,
    PRIMARY KEY (contact_id, position)
) WITHOUT ROWID;
