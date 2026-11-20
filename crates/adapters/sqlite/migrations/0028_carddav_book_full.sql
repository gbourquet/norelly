-- Bilan de la dernière synchronisation CardDAV : nouvelles fiches du
-- serveur qu'un carnet plein n'a pas pu recevoir (comptées aussi dans
-- `skipped`).
ALTER TABLE carddav_sync ADD COLUMN book_full INTEGER NOT NULL DEFAULT 0;
