-- Version 20 : authentification de l'expéditeur d'un message.
--
-- Verdict du domaine (norelly_domain::sender_auth::SenderAuth) tiré de
-- l'en-tête Authentication-Results posé par le serveur du compte, à la
-- réception : 'pass', 'fail' ou 'unknown'. Les messages déjà enregistrés
-- valent 'unknown' (pas de rattrapage depuis les corps stockés).

ALTER TABLE messages ADD COLUMN sender_auth TEXT NOT NULL DEFAULT 'unknown'
    CHECK (sender_auth IN ('pass', 'fail', 'unknown'));
