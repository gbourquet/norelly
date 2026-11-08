use std::path::PathBuf;

use norelly_domain::contacts::{ContactId, MAX_CONTACT_SELECTION};
use norelly_domain::ports::StoreError;

use super::*;

fn id(n: i64) -> ContactId {
    ContactId::new(n).unwrap_or_else(|| panic!("id"))
}

#[test]
fn selection_is_revalidated() {
    assert_eq!(
        selection_of(&ContactExportSelectionDto::All).ok(),
        Some(ContactSelection::All)
    );
    assert_eq!(
        selection_of(&ContactExportSelectionDto::Contacts { ids: vec![2, 1, 2] }).ok(),
        Some(ContactSelection::Contacts(vec![id(2), id(1)]))
    );
    let max = u32::try_from(MAX_CONTACT_SELECTION).unwrap_or(0);
    for ids in [vec![], vec![0], (1..=max + 1).collect()] {
        let e = selection_of(&ContactExportSelectionDto::Contacts { ids });
        assert_eq!(
            e.map_err(|e| (e.code, e.field)).err(),
            Some((ErrorCodeDto::InvalidInput, Some(FieldDto::Contact)))
        );
    }
}

#[test]
fn formats_are_translated() {
    assert_eq!(
        format_of(ContactExportFormatDto::Vcard4),
        ExportFormat::VCard4
    );
    assert_eq!(
        format_of(ContactExportFormatDto::Vcard3),
        ExportFormat::VCard3
    );
    assert_eq!(
        format_of(ContactExportFormatDto::GoogleCsv),
        ExportFormat::GoogleCsv
    );
    assert_eq!(format_of(ContactExportFormatDto::Ldif), ExportFormat::Ldif);
}

#[test]
fn failures_become_codes() {
    assert_eq!(
        export_error(ExportFailure::Empty).code,
        ErrorCodeDto::ContactExportEmpty
    );
    assert_eq!(
        export_error(ExportFailure::TooMany).code,
        ErrorCodeDto::ContactExportTooLarge
    );
    let store = export_error(ExportFailure::Store(StoreError("disque".into())));
    assert_eq!((store.code, store.detail), (ErrorCodeDto::Storage, None));
    for e in [ContactCodecError::TooLarge, ContactCodecError::TooManyCards] {
        assert_eq!(codec_error(e).code, ErrorCodeDto::ContactExportTooLarge);
    }
    assert_eq!(
        codec_error(ContactCodecError::UnknownFormat).code,
        ErrorCodeDto::Internal
    );
}

/// Dossier vide propre au test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("norelly-export-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    dir
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{e}"))
        .map(|e| {
            e.unwrap_or_else(|e| panic!("{e}"))
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn write_atomic_creates_then_replaces_the_file_without_leftovers() {
    let dir = scratch("atomic");
    let path = dir.join("contacts.vcf");
    assert!(write_atomic(&path, b"premier").is_ok());
    assert_eq!(std::fs::read(&path).ok().as_deref(), Some(&b"premier"[..]));
    assert!(write_atomic(&path, b"second").is_ok());
    assert_eq!(std::fs::read(&path).ok().as_deref(), Some(&b"second"[..]));
    assert_eq!(entries(&dir), ["contacts.vcf"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_atomic_refuses_an_oversized_file_and_leaves_the_target_intact() {
    let dir = scratch("borne");
    let path = dir.join("contacts.csv");
    assert!(write_atomic(&path, b"ancien").is_ok());
    let big = vec![b'x'; MAX_EXPORT_FILE_BYTES + 1];
    assert!(write_atomic(&path, &big).is_err());
    assert_eq!(std::fs::read(&path).ok().as_deref(), Some(&b"ancien"[..]));
    assert_eq!(entries(&dir), ["contacts.csv"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_atomic_fails_cleanly_in_a_missing_folder() {
    let dir = scratch("absent");
    assert!(write_atomic(&dir.join("absent").join("contacts.ldif"), b"x").is_err());
    assert!(entries(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn write_atomic_replaces_a_link_without_following_it() {
    let dir = scratch("lien");
    let victim = dir.join("victime");
    std::fs::write(&victim, b"intact").unwrap_or_else(|e| panic!("{e}"));
    let path = dir.join("contacts.vcf");
    std::os::unix::fs::symlink(&victim, &path).unwrap_or_else(|e| panic!("{e}"));
    assert!(write_atomic(&path, b"export").is_ok());
    assert_eq!(std::fs::read(&victim).ok().as_deref(), Some(&b"intact"[..]));
    assert!(
        !std::fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(true)
    );
    assert_eq!(std::fs::read(&path).ok().as_deref(), Some(&b"export"[..]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn write_atomic_keeps_the_book_private() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("droits");
    let mode = |p: &Path| {
        std::fs::metadata(p)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0)
    };
    // Nouveau fichier : lisible par son propriétaire seulement.
    let fresh = dir.join("nouveau.vcf");
    assert!(write_atomic(&fresh, b"x").is_ok());
    assert_eq!(mode(&fresh), 0o600);
    // Fichier remplacé : ses droits sont repris.
    let kept = dir.join("garde.vcf");
    std::fs::write(&kept, b"ancien").unwrap_or_else(|e| panic!("{e}"));
    std::fs::set_permissions(&kept, std::fs::Permissions::from_mode(0o640))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(write_atomic(&kept, b"nouveau").is_ok());
    assert_eq!(mode(&kept), 0o640);
    let _ = std::fs::remove_dir_all(&dir);
}
