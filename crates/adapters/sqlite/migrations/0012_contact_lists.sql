-- Version 12 : listes de diffusion du carnet (ADR-012).
--
-- Une liste regroupe des contacts du carnet (pas des adresses) : chaque
-- membre est écrit à son adresse préférée au moment de l'envoi. Les lignes
-- relues sont revalidées par le domaine
-- (norelly_domain::contact_lists::ContactListDraft::new).
--
-- - contact_lists : name_key (nom en minuscules, calculé par le domaine)
--   est unique et ordonne la liste ; description vide : NULL ; bcc : les
--   membres partent en copie cachée.
-- - contact_list_members : membres dans l'ordre de la liste (position,
--   trous permis après une suppression), chaque contact une fois par
--   liste. Supprimer une liste ou un contact retire ses lignes.

CREATE TABLE contact_lists (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL,
    name_key    TEXT    NOT NULL UNIQUE,
    description TEXT,
    bcc         INTEGER NOT NULL DEFAULT 0 CHECK (bcc IN (0, 1))
);

CREATE TABLE contact_list_members (
    list_id    INTEGER NOT NULL REFERENCES contact_lists (id) ON DELETE CASCADE,
    position   INTEGER NOT NULL CHECK (position >= 0),
    contact_id INTEGER NOT NULL REFERENCES contacts (id) ON DELETE CASCADE,
    PRIMARY KEY (list_id, position),
    UNIQUE (list_id, contact_id)
) WITHOUT ROWID;

CREATE INDEX contact_list_members_by_contact ON contact_list_members (contact_id);
