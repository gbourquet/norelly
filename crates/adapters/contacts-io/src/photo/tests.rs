use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{
    DynamicImage, ExtendedColorType, ImageEncoder, ImageFormat, ImageReader, Rgba, RgbaImage,
};
use norelly_domain::contact_photo::{MAX_PHOTO_BYTES, PhotoRefusal};

use super::*;

/// Image de test : moitié gauche rouge opaque, moitié droite transparente.
fn picture(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, _| {
        if x < width / 2 {
            Rgba([200, 0, 0, 255])
        } else {
            Rgba([0, 0, 200, 0])
        }
    })
}

fn png(image: &RgbaImage) -> Vec<u8> {
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(
            image,
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )
        .unwrap_or_else(|e| panic!("{e}"));
    out
}

fn webp(image: &RgbaImage) -> Vec<u8> {
    let mut out = Vec::new();
    WebPEncoder::new_lossless(&mut out)
        .write_image(
            image,
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )
        .unwrap_or_else(|e| panic!("{e}"));
    out
}

/// EXIF minimal : orientation `orientation`, puis un texte repérable.
fn exif(orientation: u16) -> Vec<u8> {
    let mut exif = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
    exif.extend_from_slice(&orientation.to_le_bytes());
    exif.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    exif.extend_from_slice(b"GPS 45.76N 4.83E");
    exif
}

fn jpeg(width: u32, height: u32, exif: Option<Vec<u8>>) -> Vec<u8> {
    let rgb = DynamicImage::ImageRgba8(picture(width, height)).to_rgb8();
    let mut out = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut out, 90);
    if let Some(exif) = exif {
        encoder
            .set_exif_metadata(exif)
            .unwrap_or_else(|e| panic!("{e}"));
    }
    encoder
        .write_image(rgb.as_raw(), width, height, ExtendedColorType::Rgb8)
        .unwrap_or_else(|e| panic!("{e}"));
    out
}

/// Vignette relue : format et dimensions.
fn decoded(bytes: &[u8]) -> DynamicImage {
    assert_eq!(PhotoFormat::sniff(bytes), Some(PhotoFormat::Jpeg));
    ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg)
        .decode()
        .unwrap_or_else(|e| panic!("{e}"))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn thumbnail(bytes: &[u8], format: PhotoFormat) -> Result<Vec<u8>, PhotoCodecError> {
    ImagePhotos.thumbnail(bytes, format)
}

#[test]
fn every_format_becomes_a_small_jpeg() {
    let source = picture(1000, 500);
    for (bytes, format) in [
        (png(&source), PhotoFormat::Png),
        (webp(&source), PhotoFormat::Webp),
        (jpeg(1000, 500, None), PhotoFormat::Jpeg),
    ] {
        let out = thumbnail(&bytes, format).unwrap_or_else(|e| panic!("{format:?} : {e}"));
        assert!(out.len() <= MAX_PHOTO_BYTES, "{format:?}");
        let image = decoded(&out);
        assert_eq!((image.width(), image.height()), (256, 128), "{format:?}");
    }
}

#[test]
fn transparency_is_laid_on_the_background() {
    let out = thumbnail(&png(&picture(64, 64)), PhotoFormat::Png).unwrap_or_else(|e| panic!("{e}"));
    let image = decoded(&out).to_rgb8();
    // Moitié droite transparente : blanche, à la compression près.
    let right = image.get_pixel(60, 32).0;
    assert!(right.iter().all(|&c| c > 240), "{right:?}");
    let left = image.get_pixel(4, 32).0;
    assert!(left[0] > 150 && left[1] < 60, "{left:?}");
}

#[test]
fn small_photos_are_not_enlarged() {
    let out = thumbnail(&png(&picture(96, 64)), PhotoFormat::Png).unwrap_or_else(|e| panic!("{e}"));
    let image = decoded(&out);
    assert_eq!((image.width(), image.height()), (96, 64));
}

#[test]
fn metadata_and_trailing_content_are_dropped() {
    let mut trapped = jpeg(300, 300, Some(exif(1)));
    assert!(contains(&trapped, b"GPS 45.76N"));
    // Polyglotte : un document HTML collé après l'image.
    trapped.extend_from_slice(b"<html><script>alert(1)</script></html>");
    let out = thumbnail(&trapped, PhotoFormat::Jpeg).unwrap_or_else(|e| panic!("{e}"));
    assert!(!contains(&out, b"GPS 45.76N"));
    assert!(!contains(&out, b"Exif"));
    assert!(!contains(&out, b"<script>"));
    // Le JPEG se termine par sa marque de fin, sans rien après.
    assert!(out.ends_with(&[0xFF, 0xD9]));

    let mut png_trapped = png(&picture(40, 40));
    png_trapped.extend_from_slice(b"<svg onload=alert(1)>");
    let out = thumbnail(&png_trapped, PhotoFormat::Png).unwrap_or_else(|e| panic!("{e}"));
    assert!(!contains(&out, b"<svg"));
}

#[test]
fn the_exif_orientation_is_applied_to_the_pixels() {
    // Orientation 6 : à faire pivoter d'un quart de tour vers la droite.
    let out = thumbnail(&jpeg(400, 200, Some(exif(6))), PhotoFormat::Jpeg)
        .unwrap_or_else(|e| panic!("{e}"));
    let image = decoded(&out);
    assert_eq!((image.width(), image.height()), (128, 256));
}

#[test]
fn only_the_recognised_format_is_decoded() {
    // Contenu PNG annoncé comme JPEG : le décodeur JPEG le refuse.
    assert_eq!(
        thumbnail(&png(&picture(8, 8)), PhotoFormat::Jpeg),
        Err(PhotoCodecError::Undecodable)
    );
}

#[test]
fn truncated_or_garbage_content_is_undecodable() {
    let full = png(&picture(64, 64));
    for bytes in [
        &full[..full.len() / 2],
        &b"\x89PNG\r\n\x1a\n garbage"[..],
        b"",
    ] {
        assert_eq!(
            thumbnail(bytes, PhotoFormat::Png),
            Err(PhotoCodecError::Undecodable)
        );
    }
}

/// Vrai PNG de 8 × 8 pixels dont l'en-tête annonce `width` × `height`
/// (bombe de décompression : des pixels annoncés qui n'existent pas).
fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut out = png(&picture(8, 8));
    out[16..20].copy_from_slice(&width.to_be_bytes());
    out[20..24].copy_from_slice(&height.to_be_bytes());
    let crc = crc32(&out[12..29]);
    out[29..33].copy_from_slice(&crc.to_be_bytes());
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn oversized_dimensions_are_refused_before_decoding() {
    for (w, h) in [(20_000, 10), (5000, 5000)] {
        assert_eq!(
            thumbnail(&png_header(w, h), PhotoFormat::Png),
            Err(PhotoCodecError::Refused(PhotoRefusal::Dimensions)),
            "{w}×{h}"
        );
    }
}

#[test]
fn real_codec_passes_contract() {
    let source = picture(300, 200);
    norelly_test_support::contact_photo_contracts::photo_codec_contract(
        &ImagePhotos,
        &[
            (png(&source), PhotoFormat::Png, (300, 200)),
            (jpeg(300, 200, None), PhotoFormat::Jpeg, (300, 200)),
            (webp(&source), PhotoFormat::Webp, (300, 200)),
        ],
    );
}

/// Vrai JPEG de 8 × 8 pixels dont l'en-tête (SOF0) annonce `width` ×
/// `height`.
fn jpeg_header(width: u16, height: u16) -> Vec<u8> {
    let mut out = jpeg(8, 8, None);
    let sof = out
        .windows(2)
        .position(|w| w == [0xFF, 0xC0])
        .unwrap_or_else(|| panic!("SOF0 absent"));
    out[sof + 5..sof + 7].copy_from_slice(&height.to_be_bytes());
    out[sof + 7..sof + 9].copy_from_slice(&width.to_be_bytes());
    out
}

/// En-tête WebP sans perte (VP8L) qui annonce `width` × `height` (au plus
/// 16 384 de côté), sans les données correspondantes.
fn webp_header(width: u32, height: u32) -> Vec<u8> {
    let bits = (width - 1) | ((height - 1) << 14);
    let mut vp8l = vec![0x2F];
    vp8l.extend_from_slice(&bits.to_le_bytes());
    vp8l.extend_from_slice(&[0; 16]);
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&u32::try_from(4 + 8 + vp8l.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(b"WEBPVP8L");
    out.extend_from_slice(&u32::try_from(vp8l.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&vp8l);
    out
}

#[test]
fn dimension_bombs_are_refused_in_every_format() {
    let refused = PhotoCodecError::Refused(PhotoRefusal::Dimensions);
    for (bytes, format) in [
        (jpeg_header(20_000, 10), PhotoFormat::Jpeg),
        (jpeg_header(5000, 5000), PhotoFormat::Jpeg),
        (webp_header(16_384, 16_384), PhotoFormat::Webp),
        (webp_header(5000, 5000), PhotoFormat::Webp),
        (png_header(5000, 5000), PhotoFormat::Png),
    ] {
        assert_eq!(
            ImagePhotos.dimensions(&bytes, format),
            Err(refused),
            "{format:?}"
        );
        assert_eq!(
            ImagePhotos.thumbnail(&bytes, format),
            Err(refused),
            "{format:?}"
        );
    }
}

#[test]
fn dimensions_are_read_from_the_header_only() {
    assert_eq!(
        ImagePhotos.dimensions(&jpeg_header(4000, 3000), PhotoFormat::Jpeg),
        Ok((4000, 3000))
    );
    assert_eq!(
        ImagePhotos.dimensions(&webp_header(640, 480), PhotoFormat::Webp),
        Ok((640, 480))
    );
}
