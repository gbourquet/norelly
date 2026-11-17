-- Version 21 : étiquettes.
--
-- labels : étiquettes enregistrées d'un compte, par mot-clé IMAP (atom
-- ASCII, encodé depuis le nom par norelly_domain::labels). La couleur est
-- locale (jamais synchronisée), `#rrggbb` en minuscules. `hidden` : étiquette
-- supprimée de la liste sans être retirée des messages ; elle n'est plus ni
-- listée ni affichée.
--
-- label_rewrites : réécritures en attente d'un mot-clé sur tous les
-- messages du serveur (renommage vers `to_keyword`, ou retrait si NULL).
-- Elles sont appliquées en local dès l'enregistrement, puis rejouées sur
-- le serveur dossier par dossier ; rejouer une réécriture interrompue est
-- sans danger (elle cherche les messages qui portent encore l'ancien
-- mot-clé).
--
-- mailboxes.permanent_flags : mots-clés que le dossier garde, d'après
-- PERMANENTFLAGS au dernier SELECT en écriture (NULL : inconnu, tout est
-- permis ; '*' : tout mot-clé ; '=' suivi des mots-clés permis).
--
-- pending_ops : reconstruite pour les opérations de mot-clé (flag =
-- 'keyword', mot-clé dans la colonne `keyword`) ; le compteur
-- AUTOINCREMENT est conservé (les copies provisoires désignent leur
-- opération par son identifiant).

CREATE TABLE labels (
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    keyword    TEXT    NOT NULL CHECK (length(keyword) BETWEEN 1 AND 64),
    color      TEXT    CHECK (color IS NULL OR color GLOB '#[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
    hidden     INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1)),
    PRIMARY KEY (account_id, keyword)
) STRICT;

CREATE TABLE label_rewrites (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id   INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    from_keyword TEXT    NOT NULL CHECK (length(from_keyword) BETWEEN 1 AND 64),
    to_keyword   TEXT    CHECK (to_keyword IS NULL OR length(to_keyword) BETWEEN 1 AND 64),
    attempts     INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0)
) STRICT;

CREATE INDEX label_rewrites_by_account ON label_rewrites (account_id, id);

ALTER TABLE mailboxes ADD COLUMN permanent_flags TEXT;

CREATE TABLE pending_ops_next (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id      INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    message_id      INTEGER NOT NULL CHECK (message_id > 0),
    mailbox         TEXT    NOT NULL,
    uid_validity    INTEGER NOT NULL CHECK (uid_validity BETWEEN 1 AND 4294967295),
    uid             INTEGER NOT NULL CHECK (uid BETWEEN 1 AND 4294967295),
    kind            TEXT    NOT NULL CHECK (kind IN ('flag', 'move', 'expunge')),
    flag            TEXT    CHECK (flag IS NULL OR flag IN ('seen', 'flagged', 'keyword')),
    value           INTEGER CHECK (value IS NULL OR value IN (0, 1)),
    previous        INTEGER CHECK (previous IS NULL OR previous IN (0, 1)),
    unchanged_since INTEGER CHECK (unchanged_since IS NULL
                                   OR unchanged_since BETWEEN 1 AND 9223372036854775807),
    target          TEXT,
    not_before      INTEGER NOT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    claimed         INTEGER NOT NULL DEFAULT 0 CHECK (claimed IN (0, 1)),
    keyword         TEXT    CHECK (keyword IS NULL OR length(keyword) BETWEEN 1 AND 64),
    CHECK ((kind = 'flag') = (flag IS NOT NULL AND value IS NOT NULL AND previous IS NOT NULL)),
    CHECK ((kind = 'move') = (target IS NOT NULL)),
    CHECK ((flag IS 'keyword') = (keyword IS NOT NULL))
) STRICT;

INSERT INTO pending_ops_next (id, account_id, message_id, mailbox, uid_validity, uid, kind, flag,
                              value, previous, unchanged_since, target, not_before, attempts,
                              claimed)
    SELECT id, account_id, message_id, mailbox, uid_validity, uid, kind, flag, value, previous,
           unchanged_since, target, not_before, attempts, claimed
    FROM pending_ops;

DELETE FROM sqlite_sequence WHERE name = 'pending_ops_next';
INSERT INTO sqlite_sequence (name, seq)
    SELECT 'pending_ops_next', seq FROM sqlite_sequence WHERE name = 'pending_ops';

DROP TABLE pending_ops;
ALTER TABLE pending_ops_next RENAME TO pending_ops;

CREATE INDEX pending_ops_by_account ON pending_ops (account_id, id);
