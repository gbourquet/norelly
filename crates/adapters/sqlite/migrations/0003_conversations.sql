-- Version 3 : conversations précalculées (ADR-003) et préférences de
-- lecture.
--
-- Le rattachement d'un message à sa conversation est décidé en Rust par le
-- domaine (norelly_domain::conversation) au moment de l'insertion ; les
-- agrégats par dossier (conversation_mailboxes) sont tenus à jour par des
-- déclencheurs, quel que soit le chemin d'écriture (synchronisation,
-- suppression d'une boîte ou d'un compte en cascade).

CREATE TABLE conversations (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- Clé de sujet (sans Re:, Fwd:, étiquettes de liste), pour le
    -- regroupement par sujet en dernier recours.
    subject_key TEXT    NOT NULL,
    -- Date du message le plus récent jamais rattaché.
    latest_date INTEGER NOT NULL
) STRICT;

CREATE INDEX conversations_by_subject ON conversations (account_id, subject_key, latest_date DESC);

-- Identifiants de message (propres et cités) connus de chaque
-- conversation : un nouveau message qui en cite un la rejoint.
CREATE TABLE conversation_links (
    account_id      INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    message_id      TEXT    NOT NULL,
    conversation_id INTEGER NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    PRIMARY KEY (account_id, message_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX conversation_links_by_conversation ON conversation_links (conversation_id);

-- NULL : message pas encore rattaché (base antérieure à la version 3,
-- rattachée à l'ouverture).
ALTER TABLE messages ADD COLUMN conversation_id INTEGER
    REFERENCES conversations (id) ON DELETE SET NULL;

CREATE INDEX messages_by_conversation ON messages (conversation_id, sort_date);

-- Agrégats d'une conversation dans un dossier : une ligne par couple
-- (conversation, dossier) ayant au moins un message.
CREATE TABLE conversation_mailboxes (
    conversation_id INTEGER NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    mailbox_id      INTEGER NOT NULL,
    -- Rôle du dossier, recopié pour la boîte unifiée (index par rôle).
    role            TEXT    NOT NULL,
    latest_date     INTEGER NOT NULL,
    messages        INTEGER NOT NULL CHECK (messages > 0),
    unread          INTEGER NOT NULL CHECK (unread >= 0),
    flagged         INTEGER NOT NULL CHECK (flagged IN (0, 1)),
    attachments     INTEGER NOT NULL CHECK (attachments IN (0, 1)),
    PRIMARY KEY (conversation_id, mailbox_id)
) STRICT;

CREATE INDEX conversation_mailboxes_by_mailbox
    ON conversation_mailboxes (mailbox_id, latest_date DESC, conversation_id DESC);
CREATE INDEX conversation_mailboxes_by_role
    ON conversation_mailboxes (role, latest_date DESC, conversation_id DESC);

-- Recalcul de l'agrégat (conversation, dossier) d'un message inséré,
-- supprimé ou modifié. Un dossier en cours de suppression (cascade) n'est
-- plus joignable : son agrégat disparaît.
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
     GROUP BY m.conversation_id, m.mailbox_id;
END;

CREATE TRIGGER messages_conversation_update
AFTER UPDATE OF conversation_id, mailbox_id, flags, sort_date, has_attachments ON messages
BEGIN
    DELETE FROM conversation_mailboxes
     WHERE conversation_id = OLD.conversation_id AND mailbox_id = OLD.mailbox_id;
    INSERT INTO conversation_mailboxes
        (conversation_id, mailbox_id, role, latest_date, messages, unread, flagged, attachments)
    SELECT m.conversation_id, m.mailbox_id, b.role, MAX(m.sort_date), COUNT(*),
           SUM((m.flags & 1) = 0), MAX((m.flags & 4) != 0), MAX(m.has_attachments)
      FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id
     WHERE m.conversation_id = OLD.conversation_id AND m.mailbox_id = OLD.mailbox_id
     GROUP BY m.conversation_id, m.mailbox_id;
    DELETE FROM conversation_mailboxes
     WHERE conversation_id = NEW.conversation_id AND mailbox_id = NEW.mailbox_id;
    INSERT INTO conversation_mailboxes
        (conversation_id, mailbox_id, role, latest_date, messages, unread, flagged, attachments)
    SELECT m.conversation_id, m.mailbox_id, b.role, MAX(m.sort_date), COUNT(*),
           SUM((m.flags & 1) = 0), MAX((m.flags & 4) != 0), MAX(m.has_attachments)
      FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id
     WHERE m.conversation_id = NEW.conversation_id AND m.mailbox_id = NEW.mailbox_id
     GROUP BY m.conversation_id, m.mailbox_id;
END;

-- Le rôle d'un dossier peut changer (SPECIAL-USE ajouté côté serveur).
CREATE TRIGGER mailboxes_role_update AFTER UPDATE OF role ON mailboxes
BEGIN
    UPDATE conversation_mailboxes SET role = NEW.role WHERE mailbox_id = NEW.id;
END;

-- Un dossier supprimé emporte ses agrégats (les messages partent en
-- cascade ; leurs déclencheurs ne trouvent plus le dossier).
CREATE TRIGGER mailboxes_delete AFTER DELETE ON mailboxes
BEGIN
    DELETE FROM conversation_mailboxes WHERE mailbox_id = OLD.id;
END;

-- Expéditeurs dont les images distantes sont autorisées (adresse
-- normalisée : partie locale telle quelle, domaine en minuscules).
CREATE TABLE remote_image_senders (
    address TEXT PRIMARY KEY CHECK (length(address) BETWEEN 3 AND 320)
) STRICT, WITHOUT ROWID;

-- Réglages simples (clé, valeur).
CREATE TABLE settings (
    key   TEXT PRIMARY KEY CHECK (length(key) BETWEEN 1 AND 64),
    value TEXT NOT NULL CHECK (length(value) <= 1024)
) STRICT, WITHOUT ROWID;
