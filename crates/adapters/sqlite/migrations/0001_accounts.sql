-- Version 1 : comptes et exceptions de certificats.
-- Aucun secret en base : les mots de passe sont dans le trousseau du système.

CREATE TABLE accounts (
    id            INTEGER PRIMARY KEY,
    address       TEXT    NOT NULL UNIQUE,
    display_name  TEXT    NOT NULL,
    imap_host     TEXT    NOT NULL,
    imap_port     INTEGER NOT NULL CHECK (imap_port BETWEEN 1 AND 65535),
    imap_security TEXT    NOT NULL CHECK (imap_security IN ('tls', 'starttls', 'plain')),
    imap_username TEXT    NOT NULL,
    smtp_host     TEXT    NOT NULL,
    smtp_port     INTEGER NOT NULL CHECK (smtp_port BETWEEN 1 AND 65535),
    smtp_security TEXT    NOT NULL CHECK (smtp_security IN ('tls', 'starttls', 'plain')),
    smtp_username TEXT    NOT NULL,
    created_at    INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

-- Exceptions de certificats acceptées par l'utilisateur (hôte + empreinte
-- SHA-256 du certificat feuille, en hexadécimal « AB:CD:… »).
CREATE TABLE certificate_exceptions (
    host        TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (host, fingerprint)
) STRICT;
