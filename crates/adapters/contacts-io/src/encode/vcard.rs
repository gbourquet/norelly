//! Écriture vCard 4.0 (RFC 6350) et 3.0 (RFC 2426) : `FN`, `N`,
//! `NICKNAME`, `ORG`, `EMAIL` (type, préférence), `TEL`, `NOTE` ;
//! échappements des valeurs texte, lignes repliées à 75 octets sans couper
//! un caractère, fins de ligne CRLF.

use norelly_domain::contact_export::ExportCard;
use norelly_domain::ports::ContactCodecError;

use super::Output;

/// Version écrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Version {
    V4,
    V3,
}

/// Valeur texte échappée : `\`, `,`, `;` et saut de ligne.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// Longueur maximale d'une ligne physique, en octets (RFC 6350 § 3.2).
const MAX_LINE_OCTETS: usize = 75;

/// Ligne logique repliée : au plus 75 octets par ligne physique (espace
/// de continuation comprise), coupée entre deux caractères, CRLF final.
fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + line.len() / 60 * 3 + 2);
    // Octets encore permis sur la ligne physique en cours.
    let mut room = MAX_LINE_OCTETS;
    for c in line.chars() {
        if c.len_utf8() > room {
            out.push_str("\r\n ");
            room = MAX_LINE_OCTETS - 1;
        }
        out.push(c);
        room -= c.len_utf8();
    }
    out.push_str("\r\n");
    out
}

fn optional(value: Option<&String>) -> String {
    value.map_or_else(String::new, |v| escape(v))
}

/// Cartes `cards` dans `out`.
pub(crate) fn encode(
    cards: &[ExportCard],
    version: Version,
    out: &mut Output,
) -> Result<(), ContactCodecError> {
    let version_line = match version {
        Version::V4 => "VERSION:4.0",
        Version::V3 => "VERSION:3.0",
    };
    for card in cards {
        let mut lines = vec![
            "BEGIN:VCARD".to_owned(),
            version_line.to_owned(),
            format!("FN:{}", optional(card.full_name.as_ref())),
            format!(
                "N:{};{};;;",
                optional(card.family_name.as_ref()),
                optional(card.given_name.as_ref())
            ),
        ];
        if let Some(nickname) = &card.nickname {
            lines.push(format!("NICKNAME:{}", escape(nickname)));
        }
        if let Some(organization) = &card.organization {
            lines.push(format!("ORG:{}", escape(organization)));
        }
        for email in &card.emails {
            let label = email.label.as_deref();
            let params = match version {
                Version::V4 => {
                    let mut params = String::new();
                    if let Some(label) = label {
                        params.push_str(&format!(";TYPE={label}"));
                    }
                    if email.preferred {
                        params.push_str(";PREF=1");
                    }
                    params
                }
                Version::V3 => {
                    let mut types = vec!["INTERNET"];
                    types.extend(label);
                    if email.preferred {
                        types.push("PREF");
                    }
                    format!(";TYPE={}", types.join(","))
                }
            };
            lines.push(format!("EMAIL{params}:{}", escape(&email.address)));
        }
        for phone in &card.phones {
            let params = phone
                .label
                .as_deref()
                .map_or_else(String::new, |label| format!(";TYPE={label}"));
            lines.push(format!("TEL{params}:{}", escape(&phone.number)));
        }
        if let Some(notes) = &card.notes {
            lines.push(format!("NOTE:{}", escape(notes)));
        }
        lines.push("END:VCARD".to_owned());
        for line in lines {
            out.push(&fold(&line))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use norelly_domain::contact_export::{ExportEmail, ExportPhone};

    use super::*;

    fn written(cards: &[ExportCard], version: Version) -> String {
        let mut out = Output::new();
        encode(cards, version, &mut out).unwrap_or_else(|e| panic!("{e}"));
        String::from_utf8(out.into_bytes()).unwrap_or_else(|e| panic!("{e}"))
    }

    fn card() -> ExportCard {
        ExportCard {
            full_name: Some("Anne, dite; \"Nanou\"".into()),
            given_name: Some("Anne".into()),
            family_name: Some("Martin".into()),
            nickname: Some("Nanou, Nan".into()),
            organization: Some("ACME; R&D".into()),
            notes: Some("a\\b\nc\td".into()),
            emails: vec![
                ExportEmail {
                    address: "anne@example.com".into(),
                    label: Some("travail".into()),
                    preferred: true,
                },
                ExportEmail {
                    address: "anne@example.org".into(),
                    label: None,
                    preferred: false,
                },
            ],
            phones: vec![ExportPhone {
                number: "+33 6 12".into(),
                label: Some("portable".into()),
            }],
        }
    }

    #[test]
    fn escape_handles_backslash_comma_semicolon_and_newline() {
        assert_eq!(escape("a\\b,c;d\ne:f"), "a\\\\b\\,c\\;d\\ne:f");
    }

    #[test]
    fn fold_keeps_75_octets_and_whole_characters() {
        let short = "FN:court";
        assert_eq!(fold(short), "FN:court\r\n");
        let long = format!("NOTE:{}", "é".repeat(60));
        let folded = fold(&long);
        let lines: Vec<&str> = folded.trim_end_matches("\r\n").split("\r\n").collect();
        assert!(lines.len() > 1, "{folded:?}");
        for (i, line) in lines.iter().enumerate() {
            assert!(line.len() <= 75, "{line:?}");
            assert_eq!(i > 0, line.starts_with(' '), "{line:?}");
        }
        let unfolded: String = lines
            .iter()
            .enumerate()
            .map(|(i, l)| if i == 0 { *l } else { &l[1..] })
            .collect();
        assert_eq!(unfolded, long);
        // Exactement 75 octets : pas de repli.
        let exact = format!("NOTE:{}", "a".repeat(70));
        assert_eq!(fold(&exact), format!("{exact}\r\n"));
    }

    #[test]
    fn version_4_card() {
        assert_eq!(
            written(&[card()], Version::V4),
            "BEGIN:VCARD\r\n\
             VERSION:4.0\r\n\
             FN:Anne\\, dite\\; \"Nanou\"\r\n\
             N:Martin;Anne;;;\r\n\
             NICKNAME:Nanou\\, Nan\r\n\
             ORG:ACME\\; R&D\r\n\
             EMAIL;TYPE=travail;PREF=1:anne@example.com\r\n\
             EMAIL:anne@example.org\r\n\
             TEL;TYPE=portable:+33 6 12\r\n\
             NOTE:a\\\\b\\nc\td\r\n\
             END:VCARD\r\n"
        );
    }

    #[test]
    fn version_3_card_marks_internet_addresses_and_preference_as_types() {
        assert_eq!(
            written(&[card()], Version::V3),
            "BEGIN:VCARD\r\n\
             VERSION:3.0\r\n\
             FN:Anne\\, dite\\; \"Nanou\"\r\n\
             N:Martin;Anne;;;\r\n\
             NICKNAME:Nanou\\, Nan\r\n\
             ORG:ACME\\; R&D\r\n\
             EMAIL;TYPE=INTERNET,travail,PREF:anne@example.com\r\n\
             EMAIL;TYPE=INTERNET:anne@example.org\r\n\
             TEL;TYPE=portable:+33 6 12\r\n\
             NOTE:a\\\\b\\nc\td\r\n\
             END:VCARD\r\n"
        );
    }

    #[test]
    fn empty_fields_are_omitted_but_fn_and_n_are_always_written() {
        let bare = ExportCard {
            emails: vec![ExportEmail {
                address: "x@example.com".into(),
                label: None,
                preferred: true,
            }],
            ..ExportCard::default()
        };
        assert_eq!(
            written(&[bare.clone(), bare], Version::V4),
            "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:\r\nN:;;;;\r\nEMAIL;PREF=1:x@example.com\r\nEND:VCARD\r\n"
                .repeat(2)
        );
    }

    #[test]
    fn output_limit_stops_the_writing() {
        let mut out = Output::with_limit(40);
        assert_eq!(
            encode(&[card()], Version::V4, &mut out),
            Err(ContactCodecError::TooLarge)
        );
    }
}
