use super::*;
use crate::contact_import::{ContactFormat, DecodedContacts, ImportedCard, ImportedList};
use crate::contacts::{ContactInput, EmailInput};

struct SameText;

impl TextNormalizer for SameText {
    fn nfc(&self, text: &str) -> String {
        text.to_owned()
    }
    fn nfd(&self, text: &str) -> String {
        text.to_owned()
    }
}

fn url(raw: &str) -> DavUrl {
    DavUrl::parse(raw).unwrap_or_else(|e| panic!("{raw} : {e}"))
}

fn href(raw: &str) -> Href {
    Href::parse(raw).unwrap_or_else(|e| panic!("{raw} : {e}"))
}

fn etag(raw: &str) -> ETag {
    ETag::parse(raw).unwrap_or_else(|| panic!("ETag {raw}"))
}

fn input(name: &str, address: &str) -> ContactInput {
    ContactInput {
        display_name: name.into(),
        emails: vec![EmailInput {
            address: address.into(),
            ..EmailInput::default()
        }],
        ..ContactInput::default()
    }
}

fn draft(name: &str, address: &str) -> ContactDraft {
    ContactDraft::new(input(name, address)).unwrap_or_else(|e| panic!("{e}"))
}

fn contact(id: i64, name: &str, address: &str) -> Contact {
    draft(name, address).with_id(ContactId::new(id).unwrap_or_else(|| panic!("id")))
}

fn local(id: i64, name: &str, address: &str, modified_at: i64) -> LocalCard {
    LocalCard {
        contact: contact(id, name, address),
        snapshot: format!("{name}|{address}"),
        modified_at,
    }
}

fn link(path: &str, tag: &str, state: LinkState, synced: &str) -> SyncLink {
    SyncLink {
        href: href(path),
        etag: etag(tag),
        state,
        synced: synced.into(),
        remote: format!("remote:{path}"),
    }
}

fn linked(id: i64) -> LinkState {
    LinkState::Linked(ContactId::new(id).unwrap_or_else(|| panic!("id")))
}

fn entry(path: &str, tag: &str) -> RemoteEntry {
    RemoteEntry {
        href: href(path),
        etag: etag(tag),
    }
}

fn fetched(path: &str, tag: &str, card: Option<ContactDraft>, rev: Option<&str>) -> FetchedCard {
    let raw = match rev {
        Some(rev) => format!("BEGIN:VCARD\r\nVERSION:3.0\r\nREV:{rev}\r\nEND:VCARD\r\n"),
        None => "BEGIN:VCARD\r\nVERSION:3.0\r\nEND:VCARD\r\n".to_owned(),
    };
    FetchedCard {
        entry: entry(path, tag),
        raw,
        keys: card
            .iter()
            .flat_map(|d| d.emails().iter().map(|e| e.key().clone()))
            .collect(),
        draft: card,
    }
}

/// Plan sans fiche remise à plus tard.
fn sync_plan(
    links: &[SyncLink],
    local: &[LocalCard],
    remote: &[RemoteEntry],
    fetched: &[FetchedCard],
    now: i64,
) -> Vec<SyncAction> {
    plan_sync(links, local, remote, fetched, &[], now)
}

fn kinds(actions: &[SyncAction]) -> Vec<&'static str> {
    actions.iter().map(SyncAction::kind).collect()
}

// --- Adresses ----------------------------------------------------------------

#[test]
fn a_server_address_is_https_only() {
    let u = url("https://dav.example.com/dav/");
    assert_eq!(u.to_string(), "https://dav.example.com/dav/");
    assert_eq!(u.port(), 443);
    // Sans schéma : HTTPS supposé, chemin racine.
    assert_eq!(
        url("Dav.Example.com").to_string(),
        "https://dav.example.com/"
    );
    assert_eq!(
        url("dav.example.com:8443").to_string(),
        "https://dav.example.com:8443/"
    );
    assert_eq!(
        url("https://[::1]:8443/a").to_string(),
        "https://[::1]:8443/a"
    );
    assert_eq!(
        DavUrl::parse("http://dav.example.com/"),
        Err(DavUrlError::NotHttps)
    );
    assert_eq!(
        DavUrl::parse("ftp://dav.example.com/"),
        Err(DavUrlError::NotHttps)
    );
    assert_eq!(DavUrl::parse(""), Err(DavUrlError::Empty));
    assert_eq!(DavUrl::parse("  "), Err(DavUrlError::Empty));
}

#[test]
fn a_server_address_refuses_what_could_leak_or_mislead() {
    // Identifiants dans l'adresse : jamais (ils finiraient en clair dans la base).
    assert_eq!(
        DavUrl::parse("https://alice:secret@dav.example.com/"),
        Err(DavUrlError::UserInfo)
    );
    assert_eq!(
        DavUrl::parse("https://dav.example.com/?a=1"),
        Err(DavUrlError::QueryOrFragment)
    );
    assert_eq!(
        DavUrl::parse("https://dav.example.com/#x"),
        Err(DavUrlError::QueryOrFragment)
    );
    assert_eq!(
        DavUrl::parse("https://dav.exa mple.com/"),
        Err(DavUrlError::InvalidHost)
    );
    assert_eq!(
        DavUrl::parse("https://dav.example.com:0/"),
        Err(DavUrlError::InvalidPort)
    );
    assert_eq!(
        DavUrl::parse("https://dav.example.com:99999/"),
        Err(DavUrlError::InvalidPort)
    );
    assert_eq!(
        DavUrl::parse("https://dav.example.com/a b"),
        Err(DavUrlError::InvalidPath)
    );
    assert_eq!(
        DavUrl::parse("https://dav.example.com/a/../b"),
        Err(DavUrlError::InvalidPath)
    );
    assert_eq!(
        DavUrl::parse("https://dav.example.com/%zz"),
        Err(DavUrlError::InvalidPath)
    );
    assert_eq!(
        DavUrl::parse("https://dav.example.com/é"),
        Err(DavUrlError::InvalidPath)
    );
    assert_eq!(
        DavUrl::parse("https://dav.example.com/a\\b"),
        Err(DavUrlError::InvalidPath)
    );
    let long = format!("https://dav.example.com/{}", "a".repeat(MAX_DAV_URL_CHARS));
    assert_eq!(DavUrl::parse(&long), Err(DavUrlError::TooLong));
}

#[test]
fn hrefs_resolve_on_the_same_origin_only() {
    let book = url("https://dav.example.com/books/alice/contacts/");
    assert_eq!(
        book.join("/books/alice/contacts/a.vcf")
            .map(|u| u.to_string()),
        Some("https://dav.example.com/books/alice/contacts/a.vcf".into())
    );
    assert_eq!(
        book.join("b.vcf").map(|u| u.to_string()),
        Some("https://dav.example.com/books/alice/contacts/b.vcf".into())
    );
    assert_eq!(
        book.join("https://DAV.example.com:443/x/")
            .map(|u| u.to_string()),
        Some("https://dav.example.com/x/".into())
    );
    // Autre hôte, autre port, HTTP : le mot de passe n'y part jamais.
    assert_eq!(book.join("https://evil.example.net/x.vcf"), None);
    assert_eq!(book.join("https://dav.example.com:8443/x.vcf"), None);
    assert_eq!(book.join("http://dav.example.com/x.vcf"), None);
    assert_eq!(book.join("//evil.example.net/x.vcf"), None);
    assert_eq!(book.join(""), None);
    assert!(book.same_origin(&url("https://dav.example.com/autre")));
    assert!(!book.same_origin(&url("https://dav.example.com:444/")));
}

#[test]
fn discovery_tries_the_well_known_address_from_the_root_only() {
    assert_eq!(
        discovery_candidates(&url("dav.example.com"))
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        [
            "https://dav.example.com/.well-known/carddav",
            "https://dav.example.com/"
        ]
    );
    assert_eq!(
        discovery_candidates(&url("https://dav.example.com/remote.php/dav"))
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["https://dav.example.com/remote.php/dav"]
    );
}

#[test]
fn redirects_stay_on_the_server() {
    let from = url("https://dav.example.com/.well-known/carddav");
    assert_eq!(
        redirect_target(&from, "/dav/", 0).map(|u| u.to_string()),
        Some("https://dav.example.com/dav/".into())
    );
    assert_eq!(
        redirect_target(&from, "https://other.example.com/dav/", 0),
        None
    );
    assert_eq!(
        redirect_target(&from, "http://dav.example.com/dav/", 0),
        None
    );
    assert_eq!(redirect_target(&from, "/dav/", MAX_DAV_REDIRECTS), None);
}

#[test]
fn hrefs_and_etags_are_bounded() {
    assert!(Href::parse("/a/b.vcf").is_ok());
    assert!(Href::parse("a.vcf").is_err(), "chemin absolu attendu");
    assert!(Href::parse("/a/../b").is_err());
    assert!(Href::parse(&format!("/{}", "a".repeat(MAX_DAV_URL_CHARS))).is_err());
    assert_eq!(
        ETag::parse(" \"abc\" ").map(|e| e.as_str().to_owned()),
        Some("\"abc\"".into())
    );
    assert_eq!(
        ETag::parse("W/\"abc\"").map(|e| e.as_str().to_owned()),
        Some("W/\"abc\"".into())
    );
    assert!(ETag::parse("").is_none());
    assert!(ETag::parse("a\r\nX-Injected: 1").is_none());
    assert!(ETag::parse(&"a".repeat(MAX_ETAG_CHARS + 1)).is_none());
}

#[test]
fn a_new_card_gets_a_resource_in_the_book() {
    let book = url("https://dav.example.com/books/alice");
    let uid = CardUid::parse("0123abcd-ef").unwrap_or_else(|| panic!("uid"));
    assert_eq!(
        new_card_href(&book, &uid).as_str(),
        "/books/alice/0123abcd-ef.vcf"
    );
    assert!(CardUid::parse("../x").is_none());
    assert!(CardUid::parse("").is_none());
    assert!(CardUid::parse(&"a".repeat(65)).is_none());
}

// --- Compte ------------------------------------------------------------------

#[test]
fn an_account_needs_a_basic_compatible_username_and_a_book_on_the_server() {
    assert_eq!(
        DavUsername::parse("alice:x"),
        Err(DavAccountError::UsernameColon)
    );
    assert_eq!(
        DavUsername::parse(""),
        Err(DavAccountError::Username(CredentialsError::EmptyUsername))
    );
    assert!(DavUsername::parse("alice@example.com").is_ok());
    assert_eq!(
        validate_password(&Secret::new(String::new())),
        Err(DavAccountError::Password(CredentialsError::EmptySecret))
    );
    assert_eq!(
        validate_password(&Secret::new("a\0b".into())),
        Err(DavAccountError::Password(
            CredentialsError::SecretNulCharacter
        ))
    );
    assert!(validate_password(&Secret::new("abcd efgh ijkl".into())).is_ok());

    let user = DavUsername::parse("alice").unwrap_or_else(|e| panic!("{e}"));
    let server = url("dav.example.com");
    let book = AddressBook::new(
        url("https://dav.example.com/alice/contacts/"),
        "Contacts\n\u{202e}",
    );
    assert_eq!(book.name(), "Contacts");
    let account = CardDavAccount::new(server.clone(), user.clone(), book);
    assert!(account.is_ok());
    let elsewhere = AddressBook::new(url("https://evil.example.net/contacts/"), "x");
    assert_eq!(
        CardDavAccount::new(server, user, elsewhere).err(),
        Some(DavAccountError::BookOutsideServer)
    );
    let long = AddressBook::new(url("https://dav.example.com/"), &"n".repeat(500));
    assert_eq!(long.name().chars().count(), MAX_BOOK_NAME_CHARS);
}

// --- Dernière modification gagnante -------------------------------------------

#[test]
fn the_latest_modification_wins() {
    assert_eq!(newest_side(100, Some(200), 1_000), Side::Remote);
    assert_eq!(newest_side(300, Some(200), 1_000), Side::Local);
    // Égalité : le serveur, partagé par les autres appareils, l'emporte.
    assert_eq!(newest_side(200, Some(200), 1_000), Side::Remote);
    // Date inconnue : la modification vient d'être constatée.
    assert_eq!(newest_side(900, None, 1_000), Side::Remote);
    // Date future (horloge déréglée) : ramenée à maintenant.
    assert_eq!(newest_side(999, Some(5_000), 1_000), Side::Remote);
    assert_eq!(newest_side(5_000, Some(999), 1_000), Side::Local);
}

#[test]
fn rev_dates_are_read_in_their_common_forms() {
    assert_eq!(parse_rev("19700101T000000Z"), Some(0));
    assert_eq!(parse_rev("20231118T101010Z"), Some(1_700_302_210));
    assert_eq!(parse_rev("2023-11-18T10:10:10Z"), Some(1_700_302_210));
    assert_eq!(parse_rev("2023-11-18T10:10:10.123Z"), Some(1_700_302_210));
    assert_eq!(parse_rev("20231118T121010+0200"), Some(1_700_302_210));
    assert_eq!(parse_rev("2023-11-18T05:10:10-05:00"), Some(1_700_302_210));
    assert_eq!(parse_rev("20231118"), Some(1_700_265_600));
    assert_eq!(parse_rev("2023-11-18"), Some(1_700_265_600));
    assert_eq!(parse_rev("20240229T000000Z"), Some(1_709_164_800));
    assert_eq!(parse_rev("20231318T000000Z"), None);
    assert_eq!(parse_rev("20230230T000000Z"), None);
    assert_eq!(parse_rev("demain"), None);
    assert_eq!(parse_rev(""), None);
    assert_eq!(format_rev(1_700_302_210), "20231118T101010Z");
    assert_eq!(format_rev(0), "19700101T000000Z");
}

// --- vCard -------------------------------------------------------------------

const REMOTE: &str = "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:1\r\nPRODID:-//Autre//FR\r\nFN:Ancien\r\nN:Ancien;;;;\r\nitem1.EMAIL;TYPE=work:ancien@example.com\r\nitem1.X-ABLabel:Bureau\r\nTEL:0102\r\nADR:;;1 rue du Port;Brest;;29200;France\r\nPHOTO;MEDIATYPE=image/png:data:image/png;base64,AAAA\r\n BBBB\r\nBDAY:19800102\r\nREV:20200101T000000Z\r\nEND:VCARD\r\n";

const OURS: &str = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Alice Martin\r\nN:Martin;Alice;;;\r\nEMAIL;PREF=1:alice@example.com\r\nNOTE:Une note\r\nEND:VCARD\r\n";

#[test]
fn card_properties_are_read_after_unfolding() {
    assert_eq!(
        card_property(REMOTE, "REV").as_deref(),
        Some("20200101T000000Z")
    );
    assert_eq!(card_property(REMOTE, "uid").as_deref(), Some("urn:uuid:1"));
    assert_eq!(
        card_property(REMOTE, "PHOTO").as_deref(),
        Some("data:image/png;base64,AAAABBBB")
    );
    assert_eq!(card_property(REMOTE, "NICKNAME"), None);
    assert_eq!(card_version(REMOTE), CardVersion::V4);
    assert_eq!(
        card_version("BEGIN:VCARD\nVERSION:3.0\nEND:VCARD"),
        CardVersion::V3
    );
    assert_eq!(
        card_version("BEGIN:VCARD\nVERSION:2.1\nEND:VCARD"),
        CardVersion::V3
    );
    assert_eq!(card_rev(REMOTE), Some(1_577_836_800));
}

#[test]
fn a_pushed_change_keeps_what_norelly_does_not_know() {
    let merged = merge_card(REMOTE, OURS, 1_700_302_210);
    // Propriétés inconnues du carnet : gardées telles quelles. (Adresse
    // postale et anniversaire sont tenus par le carnet : voir
    // `a_pushed_change_replaces_the_details_the_book_holds`.)
    for kept in [
        "UID:urn:uuid:1",
        "PHOTO;MEDIATYPE=image/png:data:image/png;base64,AAAA\r\n BBBB",
        "PRODID:-//Autre//FR",
    ] {
        assert!(merged.contains(kept), "{kept} perdu :\n{merged}");
    }
    // Propriétés du carnet : remplacées par les nôtres, groupe compris.
    for gone in [
        "Ancien",
        "ancien@example.com",
        "X-ABLabel",
        "TEL:0102",
        "20200101",
    ] {
        assert!(!merged.contains(gone), "{gone} resté :\n{merged}");
    }
    for ours in [
        "FN:Alice Martin",
        "N:Martin;Alice;;;",
        "EMAIL;PREF=1:alice@example.com",
        "NOTE:Une note",
    ] {
        assert!(merged.contains(ours), "{ours} absent :\n{merged}");
    }
    assert!(merged.starts_with("BEGIN:VCARD\r\nVERSION:4.0\r\n"));
    assert!(merged.ends_with("END:VCARD\r\n"));
    assert!(merged.contains("REV:20231118T101010Z\r\n"));
    assert_eq!(merged.matches("VERSION:").count(), 1);
    assert_eq!(merged.matches("BEGIN:VCARD").count(), 1);
}

/// Titre, anniversaire, adresses postales et sites : le carnet les tient
/// (ADR-012) ; poussés, ils remplacent ceux du serveur (une fois chacun,
/// une suppression ici suit sur le serveur). Une valeur que le carnet ne
/// sait pas lire (anniversaire en texte, lien qui n'est pas un site) est
/// gardée telle quelle : il ne l'a jamais eue, il ne peut pas l'effacer.
#[test]
fn a_pushed_change_replaces_the_details_the_book_holds() {
    let remote = REMOTE.replace(
        "BDAY:19800102\r\n",
        "BDAY:19800102\r\nTITLE:Ancienne fonction\r\nitem2.URL:https://ancien.example\r\nitem2.X-ABLabel:blog\r\nURL:javascript:alert(1)\r\n",
    );
    let ours = OURS.replace(
        "NOTE:Une note\r\n",
        "NOTE:Une note\r\nTITLE:Directrice\r\nBDAY:--0412\r\nADR;TYPE=home:;;2 rue Neuve;Lyon;;69001;\r\nURL:https://neuf.example\r\n",
    );
    let merged = merge_card(&remote, &ours, 1_700_302_210);
    for gone in [
        "1 rue du Port",
        "BDAY:19800102",
        "Ancienne fonction",
        "ancien.example",
        "X-ABLabel:blog",
    ] {
        assert!(!merged.contains(gone), "{gone} resté :\n{merged}");
    }
    for kept in [
        "TITLE:Directrice",
        "BDAY:--0412",
        "ADR;TYPE=home:;;2 rue Neuve;Lyon;;69001;",
        "URL:https://neuf.example",
        "URL:javascript:alert(1)",
        "PHOTO;MEDIATYPE=image/png",
    ] {
        assert_eq!(merged.matches(kept).count(), 1, "{kept} :\n{merged}");
    }
    assert_eq!(merged.matches("BDAY:").count(), 1, "{merged}");

    // Anniversaire illisible pour le carnet : gardé, même sans le nôtre.
    let text = REMOTE.replace("BDAY:19800102", "BDAY;VALUE=text:vers 1950");
    let merged = merge_card(&text, OURS, 1_700_302_210);
    assert!(merged.contains("BDAY;VALUE=text:vers 1950"), "{merged}");
}

#[test]
fn a_new_card_carries_its_uid_and_date() {
    let uid = CardUid::parse("abc-1").unwrap_or_else(|| panic!("uid"));
    let card = new_card(OURS, &uid, 0);
    assert!(
        card.starts_with("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:abc-1\r\nREV:19700101T000000Z\r\n")
    );
    assert!(card.contains("FN:Alice Martin\r\n"));
    assert!(card.ends_with("END:VCARD\r\n"));
}

#[test]
fn a_remote_card_becomes_a_contact_only_when_valid() {
    let one = DecodedContacts {
        format: ContactFormat::VCard,
        cards: vec![ImportedCard::Contact(input("Alice", "alice@example.com"))],
        photos: crate::contact_photo::CardPhotos::new(),
    };
    assert_eq!(
        remote_draft(&one, &SameText),
        Some(draft("Alice", "alice@example.com"))
    );
    let without_address = DecodedContacts {
        format: ContactFormat::VCard,
        cards: vec![ImportedCard::Contact(ContactInput {
            display_name: "Sans adresse".into(),
            ..ContactInput::default()
        })],
        photos: crate::contact_photo::CardPhotos::new(),
    };
    assert_eq!(remote_draft(&without_address, &SameText), None);
    let two = DecodedContacts {
        format: ContactFormat::VCard,
        cards: vec![
            ImportedCard::Contact(input("A", "a@example.com")),
            ImportedCard::Contact(input("B", "b@example.com")),
        ],
        photos: crate::contact_photo::CardPhotos::new(),
    };
    assert_eq!(
        remote_draft(&two, &SameText),
        None,
        "une ressource, une fiche"
    );
    let list = DecodedContacts {
        format: ContactFormat::VCard,
        cards: vec![ImportedCard::List(ImportedList::default())],
        photos: crate::contact_photo::CardPhotos::new(),
    };
    assert_eq!(remote_draft(&list, &SameText), None);
}

// --- Plan --------------------------------------------------------------------

#[test]
fn only_new_or_changed_resources_are_downloaded() {
    let links = [
        link("/c/a.vcf", "1", linked(1), "s"),
        link("/c/b.vcf", "1", linked(2), "s"),
    ];
    let remote = [
        entry("/c/a.vcf", "1"),
        entry("/c/b.vcf", "2"),
        entry("/c/n.vcf", "1"),
    ];
    assert_eq!(
        hrefs_to_fetch(&links, &remote),
        [href("/c/b.vcf"), href("/c/n.vcf")]
    );
}

#[test]
fn nothing_to_do_when_nothing_changed() {
    let links = [link("/c/a.vcf", "1", linked(1), "Alice|alice@example.com")];
    let local = [local(1, "Alice", "alice@example.com", 10)];
    let plan = sync_plan(&links, &local, &[entry("/c/a.vcf", "1")], &[], 100);
    assert!(plan.is_empty(), "{plan:?}");
}

#[test]
fn local_changes_are_pushed_and_new_contacts_created() {
    let links = [link("/c/a.vcf", "1", linked(1), "ancien")];
    let local = [
        local(1, "Alice", "alice@example.com", 10),
        local(2, "Bob", "bob@example.com", 10),
    ];
    let plan = sync_plan(&links, &local, &[entry("/c/a.vcf", "1")], &[], 100);
    assert_eq!(kinds(&plan), ["push-update", "push-create"]);
    match &plan[0] {
        SyncAction::PushUpdate {
            href: h,
            etag: e,
            base,
            contact,
        } => {
            assert_eq!(h, &href("/c/a.vcf"));
            assert_eq!(e, &etag("1"));
            assert_eq!(base, &None, "fiche distante enregistrée");
            assert_eq!(contact.id().get(), 1);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn remote_changes_are_pulled_and_keep_the_local_favorite() {
    let links = [link("/c/a.vcf", "1", linked(1), "Alice|alice@example.com")];
    let mut mine = local(1, "Alice", "alice@example.com", 10);
    let mut favorite = input("Alice", "alice@example.com");
    favorite.favorite = true;
    mine.contact = ContactDraft::new(favorite)
        .unwrap_or_else(|e| panic!("{e}"))
        .with_id(mine.contact.id());
    let card = fetched(
        "/c/a.vcf",
        "2",
        Some(draft("Alice M.", "alice@example.com")),
        None,
    );
    let plan = sync_plan(&links, &[mine], &[entry("/c/a.vcf", "2")], &[card], 100);
    assert_eq!(kinds(&plan), ["pull-update"]);
    match &plan[0] {
        SyncAction::PullUpdate { draft, card, .. } => {
            assert!(draft.favorite(), "le favori n'existe pas en vCard : gardé");
            assert_eq!(draft.display_name(), "Alice M.");
            assert_eq!(card.entry.etag, etag("2"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_conflict_goes_to_the_latest_modification() {
    let links = [link("/c/a.vcf", "1", linked(1), "ancien")];
    let remote = [entry("/c/a.vcf", "2")];
    // Local modifié à 500, serveur à 400 : la fiche locale part.
    let card = fetched(
        "/c/a.vcf",
        "2",
        Some(draft("Distant", "alice@example.com")),
        Some("19700101T000640Z"),
    );
    let plan = sync_plan(
        &links,
        &[local(1, "Alice", "alice@example.com", 500)],
        &remote,
        std::slice::from_ref(&card),
        1_000,
    );
    assert_eq!(kinds(&plan), ["push-update"]);
    match &plan[0] {
        SyncAction::PushUpdate { etag: e, base, .. } => {
            assert_eq!(e, &etag("2"), "contre la version lue");
            assert_eq!(
                base.as_ref(),
                Some(&card.raw),
                "fusionnée sur la version distante"
            );
        }
        other => panic!("{other:?}"),
    }
    // Local modifié à 300 : la version distante l'emporte.
    let plan = sync_plan(
        &links,
        &[local(1, "Alice", "alice@example.com", 300)],
        &remote,
        &[card],
        1_000,
    );
    assert_eq!(kinds(&plan), ["pull-update"]);
}

#[test]
fn deletions_follow_the_other_side_unless_it_changed() {
    // Supprimée sur le serveur, inchangée ici : supprimée ici.
    let links = [link("/c/a.vcf", "1", linked(1), "Alice|alice@example.com")];
    let plan = sync_plan(
        &links,
        &[local(1, "Alice", "alice@example.com", 10)],
        &[],
        &[],
        100,
    );
    assert_eq!(kinds(&plan), ["delete-local"]);
    // Supprimée sur le serveur, modifiée ici depuis : recréée sur le serveur.
    let links = [link("/c/a.vcf", "1", linked(1), "ancien")];
    let plan = sync_plan(
        &links,
        &[local(1, "Alice", "alice@example.com", 10)],
        &[],
        &[],
        100,
    );
    assert_eq!(kinds(&plan), ["forget", "push-create"]);
    // Supprimée ici, inchangée sur le serveur : supprimée sur le serveur.
    let links = [link("/c/a.vcf", "1", LinkState::Deleted { at: 50 }, "s")];
    let plan = sync_plan(&links, &[], &[entry("/c/a.vcf", "1")], &[], 100);
    assert_eq!(kinds(&plan), ["delete-remote"]);
    // Supprimée des deux côtés : oubliée.
    let plan = sync_plan(&links, &[], &[], &[], 100);
    assert_eq!(kinds(&plan), ["forget"]);
    // Supprimée ici à 50, modifiée sur le serveur à 80 : recréée ici.
    let card = fetched(
        "/c/a.vcf",
        "2",
        Some(draft("Alice", "alice@example.com")),
        Some("19700101T000120Z"),
    );
    let plan = sync_plan(
        &links,
        &[],
        &[entry("/c/a.vcf", "2")],
        std::slice::from_ref(&card),
        100,
    );
    assert_eq!(kinds(&plan), ["pull-create"]);
    // Supprimée ici à 90 : la suppression, plus récente, part.
    let links = [link("/c/a.vcf", "1", LinkState::Deleted { at: 90 }, "s")];
    let plan = sync_plan(&links, &[], &[entry("/c/a.vcf", "2")], &[card], 100);
    assert_eq!(kinds(&plan), ["delete-remote"]);
    match &plan[0] {
        SyncAction::DeleteRemote { etag: e, .. } => assert_eq!(e, &etag("2")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn new_remote_cards_are_created_matched_or_ignored() {
    let remote = [
        entry("/c/new.vcf", "1"),
        entry("/c/same.vcf", "1"),
        entry("/c/diff.vcf", "1"),
        entry("/c/bad.vcf", "1"),
        entry("/c/dup.vcf", "1"),
    ];
    let fetched_cards = [
        fetched(
            "/c/new.vcf",
            "1",
            Some(draft("Nouveau", "new@example.com")),
            None,
        ),
        fetched(
            "/c/same.vcf",
            "1",
            Some(draft("Bob", "bob@example.com")),
            None,
        ),
        fetched(
            "/c/diff.vcf",
            "1",
            Some(draft("Carole D.", "carole@example.com")),
            Some("19700101T000000Z"),
        ),
        fetched("/c/bad.vcf", "1", None, None),
        fetched(
            "/c/dup.vcf",
            "1",
            Some(draft("Copie", "NEW@example.com")),
            None,
        ),
    ];
    let local = [
        local(2, "Bob", "bob@example.com", 10),
        local(3, "Carole", "carole@example.com", 10),
    ];
    let plan = sync_plan(&[], &local, &remote, &fetched_cards, 100);
    // Même fiche des deux côtés : reliées ; fiche différente : la plus
    // récente gagne (ici la locale) ; fiche illisible ou adresse déjà
    // prise : ignorées.
    assert_eq!(
        kinds(&plan),
        ["pull-create", "link", "push-update", "ignore", "ignore"]
    );
}

#[test]
fn an_unusable_remote_change_keeps_the_local_contact() {
    let links = [link("/c/a.vcf", "1", linked(1), "Alice|alice@example.com")];
    let card = fetched("/c/a.vcf", "2", None, None);
    let plan = sync_plan(
        &links,
        &[local(1, "Alice", "alice@example.com", 10)],
        &[entry("/c/a.vcf", "2")],
        std::slice::from_ref(&card),
        100,
    );
    assert_eq!(kinds(&plan), ["refresh"]);
    // Modifiée des deux côtés : la plus récente gagne (sans date, la
    // version distante, qui ne peut pas être reprise, fait attendre).
    let links = [link("/c/a.vcf", "1", linked(1), "ancien")];
    let plan = sync_plan(
        &links,
        &[local(1, "Alice", "alice@example.com", 10)],
        &[entry("/c/a.vcf", "2")],
        std::slice::from_ref(&card),
        100,
    );
    assert_eq!(kinds(&plan), ["defer"]);
    let old = fetched("/c/a.vcf", "2", None, Some("19700101T000005Z"));
    let plan = sync_plan(
        &links,
        &[local(1, "Alice", "alice@example.com", 10)],
        &[entry("/c/a.vcf", "2")],
        &[old],
        100,
    );
    assert_eq!(kinds(&plan), ["push-update"]);
}

#[test]
fn ignored_resources_are_reconsidered_when_they_change() {
    let links = [link("/c/a.vcf", "1", LinkState::Ignored, "")];
    assert!(sync_plan(&links, &[], &[entry("/c/a.vcf", "1")], &[], 100).is_empty());
    let card = fetched(
        "/c/a.vcf",
        "2",
        Some(draft("Alice", "alice@example.com")),
        None,
    );
    let plan = sync_plan(&links, &[], &[entry("/c/a.vcf", "2")], &[card], 100);
    assert_eq!(kinds(&plan), ["pull-create"]);
    assert_eq!(kinds(&sync_plan(&links, &[], &[], &[], 100)), ["forget"]);
}

#[test]
fn a_resource_not_returned_by_the_server_is_left_for_later() {
    // Nouvelle : laissée de côté jusqu'à son prochain changement (elle
    // n'est plus redemandée à chaque synchronisation).
    let plan = sync_plan(&[], &[], &[entry("/c/a.vcf", "1")], &[], 100);
    assert_eq!(kinds(&plan), ["ignore"]);
    // Reliée : remise à plus tard, sans rien écrire ni la délier.
    let links = [link("/c/a.vcf", "1", linked(1), "ancien")];
    let plan = sync_plan(
        &links,
        &[local(1, "Alice", "alice@example.com", 10)],
        &[entry("/c/a.vcf", "2")],
        &[],
        100,
    );
    assert_eq!(kinds(&plan), ["defer"]);
}

#[test]
fn an_unreadable_remote_card_keeps_its_addresses() {
    // Fiche distante illisible qui porte l'adresse d'un contact non relié :
    // le contact n'est pas envoyé en double.
    let mut card = fetched("/c/bob.vcf", "1", None, None);
    card.keys = vec![
        crate::contacts::AddressKey::parse("BOB@example.com").unwrap_or_else(|e| panic!("{e}")),
    ];
    let plan = sync_plan(
        &[],
        &[local(2, "Bob", "bob@example.com", 10)],
        &[entry("/c/bob.vcf", "1")],
        &[card],
        100,
    );
    assert_eq!(kinds(&plan), ["ignore"]);
}

#[test]
fn a_report_counts_changes_on_each_side() {
    let mut report = SyncReport::default();
    report.count(&SyncAction::Forget { href: href("/a") });
    report.pulled += 2;
    report.pushed += 1;
    assert_eq!(report.changes(), 3);
}

#[test]
fn a_failure_has_a_stable_code() {
    for failure in [
        SyncFailure::Unauthorized,
        SyncFailure::Network,
        SyncFailure::Certificate,
        SyncFailure::NotFound,
        SyncFailure::TooLarge,
        SyncFailure::Server,
        SyncFailure::Protocol,
        SyncFailure::Storage,
        SyncFailure::Keyring,
    ] {
        assert_eq!(SyncFailure::parse(failure.as_str()), Some(failure));
    }
    assert_eq!(SyncFailure::parse("autre"), None);
}

#[test]
fn hrefs_are_compared_in_a_canonical_form() {
    let book = url("https://dav.example.com/c/");
    let path = |raw: &str| book.join(raw).map(|u| u.path().to_owned());
    assert_eq!(
        path("/c/%7ealice/a%2dx.vcf"),
        Some("/c/~alice/a-x.vcf".into())
    );
    assert_eq!(path("/c/%c3%a9.vcf"), Some("/c/%C3%A9.vcf".into()));
    // Adresse citée par le serveur sans encodage : encodée.
    assert_eq!(path("/c/é b.vcf"), Some("/c/%C3%A9%20b.vcf".into()));
    assert_eq!(path("/c/%2e%2E/x.vcf"), None);
    assert_eq!(
        Href::parse("/c/a%7E%41").map(|h| h.as_str().to_owned()),
        Ok("/c/a~A".into())
    );
}

#[test]
fn a_book_contains_only_its_own_cards() {
    let book = url("https://dav.example.com/books/alice/contacts");
    assert!(book.contains(&href("/books/alice/contacts/a.vcf")));
    assert!(!book.contains(&href("/books/alice/contacts/")));
    assert!(!book.contains(&href("/books/alice/contacts-bis/a.vcf")));
    assert!(!book.contains(&href("/calendars/alice/perso/evt.ics")));
    // Ressources hors du carnet : ni plan, ni écriture.
    let links = [link(
        "/calendars/alice/evt.ics",
        "1",
        linked(1),
        "Alice|alice@example.com",
    )];
    let actions = plan_sync(
        &links,
        &[local(1, "Alice", "alice@example.com", 10)],
        &[],
        &[],
        &[],
        100,
    );
    let actions = guard_deletions(actions, &links, &book, true);
    assert!(
        actions
            .iter()
            .all(|a| !matches!(a, SyncAction::DeleteLocal { .. })),
        "{actions:?}"
    );
}

#[test]
fn etags_are_visible_ascii() {
    assert!(ETag::parse("\"é\"").is_none());
    assert!(ETag::parse("W/\"a-b\"").is_some());
}

#[test]
fn cards_postponed_for_the_budget_wait_without_any_write() {
    let links = [link("/c/a.vcf", "1", linked(1), "ancien")];
    let later = [href("/c/a.vcf"), href("/c/n.vcf")];
    let actions = plan_sync(
        &links,
        &[local(1, "Alice", "alice@example.com", 10)],
        &[entry("/c/a.vcf", "2"), entry("/c/n.vcf", "1")],
        &[],
        &later,
        100,
    );
    assert_eq!(kinds(&actions), ["defer", "defer"]);
}

#[test]
fn a_card_moved_on_the_server_stays_linked_to_its_contact() {
    let links = [link(
        "/c/old.vcf",
        "1",
        linked(1),
        "Alice|alice@example.com",
    )];
    let card = fetched(
        "/c/new.vcf",
        "1",
        Some(draft("Alice", "alice@example.com")),
        None,
    );
    let actions = sync_plan(
        &links,
        &[local(1, "Alice", "alice@example.com", 10)],
        &[entry("/c/new.vcf", "1")],
        &[card],
        100,
    );
    assert_eq!(kinds(&actions), ["link", "forget"]);
}

fn linked_links(n: i64) -> Vec<SyncLink> {
    (1..=n)
        .map(|i| link(&format!("/c/{i}.vcf"), "1", linked(i), "s"))
        .collect()
}

#[test]
fn mass_local_deletions_are_held_back() {
    let book = url("https://dav.example.com/c/");
    let deletions = |n: i64| -> Vec<SyncAction> {
        (1..=n)
            .map(|i| SyncAction::DeleteLocal {
                href: href(&format!("/c/{i}.vcf")),
                contact: contact(i, "X", &format!("x{i}@example.com")),
            })
            .collect()
    };
    // Quelques suppressions : appliquées.
    let links = linked_links(40);
    let kept = guard_deletions(deletions(3), &links, &book, true);
    assert_eq!(kinds(&kept), ["delete-local"; 3]);
    // Liste du serveur pas sûre (réponses omises, liste vide) : aucune.
    let held = guard_deletions(deletions(3), &links, &book, false);
    assert!(held.iter().all(|a| a.kind() == "defer"), "{held:?}");
    // Plus de la moitié des fiches reliées, au-delà de 10 : retenues.
    let held = guard_deletions(deletions(25), &links, &book, true);
    assert!(held.iter().all(|a| a.kind() == "defer"), "{held:?}");
    // Petit carnet : la proportion ne joue pas sous le seuil.
    let small = linked_links(3);
    let kept = guard_deletions(deletions(2), &small, &book, true);
    assert_eq!(kinds(&kept), ["delete-local"; 2]);
}

#[test]
fn a_pushed_change_keeps_name_and_organization_components() {
    let remote = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Ancien;Paul;Marie;Dr;Jr\r\nORG:Ancienne;Service\\; compta;Bureau\r\nEND:VCARD\r\n";
    let ours = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Martin;Alice;;;\r\nORG:ACME\r\nEND:VCARD\r\n";
    let merged = merge_card(remote, ours, 0);
    assert!(
        merged.contains("N:Martin;Alice;Marie;Dr;Jr\r\n"),
        "{merged}"
    );
    assert!(
        merged.contains("ORG:ACME;Service\\; compta;Bureau\r\n"),
        "{merged}"
    );
}
