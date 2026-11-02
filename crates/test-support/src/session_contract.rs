//! Suite de contrat de la session IMAP ([`RemoteMailboxSession`]),
//! exécutée contre le faux serveur en mémoire et contre Dovecot : les deux
//! doivent se comporter de la même façon pour la synchronisation.

use std::collections::BTreeSet;
use std::time::Duration;

use norelly_domain::fetch_policy::BodyFetch;
use norelly_domain::mailbox::{MailboxName, MailboxRole, Uid, classify_mailboxes};
use norelly_domain::ports::{
    ConnectRequest, FetchedBody, IdleOutcome, RemoteMailboxConnector, RemoteMailboxSession,
};
use norelly_domain::sync::{QresyncParams, SelectRequest, ServerQuery, UidRanges};

use crate::imap_control::MailboxControl;

/// Message brut de test.
#[must_use]
pub fn contract_message(n: u32) -> Vec<u8> {
    format!(
        "From: =?UTF-8?Q?Exp=C3=A9diteur?= <e{n}@example.com>\r\n\
         To: dest@example.com\r\n\
         Subject: =?UTF-8?B?{}?=\r\n\
         Date: Mon, 2 Nov 2026 10:00:0{} +0000\r\n\
         Message-ID: <contrat-{n}@example.com>\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         Corps du message {n}.\r\n",
        b64(format!("Réunion n°{n}").as_bytes()),
        n % 10
    )
    .into_bytes()
}

fn b64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(A[((n >> (18 - 6 * i)) & 0x3f) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn uid(n: u32) -> Uid {
    Uid::new(n).unwrap_or_else(|| panic!("uid {n}"))
}

fn name(raw: &str) -> MailboxName {
    MailboxName::parse(raw).unwrap_or_else(|e| panic!("{raw} : {e}"))
}

/// Contrat de la session IMAP. `control` modifie la boîte côté serveur ;
/// `box_name` : boîte de travail (créée, puis supprimée).
pub async fn remote_session_contract<C, K>(
    connector: &C,
    request: &ConnectRequest,
    control: &K,
    box_name: &str,
) -> Result<(), String>
where
    C: RemoteMailboxConnector,
    K: MailboxControl,
{
    control.create(box_name).await?;
    for n in 1..=3 {
        control
            .append(box_name, &contract_message(n), n == 1)
            .await?;
    }
    let mut session = connector
        .connect(request)
        .await
        .map_err(|e| e.to_string())?;
    let ext = session
        .enable_sync_extensions()
        .await
        .map_err(|e| e.to_string())?;
    assert!(
        ext.condstore && ext.qresync,
        "CONDSTORE et QRESYNC attendus : {ext:?}"
    );

    // --- LIST ---
    let listed = session.list_mailboxes().await.map_err(|e| e.to_string())?;
    let classified = classify_mailboxes(listed);
    let inbox = classified.iter().find(|m| m.name.is_inbox());
    assert_eq!(inbox.map(|m| m.role), Some(MailboxRole::Inbox));
    assert!(
        classified
            .iter()
            .any(|m| m.name.as_str() == box_name && m.selectable),
        "boîte {box_name} absente de LIST : {classified:?}"
    );

    // --- SELECT avec CONDSTORE ---
    let work = name(box_name);
    let request_condstore = SelectRequest {
        condstore: true,
        qresync: None,
    };
    let selected = session
        .select(&work, &request_condstore)
        .await
        .map_err(|e| e.to_string())?;
    assert_eq!(selected.exists, 3);
    assert_eq!(selected.uid_next, Some(uid(4)));
    let validity = selected
        .uid_validity
        .unwrap_or_else(|| panic!("UIDVALIDITY absente"));
    let modseq = selected
        .highest_modseq
        .unwrap_or_else(|| panic!("HIGHESTMODSEQ absent"));

    // --- Drapeaux, UID ---
    let flags = session
        .fetch_flags(ServerQuery::AllFlags)
        .await
        .map_err(|e| e.to_string())?;
    assert_eq!(
        flags.iter().map(|f| f.uid.get()).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(
        flags.iter().all(|f| f.modseq.is_some()),
        "MODSEQ par message"
    );
    assert!(flags.first().is_some_and(|f| f.flags.seen));
    assert!(flags.get(1).is_some_and(|f| !f.flags.seen));
    let all = session.search_all_uids().await.map_err(|e| e.to_string())?;
    assert_eq!(all, BTreeSet::from([uid(1), uid(2), uid(3)]));

    // --- Métadonnées ---
    let metas = session
        .fetch_meta(&[uid(3), uid(1), uid(99)])
        .await
        .map_err(|e| e.to_string())?;
    assert_eq!(
        metas.iter().map(|m| m.uid.get()).collect::<Vec<_>>(),
        [1, 3]
    );
    let first = metas.first().unwrap_or_else(|| panic!("métadonnées"));
    assert_eq!(first.headers.subject, "Réunion n°1");
    assert_eq!(
        first
            .headers
            .from
            .first()
            .map(|c| (c.name.as_str(), c.address.as_str())),
        Some(("Expéditeur", "e1@example.com"))
    );
    assert_eq!(
        first.headers.message_id.as_deref(),
        Some("contrat-1@example.com")
    );
    assert_eq!(first.size as usize, contract_message(1).len());
    assert!(first.internal_date > 0);

    // --- Corps ---
    let full = session
        .fetch_body(uid(2), &BodyFetch::Full)
        .await
        .map_err(|e| e.to_string())?;
    assert_eq!(full, Some(FetchedBody::Full(contract_message(2))));
    let partial = session
        .fetch_body(
            uid(2),
            &BodyFetch::Partial {
                parts: vec![],
                deferred: vec![],
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    let Some(FetchedBody::Partial { header, .. }) = partial else {
        panic!("en-têtes attendus : {partial:?}");
    };
    assert!(header.starts_with(b"From: "));
    assert!(!header.windows(5).any(|w| w == b"Corps"));
    // Partie seule : entité MIME autonome (en-têtes de partie, contenu).
    let one =
        norelly_domain::fetch_policy::PartPath::parse("1").ok_or_else(|| "chemin".to_owned())?;
    let entity = session
        .fetch_body(
            uid(2),
            &BodyFetch::Partial {
                parts: vec![one.clone()],
                deferred: vec![],
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    let Some(FetchedBody::Partial { parts, .. }) = entity else {
        panic!("partie attendue : {entity:?}");
    };
    let part = parts
        .iter()
        .find(|(p, _)| *p == one)
        .map(|(_, b)| b.clone())
        .unwrap_or_default();
    assert!(
        part.ends_with(b"Corps du message 2.\r\n"),
        "{:?}",
        String::from_utf8_lossy(&part)
    );
    assert!(
        part.windows(4).any(|w| w == b"\r\n\r\n"),
        "en-têtes de partie séparés du contenu"
    );
    // Le corps est lu sans marquer le message comme lu (BODY.PEEK).
    let after = session
        .fetch_flags(ServerQuery::AllFlags)
        .await
        .map_err(|e| e.to_string())?;
    assert!(
        after.get(1).is_some_and(|f| !f.flags.seen),
        "PEEK : \\Seen inchangé"
    );
    assert_eq!(
        session
            .fetch_body(uid(99), &BodyFetch::Full)
            .await
            .map_err(|e| e.to_string())?,
        None
    );

    // --- Changements côté serveur : CONDSTORE puis QRESYNC ---
    control.set_seen(box_name, 2, true).await?;
    control.expunge(box_name, 3).await?;
    control
        .append(box_name, &contract_message(4), false)
        .await?;
    let selected = session
        .select(&work, &request_condstore)
        .await
        .map_err(|e| e.to_string())?;
    assert_eq!(selected.uid_validity, Some(validity));
    assert!(selected.highest_modseq > Some(modseq));
    let changed = session
        .fetch_flags(ServerQuery::ChangedFlags(modseq))
        .await
        .map_err(|e| e.to_string())?;
    assert_eq!(
        changed.iter().map(|f| f.uid.get()).collect::<BTreeSet<_>>(),
        BTreeSet::from([2, 4]),
        "CHANGEDSINCE : seulement les modifiés"
    );
    let qresync = SelectRequest {
        condstore: false,
        qresync: Some(QresyncParams {
            uid_validity: validity,
            modseq,
        }),
    };
    let selected = session
        .select(&work, &qresync)
        .await
        .map_err(|e| e.to_string())?;
    assert_eq!(
        selected.vanished,
        UidRanges::from_uids([uid(3)]),
        "VANISHED (EARLIER)"
    );
    assert_eq!(
        selected
            .changed
            .iter()
            .map(|f| f.uid.get())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([2, 4])
    );
    assert!(
        selected
            .changed
            .iter()
            .any(|f| f.uid == uid(2) && f.flags.seen)
    );

    // --- IDLE ---
    let outcome = session
        .idle(Duration::from_millis(500))
        .await
        .map_err(|e| e.to_string())?;
    assert_eq!(outcome, IdleOutcome::Timeout, "aucun changement");
    let remote = control.clone();
    let target = box_name.to_owned();
    let appender = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        remote.append(&target, &contract_message(5), false).await
    });
    let started = tokio::time::Instant::now();
    let outcome = session
        .idle(Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;
    appender.await.map_err(|e| e.to_string())??;
    assert_eq!(outcome, IdleOutcome::Changed);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "changement reçu en IDLE"
    );
    // La session reste utilisable après IDLE.
    let all = session.search_all_uids().await.map_err(|e| e.to_string())?;
    assert_eq!(all, BTreeSet::from([uid(1), uid(2), uid(4), uid(5)]));

    // --- Nom non ASCII (UTF-7 modifié) ---
    let accented = format!("{box_name}-Été");
    control.create(&accented).await?;
    control
        .append(&accented, &contract_message(6), false)
        .await?;
    let listed = session.list_mailboxes().await.map_err(|e| e.to_string())?;
    assert!(
        listed.iter().any(|m| m.name.as_str() == accented),
        "nom décodé : {listed:?}"
    );
    let selected = session
        .select(&name(&accented), &SelectRequest::default())
        .await
        .map_err(|e| e.to_string())?;
    assert_eq!(selected.exists, 1);
    control.delete(&accented).await?;

    // --- UIDVALIDITY changée (boîte supprimée puis recréée) ---
    control.delete(box_name).await?;
    control.create(box_name).await?;
    control
        .append(box_name, &contract_message(7), false)
        .await?;
    let selected = session
        .select(&work, &SelectRequest::default())
        .await
        .map_err(|e| e.to_string())?;
    assert_ne!(
        selected.uid_validity,
        Some(validity),
        "nouvelle UIDVALIDITY"
    );
    assert_eq!(selected.exists, 1);

    session.logout().await.map_err(|e| e.to_string())?;
    control.delete(box_name).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use norelly_domain::credentials::{Credentials, Secret, Username};
    use norelly_domain::endpoint::{Host, Security, ServerEndpoint};
    use norelly_domain::security::PlaintextPolicy;

    use super::*;
    use crate::fake_imap::FakeImap;
    use crate::fakes::FakeMailServers;

    #[test]
    fn base64_is_standard() {
        assert_eq!(b64(b"Man"), "TWFu");
        assert_eq!(b64(b"Ma"), "TWE=");
        assert_eq!(b64(b"M"), "TQ==");
    }

    #[tokio::test]
    async fn fake_server_passes_contract() -> Result<(), String> {
        let imap = FakeImap::new();
        let servers = FakeMailServers::with_imap(imap.clone());
        let request = ConnectRequest {
            endpoint: ServerEndpoint::new(
                Host::parse("imap.example.com").map_err(|e| e.to_string())?,
                993,
                Security::Tls,
            )
            .map_err(|e| e.to_string())?,
            credentials: Credentials::new(
                Username::parse("u").map_err(|e| e.to_string())?,
                Secret::new("p".into()),
            )
            .map_err(|e| e.to_string())?,
            certificate_exceptions: vec![],
            plaintext: PlaintextPolicy::deny_all(),
        };
        remote_session_contract(&servers, &request, &imap, "Contrat").await
    }
}
