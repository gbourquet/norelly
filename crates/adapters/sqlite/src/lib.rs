//! Adaptateur SQLite (ADR-003) : base globale, migrations, dépôts.
//!
//! Phase 3 : comptes et exceptions de certificats. La connexion unique est
//! protégée par un verrou et utilisée depuis `spawn_blocking` ; le pool de
//! lecteurs et le rédacteur dédié arrivent avec la synchronisation
//! (phase 4).
//!
//! Les lignes relues sont revalidées par les constructeurs du domaine :
//! une base modifiée à la main ne peut pas produire un compte invalide.

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
use rusqlite::{Connection, Row, params};
use rusqlite_migration::{M, Migrations};

/// Migrations, dans l'ordre. Ne jamais modifier une migration publiée :
/// en ajouter une nouvelle.
fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(include_str!("../migrations/0001_accounts.sql"))])
}

/// Erreur d'ouverture de la base.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("ouverture de la base impossible : {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration de la base impossible : {0}")]
    Migration(#[from] rusqlite_migration::Error),
}

/// Base SQLite partagée par les dépôts.
#[derive(Debug, Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Ouvre (ou crée) la base, en mode WAL, et applique les migrations.
    pub fn open(path: &Path) -> Result<Self, OpenError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    /// Base en mémoire (tests).
    pub fn open_in_memory() -> Result<Self, OpenError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, OpenError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrations().to_latest(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Exécute `f` sur la connexion, hors du fil asynchrone.
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
}

fn store_error(e: rusqlite::Error) -> StoreError {
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

fn select_accounts(
    conn: &Connection,
    filter: &str,
    param: Option<&dyn rusqlite::ToSql>,
) -> Result<Vec<AccountRow>, StoreError> {
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
        let rows = self.db.run(|c| select_accounts(c, "", None)).await?;
        into_accounts(rows)
    }

    async fn get(&self, id: AccountId) -> Result<Option<Account>, AccountStoreError> {
        let rows = self
            .db
            .run(move |c| select_accounts(c, "WHERE id = ?1", Some(&id.get())))
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
