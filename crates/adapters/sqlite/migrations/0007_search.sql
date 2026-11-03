-- Version 7 : recherche plein texte (ADR-003).
--
-- message_texts porte le texte cherchable de chaque message : sujet,
-- expéditeur (nom et adresse), destinataires À et Cc (noms et adresses),
-- texte du corps (rempli quand le corps est téléchargé). message_search
-- est un index FTS5 à contenu externe sur cette table : le texte n'est
-- stocké qu'une fois.
--
-- Les deux sont tenus à jour par des déclencheurs, dans la transaction de
-- chaque écriture et quel que soit son chemin (synchronisation, copies
-- provisoires des actions, suppressions en cascade). Les messages ne sont
-- jamais remplacés par INSERT OR REPLACE (dont la suppression implicite
-- ne déclencherait rien) : ils sont supprimés puis réinsérés.

CREATE TABLE message_texts (
    message_id INTEGER PRIMARY KEY REFERENCES messages (id) ON DELETE CASCADE,
    subject    TEXT    NOT NULL,
    sender     TEXT    NOT NULL,
    recipients TEXT    NOT NULL,
    body       TEXT    NOT NULL DEFAULT ''
) STRICT;

CREATE VIRTUAL TABLE message_search USING fts5 (
    subject, sender, recipients, body,
    content = 'message_texts',
    content_rowid = 'message_id',
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3'
);

CREATE TRIGGER message_texts_after_insert AFTER INSERT ON message_texts BEGIN
    INSERT INTO message_search (rowid, subject, sender, recipients, body)
    VALUES (new.message_id, new.subject, new.sender, new.recipients, new.body);
END;

CREATE TRIGGER message_texts_after_delete AFTER DELETE ON message_texts BEGIN
    INSERT INTO message_search (message_search, rowid, subject, sender, recipients, body)
    VALUES ('delete', old.message_id, old.subject, old.sender, old.recipients, old.body);
END;

CREATE TRIGGER message_texts_after_update AFTER UPDATE ON message_texts BEGIN
    INSERT INTO message_search (message_search, rowid, subject, sender, recipients, body)
    VALUES ('delete', old.message_id, old.subject, old.sender, old.recipients, old.body);
    INSERT INTO message_search (rowid, subject, sender, recipients, body)
    VALUES (new.message_id, new.subject, new.sender, new.recipients, new.body);
END;

-- Noms et adresses d'une liste de correspondants sérialisée en JSON
-- ([{"name": …, "address": …}]), séparés par des espaces ; un JSON
-- invalide ou un élément qui n'est pas un objet est ignoré (base modifiée
-- à la main) plutôt que de faire échouer l'écriture.
CREATE TRIGGER messages_texts_after_insert AFTER INSERT ON messages BEGIN
    INSERT INTO message_texts (message_id, subject, sender, recipients)
    VALUES (
        new.id,
        new.subject,
        new.from_name || ' ' || new.from_address,
        coalesce((
            SELECT group_concat(
                coalesce(json_extract(p.value, '$.name'), '') || ' ' ||
                coalesce(json_extract(p.value, '$.address'), ''), ' ')
            FROM (SELECT value, type FROM json_each(
                      CASE WHEN json_valid(new.to_json) THEN new.to_json ELSE '[]' END)
                  UNION ALL
                  SELECT value, type FROM json_each(
                      CASE WHEN json_valid(new.cc_json) THEN new.cc_json ELSE '[]' END)) AS p
            WHERE p.type = 'object'
        ), '')
    );
END;

CREATE TRIGGER messages_texts_after_update
AFTER UPDATE OF subject, from_name, from_address, to_json, cc_json ON messages BEGIN
    UPDATE message_texts SET
        subject = new.subject,
        sender = new.from_name || ' ' || new.from_address,
        recipients = coalesce((
            SELECT group_concat(
                coalesce(json_extract(p.value, '$.name'), '') || ' ' ||
                coalesce(json_extract(p.value, '$.address'), ''), ' ')
            FROM (SELECT value, type FROM json_each(
                      CASE WHEN json_valid(new.to_json) THEN new.to_json ELSE '[]' END)
                  UNION ALL
                  SELECT value, type FROM json_each(
                      CASE WHEN json_valid(new.cc_json) THEN new.cc_json ELSE '[]' END)) AS p
            WHERE p.type = 'object'
        ), '')
    WHERE message_id = new.id;
END;

-- Messages déjà présents (base antérieure à la version 7).
INSERT INTO message_texts (message_id, subject, sender, recipients)
SELECT
    m.id,
    m.subject,
    m.from_name || ' ' || m.from_address,
    coalesce((
        SELECT group_concat(
            coalesce(json_extract(p.value, '$.name'), '') || ' ' ||
            coalesce(json_extract(p.value, '$.address'), ''), ' ')
        FROM (SELECT value, type FROM json_each(
                      CASE WHEN json_valid(m.to_json) THEN m.to_json ELSE '[]' END)
              UNION ALL
              SELECT value, type FROM json_each(
                      CASE WHEN json_valid(m.cc_json) THEN m.cc_json ELSE '[]' END)) AS p
            WHERE p.type = 'object'
    ), '')
FROM messages AS m;
