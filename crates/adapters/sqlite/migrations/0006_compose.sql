-- Version 6 : rédaction.
--
-- - drafts : brouillons locaux. Le contenu (destinataires, sujet, corps,
--   fil, pièces jointes) est un document JSON, revalidé à la relecture
--   (adresses, identifiants de blobs). revision : enregistrements locaux ;
--   remote_* : copie dans le dossier « Brouillons » du serveur
--   (UIDVALIDITY et UID d'APPENDUID).
-- - draft_remote_trash : copies distantes à supprimer du serveur
--   (brouillon remplacé, supprimé ou envoyé) ; un ensemble.
-- - outbox : file d'envoi durable. raw : message transmis (sans Bcc) ;
--   sent_copy : copie pour « Envoyés » (avec Bcc). L'état suit
--   norelly_domain::compose::outbox.

CREATE TABLE drafts (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id       INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    content          TEXT    NOT NULL,
    revision         INTEGER NOT NULL CHECK (revision >= 1),
    remote_revision  INTEGER CHECK (remote_revision IS NULL OR remote_revision >= 1),
    remote_validity  INTEGER CHECK (remote_validity IS NULL OR remote_validity BETWEEN 1 AND 4294967295),
    remote_uid       INTEGER CHECK (remote_uid IS NULL OR remote_uid BETWEEN 1 AND 4294967295),
    last_remote_save INTEGER,
    updated_at       INTEGER NOT NULL,
    CHECK ((remote_validity IS NULL) = (remote_uid IS NULL))
);

CREATE INDEX drafts_by_account ON drafts (account_id, updated_at DESC, id DESC);

CREATE TABLE draft_remote_trash (
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    validity   INTEGER NOT NULL CHECK (validity BETWEEN 1 AND 4294967295),
    uid        INTEGER NOT NULL CHECK (uid BETWEEN 1 AND 4294967295),
    PRIMARY KEY (account_id, validity, uid)
) WITHOUT ROWID;

CREATE TABLE outbox (
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
                       'security', 'uncertain', 'too_many_attempts')),
    attempts      INTEGER NOT NULL CHECK (attempts >= 0),
    queued_at     INTEGER NOT NULL,
    CHECK ((state = 'queued') = (not_before IS NOT NULL)),
    CHECK ((state = 'failed') = (failure IS NOT NULL))
);

CREATE INDEX outbox_by_account ON outbox (account_id, id);
