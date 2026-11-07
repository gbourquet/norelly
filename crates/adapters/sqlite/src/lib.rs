//! Adaptateur SQLite (ADR-003) : base globale, migrations, dépôts.
//!
//! - comptes et exceptions de certificats ;
//! - boîtes, messages et état de synchronisation ([`mail`]) : `MailStore`
//!   côté écriture, `FolderTreeQuery` et `MessageListQuery` côté lecture ;
//! - carnet d'adresses ([`contacts`]) : `ContactBook` et `ContactDirectory`.
//!
//! Mode WAL : un seul rédacteur (connexion protégée par un verrou) et un
//! pool de lecteurs en lecture seule, utilisés depuis `spawn_blocking`.
//! Une base en mémoire (tests) n'a pas de lecteurs dédiés : tout passe par
//! le rédacteur.
//!
//! Les lignes relues sont revalidées par les constructeurs du domaine :
//! une base modifiée à la main ne peut pas produire un compte invalide.

pub mod actions;
pub mod compose;
pub mod contacts;
pub mod conversations;
pub mod mail;
pub mod preferences;
pub mod search;

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use norelly_domain::account::{Account, AccountDraft, AccountId, DisplayName, ServerSettings};
use norelly_domain::address::EmailAddress;
use norelly_domain::credentials::Username;
use norelly_domain::endpoint::{Host, Protocol, Security, ServerEndpoint};
use norelly_domain::ports::{
    AccountRepository, AccountStoreError, CertificateExceptionStore, StoreError,
};
use norelly_domain::security::{CertificateException, CertificateFingerprint};
use rusqlite::{Connection, OpenFlags, Row, params};
use rusqlite_migration::{M, Migrations};

/// Migrations, dans l'ordre. Ne jamais modifier une migration publiée :
/// en ajouter une nouvelle.
fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(include_str!("../migrations/0001_accounts.sql")),
        M::up(include_str!("../migrations/0002_mail.sql")),
        M::up(include_str!("../migrations/0003_conversations.sql")),
        M::up(include_str!("../migrations/0004_actions.sql")),
        M::up(include_str!("../migrations/0005_placeholder_copies.sql")),
        M::up(include_str!("../migrations/0006_compose.sql")),
        M::up(include_str!("../migrations/0007_search.sql")),
        M::up(include_str!("../migrations/0008_remote_copy_mailbox.sql")),
        M::up(include_str!("../migrations/0009_folder_counts.sql")),
        M::up(include_str!("../migrations/0010_scheduled_send.sql")),
        M::up(include_str!("../migrations/0011_contacts.sql")),
    ])
}

/// Nombre de connexions de lecture d'une base sur disque.
pub const READERS: usize = 4;

/// Pool de connexions en lecture seule.
#[derive(Debug)]
struct Readers {
    conns: Mutex<Vec<Connection>>,
    permits: tokio::sync::Semaphore,
}

/// Erreur d'ouverture de la base.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("ouverture de la base impossible : {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration de la base impossible : {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("rattachement des conversations impossible : {0}")]
    Backfill(String),
}

/// Base SQLite partagée par les dépôts.
#[derive(Debug, Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
    readers: Option<Arc<Readers>>,
}

impl Database {
    /// Ouvre (ou crée) la base, en mode WAL, applique les migrations et
    /// ouvre [`READERS`] connexions de lecture.
    pub fn open(path: &Path) -> Result<Self, OpenError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let mut db = Self::init(conn)?;
        let mut readers = Vec::with_capacity(READERS);
        for _ in 0..READERS {
            let reader = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            reader.pragma_update(None, "query_only", "ON")?;
            reader.busy_timeout(std::time::Duration::from_secs(5))?;
            readers.push(reader);
        }
        db.readers = Some(Arc::new(Readers {
            conns: Mutex::new(readers),
            permits: tokio::sync::Semaphore::new(READERS),
        }));
        Ok(db)
    }

    /// Base en mémoire (tests).
    pub fn open_in_memory() -> Result<Self, OpenError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, OpenError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrations().to_latest(&mut conn)?;
        // Messages d'une base antérieure à la version 3 : rattachés à leur
        // conversation une fois pour toutes.
        conversations::backfill(&mut conn).map_err(|e| OpenError::Backfill(e.0))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            readers: None,
        })
    }

    /// Exécute `f` sur la connexion du rédacteur, hors du fil asynchrone.
    async fn run<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().unwrap_or_else(PoisonError::into_inner);
            f(&mut guard)
        })
        .await
        .map_err(|e| StoreError(format!("tâche SQLite interrompue : {e}")))?
    }

    /// Contrôle d'intégrité : `PRAGMA integrity_check`,
    /// `foreign_key_check` et cohérence de l'index de recherche. Renvoie les problèmes trouvés (vide si la base
    /// est saine).
    pub async fn integrity_problems(&self) -> Result<Vec<String>, StoreError> {
        self.run(|c| {
            let mut problems = Vec::new();
            let mut stmt = c.prepare("PRAGMA integrity_check").map_err(store_error)?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(store_error)?;
            for row in rows {
                let row = row.map_err(store_error)?;
                if row != "ok" {
                    problems.push(format!("intégrité : {row}"));
                }
            }
            let mut stmt = c.prepare("PRAGMA foreign_key_check").map_err(store_error)?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(store_error)?;
            for table in rows {
                problems.push(format!(
                    "clé étrangère orpheline dans {}",
                    table.map_err(store_error)?
                ));
            }
            // Index de recherche comparé à sa table de contenu (rang 1).
            if let Err(e) = c.execute(
                "INSERT INTO message_search (message_search, rank) VALUES ('integrity-check', 1)",
                [],
            ) {
                problems.push(format!("index de recherche : {e}"));
            }
            Ok(problems)
        })
        .await
    }

    /// Exécute un texte SQL arbitraire sur le rédacteur (tests seulement :
    /// simuler une base modifiée à la main).
    #[cfg(test)]
    pub(crate) async fn write_for_tests(&self, sql: &'static str) -> Result<(), StoreError> {
        self.run(move |c| c.execute_batch(sql).map_err(store_error))
            .await
    }

    /// Lecture arbitraire (tests seulement).
    #[cfg(test)]
    pub(crate) async fn read_for_tests<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, StoreError> + Send + 'static,
    {
        self.read(f).await
    }

    /// Schéma de la base (type, nom, SQL de création), trié : permet de
    /// vérifier qu'aucune donnée reçue ne l'a modifié.
    pub async fn schema(&self) -> Result<Vec<(String, String, Option<String>)>, StoreError> {
        self.run(|c| {
            let mut stmt = c
                .prepare("SELECT type, name, sql FROM sqlite_schema ORDER BY type, name")
                .map_err(store_error)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(store_error)?;
            rows.collect::<Result<_, _>>().map_err(store_error)
        })
        .await
    }

    /// Exécute une lecture sur une connexion du pool (le rédacteur s'il
    /// n'y en a pas). Les lecteurs voient le dernier état validé (WAL).
    async fn read<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, StoreError> + Send + 'static,
    {
        let Some(readers) = &self.readers else {
            return self.run(move |c| f(c)).await;
        };
        let _permit = readers
            .permits
            .acquire()
            .await
            .map_err(|e| StoreError(format!("pool de lecture fermé : {e}")))?;
        let taken = readers
            .conns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        // Connexion perdue (tâche interrompue) : repli sur le rédacteur.
        let Some(conn) = taken else {
            return self.run(move |c| f(c)).await;
        };
        let pool = Arc::clone(readers);
        tokio::task::spawn_blocking(move || {
            let result = f(&conn);
            pool.conns
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(conn);
            result
        })
        .await
        .map_err(|e| StoreError(format!("tâche SQLite interrompue : {e}")))?
    }
}

pub(crate) fn store_error(e: rusqlite::Error) -> StoreError {
    StoreError(e.to_string())
}

const fn security_str(security: Security) -> &'static str {
    match security {
        Security::Tls => "tls",
        Security::StartTls => "starttls",
        Security::Plain => "plain",
    }
}

fn parse_security(raw: &str) -> Result<Security, String> {
    match raw {
        "tls" => Ok(Security::Tls),
        "starttls" => Ok(Security::StartTls),
        "plain" => Ok(Security::Plain),
        other => Err(format!("sécurité inconnue : {other}")),
    }
}

// ---------------------------------------------------------------------------
// Comptes
// ---------------------------------------------------------------------------

/// Dépôt de comptes SQLite.
#[derive(Debug, Clone)]
pub struct SqliteAccountRepository {
    db: Database,
}

impl SqliteAccountRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

const ACCOUNT_COLUMNS: &str = "id, address, display_name, \
     imap_host, imap_port, imap_security, imap_username, \
     smtp_host, smtp_port, smtp_security, smtp_username";

/// Ligne brute, validée ensuite par le domaine.
struct AccountRow {
    id: i64,
    address: String,
    display_name: String,
    servers: [(String, i64, String, String); 2],
}

impl AccountRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            address: row.get(1)?,
            display_name: row.get(2)?,
            servers: [
                (row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?),
                (row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?),
            ],
        })
    }

    fn into_account(self) -> Result<Account, AccountStoreError> {
        let corrupted = |what: &str, e: &dyn std::fmt::Display| {
            AccountStoreError::Corrupted(format!("compte {} : {what} : {e}", self.id))
        };
        let id = AccountId::new(self.id).map_err(|e| corrupted("identifiant", &e))?;
        let address = EmailAddress::parse(&self.address).map_err(|e| corrupted("adresse", &e))?;
        let name = DisplayName::parse(&self.display_name).map_err(|e| corrupted("nom", &e))?;
        let [imap, smtp] = [Protocol::Imap, Protocol::Smtp];
        let mut settings = Vec::with_capacity(2);
        for (protocol, (host, port, security, username)) in
            [imap, smtp].into_iter().zip(&self.servers)
        {
            let host = Host::parse(host).map_err(|e| corrupted("hôte", &e))?;
            let port = u16::try_from(*port).map_err(|e| corrupted("port", &e))?;
            let security = parse_security(security).map_err(|e| corrupted("sécurité", &e))?;
            let endpoint =
                ServerEndpoint::new(host, port, security).map_err(|e| corrupted("serveur", &e))?;
            let username = Username::parse(username).map_err(|e| corrupted("utilisateur", &e))?;
            settings.push(ServerSettings {
                protocol,
                endpoint,
                username,
            });
        }
        let (Some(smtp), Some(imap)) = (settings.pop(), settings.pop()) else {
            return Err(corrupted("serveurs", &"incomplets"));
        };
        let draft = AccountDraft::restore(name, address, imap, smtp)
            .map_err(|e| corrupted("compte", &e))?;
        Ok(draft.with_id(id))
    }
}

/// `filter` est un fragment SQL littéral du code (`&'static str` : aucune
/// donnée reçue ne peut y parvenir) ; la valeur cherchée passe par `param`,
/// lié par SQLite.
fn select_accounts(
    conn: &Connection,
    filter: &'static str,
    param: Option<&dyn rusqlite::ToSql>,
) -> Result<Vec<AccountRow>, StoreError> {
    // sql-const : colonnes et filtre sont des constantes du code.
    let sql = format!("SELECT {ACCOUNT_COLUMNS} FROM accounts {filter} ORDER BY id");
    let mut stmt = conn.prepare(&sql).map_err(store_error)?;
    let rows = match param {
        Some(p) => stmt.query_map([p], AccountRow::read),
        None => stmt.query_map([], AccountRow::read),
    }
    .map_err(store_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(store_error)
}

fn into_accounts(rows: Vec<AccountRow>) -> Result<Vec<Account>, AccountStoreError> {
    rows.into_iter().map(AccountRow::into_account).collect()
}

impl AccountRepository for SqliteAccountRepository {
    async fn list(&self) -> Result<Vec<Account>, AccountStoreError> {
        // Pool de lecteurs : pas d'attente derrière une écriture en cours.
        let rows = self.db.read(|c| select_accounts(c, "", None)).await?;
        into_accounts(rows)
    }

    async fn get(&self, id: AccountId) -> Result<Option<Account>, AccountStoreError> {
        let rows = self
            .db
            .read(move |c| select_accounts(c, "WHERE id = ?1", Some(&id.get())))
            .await?;
        Ok(into_accounts(rows)?.into_iter().next())
    }

    async fn find_by_address(
        &self,
        address: &EmailAddress,
    ) -> Result<Option<Account>, AccountStoreError> {
        let address = address.to_string();
        let rows = self
            .db
            .run(move |c| select_accounts(c, "WHERE address = ?1", Some(&address)))
            .await?;
        Ok(into_accounts(rows)?.into_iter().next())
    }

    async fn insert(&self, draft: &AccountDraft) -> Result<Account, AccountStoreError> {
        let owned = draft.clone();
        let result = self
            .db
            .run(move |c| {
                let d = &owned;
                let (imap, smtp) = (d.imap(), d.smtp());
                let inserted = c.execute(
                    "INSERT INTO accounts (address, display_name, \
                     imap_host, imap_port, imap_security, imap_username, \
                     smtp_host, smtp_port, smtp_security, smtp_username) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
                     ON CONFLICT (address) DO NOTHING",
                    params![
                        d.address().to_string(),
                        d.display_name().as_str(),
                        imap.endpoint.host().as_str(),
                        imap.endpoint.port(),
                        security_str(imap.endpoint.security()),
                        imap.username.as_str(),
                        smtp.endpoint.host().as_str(),
                        smtp.endpoint.port(),
                        security_str(smtp.endpoint.security()),
                        smtp.username.as_str(),
                    ],
                );
                match inserted.map_err(store_error)? {
                    0 => Ok(None),
                    _ => Ok(Some(c.last_insert_rowid())),
                }
            })
            .await?;
        let Some(raw_id) = result else {
            return Err(AccountStoreError::Duplicate(draft.address().clone()));
        };
        let id = AccountId::new(raw_id)
            .map_err(|e| AccountStoreError::Corrupted(format!("identifiant {raw_id} : {e}")))?;
        Ok(draft.clone().with_id(id))
    }

    async fn delete(&self, id: AccountId) -> Result<bool, AccountStoreError> {
        let deleted = self
            .db
            .run(move |c| {
                c.execute("DELETE FROM accounts WHERE id = ?1", [id.get()])
                    .map_err(store_error)
            })
            .await?;
        Ok(deleted > 0)
    }
}

// ---------------------------------------------------------------------------
// Exceptions de certificats
// ---------------------------------------------------------------------------

/// Exceptions de certificats en base.
#[derive(Debug, Clone)]
pub struct SqliteCertificateExceptionStore {
    db: Database,
}

impl SqliteCertificateExceptionStore {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

impl CertificateExceptionStore for SqliteCertificateExceptionStore {
    async fn exceptions_for(&self, host: &Host) -> Result<Vec<CertificateException>, StoreError> {
        let host = host.clone();
        self.db
            .run(move |c| {
                let mut stmt = c
                    .prepare(
                        "SELECT fingerprint FROM certificate_exceptions \
                         WHERE host = ?1 ORDER BY created_at, fingerprint",
                    )
                    .map_err(store_error)?;
                let rows = stmt
                    .query_map([host.as_str()], |r| r.get::<_, String>(0))
                    .map_err(store_error)?;
                let mut out = Vec::new();
                for raw in rows {
                    let raw = raw.map_err(store_error)?;
                    // Une empreinte illisible n'accorde aucune confiance.
                    if let Ok(fingerprint) = CertificateFingerprint::parse_hex(&raw) {
                        out.push(CertificateException {
                            host: host.clone(),
                            fingerprint,
                        });
                    }
                }
                Ok(out)
            })
            .await
    }

    async fn add(&self, exception: &CertificateException) -> Result<(), StoreError> {
        let (host, fp) = (exception.host.clone(), exception.fingerprint.to_string());
        self.db
            .run(move |c| {
                c.execute(
                    "INSERT OR IGNORE INTO certificate_exceptions (host, fingerprint) \
                     VALUES (?1, ?2)",
                    params![host.as_str(), fp],
                )
                .map(|_| ())
                .map_err(store_error)
            })
            .await
    }

    async fn remove(&self, exception: &CertificateException) -> Result<(), StoreError> {
        let (host, fp) = (exception.host.clone(), exception.fingerprint.to_string());
        self.db
            .run(move |c| {
                c.execute(
                    "DELETE FROM certificate_exceptions WHERE host = ?1 AND fingerprint = ?2",
                    params![host.as_str(), fp],
                )
                .map(|_| ())
                .map_err(store_error)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use norelly_test_support::contracts::{
        account_repository_contract, certificate_exception_store_contract, sample_account,
    };

    use super::*;

    fn db() -> Database {
        Database::open_in_memory().unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn migrations_are_valid() {
        assert!(migrations().validate().is_ok());
    }

    /// Données représentatives de chaque version du schéma, insérées juste
    /// après la migration vers cette version (index 0 : version 1).
    const FIXTURES: &[&str] = &[
        // Version 1 : un compte et une exception de certificat.
        "INSERT INTO accounts (address, display_name, imap_host, imap_port, imap_security, \
         imap_username, smtp_host, smtp_port, smtp_security, smtp_username) \
         VALUES ('v1@example.com', 'V1', 'imap.example.com', 993, 'tls', 'v1', \
         'smtp.example.com', 465, 'tls', 'v1'); \
         INSERT INTO certificate_exceptions (host, fingerprint) VALUES ('imap.example.com', 'AB');",
        // Version 2 : une boîte et un message.
        "INSERT INTO mailboxes (account_id, name, role, selectable, uid_validity) \
         VALUES (1, 'INBOX', 'inbox', 1, 42); \
         INSERT INTO messages (mailbox_id, uid, internal_date, sort_date, size, subject, \
         from_name, from_address, has_attachments) \
         VALUES (1, 7, 100, 100, 10, 'v2', '', 'a@example.com', 0);",
        // Version 3 : une conversation, un lien, des préférences.
        "INSERT INTO conversations (account_id, subject_key, latest_date) VALUES (1, 'v3', 100); \
         INSERT INTO conversation_links (account_id, message_id, conversation_id) \
         VALUES (1, 'v3@example.com', 1); \
         UPDATE messages SET conversation_id = 1; \
         INSERT INTO remote_image_senders (address) VALUES ('a@example.com'); \
         INSERT INTO settings (key, value) VALUES ('reading.prefer_plain_text', '1');",
        // Version 4 : un message masqué, une opération en attente, un
        // dossier de repli d'archivage.
        "UPDATE messages SET hidden = 1; \
         INSERT INTO pending_ops (account_id, message_id, mailbox, uid_validity, uid, kind, \
         target, not_before) VALUES (1, 1, 'INBOX', 42, 7, 'move', 'Archives', 0); \
         INSERT INTO action_settings (account_id, archive_mailbox) VALUES (1, 1);",
        // Version 5 : une copie provisoire (message déplacé, pas encore
        // synchronisé dans la destination).
        "INSERT INTO messages (mailbox_id, uid, internal_date, sort_date, size, subject, \
         from_name, from_address, has_attachments, pending_op) \
         VALUES (1, 4294967294, 100, 100, 10, 'v5', '', 'a@example.com', 0, 1);",
        // Version 6 : un brouillon copié sur le serveur, une copie à
        // supprimer, un message en file d'envoi.
        "INSERT INTO drafts (account_id, content, revision, remote_revision, remote_validity, \
         remote_uid, last_remote_save, updated_at) VALUES (1, '{}', 2, 1, 9, 3, 50, 60); \
         INSERT INTO draft_remote_trash (account_id, validity, uid) VALUES (1, 9, 2); \
         INSERT INTO outbox (account_id, envelope_from, recipients, raw, sent_copy, subject, \
         state, not_before, attempts, queued_at) \
         VALUES (1, 'v6@example.com', '[]', x'00', x'00', 'v6', 'queued', 0, 0, 70);",
        // Version 7 : le texte d'un message connu.
        "UPDATE message_texts SET body = 'texte v7' WHERE message_id = 1;",
        // Version 8 : une copie distante rattachée à sa boîte.
        "UPDATE drafts SET remote_mailbox = 'INBOX' WHERE id = 1;",
        // Version 9 : index seulement, un message marqué lu.
        "UPDATE messages SET flags = 1 WHERE id = 1;",
        // Version 10 : un message programmé avec son brouillon.
        "UPDATE outbox SET draft_content = '{}' WHERE id = 1;",
        // Version 11 : un contact, son adresse et son téléphone.
        "INSERT INTO contacts (display_name, favorite, sort_key, search_text) \
         VALUES ('V11', 1, 'v11', 'v11\nv11@example.com'); \
         INSERT INTO contact_emails (contact_id, position, address, address_key, label, preferred) \
         VALUES (1, 0, 'V11@example.com', 'v11@example.com', NULL, 1); \
         INSERT INTO contact_phones (contact_id, position, number, label) \
         VALUES (1, 0, '01 02 03', 'maison');",
    ];

    /// Montée de version depuis chaque version précédente, données
    /// conservées.
    #[test]
    fn upgrade_from_every_previous_version_keeps_data() {
        let latest = FIXTURES.len();
        // Une fixture par migration : il n'existe pas de version au-delà.
        let mut fresh = Connection::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        assert!(migrations().to_version(&mut fresh, latest).is_ok());
        assert!(
            migrations().to_version(&mut fresh, latest + 1).is_err(),
            "migration sans fixture dans FIXTURES"
        );
        for start in 0..latest {
            let mut conn = Connection::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
            conn.pragma_update(None, "foreign_keys", "ON")
                .unwrap_or_else(|e| panic!("{e}"));
            let m = migrations();
            for (version, fixture) in FIXTURES.iter().enumerate().take(start + 1) {
                m.to_version(&mut conn, version + 1)
                    .unwrap_or_else(|e| panic!("vers v{} : {e}", version + 1));
                conn.execute_batch(fixture)
                    .unwrap_or_else(|e| panic!("fixture v{} : {e}", version + 1));
            }
            m.to_latest(&mut conn)
                .unwrap_or_else(|e| panic!("depuis v{} : {e}", start + 1));
            let accounts: i64 = conn
                .query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(accounts, 1, "depuis v{}", start + 1);
            // Le schéma le plus récent est utilisable.
            conn.execute_batch(
                "INSERT OR IGNORE INTO mailboxes (account_id, name, role, selectable) \
                 VALUES (1, 'Sent', 'sent', 1);",
            )
            .unwrap_or_else(|e| panic!("depuis v{} : {e}", start + 1));
        }
    }

    /// Base de version 7 : à la montée en version 8, les copies distantes
    /// des brouillons sont rattachées au dossier « Brouillons » du compte ;
    /// sans tel dossier, elles sont oubliées.
    #[tokio::test]
    async fn upgrading_to_v8_attaches_remote_copies_to_their_mailbox() -> Result<(), StoreError> {
        use norelly_domain::ports::DraftStore;
        let mut conn = Connection::open_in_memory().map_err(store_error)?;
        migrations()
            .to_version(&mut conn, 7)
            .map_err(|e| StoreError(e.to_string()))?;
        for fixture in FIXTURES.iter().take(7) {
            conn.execute_batch(fixture).map_err(store_error)?;
        }
        conn.execute_batch(
            "INSERT INTO accounts (address, display_name, imap_host, imap_port, imap_security, \
             imap_username, smtp_host, smtp_port, smtp_security, smtp_username) \
             VALUES ('v8@example.com', 'V8', 'imap.example.com', 993, 'tls', 'v8', \
             'smtp.example.com', 465, 'tls', 'v8'); \
             INSERT INTO mailboxes (account_id, name, role, selectable) \
             VALUES (2, 'Brouillons', 'drafts', 1); \
             INSERT INTO drafts (account_id, content, revision, remote_revision, remote_validity, \
             remote_uid, last_remote_save, updated_at) VALUES (2, '{\"to\":[],\"cc\":[],\"bcc\":[],\
             \"subject\":\"\",\"text\":\"\",\"html\":null,\"in_reply_to\":null,\"references\":[],\
             \"attachments\":[]}', 1, 1, 9, 4, 50, 60); \
             INSERT INTO draft_remote_trash (account_id, validity, uid) VALUES (2, 9, 3);",
        )
        .map_err(store_error)?;
        migrations()
            .to_latest(&mut conn)
            .map_err(|e| StoreError(e.to_string()))?;
        // Compte 1 : aucun dossier « Brouillons », copie oubliée.
        let forgotten: Option<i64> = conn
            .query_row(
                "SELECT remote_uid FROM drafts WHERE account_id = 1",
                [],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        assert_eq!(forgotten, None);
        let db = Database {
            conn: Arc::new(Mutex::new(conn)),
            readers: None,
        };
        let store = compose::SqliteComposeStore::new(db);
        let two = AccountId::new(2).map_err(|e| StoreError(e.to_string()))?;
        let one = AccountId::new(1).map_err(|e| StoreError(e.to_string()))?;
        let drafts = store.drafts(two).await?;
        let copy = drafts
            .first()
            .and_then(|d| d.sync.remote.clone())
            .ok_or_else(|| StoreError("copie perdue".into()))?;
        assert_eq!((copy.mailbox.as_str(), copy.uid.get()), ("Brouillons", 4));
        let trash = store.remote_trash(two).await?;
        assert_eq!(
            trash
                .iter()
                .map(|c| (c.mailbox.as_str(), c.uid.get()))
                .collect::<Vec<_>>(),
            [("Brouillons", 3)]
        );
        assert!(store.remote_trash(one).await?.is_empty());
        Ok(())
    }

    /// Base de version 6 : à la montée en version 7, les messages
    /// existants sont indexés, même avec des destinataires illisibles.
    #[tokio::test]
    async fn upgrading_to_v7_indexes_existing_messages() -> Result<(), StoreError> {
        use norelly_domain::ports::{SearchIndex, SearchRequest, SearchScope};
        let dir = std::env::temp_dir().join(format!("norelly-v7-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| StoreError(e.to_string()))?;
        let path = dir.join("v6.sqlite3");
        {
            let mut conn = Connection::open(&path).map_err(store_error)?;
            migrations()
                .to_version(&mut conn, 6)
                .map_err(|e| StoreError(e.to_string()))?;
            for fixture in FIXTURES.iter().take(6) {
                conn.execute_batch(fixture).map_err(store_error)?;
            }
            conn.execute_batch(
                "INSERT INTO messages (mailbox_id, uid, internal_date, sort_date, size, subject, \
                 from_name, from_address, has_attachments, to_json, cc_json) VALUES \
                 (1, 8, 300, 300, 10, 'Ancien rapport', 'Zoé', 'zoe@example.com', 0, \
                  '[{\"name\":\"Yves\",\"address\":\"yves@example.com\"}]', 'pas du json'), \
                 (1, 9, 200, 200, 10, 'Autre', '', 'x@example.com', 0, '[\"texte\", 3]', '[]');",
            )
            .map_err(store_error)?;
        }
        let db = Database::open(&path).map_err(|e| StoreError(e.to_string()))?;
        let store = mail::SqliteMailStore::new(db);
        let search = |text: &str| SearchRequest {
            query: norelly_domain::search::parse_query(text).unwrap_or_default(),
            scope: SearchScope::All,
            utc_offset_seconds: 0,
            offset: 0,
            limit: 10,
        };
        for (text, expected) in [
            ("rapport", vec!["Ancien rapport"]),
            ("from:zoe", vec!["Ancien rapport"]),
            ("to:yves", vec!["Ancien rapport"]),
            ("autre", vec!["Autre"]),
        ] {
            let page = store.search(search(text)).await?;
            let subjects: Vec<String> = page.items.into_iter().map(|h| h.subject).collect();
            assert_eq!(subjects, expected, "{text}");
        }
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// Base de version 2 (messages sans conversation) : à l'ouverture, les
    /// messages sont rattachés (réponse avant l'original comprise).
    #[tokio::test]
    async fn opening_a_v2_database_threads_existing_messages() -> Result<(), StoreError> {
        let dir = std::env::temp_dir().join(format!("norelly-backfill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| StoreError(e.to_string()))?;
        let path = dir.join("v2.sqlite3");
        {
            let mut conn = Connection::open(&path).map_err(store_error)?;
            migrations()
                .to_version(&mut conn, 2)
                .map_err(|e| StoreError(e.to_string()))?;
            for fixture in FIXTURES.iter().take(2) {
                conn.execute_batch(fixture).map_err(store_error)?;
            }
            conn.execute_batch(
                "INSERT INTO messages (mailbox_id, uid, internal_date, sort_date, size, subject, \
                 from_name, from_address, has_attachments, message_id, refs) VALUES \
                 (1, 8, 300, 300, 10, 'Re: v2', '', 'b@example.com', 0, 'r@x', '[\"o@x\"]'), \
                 (1, 9, 200, 200, 10, 'v2', '', 'c@example.com', 0, 'o@x', '[]');",
            )
            .map_err(store_error)?;
        }
        let db = Database::open(&path).map_err(|e| StoreError(e.to_string()))?;
        let store = mail::SqliteMailStore::new(db);
        let inbox =
            norelly_domain::mailbox::MailboxId::new(1).ok_or_else(|| StoreError("id".into()))?;
        let page = norelly_domain::ports::ConversationQuery::conversations(
            &store,
            norelly_domain::conversation::ConversationScope::Mailbox(inbox),
            None,
            10,
        )
        .await?;
        let counts: Vec<(i64, u32)> = page.items.iter().map(|i| (i.date, i.messages)).collect();
        assert_eq!(counts, [(300, 2), (100, 1)]);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[tokio::test]
    async fn account_repository_passes_contract() -> Result<(), AccountStoreError> {
        account_repository_contract(&SqliteAccountRepository::new(db())).await
    }

    #[tokio::test]
    async fn exception_store_passes_contract() -> Result<(), StoreError> {
        certificate_exception_store_contract(&SqliteCertificateExceptionStore::new(db())).await
    }

    #[tokio::test]
    async fn file_database_persists_across_reopen() -> Result<(), AccountStoreError> {
        let dir = std::env::temp_dir().join(format!("norelly-sqlite-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.sqlite3");
        let _ = std::fs::remove_file(&path);
        let stored = {
            let repo = SqliteAccountRepository::new(
                Database::open(&path).unwrap_or_else(|e| panic!("{e}")),
            );
            repo.insert(&sample_account("alice@example.com", true))
                .await?
        };
        let reopened =
            SqliteAccountRepository::new(Database::open(&path).unwrap_or_else(|e| panic!("{e}")));
        assert_eq!(reopened.list().await?, vec![stored]);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// Une ligne modifiée à la main (injection dans l'hôte) n'est pas
    /// acceptée silencieusement.
    /// La lecture des comptes n'attend pas
    /// une écriture en cours (synchronisation qui démarre en même temps).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn account_reads_do_not_wait_for_the_writer() -> Result<(), AccountStoreError> {
        let dir = std::env::temp_dir().join(format!("norelly-readers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| StoreError(e.to_string()))?;
        let db = Database::open(&dir.join("t.sqlite3")).map_err(|e| StoreError(e.to_string()))?;
        let repo = SqliteAccountRepository::new(db.clone());
        let stored = repo
            .insert(&sample_account("alice@example.com", false))
            .await?;
        let busy = db.clone();
        let writer = tokio::spawn(async move {
            busy.run(|_| {
                std::thread::sleep(std::time::Duration::from_millis(500));
                Ok(())
            })
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let started = std::time::Instant::now();
        assert_eq!(repo.list().await?.len(), 1);
        assert!(repo.get(stored.id()).await?.is_some());
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(200),
            "{elapsed:?}"
        );
        let _ = writer.await;
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[tokio::test]
    async fn corrupted_rows_are_reported() -> Result<(), AccountStoreError> {
        let db = db();
        let repo = SqliteAccountRepository::new(db.clone());
        let stored = repo
            .insert(&sample_account("alice@example.com", false))
            .await?;
        db.run(|c| {
            c.execute(
                "UPDATE accounts SET imap_host = 'imap.example.com\r\nA1 LOGIN'",
                [],
            )
            .map(|_| ())
            .map_err(store_error)
        })
        .await?;
        assert!(matches!(
            repo.get(stored.id()).await,
            Err(AccountStoreError::Corrupted(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn schema_rejects_invalid_ports_and_security() {
        let db = db();
        let result = db
            .run(|c| {
                c.execute(
                    "INSERT INTO accounts (address, display_name, imap_host, imap_port, \
                     imap_security, imap_username, smtp_host, smtp_port, smtp_security, \
                     smtp_username) VALUES ('a@b.c', 'a', 'h', 0, 'tls', 'u', 'h', 25, 'none', 'u')",
                    [],
                )
                .map_err(store_error)
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn unreadable_fingerprint_grants_nothing() -> Result<(), StoreError> {
        let db = db();
        db.run(|c| {
            c.execute(
                "INSERT INTO certificate_exceptions (host, fingerprint) VALUES ('a.test', 'zz')",
                [],
            )
            .map(|_| ())
            .map_err(store_error)
        })
        .await?;
        let store = SqliteCertificateExceptionStore::new(db);
        let host = Host::parse("a.test").map_err(|e| StoreError(e.to_string()))?;
        assert!(store.exceptions_for(&host).await?.is_empty());
        Ok(())
    }
}
