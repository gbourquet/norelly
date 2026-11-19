//! Photos de contact (ADR-012) : ce qu'un fichier importé peut apporter
//! comme photo, et ce que Norelly en garde.
//!
//! Une photo n'est acceptée qu'**intégrée** à la fiche (une `PHOTO` vCard
//! donnée par adresse est ignorée : aucune requête réseau), au plus
//! [`MAX_PHOTO_BYTES`] octets, en PNG, JPEG ou WebP reconnu à ses premiers
//! octets ([`check_embedded`]). Elle n'est jamais gardée telle quelle :
//! l'adaptateur la décode avec le seul décodeur de son format, après avoir
//! contrôlé ses dimensions ([`check_dimensions`]), puis la réencode en
//! JPEG d'au plus [`PHOTO_SIDE`] pixels de côté ([`thumbnail_size`]) :
//! métadonnées retirées, contenu piégé (fichier polyglotte, octets ajoutés
//! après l'image) neutralisé. C'est cette image réencodée, seule, que
//! l'interface affiche et que l'export écrit.

use std::collections::BTreeMap;
use std::time::Duration;

/// Taille maximale d'une photo intégrée à une fiche, en octets (une fois
/// décodée du base64).
pub const MAX_PHOTO_BYTES: usize = 1024 * 1024;
/// Côté maximal de la photo gardée, en pixels.
pub const PHOTO_SIDE: u32 = 256;
/// Côté maximal d'une photo importée, en pixels : au-delà, elle est
/// refusée sans être décodée.
pub const MAX_PHOTO_SOURCE_SIDE: u32 = 8192;
/// Nombre maximal de pixels d'une photo importée : borne la mémoire du
/// décodage (64 Mio en RGBA), quelle que soit la compression du fichier.
pub const MAX_PHOTO_SOURCE_PIXELS: u64 = 4096 * 4096;
/// Qualité JPEG de la photo gardée (de 1 à 100).
pub const PHOTO_JPEG_QUALITY: u8 = 85;
/// Couleur de fond (RVB) d'une photo transparente, que le JPEG ne garde
/// pas.
pub const PHOTO_BACKGROUND: [u8; 3] = [255, 255, 255];
/// Type de la photo gardée.
pub const PHOTO_MEDIA_TYPE: &str = "image/jpeg";
/// Photos réencodées au plus par un import : au-delà, les fiches suivantes
/// sont importées sans photo.
pub const MAX_IMPORT_PHOTOS: usize = 2_000;
/// Pixels décodés au plus par un import, toutes photos comprises (borne le
/// travail du décodage, que la taille du fichier ne borne pas : une image
/// unie se compresse en quelques octets).
pub const MAX_IMPORT_PHOTO_PIXELS: u64 = 500_000_000;
/// Âge minimal d'une photo qu'aucun contact ne porte pour que le ménage la
/// supprime : une photo tout juste enregistrée par un import en cours
/// (d'une autre instance de l'application) n'est pas encore donnée à son
/// contact.
pub const PHOTO_SWEEP_MIN_AGE: Duration = Duration::from_secs(10 * 60);

/// Budget de décodage d'un import, en pixels ([`MAX_IMPORT_PHOTO_PIXELS`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecodeBudget {
    used: u64,
}

impl DecodeBudget {
    /// La photo de `width` × `height` pixels peut-elle encore être
    /// décodée ? Si oui, ses pixels sont décomptés.
    pub fn take(&mut self, width: u32, height: u32) -> bool {
        let pixels = u64::from(width) * u64::from(height);
        match self.used.checked_add(pixels) {
            Some(total) if total <= MAX_IMPORT_PHOTO_PIXELS => {
                self.used = total;
                true
            }
            _ => false,
        }
    }
}

/// Le contenu relu du magasin est-il bien une photo gardée (JPEG) ? Seule
/// une telle photo est affichée ou exportée.
#[must_use]
pub fn is_kept_photo(bytes: &[u8]) -> bool {
    PhotoFormat::sniff(bytes) == Some(PhotoFormat::Jpeg)
}

/// Format d'une photo importée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhotoFormat {
    Png,
    Jpeg,
    Webp,
}

impl PhotoFormat {
    /// Format reconnu aux premiers octets du contenu (signature), quel que
    /// soit le type annoncé.
    #[must_use]
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
            Some(Self::Webp)
        } else {
            None
        }
    }

    /// Format désigné par un type annoncé : type MIME (`image/jpeg`, vCard
    /// 4.0) ou type vCard (`JPEG`, vCard 2.1 et 3.0), sans distinction de
    /// casse. `None` pour tout autre type (GIF, SVG, BMP, inconnu).
    #[must_use]
    pub fn from_declared(declared: &str) -> Option<Self> {
        let raw = declared.trim().to_ascii_lowercase();
        let name = raw.strip_prefix("image/").unwrap_or(&raw);
        match name {
            "png" => Some(Self::Png),
            "jpeg" | "jpg" => Some(Self::Jpeg),
            "webp" => Some(Self::Webp),
            _ => None,
        }
    }
}

/// Photo intégrée lue dans une fiche, non contrôlée.
#[derive(Clone, PartialEq, Eq)]
pub struct EmbeddedPhoto {
    /// Type annoncé par le fichier (paramètre `TYPE` ou type du `data:`),
    /// tel quel.
    pub declared: Option<String>,
    /// Contenu, décodé du base64.
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for EmbeddedPhoto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Le contenu n'est jamais écrit dans un journal.
        f.debug_struct("EmbeddedPhoto")
            .field("declared", &self.declared)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// Photos intégrées d'un fichier, par rang de fiche (à partir de 0, celui
/// de [`crate::contact_import::DecodedContacts::cards`]).
pub type CardPhotos = BTreeMap<usize, EmbeddedPhoto>;

/// Raison du refus d'une photo (la fiche, elle, est importée sans photo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PhotoRefusal {
    #[error("photo vide")]
    Empty,
    #[error("photo trop grande")]
    TooLarge,
    /// Ni PNG, ni JPEG, ni WebP (GIF, SVG, BMP, contenu inconnu), ou type
    /// annoncé hors de ces trois.
    #[error("format de photo refusé")]
    Unsupported,
    /// Le type annoncé n'est pas celui du contenu.
    #[error("type de photo incohérent")]
    Mismatch,
    /// Dimensions nulles ou au-delà des bornes.
    #[error("dimensions de photo refusées")]
    Dimensions,
}

/// Contrôle d'une photo intégrée, avant tout décodage : taille, format
/// reconnu à la signature, type annoncé (s'il y en a un) identique.
pub fn check_embedded(photo: &EmbeddedPhoto) -> Result<PhotoFormat, PhotoRefusal> {
    if photo.bytes.is_empty() {
        return Err(PhotoRefusal::Empty);
    }
    if photo.bytes.len() > MAX_PHOTO_BYTES {
        return Err(PhotoRefusal::TooLarge);
    }
    let format = PhotoFormat::sniff(&photo.bytes).ok_or(PhotoRefusal::Unsupported)?;
    if let Some(declared) = photo.declared.as_deref() {
        match PhotoFormat::from_declared(declared) {
            Some(d) if d == format => {}
            Some(_) => return Err(PhotoRefusal::Mismatch),
            None => return Err(PhotoRefusal::Unsupported),
        }
    }
    Ok(format)
}

/// Contrôle des dimensions annoncées par l'en-tête de l'image, avant le
/// décodage de ses pixels.
pub fn check_dimensions(width: u32, height: u32) -> Result<(), PhotoRefusal> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0
        || height == 0
        || width > MAX_PHOTO_SOURCE_SIDE
        || height > MAX_PHOTO_SOURCE_SIDE
        || pixels > MAX_PHOTO_SOURCE_PIXELS
    {
        return Err(PhotoRefusal::Dimensions);
    }
    Ok(())
}

/// Dimensions de la photo gardée : celles de l'image réduites pour tenir
/// dans [`PHOTO_SIDE`] pixels de côté, proportions gardées, jamais
/// agrandies, au moins un pixel.
#[must_use]
pub fn thumbnail_size(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= PHOTO_SIDE {
        return (width.max(1), height.max(1));
    }
    let scale = |side: u32| {
        let scaled =
            (u64::from(side) * u64::from(PHOTO_SIDE) + u64::from(longest) / 2) / u64::from(longest);
        u32::try_from(scaled).unwrap_or(PHOTO_SIDE).max(1)
    };
    (scale(width), scale(height))
}

#[cfg(test)]
mod tests;
