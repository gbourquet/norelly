-- Version 5 : copies provisoires des messages déplacés.
--
-- Un déplacement (déplacer, archiver, supprimer vers la Corbeille) crée
-- tout de suite, dans le dossier de destination, une copie locale du
-- message : l'utilisateur voit l'état final sans attendre le serveur.
-- messages.pending_op désigne l'opération qui l'a créée (NULL : message
-- réel). Sans clé étrangère : l'opération est retirée de la file dès le
-- rejeu, la copie seulement après la synchronisation de la destination,
-- qui apporte le vrai message.
--
-- La copie porte un UID synthétique tout en haut de l'espace des UID
-- (voir norelly_sqlite::actions) ; la synchronisation l'ignore.

ALTER TABLE messages ADD COLUMN pending_op INTEGER CHECK (pending_op IS NULL OR pending_op > 0);

CREATE INDEX messages_by_pending_op ON messages (pending_op) WHERE pending_op IS NOT NULL;
