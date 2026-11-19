//! Réencodage des photos de contact ([`PhotoCodec`], ADR-012) par la crate
//! `image`, réduite à ses décodeurs PNG, JPEG et WebP.
//!
//! Traduit sans décider : le format (reconnu par le domaine), les bornes
//! des dimensions, la taille de la vignette, la qualité et le fond sont
//! ceux de [`norelly_domain::contact_photo`]. Le contenu n'est lu que par
//! le décodeur du format reconnu (jamais deviné à nouveau) ; seuls ses
//! pixels sont gardés, réencodés en JPEG : métadonnées (EXIF, XMP, profil
//! ICC, commentaires) et octets ajoutés après l'image disparaissent.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::error::{ImageError, LimitErrorKind};
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, Rgb, RgbImage};
use norelly_domain::contact_photo::{
    MAX_PHOTO_SOURCE_PIXELS, MAX_PHOTO_SOURCE_SIDE, PHOTO_BACKGROUND, PHOTO_JPEG_QUALITY,
    PhotoFormat, PhotoRefusal, check_dimensions, thumbnail_size,
};
use norelly_domain::ports::{PhotoCodec, PhotoCodecError};

/// Mémoire allouée au plus par un décodage, en octets : les pixels d'une
/// image à la borne du domaine en RGBA 16 bits, plus une marge pour les
/// tampons du décodeur.
const MAX_DECODE_ALLOC: u64 = MAX_PHOTO_SOURCE_PIXELS * 8 + 16 * 1024 * 1024;

fn image_format(format: PhotoFormat) -> ImageFormat {
    match format {
        PhotoFormat::Png => ImageFormat::Png,
        PhotoFormat::Jpeg => ImageFormat::Jpeg,
        PhotoFormat::Webp => ImageFormat::WebP,
    }
}

/// Erreur du décodeur : une limite atteinte est un refus du domaine
/// (dimensions), tout le reste un contenu illisible.
fn decode_error(e: &ImageError) -> PhotoCodecError {
    match e {
        ImageError::Limits(l) if matches!(l.kind(), LimitErrorKind::DimensionError) => {
            PhotoCodecError::Refused(PhotoRefusal::Dimensions)
        }
        ImageError::Limits(_) => PhotoCodecError::Refused(PhotoRefusal::Dimensions),
        _ => PhotoCodecError::Undecodable,
    }
}

/// Pixels de `image` posés sur [`PHOTO_BACKGROUND`] (le JPEG n'a pas de
/// transparence).
fn flatten(image: &DynamicImage) -> RgbImage {
    let rgba = image.to_rgba8();
    let [r, g, b] = PHOTO_BACKGROUND.map(u32::from);
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [pr, pg, pb, a] = rgba.get_pixel(x, y).0.map(u32::from);
        let mix =
            |c: u32, bg: u32| u8::try_from((c * a + bg * (255 - a) + 127) / 255).unwrap_or(u8::MAX);
        Rgb([mix(pr, r), mix(pg, g), mix(pb, b)])
    })
}

/// Décodeur du seul format `format`, bornes appliquées, dont l'en-tête a
/// été lu et les dimensions contrôlées ([`check_dimensions`]) ; aucun
/// pixel n'est encore décodé.
fn open(bytes: &[u8], format: PhotoFormat) -> Result<impl ImageDecoder + '_, PhotoCodecError> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_PHOTO_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_PHOTO_SOURCE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), image_format(format));
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|e| decode_error(&e))?;
    let (width, height) = decoder.dimensions();
    check_dimensions(width, height)?;
    Ok(decoder)
}

/// Réencodage des photos par la crate `image`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ImagePhotos;

impl PhotoCodec for ImagePhotos {
    fn dimensions(&self, bytes: &[u8], format: PhotoFormat) -> Result<(u32, u32), PhotoCodecError> {
        Ok(open(bytes, format)?.dimensions())
    }

    fn thumbnail(&self, bytes: &[u8], format: PhotoFormat) -> Result<Vec<u8>, PhotoCodecError> {
        let mut decoder = open(bytes, format)?;
        let orientation = decoder.orientation().map_err(|e| decode_error(&e))?;
        let mut image = DynamicImage::from_decoder(decoder).map_err(|e| decode_error(&e))?;
        // L'orientation EXIF disparaît avec les métadonnées : elle est
        // appliquée aux pixels.
        image.apply_orientation(orientation);
        let (w, h) = thumbnail_size(image.width(), image.height());
        if (w, h) != (image.width(), image.height()) {
            image = image.resize_exact(w, h, FilterType::Triangle);
        }
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, PHOTO_JPEG_QUALITY)
            .encode_image(&flatten(&image))
            .map_err(|_| PhotoCodecError::Undecodable)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
