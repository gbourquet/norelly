//! Écriture LDIF (RFC 2849) telle que l'exporte Thunderbird : un
//! enregistrement `inetOrgPerson` / `mozillaAbPersonAlpha` par fiche,
//! séparés par une ligne vide ; valeur en base64 (`attr:: …`) quand elle
//! n'est pas une chaîne sûre ; lignes repliées à 76 octets. Les listes
//! suivent les fiches, en enregistrements `groupOfNames` dont chaque
//! membre est le nom distinctif de sa fiche, copie cachée notée
//! `x-norelly-bcc: TRUE`.

use norelly_domain::contact_export::{ExportCard, ExportList};
use norelly_domain::ports::ContactCodecError;

use super::Output;

/// La valeur peut-elle s'écrire telle quelle (RFC 2849 `SAFE-STRING`,
/// sans espace finale) ?
fn is_safe(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes
        .iter()
        .all(|&b| b.is_ascii() && !matches!(b, b'\0' | b'\n' | b'\r'))
        && !matches!(bytes.first(), Some(b' ' | b':' | b'<'))
        && bytes.last() != Some(&b' ')
}

/// Longueur maximale d'une ligne physique, en octets (RFC 2849 § 2).
const MAX_LINE_OCTETS: usize = 76;

/// Ligne `attr: valeur` ou `attr:: base64`, repliée, CRLF final.
fn line(attribute: &str, value: &str) -> String {
    let logical = if is_safe(value) {
        format!("{attribute}: {value}")
    } else {
        format!("{attribute}:: {}", super::base64(value.as_bytes()))
    };
    // Ligne logique toute en ASCII : le repli peut couper n'importe où.
    let mut out = String::with_capacity(logical.len() + logical.len() / 70 * 3 + 2);
    let mut rest = logical.as_str();
    let mut room = MAX_LINE_OCTETS;
    while rest.len() > room {
        let (head, tail) = rest.split_at(room);
        out.push_str(head);
        out.push_str("\r\n ");
        rest = tail;
        room = MAX_LINE_OCTETS - 1;
    }
    out.push_str(rest);
    out.push_str("\r\n");
    out
}

/// Valeur d'un attribut d'un nom distinctif (RFC 4514) : `,`, `+`, `"`,
/// `\`, `<`, `>`, `;` et `=` échappés, ainsi qu'une espace ou un `#`
/// initial et une espace finale.
fn rdn_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let last = value.chars().count().saturating_sub(1);
    for (i, c) in value.chars().enumerate() {
        let edge = (i == 0 && matches!(c, ' ' | '#')) || (i == last && c == ' ');
        if edge || matches!(c, ',' | '+' | '"' | '\\' | '<' | '>' | ';' | '=') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Attribut Thunderbird d'un téléphone, d'après son étiquette (fixée par
/// le domaine).
fn phone_attribute(label: Option<&str>) -> &'static str {
    match label {
        Some("home") => "homePhone",
        Some("cell") => "mobile",
        Some("fax") => "facsimileTelephoneNumber",
        Some("pager") => "pager",
        _ => "telephoneNumber",
    }
}

/// Attributs Thunderbird d'une adresse postale au domicile : rue, suite de
/// la rue, ville, région, code postal, pays (étiquette fixée par le
/// domaine à `home`).
const HOME_PLACE: [&str; 6] = [
    "mozillaHomeStreet",
    "mozillaHomeStreet2",
    "mozillaHomeLocalityName",
    "mozillaHomeState",
    "mozillaHomePostalCode",
    "mozillaHomeCountryName",
];

/// Attributs Thunderbird d'une adresse postale au travail, dans le même
/// ordre.
const WORK_PLACE: [&str; 6] = ["street", "mozillaWorkStreet2", "l", "st", "postalCode", "c"];

const OBJECT_CLASSES: [&str; 5] = [
    "top",
    "person",
    "organizationalPerson",
    "inetOrgPerson",
    "mozillaAbPersonAlpha",
];

/// Fiches `cards` dans `out`.
pub(crate) fn encode(
    cards: &[ExportCard],
    lists: &[ExportList],
    out: &mut Output,
) -> Result<(), ContactCodecError> {
    for card in cards {
        out.push(&line("dn", &dn(card)))?;
        for class in OBJECT_CLASSES {
            out.push(&line("objectclass", class))?;
        }
        let fields = [
            ("givenName", &card.given_name),
            ("sn", &card.family_name),
            ("cn", &card.full_name),
            ("mozillaNickName", &card.nickname),
        ];
        for (attribute, value) in fields {
            if let Some(value) = value {
                out.push(&line(attribute, value))?;
            }
        }
        for (i, email) in card.emails.iter().enumerate() {
            let attribute = if i == 1 { "mozillaSecondEmail" } else { "mail" };
            out.push(&line(attribute, &email.address))?;
        }
        for phone in &card.phones {
            out.push(&line(
                phone_attribute(phone.label.as_deref()),
                &phone.number,
            ))?;
        }
        if let Some(organization) = &card.organization {
            out.push(&line("o", organization))?;
        }
        if let Some(title) = &card.title {
            out.push(&line("title", title))?;
        }
        if let Some(birthday) = card.birthday {
            if let Some(year) = birthday.year() {
                out.push(&line("birthyear", &year.to_string()))?;
            }
            out.push(&line("birthmonth", &format!("{:02}", birthday.month())))?;
            out.push(&line("birthday", &format!("{:02}", birthday.day())))?;
        }
        for place in &card.postal_addresses {
            let names = if place.label.as_deref() == Some("home") {
                &HOME_PLACE
            } else {
                &WORK_PLACE
            };
            // Première ligne de la rue, puis les suivantes ensemble.
            let (first, rest) = match place.street.as_deref().map(|s| s.split_once('\n')) {
                Some(Some((first, rest))) => (Some(first), Some(rest)),
                Some(None) => (place.street.as_deref(), None),
                None => (None, None),
            };
            let values = [
                first,
                rest,
                place.locality.as_deref(),
                place.region.as_deref(),
                place.postal_code.as_deref(),
                place.country.as_deref(),
            ];
            for (attribute, value) in names.iter().zip(values) {
                if let Some(value) = value {
                    out.push(&line(attribute, value))?;
                }
            }
        }
        for site in &card.websites {
            let attribute = if site.label.as_deref() == Some("home") {
                "mozillaHomeUrl"
            } else {
                "mozillaWorkUrl"
            };
            out.push(&line(attribute, &site.url))?;
        }
        if let Some(notes) = &card.notes {
            out.push(&line("description", notes))?;
        }
        out.push("\r\n")?;
    }
    for list in lists {
        out.push(&line("dn", &format!("cn={}", rdn_value(&list.name))))?;
        out.push(&line("objectclass", "top"))?;
        out.push(&line("objectclass", "groupOfNames"))?;
        out.push(&line("cn", &list.name))?;
        if let Some(description) = &list.description {
            out.push(&line("description", description))?;
        }
        if list.bcc {
            out.push(&line("x-norelly-bcc", "TRUE"))?;
        }
        for card in list.members.iter().filter_map(|&i| cards.get(i)) {
            out.push(&line("member", &dn(card)))?;
        }
        out.push("\r\n")?;
    }
    Ok(())
}

/// Nom distinctif d'une fiche, comme Thunderbird : `cn=<nom>,mail=<adresse
/// préférée>`, ou `mail=<adresse>` sans nom.
fn dn(card: &ExportCard) -> String {
    let preferred = card.emails.first().map_or("", |e| e.address.as_str());
    let mail = format!("mail={}", rdn_value(preferred));
    match &card.full_name {
        Some(name) => format!("cn={},{mail}", rdn_value(name)),
        None => mail,
    }
}

#[cfg(test)]
mod tests {
    use norelly_domain::contact_export::{ExportEmail, ExportPhone};

    use super::*;

    fn written(cards: &[ExportCard]) -> String {
        let mut out = Output::new();
        encode(cards, &[], &mut out).unwrap_or_else(|e| panic!("{e}"));
        String::from_utf8(out.into_bytes()).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn safe_strings_follow_rfc_2849() {
        assert!(is_safe("Anne Martin"));
        assert!(is_safe("a:b<c"));
        assert!(is_safe("tab\tok"));
        assert!(is_safe(""));
        for unsafe_value in [
            " lead", ":colon", "<angle", "trail ", "é", "a\nb", "a\rb", "a\0b",
        ] {
            assert!(!is_safe(unsafe_value), "{unsafe_value:?}");
        }
    }

    #[test]
    fn unsafe_values_are_base64_and_long_lines_folded() {
        assert_eq!(line("cn", "Anne"), "cn: Anne\r\n");
        assert_eq!(line("cn", "Hélène"), "cn:: SMOpbMOobmU=\r\n");
        let long = "a".repeat(100);
        let folded = line("description", &long);
        let lines: Vec<&str> = folded.trim_end_matches("\r\n").split("\r\n").collect();
        assert_eq!(lines.len(), 2, "{folded:?}");
        assert!(lines.iter().all(|l| l.len() <= 76));
        assert!(lines[1].starts_with(' '));
        assert_eq!(
            format!("{}{}", lines[0], &lines[1][1..]),
            format!("description: {long}")
        );
    }

    #[test]
    fn rdn_values_are_escaped() {
        assert_eq!(rdn_value("Martin, Anne"), "Martin\\, Anne");
        assert_eq!(
            rdn_value("a+b=\"c\";<d>\\"),
            "a\\+b\\=\\\"c\\\"\\;\\<d\\>\\\\"
        );
        assert_eq!(rdn_value("#x "), "\\#x\\ ");
        assert_eq!(rdn_value(" x"), "\\ x");
    }

    #[test]
    fn thunderbird_record() {
        let card = ExportCard {
            photo: None,
            full_name: Some("Anne Martin".into()),
            given_name: Some("Anne".into()),
            family_name: Some("Martin".into()),
            nickname: Some("Nanou".into()),
            organization: Some("ACME".into()),
            notes: Some("Deux\nlignes".into()),
            emails: vec![
                ExportEmail {
                    address: "anne@example.com".into(),
                    label: None,
                    preferred: true,
                },
                ExportEmail {
                    address: "anne@example.org".into(),
                    label: None,
                    preferred: false,
                },
                ExportEmail {
                    address: "anne@example.net".into(),
                    label: None,
                    preferred: false,
                },
            ],
            phones: ["work", "home", "cell", "fax", "pager"]
                .iter()
                .enumerate()
                .map(|(i, label)| ExportPhone {
                    number: format!("0{i}"),
                    label: Some((*label).to_owned()),
                })
                .collect(),
            ..ExportCard::default()
        };
        assert_eq!(
            written(&[card]),
            "dn: cn=Anne Martin,mail=anne@example.com\r\n\
             objectclass: top\r\n\
             objectclass: person\r\n\
             objectclass: organizationalPerson\r\n\
             objectclass: inetOrgPerson\r\n\
             objectclass: mozillaAbPersonAlpha\r\n\
             givenName: Anne\r\n\
             sn: Martin\r\n\
             cn: Anne Martin\r\n\
             mozillaNickName: Nanou\r\n\
             mail: anne@example.com\r\n\
             mozillaSecondEmail: anne@example.org\r\n\
             mail: anne@example.net\r\n\
             telephoneNumber: 00\r\n\
             homePhone: 01\r\n\
             mobile: 02\r\n\
             facsimileTelephoneNumber: 03\r\n\
             pager: 04\r\n\
             o: ACME\r\n\
             description:: RGV1eApsaWduZXM=\r\n\
             \r\n"
        );
    }

    #[test]
    fn details_use_thunderbird_attributes() {
        use norelly_domain::contact_details::Birthday;
        use norelly_domain::contact_export::{ExportPostalAddress, ExportWebsite};
        let card = ExportCard {
            title: Some("Chef".into()),
            birthday: Birthday::new(Some(1985), 4, 12),
            emails: vec![ExportEmail {
                address: "x@example.com".into(),
                label: None,
                preferred: true,
            }],
            postal_addresses: vec![
                ExportPostalAddress {
                    street: Some("1 rue A\nBat. B\nEsc. 3".into()),
                    locality: Some("Lyon".into()),
                    region: Some("Rhone".into()),
                    postal_code: Some("69001".into()),
                    country: Some("France".into()),
                    label: Some("work".into()),
                },
                ExportPostalAddress {
                    street: Some("2 rue B".into()),
                    locality: Some("Nantes".into()),
                    label: Some("home".into()),
                    ..ExportPostalAddress::default()
                },
            ],
            websites: vec![
                ExportWebsite {
                    url: "https://example.org".into(),
                    label: Some("home".into()),
                },
                ExportWebsite {
                    url: "https://example.com".into(),
                    label: Some("work".into()),
                },
            ],
            ..ExportCard::default()
        };
        let text = written(&[card]);
        let details: Vec<&str> = text
            .split("\r\n")
            .skip_while(|l| !l.starts_with("mail:"))
            .skip(1)
            .collect();
        assert_eq!(
            details,
            [
                "title: Chef",
                "birthyear: 1985",
                "birthmonth: 04",
                "birthday: 12",
                "street: 1 rue A",
                "mozillaWorkStreet2:: QmF0LiBCCkVzYy4gMw==",
                "l: Lyon",
                "st: Rhone",
                "postalCode: 69001",
                "c: France",
                "mozillaHomeStreet: 2 rue B",
                "mozillaHomeLocalityName: Nantes",
                "mozillaHomeUrl: https://example.org",
                "mozillaWorkUrl: https://example.com",
                "",
                "",
            ]
        );
        let no_year = ExportCard {
            birthday: Birthday::new(None, 2, 29),
            ..ExportCard::default()
        };
        let text = written(&[no_year]);
        assert!(
            text.contains("\r\nbirthmonth: 02\r\nbirthday: 29\r\n"),
            "{text}"
        );
        assert!(!text.contains("birthyear"), "{text}");
    }

    #[test]
    fn a_card_without_name_is_named_by_its_address() {
        let card = ExportCard {
            emails: vec![ExportEmail {
                address: "x@example.com".into(),
                label: None,
                preferred: true,
            }],
            ..ExportCard::default()
        };
        assert!(written(&[card]).starts_with("dn: mail=x@example.com\r\nobjectclass: top\r\n"));
        let accented = ExportCard {
            full_name: Some("Zoé".into()),
            emails: vec![ExportEmail {
                address: "z@example.com".into(),
                label: None,
                preferred: true,
            }],
            ..ExportCard::default()
        };
        // Nom distinctif non ASCII : en base64, comme Thunderbird.
        assert!(written(&[accented]).starts_with("dn:: Y249Wm/DqSxtYWlsPXpAZXhhbXBsZS5jb20=\r\n"));
    }

    #[test]
    fn output_limit_stops_the_writing() {
        let card = ExportCard {
            emails: vec![ExportEmail {
                address: "x@example.com".into(),
                label: None,
                preferred: true,
            }],
            ..ExportCard::default()
        };
        let mut out = Output::with_limit(20);
        assert_eq!(
            encode(&[card], &[], &mut out),
            Err(ContactCodecError::TooLarge)
        );
    }
}
