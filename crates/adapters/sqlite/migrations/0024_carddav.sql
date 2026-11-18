-- Version 24 : synchronisation CardDAV du carnet (ADR-014).
--
-- - contacts.modified_at : date (secondes depuis 1970) de la dernière
--   écriture d'une fiche, posée par déclencheur quel que soit le chemin
--   d'écriture (saisie, import, fusion, synchronisation) : toute écriture
--   réécrit la ligne de contacts (norelly_sqlite::contacts::write). Elle
--   départage un conflit (la dernière modification gagne). Les fiches
--   existantes, de date inconnue, valent 0 : plus anciennes que toute
--   fiche d'un serveur, elles ne l'emportent pas à la première
--   synchronisation.
-- - carddav_account : le compte CardDAV, un seul (id = 1) : serveur,
--   identifiant, carnet choisi. Le mot de passe est dans le trousseau,
--   jamais ici.
-- - carddav_links : ressources du carnet distant connues depuis la
--   dernière synchronisation. contact_id renseigné : reliée à ce contact ;
--   NULL avec deleted_at : son contact a été supprimé ici, à cette date
--   (déclencheur avant suppression, puis ON DELETE SET NULL) ; NULL sans
--   deleted_at : fiche distante que le carnet ne reprend pas. synced :
--   fiche locale telle qu'à la dernière synchronisation ; remote : fiche
--   distante telle que lue ou écrite. Un contact n'est relié qu'à une
--   ressource.
-- - carddav_sync : la dernière synchronisation (id = 1) : date, bilan ou
--   code d'échec.

ALTER TABLE contacts ADD COLUMN modified_at INTEGER NOT NULL DEFAULT 0;

CREATE TRIGGER contacts_modified_on_insert AFTER INSERT ON contacts
BEGIN
    UPDATE contacts SET modified_at = unixepoch() WHERE id = NEW.id;
END;

CREATE TRIGGER contacts_modified_on_update
AFTER UPDATE OF display_name, given_name, family_name, nickname, organization, notes, favorite
ON contacts
BEGIN
    UPDATE contacts SET modified_at = unixepoch() WHERE id = NEW.id;
END;

CREATE TABLE carddav_account (
    id        INTEGER PRIMARY KEY CHECK (id = 1),
    server    TEXT NOT NULL,
    username  TEXT NOT NULL,
    book_url  TEXT NOT NULL,
    book_name TEXT NOT NULL
);

CREATE TABLE carddav_links (
    href       TEXT PRIMARY KEY,
    etag       TEXT NOT NULL,
    contact_id INTEGER UNIQUE REFERENCES contacts (id) ON DELETE SET NULL,
    deleted_at INTEGER,
    synced     TEXT NOT NULL,
    remote     TEXT NOT NULL
);

CREATE TRIGGER carddav_links_contact_deleted BEFORE DELETE ON contacts
BEGIN
    UPDATE carddav_links SET deleted_at = unixepoch() WHERE contact_id = OLD.id;
END;

CREATE TABLE carddav_sync (
    id      INTEGER PRIMARY KEY CHECK (id = 1),
    at      INTEGER NOT NULL,
    pulled  INTEGER NOT NULL DEFAULT 0,
    pushed  INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    failure TEXT
);
