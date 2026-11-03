//! Outils communs aux tests d'intégration de la synchronisation.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use norelly_application::sync::{SyncAccount, SyncFailure};
use norelly_blobs::FsBlobStore;
use norelly_domain::account::AccountId;
use norelly_domain::credentials::{Credentials, Secret, Username};
use norelly_domain::endpoint::{Host, Security, ServerEndpoint};
use norelly_domain::fetch_policy::FetchPolicy;
use norelly_domain::mailbox::{Flags, MailboxId, MailboxName, Uid};
use norelly_domain::ports::{
    AccountRepository, BlobStore, ConnectRequest, FolderTreeQuery, MailStore, PendingOpStore,
    RemoteMailboxConnector, RemoteMailboxSession, StoredBody,
};
use norelly_domain::security::PlaintextPolicy;
use norelly_domain::sync::{SelectRequest, ServerQuery};
use norelly_imap::{ImapConnector, Timeouts};
use norelly_sqlite::mail::SqliteMailStore;
use norelly_sqlite::{Database, SqliteAccountRepository};
use norelly_test_support::certs::TestCa;
use norelly_test_support::containers::{TEST_PASSWORD, TEST_USER};
use norelly_test_support::contracts::sample_account;
use norelly_tls::{TlsClientFactory, TrustRoots};

pub type TestResult = Result<(), Box<dyn std::error::Error>>;

pub fn request(host: &str, port: u16) -> ConnectRequest {
    ConnectRequest {
        endpoint: ServerEndpoint::new(
            Host::parse(host).unwrap_or_else(|_| panic!("hôte {host}")),
            port,
            Security::Tls,
        )
        .unwrap_or_else(|_| panic!("serveur")),
        credentials: Credentials::new(
            Username::parse(TEST_USER).unwrap_or_else(|_| panic!("utilisateur")),
            Secret::new(TEST_PASSWORD.to_owned()),
        )
        .unwrap_or_else(|_| panic!("identifiants")),
        certificate_exceptions: vec![],
        plaintext: PlaintextPolicy::deny_all(),
    }
}

pub fn connector(ca: &TestCa, timeouts: Timeouts) -> ImapConnector {
    ImapConnector::new(
        TlsClientFactory::new(TrustRoots::Custom(vec![ca.root()]))
            .with_handshake_timeout(timeouts.response),
    )
    .with_timeouts(timeouts)
}

pub fn raw_message(n: u32) -> Vec<u8> {
    format!(
        "From: Expediteur {n} <e{n}@example.com>\r\nTo: moi@example.com\r\n\
         Subject: Message {n}\r\nMessage-ID: <{n}@integration.example>\r\n\
         Date: Mon, 2 Nov 2026 10:00:00 +0000\r\n\r\n{}\r\n",
        format!("Ligne {n} du corps.\r\n").repeat(20)
    )
    .into_bytes()
}

/// Stockage local réel (SQLite sur disque, fichiers) dans un répertoire
/// temporaire.
pub struct Local {
    pub dir: PathBuf,
    pub db: Database,
    pub store: Arc<SqliteMailStore>,
    pub blobs: Arc<FsBlobStore>,
    pub account: AccountId,
}

impl Drop for Local {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub async fn local(name: &str) -> Local {
    let dir = std::env::temp_dir().join(format!(
        "norelly-it-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let db = Database::open(&dir.join("norelly.sqlite3")).unwrap_or_else(|e| panic!("{e}"));
    let account = SqliteAccountRepository::new(db.clone())
        .insert(&sample_account("it@example.com", false))
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .id();
    Local {
        store: Arc::new(SqliteMailStore::new(db.clone())),
        db,
        blobs: Arc::new(FsBlobStore::new(dir.join("accounts"))),
        dir,
        account,
    }
}

impl Local {
    /// Une synchronisation complète sur une session neuve.
    pub async fn sync(
        &self,
        connector: &ImapConnector,
        request: &ConnectRequest,
    ) -> Result<(), SyncFailure> {
        let mut session = connector.connect(request).await?;
        let ext = session.enable_sync_extensions().await?;
        SyncAccount {
            account: self.account,
            store: &*self.store,
            blobs: &*self.blobs,
            policy: FetchPolicy::default(),
            text: std::sync::Arc::new(norelly_render::search_text::MimeTextExtractor),
        }
        .sync_all(&mut session, ext, |_, _| {})
        .await?;
        // Comme l'acteur : effets des actions en attente conservés, copies
        // provisoires des déplacements rejoués remplacées.
        self.store.reapply(self.account).await?;
        self.store.purge_placeholders(self.account, None).await?;
        let _ = session.logout().await;
        Ok(())
    }

    pub async fn id(&self, name: &str) -> Option<MailboxId> {
        self.store
            .tree(self.account)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .find(|n| n.name.as_str() == name)
            .map(|n| n.id)
    }

    pub async fn messages(&self, name: &str) -> BTreeMap<Uid, Flags> {
        let Some(id) = self.id(name).await else {
            return BTreeMap::new();
        };
        self.store
            .local_mailbox(id)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .messages
    }

    /// Vérifie que la boîte locale reflète le serveur, corps compris
    /// (contenu identique au message ajouté, retrouvé par son numéro).
    pub async fn assert_converged(
        &self,
        connector: &ImapConnector,
        request: &ConnectRequest,
        name: &str,
    ) {
        let expected = server_flags(connector, request, name).await;
        let local = self.messages(name).await;
        assert_eq!(local, expected, "boîte {name}");
        let id = self.id(name).await.unwrap_or_else(|| panic!("{name}"));
        for uid in local.keys() {
            let body = self
                .store
                .body_of(id, *uid)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            let Some(StoredBody::Full(blob)) = body else {
                panic!("{name} UID {uid} : corps {body:?}");
            };
            let raw = self
                .blobs
                .get(self.account, &blob)
                .await
                .unwrap_or_else(|e| panic!("{e}"))
                .unwrap_or_else(|| panic!("contenu absent"));
            assert!(raw.starts_with(b"From: "), "{name} UID {uid}");
        }
    }
}

/// Drapeaux des messages d'une boîte, lus directement sur le serveur.
pub async fn server_flags(
    connector: &ImapConnector,
    request: &ConnectRequest,
    name: &str,
) -> BTreeMap<Uid, Flags> {
    let mut session = connector
        .connect(request)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mailbox = MailboxName::parse(name).unwrap_or_else(|e| panic!("{e}"));
    session
        .select(&mailbox, &SelectRequest::default())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let flags = session
        .fetch_flags(ServerQuery::AllFlags)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let _ = session.logout().await;
    flags.into_iter().map(|f| (f.uid, f.flags)).collect()
}

pub const FAST: Timeouts = Timeouts {
    connect: Duration::from_secs(5),
    response: Duration::from_secs(5),
};
