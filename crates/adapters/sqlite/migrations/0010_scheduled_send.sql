-- Version 10 : envoi différé (phase 9).
--
-- Un message programmé garde le contenu de son brouillon (même document
-- JSON que drafts.content, revalidé à la relecture) : avant l'échéance,
-- l'utilisateur peut le reprendre pour le modifier. NULL : message mis en
-- file sans échéance.

ALTER TABLE outbox ADD COLUMN draft_content TEXT;
