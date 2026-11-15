-- Version 15 : cause d'échec d'envoi « connexion coupée avant le
-- chiffrement » (encryption_interrupted).
--
-- La contrainte CHECK de outbox.failure ne se modifie pas, et la colonne
-- ne peut pas être remplacée (une contrainte de table la cite) : la table
-- est reconstruite, lignes et numérotation (sqlite_sequence) recopiées.
-- Aucune table ne référence outbox.

CREATE TABLE outbox_v15 (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id    INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    envelope_from TEXT    NOT NULL,
    recipients    TEXT    NOT NULL,
    raw           BLOB    NOT NULL,
    sent_copy     BLOB    NOT NULL,
    subject       TEXT    NOT NULL,
    state         TEXT    NOT NULL CHECK (state IN ('queued', 'sending', 'sent', 'failed')),
    not_before    INTEGER,
    failure       TEXT    CHECK (failure IS NULL OR failure IN
                      ('unreachable', 'temporary', 'rejected', 'authentication',
                       'security', 'uncertain', 'too_many_attempts',
                       'encryption_interrupted')),
    attempts      INTEGER NOT NULL CHECK (attempts >= 0),
    queued_at     INTEGER NOT NULL,
    draft_content TEXT,
    CHECK ((state = 'queued') = (not_before IS NOT NULL)),
    CHECK ((state = 'failed') = (failure IS NOT NULL))
);

INSERT INTO outbox_v15 (id, account_id, envelope_from, recipients, raw, sent_copy, subject,
                        state, not_before, failure, attempts, queued_at, draft_content)
    SELECT id, account_id, envelope_from, recipients, raw, sent_copy, subject,
           state, not_before, failure, attempts, queued_at, draft_content
    FROM outbox;

DELETE FROM sqlite_sequence WHERE name = 'outbox_v15';
INSERT INTO sqlite_sequence (name, seq)
    SELECT 'outbox_v15', seq FROM sqlite_sequence WHERE name = 'outbox';

DROP TABLE outbox;
ALTER TABLE outbox_v15 RENAME TO outbox;

CREATE INDEX outbox_by_account ON outbox (account_id, id);
