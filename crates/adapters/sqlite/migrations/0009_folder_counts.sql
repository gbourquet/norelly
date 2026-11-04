-- Version 9 : compteurs de l'arbre des dossiers (phase 9, mesure de
-- démarrage).
--
-- Le total et le nombre de non lus de chaque dossier se comptent sur
-- (mailbox_id, hidden, flags) : avec cet index couvrant, SQLite ne lit
-- plus la table des messages (50 000 lignes par dossier dans la mesure).

CREATE INDEX messages_counts ON messages (mailbox_id, hidden, flags);
