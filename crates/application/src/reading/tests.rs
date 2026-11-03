//! Tests des cas d'usage de lecture, sur les faux en mémoire.

use std::sync::Mutex;

use norelly_domain::account::AccountId;
use norelly_domain::address::EmailAddress;
use norelly_domain::connection::ConnectionError;
use norelly_domain::mailbox::{MailboxName, MailboxRole, RemoteMailbox, Uid};
use norelly_domain::ports::{ConversationQuery, StoredMessage, SyncBatch};
use norelly_test_support::mail_contracts::{InMemoryBlobStore, InMemoryMailStore, sample_message};
use norelly_test_support::reading_contracts::InMemoryReadingPreferences;

use super::*;

#[derive(Debug, Default)]
struct FakeFetcher {
    calls: Mutex<Vec<(MailboxName, Uid, BodyFetch)>>,
    entity: Vec<u8>,
}

impl RemoteBodyFetcher for FakeFetcher {
    async fn fetch(
        &self,
        _account: AccountId,
        mailbox: &MailboxName,
        uid: Uid,
        fetch: &BodyFetch,
    ) -> Result<Option<FetchedBody>, ConnectionError> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((mailbox.clone(), uid, fetch.clone()));
        let BodyFetch::Partial { parts, .. } = fetch else {
            return Ok(Some(FetchedBody::Full(self.entity.clone())));
        };
        Ok(Some(FetchedBody::Partial {
            header: b"Subject: x\r\n\r\n".to_vec(),
            parts: parts
                .iter()
                .map(|p| (p.clone(), self.entity.clone()))
                .collect(),
        }))
    }
}

fn path(raw: &str) -> PartPath {
    PartPath::parse(raw).unwrap_or_else(|| panic!("{raw}"))
}

async fn setup(
    store: &InMemoryMailStore,
    blobs: &InMemoryBlobStore,
    body: impl FnOnce(BlobPair) -> StoredBody,
) -> Result<MessageId, ReadError> {
    let account = AccountId::new(1).map_err(|e| ReadError::Download(e.to_string()))?;
    let boxes = store
        .replace_mailboxes(
            account,
            &[RemoteMailbox {
                name: MailboxName::inbox(),
                delimiter: None,
                role: MailboxRole::Inbox,
                selectable: true,
            }],
        )
        .await?;
    let inbox = boxes.first().map(|b| b.id).ok_or(ReadError::NotFound)?;
    let pair = BlobPair {
        header: blobs
            .put(account, b"From: e1@example.com\r\n\r\n".to_vec())
            .await?,
        text: blobs.put(account, b"\r\nBonjour".to_vec()).await?,
    };
    let mut message: StoredMessage = sample_message(1, 1000, false);
    message.body = body(pair);
    store
        .apply_sync(
            inbox,
            &SyncBatch {
                new: vec![message],
                ..SyncBatch::default()
            },
        )
        .await?;
    let page = store
        .conversations(
            norelly_domain::conversation::ConversationScope::Mailbox(inbox),
            None,
            10,
        )
        .await?;
    let conversation = page
        .items
        .first()
        .map(|i| i.id)
        .ok_or(ReadError::NotFound)?;
    let messages = store.conversation_messages(conversation).await?;
    messages.first().map(|m| m.id).ok_or(ReadError::NotFound)
}

struct BlobPair {
    header: norelly_domain::ports::BlobId,
    text: norelly_domain::ports::BlobId,
}

#[tokio::test]
async fn load_decides_remote_images_per_sender() -> Result<(), ReadError> {
    let (store, blobs, prefs) = (
        InMemoryMailStore::default(),
        InMemoryBlobStore::default(),
        InMemoryReadingPreferences::default(),
    );
    let id = setup(&store, &blobs, |p| StoredBody::Full(p.header)).await?;
    let read = ReadMessage {
        locator: &store,
        blobs: &blobs,
        prefs: &prefs,
    };
    let loaded = read.load(id).await?.ok_or(ReadError::NotFound)?;
    assert_eq!(loaded.remote, RemoteContent::Blocked);
    assert!(!loaded.prefer_plain_text);
    assert_eq!(
        loaded.body,
        LoadedBody::Full(b"From: e1@example.com\r\n\r\n".to_vec())
    );

    allow_remote_images(&store, &prefs, id).await?;
    let sender =
        EmailAddress::parse("e1@example.com").map_err(|e| ReadError::Download(e.to_string()))?;
    assert!(prefs.remote_images_allowed(&sender).await?);
    let loaded = read.load(id).await?.ok_or(ReadError::NotFound)?;
    assert_eq!(loaded.remote, RemoteContent::Allowed);

    prefs.set_prefer_plain_text(true).await?;
    assert!(read.load(id).await?.is_some_and(|l| l.prefer_plain_text));

    let unknown = MessageId::new(999).ok_or(ReadError::NotFound)?;
    assert!(read.load(unknown).await?.is_none());
    assert_eq!(
        allow_remote_images(&store, &prefs, unknown).await,
        Err(ReadError::NotFound)
    );
    Ok(())
}

#[tokio::test]
async fn missing_blobs_degrade_to_missing_or_deferred() -> Result<(), ReadError> {
    let (store, blobs, prefs) = (
        InMemoryMailStore::default(),
        InMemoryBlobStore::default(),
        InMemoryReadingPreferences::default(),
    );
    let lost = norelly_domain::ports::BlobId::parse(&"0".repeat(64)).ok_or(ReadError::NotFound)?;
    let lost_clone = lost.clone();
    let id = setup(&store, &blobs, move |p| StoredBody::Partial {
        header: p.header,
        parts: vec![(path("1"), p.text), (path("2"), lost_clone)],
        deferred: vec![path("3")],
    })
    .await?;
    let read = ReadMessage {
        locator: &store,
        blobs: &blobs,
        prefs: &prefs,
    };
    let loaded = read.load(id).await?.ok_or(ReadError::NotFound)?;
    let LoadedBody::Partial {
        parts, deferred, ..
    } = loaded.body
    else {
        panic!("partiel attendu");
    };
    assert_eq!(parts.len(), 1);
    assert_eq!(deferred, [path("3"), path("2")]);
    let _ = lost;
    Ok(())
}

#[tokio::test]
async fn deferred_part_is_downloaded_once_and_stored() -> Result<(), ReadError> {
    let (store, blobs) = (InMemoryMailStore::default(), InMemoryBlobStore::default());
    let id = setup(&store, &blobs, |p| StoredBody::Partial {
        header: p.header,
        parts: vec![(path("1"), p.text)],
        deferred: vec![path("2")],
    })
    .await?;
    let fetcher = FakeFetcher {
        entity: b"Content-Type: application/pdf\r\n\r\nJVBERi0=".to_vec(),
        ..FakeFetcher::default()
    };
    let download = DownloadPart {
        locator: &store,
        blobs: &blobs,
        store: &store,
        fetcher: &fetcher,
    };
    download.execute(id, &path("2")).await?;
    let location = store.locate(id).await?.ok_or(ReadError::NotFound)?;
    let StoredBody::Partial {
        parts, deferred, ..
    } = location.body
    else {
        panic!("partiel attendu");
    };
    assert!(deferred.is_empty());
    assert_eq!(parts.len(), 2);
    // Déjà présente : aucun nouvel appel.
    download.execute(id, &path("2")).await?;
    download.execute(id, &path("1")).await?;
    assert_eq!(fetcher.calls.lock().map(|c| c.len()).unwrap_or(0), 1);
    assert_eq!(
        download.execute(id, &path("9")).await,
        Err(ReadError::PartNotFound)
    );
    Ok(())
}

#[tokio::test]
async fn missing_body_is_downloaded_on_open() -> Result<(), ReadError> {
    let (store, blobs) = (InMemoryMailStore::default(), InMemoryBlobStore::default());
    let id = setup(&store, &blobs, |_| StoredBody::Missing).await?;
    let fetcher = FakeFetcher {
        entity: b"Subject: x\r\n\r\ncorps".to_vec(),
        ..FakeFetcher::default()
    };
    let download = DownloadBody {
        locator: &store,
        blobs: &blobs,
        store: &store,
        fetcher: &fetcher,
        policy: FetchPolicy::default(),
        text: &norelly_render::search_text::MimeTextExtractor,
    };
    assert!(download.execute(id).await?);
    let location = store.locate(id).await?.ok_or(ReadError::NotFound)?;
    assert!(matches!(location.body, StoredBody::Full(_)));
    // Déjà présent : rien à faire.
    assert!(!download.execute(id).await?);
    assert_eq!(fetcher.calls.lock().map(|c| c.len()).unwrap_or(0), 1);
    Ok(())
}

#[tokio::test]
async fn body_downloaded_on_open_is_searchable() -> Result<(), ReadError> {
    use norelly_domain::ports::{SearchIndex, SearchRequest, SearchScope};
    let (store, blobs) = (InMemoryMailStore::default(), InMemoryBlobStore::default());
    let id = setup(&store, &blobs, |_| StoredBody::Missing).await?;
    let fetcher = FakeFetcher {
        entity: b"Subject: x\r\n\r\nmotdutexte".to_vec(),
        ..FakeFetcher::default()
    };
    let download = DownloadBody {
        locator: &store,
        blobs: &blobs,
        store: &store,
        fetcher: &fetcher,
        policy: FetchPolicy::default(),
        text: &norelly_render::search_text::MimeTextExtractor,
    };
    assert!(download.execute(id).await?);
    let page = store
        .search(SearchRequest {
            query: norelly_domain::search::parse_query("motdutexte").unwrap_or_default(),
            scope: SearchScope::All,
            utc_offset_seconds: 0,
            offset: 0,
            limit: 10,
        })
        .await?;
    assert_eq!(page.items.iter().map(|h| h.id).collect::<Vec<_>>(), [id]);
    Ok(())
}
