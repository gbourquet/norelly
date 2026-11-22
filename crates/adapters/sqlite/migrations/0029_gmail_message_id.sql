-- Version 29 : identifiant Gmail des messages (ADR-016).
--
-- gmail_msgid : X-GM-MSGID, identifiant unique et stable d'un message Gmail,
-- le même dans tous les dossiers du compte ; NULL si le serveur n'annonce
-- pas X-GM-EXT-1 (ou pour un message d'avant cette version). L'entier non
-- signé de 64 bits est rangé sous le même motif binaire en entier signé.
-- Sert à recopier le corps d'un message déjà téléchargé dans un autre
-- dossier du même compte, au lieu de le télécharger à nouveau.
ALTER TABLE messages ADD COLUMN gmail_msgid INTEGER;

CREATE INDEX messages_by_gmail_msgid ON messages (gmail_msgid)
    WHERE gmail_msgid IS NOT NULL;
