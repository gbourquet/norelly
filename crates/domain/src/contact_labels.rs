//! Étiquettes d'adresse et de téléphone d'une fiche contact : genre
//! reconnu (travail, domicile, mobile…) pour qu'une étiquette connue
//! s'affiche dans la langue de l'interface, et libellés d'Apple.
//!
//! L'étiquette reste un texte libre, enregistré tel quel : le genre est
//! seulement déduit pour l'affichage (et pour l'export LDIF, qui n'a que
//! quelques attributs de téléphone).

use crate::contacts::{MAX_CONTACT_LABEL_CHARS, forbidden};

/// Genre d'une étiquette connue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactLabelKind {
    Work,
    Home,
    Mobile,
    Fax,
    Pager,
    Main,
    Other,
}

impl ContactLabelKind {
    /// Genre d'une étiquette d'un seul mot, par ses synonymes anglais et
    /// français, casse et espaces de bord ignorées (`work`, `Travail`,
    /// `cell`, `Portable`) ; `None` pour une étiquette inconnue, vide ou
    /// composée (`Work Fax` garde son texte, qui précise le genre).
    #[must_use]
    pub fn of(label: &str) -> Option<Self> {
        let label = label.trim();
        if label.is_empty() || label.contains(char::is_whitespace) {
            return None;
        }
        Some(match label.to_lowercase().as_str() {
            "work" | "travail" => Self::Work,
            "home" | "maison" | "domicile" | "perso" | "personnel" => Self::Home,
            "cell" | "mobile" | "portable" => Self::Mobile,
            "fax" | "télécopie" | "telecopie" => Self::Fax,
            "pager" | "bipeur" | "radiomessagerie" => Self::Pager,
            "main" | "principal" => Self::Main,
            "other" | "autre" => Self::Other,
            _ => return None,
        })
    }

    /// Type vCard usuel qui représente le genre (`work`, `home`, `cell`…) :
    /// étiquette enregistrée pour un libellé réservé d'Apple, relue telle
    /// quelle par les autres logiciels à l'export.
    #[must_use]
    pub const fn canonical(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Home => "home",
            Self::Mobile => "cell",
            Self::Fax => "fax",
            Self::Pager => "pager",
            Self::Main => "main",
            Self::Other => "other",
        }
    }
}

/// Étiquette tirée d'un libellé Apple (`X-ABLabel`) : un libellé réservé
/// (`_$!<Work>!$_`, `_$!<HomeFAX>!$_`…) devient l'étiquette connue
/// correspondante, un libellé réservé inconnu son mot (`School`) ; un
/// libellé personnalisé (`bureau`) est gardé tel quel, sans espaces de
/// bord. `None` si l'étiquette serait refusée par une fiche (vide, plus
/// de [`MAX_CONTACT_LABEL_CHARS`] caractères, caractère de contrôle ou
/// invisible) : la fiche garde alors son étiquette d'origine.
#[must_use]
pub fn apple_label(raw: &str) -> Option<String> {
    let label = apple_text(raw.trim());
    let kept = !label.is_empty()
        && label.chars().count() <= MAX_CONTACT_LABEL_CHARS
        && !label.chars().any(forbidden);
    kept.then(|| label.to_owned())
}

fn apple_text(raw: &str) -> &str {
    let Some(word) = raw
        .strip_prefix("_$!<")
        .and_then(|rest| rest.strip_suffix(">!$_"))
    else {
        return raw;
    };
    let kind = match word.to_ascii_lowercase().as_str() {
        "homefax" | "workfax" | "otherfax" => Some(ContactLabelKind::Fax),
        "work" | "home" | "mobile" | "main" | "other" | "pager" => ContactLabelKind::of(word),
        _ => None,
    };
    match kind {
        Some(kind) => kind.canonical(),
        None => word.trim(),
    }
}

#[cfg(test)]
mod tests {
    use super::ContactLabelKind::{Fax, Home, Main, Mobile, Other, Pager, Work};
    use super::*;

    #[test]
    fn known_labels_in_english_and_french_whatever_the_case() {
        for (label, kind) in [
            ("work", Work),
            ("WORK", Work),
            ("Travail", Work),
            ("home", Home),
            ("Domicile", Home),
            ("maison", Home),
            ("perso", Home),
            ("personnel", Home),
            ("cell", Mobile),
            ("CELL", Mobile),
            ("Mobile", Mobile),
            ("portable", Mobile),
            ("fax", Fax),
            ("Télécopie", Fax),
            ("telecopie", Fax),
            ("pager", Pager),
            ("Bipeur", Pager),
            ("radiomessagerie", Pager),
            ("main", Main),
            ("Principal", Main),
            ("other", Other),
            ("Autre", Other),
            ("  work  ", Work),
        ] {
            assert_eq!(ContactLabelKind::of(label), Some(kind), "{label}");
        }
    }

    #[test]
    fn unknown_labels_have_no_kind() {
        // Une étiquette composée (« Work Fax ») garde son texte : la
        // traduire en « Fax » effacerait la distinction travail/domicile.
        for label in [
            "",
            "  ",
            "bureau",
            "iPhone",
            "workshop",
            "w ork",
            "x-work",
            "Work Fax",
            "Home Fax",
            "fax domicile",
        ] {
            assert_eq!(ContactLabelKind::of(label), None, "{label}");
        }
    }

    #[test]
    fn canonical_words_are_known_labels_of_their_kind() {
        for kind in [Work, Home, Mobile, Fax, Pager, Main, Other] {
            assert_eq!(ContactLabelKind::of(kind.canonical()), Some(kind));
        }
    }

    #[test]
    fn apple_reserved_labels_become_known_labels() {
        // Types vCard usuels (`cell`, pas `mobile`) : l'export les relit.
        for (raw, label) in [
            ("_$!<Work>!$_", "work"),
            ("_$!<Home>!$_", "home"),
            ("_$!<Mobile>!$_", "cell"),
            ("_$!<Main>!$_", "main"),
            ("_$!<Other>!$_", "other"),
            ("_$!<Pager>!$_", "pager"),
            ("_$!<HomeFAX>!$_", "fax"),
            ("_$!<WorkFAX>!$_", "fax"),
            ("_$!<OtherFAX>!$_", "fax"),
            ("_$!<work>!$_", "work"),
            ("_$!<School>!$_", "School"),
        ] {
            assert_eq!(apple_label(raw).as_deref(), Some(label), "{raw}");
        }
    }

    #[test]
    fn apple_custom_labels_are_kept() {
        assert_eq!(apple_label("bureau").as_deref(), Some("bureau"));
        assert_eq!(apple_label("  Chalet ").as_deref(), Some("Chalet"));
        assert_eq!(apple_label("_$!<Work").as_deref(), Some("_$!<Work"));
        let longest = "é".repeat(MAX_CONTACT_LABEL_CHARS);
        assert_eq!(apple_label(&longest).as_deref(), Some(longest.as_str()));
    }

    #[test]
    fn apple_labels_the_card_could_not_keep_are_dropped() {
        // Vide, trop long ou avec un caractère qu'une fiche refuse : la
        // fiche garde son étiquette d'origine plutôt que d'être refusée.
        for raw in [
            String::new(),
            "  ".to_owned(),
            "_$!<>!$_".to_owned(),
            "x".repeat(MAX_CONTACT_LABEL_CHARS + 1),
            "Maison \u{2764}\u{FE0F}".to_owned(),
            "bu\u{200B}reau".to_owned(),
            "a\tb".to_owned(),
        ] {
            assert_eq!(apple_label(&raw), None, "{raw:?}");
        }
    }
}
