-- Version 2 : boîtes, messages et état de synchronisation.

CREATE TABLE mailboxes (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id     INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    name           TEXT    NOT NULL,
    -- Séparateur de hiérarchie (un caractère) ou NULL (nom plat).
    delimiter      TEXT    CHECK (delimiter IS NULL OR length(delimiter) = 1),
    role           TEXT    NOT NULL CHECK (role IN ('inbox', 'sent', 'drafts', 'trash',
                                                    'junk', 'archive', 'all', 'flagged',
                                                    'normal')),
    selectable     INTEGER NOT NULL CHECK (selectable IN (0, 1)),
    -- Ordre renvoyé par le serveur.
    position       INTEGER NOT NULL DEFAULT 0,
    -- État de synchronisation (NULL : inconnu).
    uid_validity   INTEGER CHECK (uid_validity BETWEEN 1 AND 4294967295),
    highest_modseq INTEGER CHECK (highest_modseq BETWEEN 1 AND 9223372036854775807),
    uid_next       INTEGER CHECK (uid_next BETWEEN 1 AND 4294967295),
    UNIQUE (account_id, name)
) STRICT;

CREATE TABLE messages (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    mailbox_id      INTEGER NOT NULL REFERENCES mailboxes (id) ON DELETE CASCADE,
    uid             INTEGER NOT NULL CHECK (uid BETWEEN 1 AND 4294967295),
    modseq          INTEGER CHECK (modseq BETWEEN 1 AND 9223372036854775807),
    -- Drapeaux système en masque (\Seen = 1, \Answered = 2, \Flagged = 4,
    -- \Deleted = 8, \Draft = 16) et mots-clés séparés par des espaces.
    flags           INTEGER NOT NULL DEFAULT 0 CHECK (flags BETWEEN 0 AND 31),
    keywords        TEXT    NOT NULL DEFAULT '',
    internal_date   INTEGER NOT NULL,
    -- Date de tri (en-tête Date plausible, sinon INTERNALDATE).
    sort_date       INTEGER NOT NULL,
    size            INTEGER NOT NULL CHECK (size >= 0),
    subject         TEXT    NOT NULL,
    from_name       TEXT    NOT NULL,
    from_address    TEXT    NOT NULL,
    -- Listes sérialisées en JSON (destinataires, références, structure).
    to_json         TEXT    NOT NULL DEFAULT '[]',
    cc_json         TEXT    NOT NULL DEFAULT '[]',
    message_id      TEXT,
    in_reply_to     TEXT    NOT NULL DEFAULT '[]',
    refs            TEXT    NOT NULL DEFAULT '[]',
    header_date     INTEGER,
    has_attachments INTEGER NOT NULL CHECK (has_attachments IN (0, 1)),
    structure       TEXT,
    -- Corps : absent, complet (body_blob = message brut) ou partiel
    -- (body_blob = en-têtes, parties dans message_parts).
    body_state      TEXT    NOT NULL DEFAULT 'missing'
                    CHECK (body_state IN ('missing', 'full', 'partial')),
    body_blob       TEXT    CHECK (body_blob IS NULL OR length(body_blob) = 64),
    UNIQUE (mailbox_id, uid)
) STRICT;

-- Liste d'un dossier, du plus récent au plus ancien (pagination par
-- curseur sur (sort_date, id)).
CREATE INDEX messages_by_date ON messages (mailbox_id, sort_date DESC, id DESC);

-- Parties téléchargées (blob) ou différées (blob NULL) d'un message
-- partiel.
CREATE TABLE message_parts (
    message_id INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    section    TEXT    NOT NULL,
    blob       TEXT    CHECK (blob IS NULL OR length(blob) = 64),
    PRIMARY KEY (message_id, section)
) STRICT;
