//! Politique de rapatriement des corps : en-têtes et texte pour tous les
//! messages ; pièces jointes seulement sous un seuil configurable (les
//! autres seront téléchargées à la demande).
//!
//! La décision s'appuie sur la structure MIME annoncée par le serveur
//! (`BODYSTRUCTURE`), convertie en [`MimePart`] par l'adaptateur IMAP.

use serde::{Deserialize, Serialize};

/// Profondeur maximale d'imbrication acceptée dans une structure MIME.
pub const MAX_MIME_DEPTH: usize = 16;
/// Nombre maximal de parties feuilles prises en compte.
pub const MAX_MIME_PARTS: usize = 256;

/// Chemin d'une partie (`1.2.3`), au sens des sections IMAP.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PartPath(Vec<u32>);

impl PartPath {
    /// Chemin à partir de ses composantes ; `None` si l'une est nulle ou
    /// si le chemin est vide ou trop profond.
    #[must_use]
    pub fn new(parts: Vec<u32>) -> Option<Self> {
        let valid = !parts.is_empty() && parts.len() <= MAX_MIME_DEPTH && !parts.contains(&0);
        valid.then_some(Self(parts))
    }

    /// Lecture de la forme `1.2.3`.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let parts: Option<Vec<u32>> = raw.split('.').map(|p| p.parse().ok()).collect();
        Self::new(parts?)
    }

    #[must_use]
    pub fn components(&self) -> &[u32] {
        &self.0
    }

    fn child(&self, index: u32) -> Option<Self> {
        let mut parts = self.0.clone();
        parts.push(index);
        Self::new(parts)
    }
}

impl std::fmt::Display for PartPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut first = true;
        for p in &self.0 {
            if !first {
                f.write_str(".")?;
            }
            first = false;
            write!(f, "{p}")?;
        }
        Ok(())
    }
}

/// Disposition d'une partie (`Content-Disposition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Disposition {
    Inline,
    Attachment,
    /// Absente ou inconnue.
    Unspecified,
}

/// Partie MIME, d'après `BODYSTRUCTURE`. Types et sous-types en
/// minuscules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MimePart {
    Leaf {
        media_type: String,
        subtype: String,
        /// Taille encodée annoncée, en octets.
        size: u32,
        disposition: Disposition,
        /// Nom de fichier annoncé (non fiable : à assainir avant usage).
        filename: Option<String>,
    },
    Multipart {
        subtype: String,
        parts: Vec<MimePart>,
    },
    /// `message/rfc822` encapsulé : traité comme une pièce jointe.
    Message { size: u32 },
}

/// Partie feuille, avec son chemin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leaf<'a> {
    pub path: PartPath,
    pub part: &'a MimePart,
}

impl MimePart {
    /// Parties feuilles (et messages encapsulés), en ordre, bornées à
    /// [`MAX_MIME_PARTS`] et [`MAX_MIME_DEPTH`].
    ///
    /// Une partie unique (message non multipart) a le chemin `1`.
    #[must_use]
    pub fn leaves(&self) -> Vec<Leaf<'_>> {
        let mut out = Vec::new();
        match self {
            Self::Multipart { parts, .. } => {
                for (i, part) in parts.iter().enumerate() {
                    if let Some(path) = u32::try_from(i + 1)
                        .ok()
                        .and_then(|i| PartPath::new(vec![i]))
                    {
                        part.collect(path, &mut out);
                    }
                }
            }
            _ => {
                if let Some(path) = PartPath::new(vec![1]) {
                    out.push(Leaf { path, part: self });
                }
            }
        }
        out
    }

    fn collect<'a>(&'a self, path: PartPath, out: &mut Vec<Leaf<'a>>) {
        if out.len() >= MAX_MIME_PARTS {
            return;
        }
        match self {
            Self::Multipart { parts, .. } => {
                for (i, part) in parts.iter().enumerate() {
                    let child = u32::try_from(i + 1).ok().and_then(|i| path.child(i));
                    if let Some(child) = child {
                        part.collect(child, out);
                    }
                }
            }
            _ => out.push(Leaf { path, part: self }),
        }
    }

    /// Vrai pour une partie de texte lisible (corps du message) :
    /// `text/plain` ou `text/html` non marquée comme pièce jointe.
    #[must_use]
    pub fn is_body_text(&self) -> bool {
        matches!(
            self,
            Self::Leaf { media_type, subtype, disposition, .. }
                if media_type == "text"
                    && (subtype == "plain" || subtype == "html")
                    && *disposition != Disposition::Attachment
        )
    }

    /// Taille annoncée d'une feuille (0 pour un multipart).
    #[must_use]
    pub fn size(&self) -> u32 {
        match self {
            Self::Leaf { size, .. } | Self::Message { size } => *size,
            Self::Multipart { .. } => 0,
        }
    }

    /// Vrai si une partie est une pièce jointe (tout ce qui n'est pas du
    /// texte de corps, sauf les images en ligne sans nom de fichier).
    #[must_use]
    pub fn has_attachments(&self) -> bool {
        self.leaves().iter().any(|l| is_attachment(l.part))
    }
}

fn is_attachment(part: &MimePart) -> bool {
    match part {
        MimePart::Message { .. } => true,
        MimePart::Multipart { .. } => false,
        MimePart::Leaf {
            disposition,
            filename,
            ..
        } => {
            !part.is_body_text() && (*disposition == Disposition::Attachment || filename.is_some())
        }
    }
}

/// Seuil par défaut des pièces jointes rapatriées automatiquement.
pub const DEFAULT_ATTACHMENT_THRESHOLD: u32 = 2 * 1024 * 1024;

/// Politique de rapatriement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchPolicy {
    /// Les parties non textuelles plus grandes ne sont pas téléchargées
    /// pendant la synchronisation.
    pub attachment_threshold: u32,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        Self {
            attachment_threshold: DEFAULT_ATTACHMENT_THRESHOLD,
        }
    }
}

/// Ce qu'il faut télécharger d'un message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyFetch {
    /// Le message brut complet (`BODY.PEEK[]`).
    Full,
    /// En-têtes (`BODY.PEEK[HEADER]`) et ces parties seulement ; les
    /// parties de `deferred` seront téléchargées à la demande.
    Partial {
        parts: Vec<PartPath>,
        deferred: Vec<PartPath>,
    },
}

impl FetchPolicy {
    /// Décide du téléchargement d'un message de taille totale `size`.
    ///
    /// - aucune partie non textuelle au-dessus du seuil : message complet ;
    /// - sinon : en-têtes, textes, et parties sous le seuil ;
    /// - structure inconnue : complet si la taille totale est sous le
    ///   seuil, sinon en-têtes seulement (rien d'autre n'est identifiable
    ///   sans risque).
    #[must_use]
    pub fn decide(&self, size: u32, structure: Option<&MimePart>) -> BodyFetch {
        let Some(structure) = structure else {
            return if size <= self.attachment_threshold {
                BodyFetch::Full
            } else {
                BodyFetch::Partial {
                    parts: Vec::new(),
                    deferred: Vec::new(),
                }
            };
        };
        let leaves = structure.leaves();
        let too_big =
            |l: &Leaf<'_>| !l.part.is_body_text() && l.part.size() > self.attachment_threshold;
        if !leaves.iter().any(too_big) {
            return BodyFetch::Full;
        }
        let (deferred, parts): (Vec<_>, Vec<_>) = leaves.iter().partition(|l| too_big(l));
        BodyFetch::Partial {
            parts: parts.into_iter().map(|l| l.path.clone()).collect(),
            deferred: deferred.into_iter().map(|l| l.path.clone()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(t: &str, s: &str, size: u32, d: Disposition, name: Option<&str>) -> MimePart {
        MimePart::Leaf {
            media_type: t.into(),
            subtype: s.into(),
            size,
            disposition: d,
            filename: name.map(Into::into),
        }
    }

    fn path(raw: &str) -> PartPath {
        PartPath::parse(raw).unwrap_or_else(|| panic!("chemin {raw}"))
    }

    fn mixed(big: u32) -> MimePart {
        MimePart::Multipart {
            subtype: "mixed".into(),
            parts: vec![
                MimePart::Multipart {
                    subtype: "alternative".into(),
                    parts: vec![
                        leaf("text", "plain", 10, Disposition::Unspecified, None),
                        leaf("text", "html", 20, Disposition::Unspecified, None),
                    ],
                },
                leaf(
                    "application",
                    "pdf",
                    big,
                    Disposition::Attachment,
                    Some("a.pdf"),
                ),
                leaf("image", "png", 100, Disposition::Inline, None),
            ],
        }
    }

    #[test]
    fn leaves_have_imap_section_paths() {
        let s = mixed(5);
        let paths: Vec<String> = s.leaves().iter().map(|l| l.path.to_string()).collect();
        assert_eq!(paths, ["1.1", "1.2", "2", "3"]);
        let single = leaf("text", "plain", 1, Disposition::Unspecified, None);
        assert_eq!(
            single.leaves().first().map(|l| l.path.to_string()),
            Some("1".into())
        );
    }

    #[test]
    fn small_message_is_fetched_whole() {
        let policy = FetchPolicy {
            attachment_threshold: 1000,
        };
        assert_eq!(policy.decide(2000, Some(&mixed(999))), BodyFetch::Full);
        assert_eq!(policy.decide(1000, None), BodyFetch::Full);
    }

    #[test]
    fn large_attachment_is_deferred_text_is_kept() {
        let policy = FetchPolicy {
            attachment_threshold: 1000,
        };
        assert_eq!(
            policy.decide(10_000, Some(&mixed(5000))),
            BodyFetch::Partial {
                parts: vec![path("1.1"), path("1.2"), path("3")],
                deferred: vec![path("2")],
            }
        );
    }

    #[test]
    fn large_text_is_always_fetched() {
        let policy = FetchPolicy {
            attachment_threshold: 10,
        };
        let s = leaf("text", "plain", 1_000_000, Disposition::Unspecified, None);
        assert_eq!(policy.decide(1_000_000, Some(&s)), BodyFetch::Full);
    }

    #[test]
    fn unknown_structure_over_threshold_fetches_headers_only() {
        let policy = FetchPolicy {
            attachment_threshold: 10,
        };
        assert_eq!(
            policy.decide(11, None),
            BodyFetch::Partial {
                parts: vec![],
                deferred: vec![]
            }
        );
    }

    #[test]
    fn attachments_detection() {
        assert!(mixed(1).has_attachments());
        let alt = MimePart::Multipart {
            subtype: "alternative".into(),
            parts: vec![leaf("text", "plain", 1, Disposition::Unspecified, None)],
        };
        assert!(!alt.has_attachments());
        assert!(MimePart::Message { size: 3 }.has_attachments());
        // Texte explicitement joint : pièce jointe.
        assert!(leaf("text", "plain", 1, Disposition::Attachment, Some("x.txt")).has_attachments());
    }

    #[test]
    fn deep_or_huge_structures_are_bounded() {
        let mut s = leaf("text", "plain", 1, Disposition::Unspecified, None);
        for _ in 0..100 {
            s = MimePart::Multipart {
                subtype: "mixed".into(),
                parts: vec![s],
            };
        }
        assert!(s.leaves().is_empty(), "au-delà de la profondeur maximale");
        let wide = MimePart::Multipart {
            subtype: "mixed".into(),
            parts: vec![leaf("a", "b", 1, Disposition::Attachment, None); 10_000],
        };
        assert_eq!(wide.leaves().len(), MAX_MIME_PARTS);
    }

    #[test]
    fn part_path_validation() {
        assert!(PartPath::parse("1.2.3").is_some());
        for bad in ["", "0", "1..2", "a", "1.0", "-1"] {
            assert!(PartPath::parse(bad).is_none(), "{bad}");
        }
    }
}
