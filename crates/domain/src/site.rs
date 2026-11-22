//! Pages du site de Norelly ouvertes depuis l'application (aide, accueil).
//!
//! L'adresse ouverte ne vient jamais de l'interface : elle choisit une page
//! et une langue parmi une liste fermée, et l'adresse est construite ici à
//! partir de l'origine du site.

/// Origine du site, en littéral pour `concat!`.
macro_rules! site_origin {
    () => {
        "https://norelly.fr"
    };
}

/// Origine du site de Norelly.
pub const SITE_ORIGIN: &str = site_origin!();

/// Page du site proposée dans l'application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SitePage {
    /// Accueil : présentation de Norelly (« Site de Norelly »).
    Home,
    /// Guide d'utilisation.
    Guide,
}

/// Langue des pages du site : celles de l'interface (ADR-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteLanguage {
    French,
    English,
}

/// Adresse d'une page du site dans une langue, sous [`SITE_ORIGIN`]. Les
/// chemins suivent ceux du site : `/<langue>/` pour l'accueil,
/// `/<langue>/guide/` pour le guide.
#[must_use]
pub const fn site_page_url(page: SitePage, language: SiteLanguage) -> &'static str {
    match (page, language) {
        (SitePage::Home, SiteLanguage::French) => concat!(site_origin!(), "/fr/"),
        (SitePage::Home, SiteLanguage::English) => concat!(site_origin!(), "/en/"),
        (SitePage::Guide, SiteLanguage::French) => concat!(site_origin!(), "/fr/guide/"),
        (SitePage::Guide, SiteLanguage::English) => concat!(site_origin!(), "/en/guide/"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading::{ExternalLink, LinkScheme};

    #[test]
    fn each_page_has_its_address_in_each_language() {
        use SiteLanguage::{English, French};
        use SitePage::{Guide, Home};
        assert_eq!(site_page_url(Home, French), "https://norelly.fr/fr/");
        assert_eq!(site_page_url(Home, English), "https://norelly.fr/en/");
        assert_eq!(site_page_url(Guide, French), "https://norelly.fr/fr/guide/");
        assert_eq!(
            site_page_url(Guide, English),
            "https://norelly.fr/en/guide/"
        );
    }

    #[test]
    fn every_address_is_an_https_link_on_the_site() {
        for page in [SitePage::Home, SitePage::Guide] {
            for language in [SiteLanguage::French, SiteLanguage::English] {
                let url = site_page_url(page, language);
                let link = ExternalLink::parse(url).unwrap_or_else(|e| panic!("{url} : {e}"));
                assert_eq!(link.scheme(), LinkScheme::Https);
                assert!(url.starts_with(&format!("{SITE_ORIGIN}/")), "{url}");
            }
        }
    }
}
