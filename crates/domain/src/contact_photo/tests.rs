use super::*;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];
const WEBP: &[u8] = b"RIFF\x24\0\0\0WEBPVP8 ";

fn photo(declared: Option<&str>, bytes: &[u8]) -> EmbeddedPhoto {
    EmbeddedPhoto {
        declared: declared.map(str::to_owned),
        bytes: bytes.to_vec(),
    }
}

#[test]
fn formats_are_recognised_by_their_signature() {
    assert_eq!(PhotoFormat::sniff(PNG), Some(PhotoFormat::Png));
    assert_eq!(PhotoFormat::sniff(JPEG), Some(PhotoFormat::Jpeg));
    assert_eq!(PhotoFormat::sniff(WEBP), Some(PhotoFormat::Webp));
    for other in [
        &b"GIF89a"[..],
        b"<svg xmlns=\"http://www.w3.org/2000/svg\">",
        b"BM\0\0",
        b"RIFF\0\0\0\0WAVE",
        b"RIFF",
        b"<html><script>",
        b"",
    ] {
        assert_eq!(PhotoFormat::sniff(other), None, "{other:?}");
    }
}

#[test]
fn declared_types_cover_mime_and_vcard_names() {
    for (declared, format) in [
        ("image/jpeg", PhotoFormat::Jpeg),
        ("JPEG", PhotoFormat::Jpeg),
        (" jpg ", PhotoFormat::Jpeg),
        ("IMAGE/PNG", PhotoFormat::Png),
        ("png", PhotoFormat::Png),
        ("image/webp", PhotoFormat::Webp),
    ] {
        assert_eq!(
            PhotoFormat::from_declared(declared),
            Some(format),
            "{declared}"
        );
    }
    for other in [
        "image/gif",
        "GIF",
        "image/svg+xml",
        "text/html",
        "",
        "image/",
    ] {
        assert_eq!(PhotoFormat::from_declared(other), None, "{other}");
    }
}

#[test]
fn an_embedded_photo_is_accepted_by_its_content() {
    assert_eq!(check_embedded(&photo(None, PNG)), Ok(PhotoFormat::Png));
    assert_eq!(
        check_embedded(&photo(Some("JPEG"), JPEG)),
        Ok(PhotoFormat::Jpeg)
    );
    assert_eq!(
        check_embedded(&photo(Some("image/webp"), WEBP)),
        Ok(PhotoFormat::Webp)
    );
}

#[test]
fn a_trapped_or_unsupported_photo_is_refused() {
    assert_eq!(check_embedded(&photo(None, b"")), Err(PhotoRefusal::Empty));
    assert_eq!(
        check_embedded(&photo(None, b"GIF89a....")),
        Err(PhotoRefusal::Unsupported)
    );
    // SVG annoncé comme une image matricielle : contenu non reconnu.
    assert_eq!(
        check_embedded(&photo(Some("image/png"), b"<svg onload=x>")),
        Err(PhotoRefusal::Unsupported)
    );
    // Contenu PNG annoncé comme GIF, ou comme JPEG.
    assert_eq!(
        check_embedded(&photo(Some("GIF"), PNG)),
        Err(PhotoRefusal::Unsupported)
    );
    assert_eq!(
        check_embedded(&photo(Some("image/jpeg"), PNG)),
        Err(PhotoRefusal::Mismatch)
    );
}

#[test]
fn the_size_bound_is_inclusive() {
    let mut bytes = JPEG.to_vec();
    bytes.resize(MAX_PHOTO_BYTES, 0);
    assert_eq!(check_embedded(&photo(None, &bytes)), Ok(PhotoFormat::Jpeg));
    bytes.push(0);
    assert_eq!(
        check_embedded(&photo(None, &bytes)),
        Err(PhotoRefusal::TooLarge)
    );
}

#[test]
fn dimensions_are_bounded_before_decoding() {
    assert_eq!(check_dimensions(1, 1), Ok(()));
    assert_eq!(check_dimensions(4096, 4096), Ok(()));
    assert_eq!(check_dimensions(MAX_PHOTO_SOURCE_SIDE, 16), Ok(()));
    for (w, h) in [
        (0, 10),
        (10, 0),
        (MAX_PHOTO_SOURCE_SIDE + 1, 1),
        (1, MAX_PHOTO_SOURCE_SIDE + 1),
        (4097, 4096),
        (u32::MAX, u32::MAX),
    ] {
        assert_eq!(
            check_dimensions(w, h),
            Err(PhotoRefusal::Dimensions),
            "{w}×{h}"
        );
    }
}

#[test]
fn thumbnails_fit_the_side_and_keep_proportions() {
    assert_eq!(thumbnail_size(1024, 1024), (256, 256));
    assert_eq!(thumbnail_size(1000, 500), (256, 128));
    assert_eq!(thumbnail_size(300, 1200), (64, 256));
    assert_eq!(thumbnail_size(8192, 1), (256, 1));
    // Jamais agrandie.
    assert_eq!(thumbnail_size(96, 64), (96, 64));
    assert_eq!(thumbnail_size(256, 10), (256, 10));
}

#[test]
fn the_content_of_a_photo_never_reaches_debug_output() {
    let shown = format!("{:?}", photo(Some("JPEG"), b"\xFF\xD8\xFFsecret"));
    assert!(
        !shown.contains("secret") && !shown.contains("255"),
        "{shown}"
    );
    assert!(shown.contains("JPEG"), "{shown}");
}

#[test]
fn the_decoding_budget_of_an_import_is_bounded() {
    let mut budget = DecodeBudget::default();
    // Une trentaine de photos aux dimensions maximales.
    let full = MAX_PHOTO_SOURCE_PIXELS;
    let fits = MAX_IMPORT_PHOTO_PIXELS / full;
    for _ in 0..fits {
        assert!(budget.take(4096, 4096));
    }
    assert!(!budget.take(4096, 4096));
    // Une petite photo tient encore dans le reste.
    assert!(budget.take(16, 16));
    assert!(!budget.take(u32::MAX, u32::MAX));
}

#[test]
fn only_a_jpeg_is_a_kept_photo() {
    assert!(is_kept_photo(JPEG));
    assert!(!is_kept_photo(PNG));
    assert!(!is_kept_photo(b"<svg onload=alert(1)>"));
    assert!(!is_kept_photo(b""));
}
