-- Version 11 : carnet d'adresses (ADR-012).
--
-- Carnet unique, commun à tous les comptes (pas de account_id), calqué
-- sur vCard. Les lignes relues sont revalidées par le domaine
-- (norelly_domain::contacts::ContactDraft::new).
--
-- - contacts : un champ vide est NULL. sort_key (nom affiché en
--   minuscules) ordonne la liste ; search_text (noms, organisation et
--   adresses en minuscules, un par ligne) sert au filtre. Les deux sont
--   calculés par le domaine à chaque écriture.
-- - contact_emails : adresses dans l'ordre de la fiche (position).
--   address_key (adresse en minuscules) est unique dans tout le carnet :
--   une adresse n'appartient qu'à un contact. Une seule adresse préférée
--   par contact.
-- - contact_phones : téléphones dans l'ordre de la fiche.

CREATE TABLE contacts (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    display_name TEXT,
    given_name   TEXT,
    family_name  TEXT,
    nickname     TEXT,
    organization TEXT,
    notes        TEXT,
    favorite     INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1)),
    sort_key     TEXT    NOT NULL,
    search_text  TEXT    NOT NULL
);

CREATE INDEX contacts_by_sort ON contacts (sort_key, id);
CREATE INDEX contacts_favorites_by_sort ON contacts (sort_key, id) WHERE favorite = 1;

CREATE TABLE contact_emails (
    contact_id  INTEGER NOT NULL REFERENCES contacts (id) ON DELETE CASCADE,
    position    INTEGER NOT NULL CHECK (position >= 0),
    address     TEXT    NOT NULL,
    address_key TEXT    NOT NULL UNIQUE,
    label       TEXT,
    preferred   INTEGER NOT NULL CHECK (preferred IN (0, 1)),
    PRIMARY KEY (contact_id, position)
) WITHOUT ROWID;

CREATE UNIQUE INDEX contact_emails_one_preferred ON contact_emails (contact_id) WHERE preferred = 1;

CREATE TABLE contact_phones (
    contact_id INTEGER NOT NULL REFERENCES contacts (id) ON DELETE CASCADE,
    position   INTEGER NOT NULL CHECK (position >= 0),
    number     TEXT    NOT NULL,
    label      TEXT,
    PRIMARY KEY (contact_id, position)
) WITHOUT ROWID;
