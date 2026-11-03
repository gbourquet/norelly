use super::*;
use crate::mailbox::{Flags, MessageHeaders, Uid};

fn meta(n: u32, date: i64, seen: bool) -> MessageMeta {
    MessageMeta {
        uid: Uid::new(n).unwrap_or_else(|| panic!("uid")),
        flags: Flags::from_tokens(if seen { vec!["\\Seen"] } else { vec![] }),
        modseq: None,
        internal_date: date,
        size: 10,
        headers: MessageHeaders {
            subject: format!("Sujet {n}"),
            from: vec![Correspondent::new(
                &format!("Nom {n}"),
                &format!("e{n}@example.com"),
            )],
            date: Some(date),
            ..MessageHeaders::default()
        },
        structure: None,
    }
}

#[test]
fn unread_arrivals_are_grouped_with_the_latest() {
    let got = new_mail(
        false,
        &[
            meta(1, 100, false),
            meta(2, 300, false),
            meta(3, 200, false),
        ],
    );
    assert_eq!(
        got,
        Some(NewMail {
            count: 3,
            latest: Some(Arrival {
                from: Some(Correspondent::new("Nom 2", "e2@example.com")),
                subject: "Sujet 2".into(),
                date: 300,
            }),
        })
    );
}

#[test]
fn read_messages_and_first_sync_are_not_signalled() {
    assert_eq!(new_mail(false, &[meta(1, 100, true)]), None);
    assert_eq!(new_mail(false, &[]), None);
    assert_eq!(new_mail(true, &[meta(1, 100, false)]), None);
    assert_eq!(
        new_mail(false, &[meta(1, 100, true), meta(2, 50, false)]).map(|m| m.count),
        Some(1)
    );
}

#[test]
fn batches_merge_keeping_the_most_recent() {
    let a = new_mail(false, &[meta(1, 100, false)]).unwrap_or_else(|| panic!("a"));
    let b =
        new_mail(false, &[meta(2, 400, false), meta(3, 50, false)]).unwrap_or_else(|| panic!("b"));
    let merged = a.clone().merge(b.clone());
    assert_eq!(merged.count, 3);
    assert_eq!(merged.latest.map(|l| l.date), Some(400));
    assert_eq!(b.merge(a).count, 3);
}

#[test]
fn privacy_removes_sender_and_subject() {
    let m = new_mail(false, &[meta(1, 100, false)])
        .unwrap_or_else(|| panic!("m"))
        .without_content();
    assert_eq!(
        m,
        NewMail {
            count: 1,
            latest: None
        }
    );
}
