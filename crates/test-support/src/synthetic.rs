//! Générateur de boîtes synthétiques (bench et tests de volume) :
//! messages déterministes (même graine, mêmes messages), dates étalées,
//! expéditeurs et sujets variés, un tiers non lus, quelques pièces jointes ;
//! carnets d'adresses synthétiques ([`synthetic_contacts`]).

use norelly_domain::contacts::{ContactDraft, ContactInput, EmailInput, PhoneInput};
use norelly_domain::fetch_policy::{Disposition, MimePart};
use norelly_domain::mailbox::{Correspondent, Flags, MessageHeaders, MessageMeta, ModSeq, Uid};
use norelly_domain::ports::{StoredBody, StoredMessage};

/// Générateur pseudo-aléatoire minimal (xorshift64*), déterministe.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Entier dans `[0, n)` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
}

const WORDS: &[&str] = &[
    "réunion",
    "facture",
    "projet",
    "rappel",
    "compte rendu",
    "invitation",
    "devis",
    "livraison",
    "question",
    "mise à jour",
    "contrat",
    "planning",
    "budget",
    "relance",
    "bilan",
    "commande",
];
const NAMES: &[&str] = &[
    "Alice Martin",
    "Bruno Petit",
    "Chloé Durand",
    "David Leroy",
    "Émilie Moreau",
    "François Simon",
    "Gaëlle Laurent",
    "Hugo Michel",
    "Inès Garcia",
    "Julien Roux",
];

/// `count` messages, UID 1 à `count`, dates croissantes à partir de
/// `start` (secondes Unix), espacées de 1 à 30 minutes ; environ un sur
/// trois répond à un message précédent.
#[must_use]
pub fn synthetic_messages(count: u32, start: i64, seed: u64) -> Vec<StoredMessage> {
    let mut rng = Rng::new(seed);
    let mut date = start;
    (1..=count)
        .filter_map(|n| {
            date += i64::try_from(60 + rng.below(1800)).unwrap_or(60);
            let name = NAMES[usize::try_from(rng.below(NAMES.len() as u64)).unwrap_or(0)];
            let word = WORDS[usize::try_from(rng.below(WORDS.len() as u64)).unwrap_or(0)];
            let seen = rng.below(3) != 0;
            let mut flags = Flags::from_tokens(if seen { vec!["\\Seen"] } else { vec![] });
            flags.flagged = rng.below(20) == 0;
            let attachment = rng.below(10) == 0;
            let structure = attachment.then(|| MimePart::Multipart {
                subtype: "mixed".into(),
                parts: vec![
                    MimePart::Leaf {
                        media_type: "text".into(),
                        subtype: "plain".into(),
                        size: 2_000,
                        disposition: Disposition::Unspecified,
                        filename: None,
                    },
                    MimePart::Leaf {
                        media_type: "application".into(),
                        subtype: "pdf".into(),
                        size: 200_000,
                        disposition: Disposition::Attachment,
                        filename: Some(format!("{word}-{n}.pdf")),
                    },
                ],
            });
            let login = name.split(' ').next().unwrap_or("x").to_lowercase();
            // Un message sur trois répond à un message précédent
            // (conversations).
            let parent = (n > 1 && rng.below(3) == 0)
                .then(|| u32::try_from(1 + rng.below(u64::from(n - 1))).unwrap_or(1));
            let (subject, references) = match parent {
                Some(p) => (
                    format!("Re: {word} n° {p}"),
                    vec![format!("{p}.{seed}@synthetic.example")],
                ),
                None => (format!("{word} n° {n}"), Vec::new()),
            };
            Some(StoredMessage {
                meta: MessageMeta {
                    uid: Uid::new(n)?,
                    flags,
                    modseq: ModSeq::new(u64::from(n)),
                    internal_date: date,
                    size: u32::try_from(2_000 + rng.below(50_000)).unwrap_or(2_000),
                    headers: MessageHeaders {
                        subject,
                        references,
                        from: vec![Correspondent::new(name, &format!("{login}@example.com"))],
                        to: vec![Correspondent::new("Moi", "moi@example.com")],
                        date: Some(date),
                        message_id: Some(format!("{n}.{seed}@synthetic.example")),
                        ..MessageHeaders::default()
                    },
                    structure,
                },
                body: StoredBody::Missing,
            })
        })
        .collect()
}

const FILLER: &[&str] = &[
    "bonjour",
    "merci",
    "pour",
    "votre",
    "message",
    "nous",
    "avons",
    "bien",
    "reçu",
    "le",
    "document",
    "joint",
    "cordialement",
    "semaine",
    "prochaine",
    "équipe",
    "client",
    "dossier",
    "délai",
    "prévu",
    "suite",
    "échange",
    "téléphone",
    "vendredi",
    "matin",
    "après-midi",
    "proposition",
    "validation",
    "rapport",
    "version",
    "modifications",
    "retour",
    "urgent",
    "information",
    "sujet",
    "réponse",
    "demande",
    "point",
    "étape",
    "calendrier",
];

/// Texte de corps déterministe d'environ `words` mots (vocabulaire courant,
/// sujets du générateur, un nom propre rare par message : `ref{n}`).
#[must_use]
pub fn synthetic_text(n: u32, seed: u64, words: usize) -> String {
    let mut rng = Rng::new(seed.wrapping_mul(1_000_003).wrapping_add(u64::from(n)));
    let mut out: Vec<String> = Vec::with_capacity(words + 1);
    for i in 0..words {
        let pick = |rng: &mut Rng, list: &[&str]| {
            list[usize::try_from(rng.below(list.len() as u64)).unwrap_or(0)].to_owned()
        };
        out.push(if i % 7 == 3 {
            pick(&mut rng, WORDS)
        } else {
            pick(&mut rng, FILLER)
        });
    }
    out.push(format!("ref{n}"));
    out.join(" ")
}

const GIVEN_NAMES: &[&str] = &[
    "Alice",
    "Bruno",
    "Chloé",
    "David",
    "Émilie",
    "François",
    "Gaëlle",
    "Hugo",
    "Inès",
    "Julien",
    "Karine",
    "Louis",
    "Manon",
    "Nicolas",
    "Océane",
    "Pierre",
    "Quentin",
    "Rose",
    "Samuel",
    "Théo",
];
const FAMILY_NAMES: &[&str] = &[
    "Martin",
    "Petit",
    "Durand",
    "Leroy",
    "Moreau",
    "Simon",
    "Laurent",
    "Michel",
    "Garcia",
    "Roux",
    "Bernard",
    "Thomas",
    "Robert",
    "Richard",
    "Dubois",
    "Lefebvre",
    "Fontaine",
    "Chevalier",
    "Mercier",
    "Blanc",
];
const ORGANIZATIONS: &[&str] = &["ACME", "Initech", "Globex", "Umbrella", "Hooli", ""];

/// `count` fiches de contact valides et déterministes (même graine, mêmes
/// fiches) : prénom, nom, organisation, une ou deux adresses uniques
/// (`<prénom>.<nom>.<n>@s<graine>.example`), un téléphone, un favori sur
/// cinquante environ. Deux graines différentes ne partagent aucune adresse.
#[must_use]
pub fn synthetic_contacts(count: usize, seed: u64) -> Vec<ContactDraft> {
    let mut rng = Rng::new(seed.wrapping_mul(7_919).wrapping_add(3));
    let pick = |rng: &mut Rng, list: &[&'static str]| {
        list[usize::try_from(rng.below(list.len() as u64)).unwrap_or(0)]
    };
    (0..count)
        .filter_map(|n| {
            let given = pick(&mut rng, GIVEN_NAMES);
            let family = pick(&mut rng, FAMILY_NAMES);
            let organization = pick(&mut rng, ORGANIZATIONS);
            let login = format!("{}.{}.{n}", ascii(given), ascii(family));
            let mut emails = vec![EmailInput {
                address: format!("{login}@s{seed}.example"),
                label: "travail".into(),
                preferred: true,
            }];
            if rng.below(4) == 0 {
                emails.push(EmailInput {
                    address: format!("{login}@perso.s{seed}.example"),
                    ..EmailInput::default()
                });
            }
            ContactDraft::new(ContactInput {
                given_name: given.into(),
                family_name: family.into(),
                organization: organization.into(),
                emails,
                phones: vec![PhoneInput {
                    number: format!("+33 6 {:08}", rng.below(100_000_000)),
                    label: "mobile".into(),
                }],
                favorite: rng.below(50) == 0,
                ..ContactInput::default()
            })
            .ok()
        })
        .collect()
}

/// Forme ASCII minuscule d'un prénom ou d'un nom, pour une adresse.
fn ascii(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'é' | 'É' | 'è' | 'ë' => 'e',
            'ç' => 'c',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_contacts_are_deterministic_valid_and_distinct() {
        let a = synthetic_contacts(500, 1);
        assert_eq!(a.len(), 500);
        assert_eq!(a, synthetic_contacts(500, 1));
        let mut keys: Vec<String> = a
            .iter()
            .chain(&synthetic_contacts(500, 2))
            .flat_map(|c| c.emails().iter().map(|e| e.key().as_str().to_owned()))
            .collect();
        let total = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), total, "adresse partagée");
        let favorites = a.iter().filter(|c| c.favorite()).count();
        assert!((1..50).contains(&favorites), "{favorites}");
    }

    #[test]
    fn synthetic_text_is_deterministic_and_distinct() {
        let a = synthetic_text(5, 1, 80);
        assert_eq!(a, synthetic_text(5, 1, 80));
        assert_ne!(a, synthetic_text(6, 1, 80));
        assert!(a.ends_with(" ref5"), "{a}");
        assert!(a.split_whitespace().count() >= 80);
    }

    #[test]
    fn generator_is_deterministic_and_complete() {
        let a = synthetic_messages(1000, 1_700_000_000, 7);
        let b = synthetic_messages(1000, 1_700_000_000, 7);
        assert_eq!(a, b);
        assert_eq!(a.len(), 1000);
        assert_eq!(a.last().map(|m| m.meta.uid.get()), Some(1000));
        let unread = a.iter().filter(|m| !m.meta.flags.seen).count();
        assert!((200..500).contains(&unread), "{unread}");
        assert!(
            a.windows(2)
                .all(|w| matches!(w, [x, y] if x.meta.internal_date < y.meta.internal_date))
        );
        assert_ne!(a, synthetic_messages(1000, 1_700_000_000, 8));
    }
}
