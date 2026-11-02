-- Version 4 : actions sur les messages (phase 6).
--
-- - messages.hidden : message déplacé ou supprimé localement, en attente
--   du serveur (application optimiste) ; il reste en base pour que la
--   synchronisation continue de le connaître, et réapparaît si l'action
--   est annulée ou abandonnée.
-- - Les agrégats des conversations ignorent les messages masqués et ceux
--   marqués \Deleted (flags & 8).
-- - pending_ops : file durable des opérations à rejouer sur le serveur.
-- - action_settings : dossier de repli de « archiver » par compte.

ALTER TABLE messages ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1));

DROP TRIGGER messages_conversation_insert;
DROP TRIGGER messages_conversation_delete;
DROP TRIGGER messages_conversation_update;

CREATE TRIGGER messages_conversation_insert AFTER INSERT ON messages
BEGIN
    DELETE FROM conversation_mailboxes
     WHERE conversation_id = NEW.conversation_id AND mailbox_id = NEW.mailbox_id;
    INSERT INTO conversation_mailboxes
        (conversation_id, mailbox_id, role, latest_date, messages, unread, flagged, attachments)
    SELECT m.conversation_id, m.mailbox_id, b.role, MAX(m.sort_date), COUNT(*),
           SUM((m.flags & 1) = 0), MAX((m.flags & 4) != 0), MAX(m.has_attachments)
      FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id
     WHERE m.conversation_id = NEW.conversation_id AND m.mailbox_id = NEW.mailbox_id
       AND m.hidden = 0 AND (m.flags & 8) = 0
     GROUP BY m.conversation_id, m.mailbox_id;
END;

CREATE TRIGGER messages_conversation_delete AFTER DELETE ON messages
BEGIN
    DELETE FROM conversation_mailboxes
     WHERE conversation_id = OLD.conversation_id AND mailbox_id = OLD.mailbox_id;
    INSERT INTO conversation_mailboxes
        (conversation_id, mailbox_id, role, latest_date, messages, unread, flagged, attachments)
    SELECT m.conversation_id, m.mailbox_id, b.role, MAX(m.sort_date), COUNT(*),
           SUM((m.flags & 1) = 0), MAX((m.flags & 4) != 0), MAX(m.has_attachments)
      FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id
     WHERE m.conversation_id = OLD.conversation_id AND m.mailbox_id = OLD.mailbox_id
       AND m.hidden = 0 AND (m.flags & 8) = 0
     GROUP BY m.conversation_id, m.mailbox_id;
END;

CREATE TRIGGER messages_conversation_update
AFTER UPDATE OF conversation_id, mailbox_id, flags, sort_date, has_attachments, hidden ON messages
BEGIN
    DELETE FROM conversation_mailboxes
     WHERE conversation_id = OLD.conversation_id AND mailbox_id = OLD.mailbox_id;
    INSERT INTO conversation_mailboxes
        (conversation_id, mailbox_id, role, latest_date, messages, unread, flagged, attachments)
    SELECT m.conversation_id, m.mailbox_id, b.role, MAX(m.sort_date), COUNT(*),
           SUM((m.flags & 1) = 0), MAX((m.flags & 4) != 0), MAX(m.has_attachments)
      FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id
     WHERE m.conversation_id = OLD.conversation_id AND m.mailbox_id = OLD.mailbox_id
       AND m.hidden = 0 AND (m.flags & 8) = 0
     GROUP BY m.conversation_id, m.mailbox_id;
    DELETE FROM conversation_mailboxes
     WHERE conversation_id = NEW.conversation_id AND mailbox_id = NEW.mailbox_id;
    INSERT INTO conversation_mailboxes
        (conversation_id, mailbox_id, role, latest_date, messages, unread, flagged, attachments)
    SELECT m.conversation_id, m.mailbox_id, b.role, MAX(m.sort_date), COUNT(*),
           SUM((m.flags & 1) = 0), MAX((m.flags & 4) != 0), MAX(m.has_attachments)
      FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id
     WHERE m.conversation_id = NEW.conversation_id AND m.mailbox_id = NEW.mailbox_id
       AND m.hidden = 0 AND (m.flags & 8) = 0
     GROUP BY m.conversation_id, m.mailbox_id;
END;

-- Agrégats existants recalculés sans les messages marqués \Deleted.
DELETE FROM conversation_mailboxes;
INSERT INTO conversation_mailboxes
    (conversation_id, mailbox_id, role, latest_date, messages, unread, flagged, attachments)
SELECT m.conversation_id, m.mailbox_id, b.role, MAX(m.sort_date), COUNT(*),
       SUM((m.flags & 1) = 0), MAX((m.flags & 4) != 0), MAX(m.has_attachments)
  FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id
 WHERE m.conversation_id IS NOT NULL AND m.hidden = 0 AND (m.flags & 8) = 0
 GROUP BY m.conversation_id, m.mailbox_id;

CREATE TABLE pending_ops (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id      INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- Message local visé ; sans clé étrangère : la ligne peut disparaître
    -- (synchronisation) pendant que l'opération attend.
    message_id      INTEGER NOT NULL CHECK (message_id > 0),
    mailbox         TEXT    NOT NULL,
    uid_validity    INTEGER NOT NULL CHECK (uid_validity BETWEEN 1 AND 4294967295),
    uid             INTEGER NOT NULL CHECK (uid BETWEEN 1 AND 4294967295),
    kind            TEXT    NOT NULL CHECK (kind IN ('flag', 'move', 'expunge')),
    -- Opération sur un drapeau.
    flag            TEXT    CHECK (flag IS NULL OR flag IN ('seen', 'flagged')),
    value           INTEGER CHECK (value IS NULL OR value IN (0, 1)),
    previous        INTEGER CHECK (previous IS NULL OR previous IN (0, 1)),
    unchanged_since INTEGER CHECK (unchanged_since IS NULL
                                   OR unchanged_since BETWEEN 1 AND 9223372036854775807),
    -- Déplacement : boîte de destination.
    target          TEXT,
    not_before      INTEGER NOT NULL,
    -- Essais de rejeu (incrémenté avant tout envoi au serveur) ; au-delà
    -- d'un seuil, l'opération est abandonnée.
    attempts        INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    -- Réservée par le rejeu : l'état du serveur est incertain, l'utilisateur
    -- ne peut plus l'annuler.
    claimed         INTEGER NOT NULL DEFAULT 0 CHECK (claimed IN (0, 1)),
    CHECK ((kind = 'flag') = (flag IS NOT NULL AND value IS NOT NULL AND previous IS NOT NULL)),
    CHECK ((kind = 'move') = (target IS NOT NULL))
) STRICT;

CREATE INDEX pending_ops_by_account ON pending_ops (account_id, id);

CREATE TABLE action_settings (
    account_id      INTEGER PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    archive_mailbox INTEGER REFERENCES mailboxes (id) ON DELETE SET NULL
) STRICT;
