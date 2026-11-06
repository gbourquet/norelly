-- Version 8 : boîte des copies distantes des brouillons.
--
-- Une copie distante (UIDVALIDITY, UID) n'a de sens que dans la boîte où
-- elle a été déposée. Jusqu'ici, la corbeille supprimait les anciennes
-- copies dans le dossier qui porte aujourd'hui le rôle \Drafts ; la boîte
-- est désormais enregistrée avec chaque copie.
--
-- Copies antérieures : rattachées au dossier qui porte le rôle \Drafts du
-- compte (celui où elles ont été déposées, sauf changement de rôle) ; sans
-- tel dossier, elles sont oubliées (au pire, une copie reste sur le
-- serveur, jamais un autre message supprimé).

ALTER TABLE drafts ADD COLUMN remote_mailbox TEXT;

UPDATE drafts SET remote_mailbox = (
    SELECT b.name FROM mailboxes b
    WHERE b.account_id = drafts.account_id AND b.role = 'drafts'
    ORDER BY b.id LIMIT 1
) WHERE remote_uid IS NOT NULL;

UPDATE drafts SET remote_validity = NULL, remote_uid = NULL
WHERE remote_uid IS NOT NULL AND remote_mailbox IS NULL;

CREATE TABLE draft_remote_trash_v8 (
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    mailbox    TEXT    NOT NULL,
    validity   INTEGER NOT NULL CHECK (validity BETWEEN 1 AND 4294967295),
    uid        INTEGER NOT NULL CHECK (uid BETWEEN 1 AND 4294967295),
    PRIMARY KEY (account_id, mailbox, validity, uid)
) WITHOUT ROWID;

INSERT INTO draft_remote_trash_v8 (account_id, mailbox, validity, uid)
SELECT t.account_id, b.name, t.validity, t.uid
FROM draft_remote_trash t
JOIN mailboxes b ON b.id = (
    SELECT id FROM mailboxes
    WHERE account_id = t.account_id AND role = 'drafts'
    ORDER BY id LIMIT 1
);

DROP TABLE draft_remote_trash;
ALTER TABLE draft_remote_trash_v8 RENAME TO draft_remote_trash;
