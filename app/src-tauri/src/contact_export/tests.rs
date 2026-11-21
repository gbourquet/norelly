use std::path::PathBuf;

use norelly_domain::contacts::{ContactId, MAX_CONTACT_SELECTION};
use norelly_domain::ports::{ContactCodec, StoreError};

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
    for ids in [vec![], vec![0]] {
        let e = selection_of(&ContactExportSelectionDto::Contacts { ids });
        assert_eq!(
            e.map_err(|e| (e.code, e.field)).err(),
            Some((ErrorCodeDto::InvalidInput, Some(FieldDto::Contact)))
        );
    }
    let e = selection_of(&ContactExportSelectionDto::Contacts {
        ids: (1..=max + 1).collect(),
    });
    assert_eq!(
        e.map_err(|e| (e.code, e.field)).err(),
        Some((ErrorCodeDto::ContactSelectionTooMany, None))
    );
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
    assert_eq!(
        export_error(ExportFailure::Codec(ContactCodecError::TooLarge)).code,
        ErrorCodeDto::ContactExportTooLarge
    );
    // Carnet modifié pendant l'écriture : réessayer.
    assert_eq!(
        export_error(ExportFailure::Codec(ContactCodecError::NotSurveyed)).code,
        ErrorCodeDto::ContactExportChanged
    );
    assert_eq!(
        export_error(ExportFailure::Interrupted).code,
        ErrorCodeDto::Internal
    );
    // La borne revérifiée à l'écriture garde le code d'un fichier trop gros.
    assert_eq!(
        write_error(std::io::Error::from(std::io::ErrorKind::FileTooLarge)).code,
        ErrorCodeDto::ContactExportTooLarge
    );
    assert_eq!(
        write_error(std::io::Error::other("disque plein")).code,
        ErrorCodeDto::WriteFailed
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
    // Fichier remplacé exécutable ou setuid : ni bit spécial ni exécution.
    let special = dir.join("special.vcf");
    std::fs::write(&special, b"ancien").unwrap_or_else(|e| panic!("{e}"));
    std::fs::set_permissions(&special, std::fs::Permissions::from_mode(0o4755))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(write_atomic(&special, b"nouveau").is_ok());
    assert_eq!(mode(&special), 0o644);
    // Lien remplacé : les droits de sa cible ne sont pas repris.
    let target = dir.join("cible");
    std::fs::write(&target, b"autre").unwrap_or_else(|e| panic!("{e}"));
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644))
        .unwrap_or_else(|e| panic!("{e}"));
    let link = dir.join("lien.vcf");
    std::os::unix::fs::symlink(&target, &link).unwrap_or_else(|e| panic!("{e}"));
    assert!(write_atomic(&link, b"export").is_ok());
    assert_eq!(mode(&link), 0o600);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Nom d'un temporaire d'export de `target`, comme l'écrit `write_atomic`.
fn temporary(target: &str, n: u32) -> String {
    format!(".{target}.{}-{n}.tmp", std::process::id())
}

/// Recule la date de modification de `path` d'une heure.
fn age(path: &Path) {
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(old))
        .unwrap_or_else(|e| panic!("{e}"));
}

/// Crée `path` avec un contenu quelconque, modifié il y a une heure.
fn old_file(path: &Path) {
    std::fs::write(path, b"ancien carnet").unwrap_or_else(|e| panic!("{e}"));
    age(path);
}

#[test]
fn write_atomic_removes_the_temporaries_left_by_an_interrupted_export() {
    let dir = scratch("orphelins");
    let path = dir.join("contacts.vcf");
    // Le même numéro de processus d'un lancement à l'autre (bac à sable
    // Flatpak) n'empêche pas le nettoyage ; la casse non plus.
    for stale in [
        temporary("contacts.vcf", 0),
        temporary("contacts.vcf", 7),
        temporary("Contacts.VCF", 1),
    ] {
        old_file(&dir.join(stale));
    }
    // Rien d'autre n'est touché : autre cible, noms approchants.
    let others = [
        temporary("autre.vcf", 0),
        format!("{}.bak", temporary("contacts.vcf", 0)),
        temporary("contacts.vcf", 0).replacen('.', "", 1),
        "notes.txt".to_owned(),
    ];
    for other in &others {
        old_file(&dir.join(other));
    }
    assert!(write_atomic(&path, b"export").is_ok());
    assert_eq!(std::fs::read(&path).ok().as_deref(), Some(&b"export"[..]));
    let mut expected = others.to_vec();
    expected.push("contacts.vcf".to_owned());
    expected.sort();
    assert_eq!(entries(&dir), expected);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_atomic_keeps_the_temporaries_of_an_export_in_progress() {
    let dir = scratch("orphelins-vivants");
    let path = dir.join("contacts.vcf");
    // Temporaire récent (un export vient de le créer).
    let recent = temporary("contacts.vcf", 0);
    std::fs::write(dir.join(&recent), b"en cours").unwrap_or_else(|e| panic!("{e}"));
    // Temporaire ancien mais verrouillé par l'export qui l'écrit (autre
    // fenêtre, autre poste, écriture lente).
    let locked = temporary("contacts.vcf", 50);
    old_file(&dir.join(&locked));
    let writer = std::fs::File::options()
        .write(true)
        .open(dir.join(&locked))
        .unwrap_or_else(|e| panic!("{e}"));
    writer.try_lock().unwrap_or_else(|e| panic!("{e}"));
    assert!(write_atomic(&path, b"export").is_ok());
    let mut expected = vec![recent, locked.clone(), "contacts.vcf".to_owned()];
    expected.sort();
    assert_eq!(entries(&dir), expected);
    // Verrou relâché (export terminé ou interrompu) : effacé au suivant.
    drop(writer);
    assert!(write_atomic(&path, b"export").is_ok());
    assert!(!entries(&dir).contains(&locked));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_export_locks_its_temporary_while_writing_it() {
    // Le verrou de l'écrivain est vu par un autre descripteur du même
    // fichier, comme le verrait un autre export.
    let dir = scratch("verrou");
    let (temp, writer) =
        create_temporary(&dir, OsStr::new("contacts.vcf")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        temp.file_name().map(|n| n.to_string_lossy().into_owned()),
        Some(temporary("contacts.vcf", 0))
    );
    let other = open_temporary(&temp).unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        other.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    drop(writer);
    assert!(other.try_lock().is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn cleanup_never_removes_a_link_a_folder_or_a_socket_named_like_a_temporary() {
    let dir = scratch("orphelin-lien");
    let victim = dir.join("victime.vcf");
    old_file(&victim);
    let link = dir.join(temporary("contacts.vcf", 0));
    std::os::unix::fs::symlink(&victim, &link).unwrap_or_else(|e| panic!("{e}"));
    let socket = dir.join(temporary("contacts.vcf", 1));
    let _listener =
        std::os::unix::net::UnixListener::bind(&socket).unwrap_or_else(|e| panic!("{e}"));
    let folder = dir.join(temporary("contacts.vcf", 2));
    std::fs::create_dir(&folder).unwrap_or_else(|e| panic!("{e}"));
    // Témoin : un fichier ordinaire du même âge est bien effacé.
    let stale = dir.join(temporary("contacts.vcf", 3));
    std::fs::write(&stale, b"ancien carnet").unwrap_or_else(|e| panic!("{e}"));
    // Lien, socket et dossier ont leur propre date, récente : l'horloge est
    // avancée pour qu'aucun ne soit gardé à cause de son âge.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(86_400);
    remove_stale_temporaries(&dir, OsStr::new("contacts.vcf"), later);
    assert!(std::fs::symlink_metadata(&stale).is_err());
    assert_eq!(
        std::fs::read(&victim).ok().as_deref(),
        Some(&b"ancien carnet"[..])
    );
    assert!(
        std::fs::symlink_metadata(&link)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    );
    assert!(std::fs::symlink_metadata(&socket).is_ok());
    assert!(folder.is_dir());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn opening_a_temporary_never_follows_a_link_nor_blocks() {
    let dir = scratch("ouverture");
    let victim = dir.join("victime.vcf");
    old_file(&victim);
    let link = dir.join(temporary("contacts.vcf", 0));
    std::os::unix::fs::symlink(&victim, &link).unwrap_or_else(|e| panic!("{e}"));
    assert!(open_temporary(&link).is_err());
    assert!(open_temporary(&victim).is_ok());
    // Un tube nommé sans écrivain s'ouvre sans attendre.
    let fifo = dir.join(temporary("contacts.vcf", 1));
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(made.success());
    let (tx, rx) = std::sync::mpsc::channel();
    let opener = fifo.clone();
    std::thread::spawn(move || {
        let _ = tx.send(open_temporary(&opener).is_ok());
    });
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(5)).ok(),
        Some(true)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_export_file_written_in_pieces_replaces_the_target_once_committed() {
    let dir = scratch("morceaux");
    let path = dir.join("contacts.vcf");
    assert!(write_atomic(&path, b"ancien").is_ok());
    let mut file = ExportFile::create(&path).unwrap_or_else(|e| panic!("{e}"));
    for piece in [&b"pre"[..], b"", b"mier"] {
        assert!(file.write(piece).is_ok());
    }
    // Rien n'est remplacé avant la fin.
    assert_eq!(std::fs::read(&path).ok().as_deref(), Some(&b"ancien"[..]));
    assert!(file.commit().is_ok());
    assert_eq!(std::fs::read(&path).ok().as_deref(), Some(&b"premier"[..]));
    assert_eq!(entries(&dir), ["contacts.vcf"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_abandoned_export_file_leaves_the_target_intact_and_no_temporary() {
    let dir = scratch("abandon");
    let path = dir.join("contacts.ldif");
    assert!(write_atomic(&path, b"ancien").is_ok());
    let mut file = ExportFile::create(&path).unwrap_or_else(|e| panic!("{e}"));
    assert!(file.write(b"en cours").is_ok());
    // Une erreur de l'export abandonne le fichier.
    drop(file);
    assert_eq!(std::fs::read(&path).ok().as_deref(), Some(&b"ancien"[..]));
    assert_eq!(entries(&dir), ["contacts.ldif"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_export_file_refuses_to_grow_past_the_bound() {
    let dir = scratch("borne-morceaux");
    let path = dir.join("contacts.csv");
    let mut file = ExportFile::create(&path).unwrap_or_else(|e| panic!("{e}"));
    let half = vec![b'x'; MAX_EXPORT_FILE_BYTES / 2 + 1];
    assert!(file.write(&half).is_ok());
    let refused = file.write(&half).map_err(|e| e.kind());
    assert_eq!(refused, Err(std::io::ErrorKind::FileTooLarge));
    // Rien de plus n'est écrit dans le temporaire.
    let temp = dir.join(&entries(&dir)[0]);
    assert_eq!(
        std::fs::metadata(&temp).map(|m| m.len()).ok(),
        u64::try_from(half.len()).ok()
    );
    drop(file);
    assert!(entries(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Format qui refuse les fiches de sa deuxième page.
#[derive(Debug, Clone, Copy)]
struct SecondPageRefused;

/// Écriture du faux : un octet par fiche, deuxième page refusée.
#[derive(Debug)]
struct SecondPageEncoder(usize);

impl ContactCodec for SecondPageRefused {
    fn decode(
        &self,
        _: &[u8],
    ) -> Result<norelly_domain::contact_import::DecodedContacts, ContactCodecError> {
        Err(ContactCodecError::UnknownFormat)
    }

    fn encode(
        &self,
        _: &[norelly_domain::contact_export::ExportCard],
        _: &[norelly_domain::contact_export::ExportList],
        _: ExportFormat,
    ) -> Result<Vec<u8>, ContactCodecError> {
        Err(ContactCodecError::TooLarge)
    }

    type Encoder = SecondPageEncoder;

    fn encoder(&self, _: ExportFormat) -> SecondPageEncoder {
        SecondPageEncoder(0)
    }
}

impl ContactEncoder for SecondPageEncoder {
    fn survey(&mut self, _: &[norelly_domain::contact_export::ExportCard]) {}

    fn cards(
        &mut self,
        cards: &[norelly_domain::contact_export::ExportCard],
    ) -> Result<Vec<u8>, ContactCodecError> {
        self.0 += 1;
        if self.0 == 2 {
            return Err(ContactCodecError::TooLarge);
        }
        Ok(vec![b'x'; cards.len()])
    }

    fn lists(
        &mut self,
        _: &[norelly_domain::contact_export::ExportList],
        _: &[norelly_domain::contact_export::ExportMember],
    ) -> Result<Vec<u8>, ContactCodecError> {
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<Vec<u8>, ContactCodecError> {
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn an_export_refused_midway_leaves_the_target_intact_and_no_temporary() {
    use norelly_domain::contact_export::EXPORT_PAGE_SIZE;
    use norelly_domain::ports::ContactBook;
    use norelly_test_support::contact_photo_contracts::InMemoryPhotoStore;
    use norelly_test_support::contacts_contracts::{InMemoryContactBook, sample_contact};

    let book = InMemoryContactBook::default();
    let drafts: Vec<_> = (0..=EXPORT_PAGE_SIZE)
        .map(|i| sample_contact(&format!("c{i:04}"), &[&format!("c{i}@example.com")]))
        .collect();
    book.import(&drafts, &[], &[], &[])
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let photos = InMemoryPhotoStore::default();
    let mut export = PrepareContactExport {
        directory: &book,
        photos: &photos,
        codec: &SecondPageRefused,
    }
    .execute(&ContactSelection::All, ExportFormat::VCard4)
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    let dir = scratch("refus-en-route");
    let path = dir.join("contacts.vcf");
    assert!(write_atomic(&path, b"ancien").is_ok());
    let file = ExportFile::create(&path).unwrap_or_else(|e| panic!("{e}"));
    let e = write_export(&mut export, file).await.err().map(|e| e.code);
    assert_eq!(e, Some(ErrorCodeDto::ContactExportTooLarge));
    assert_eq!(std::fs::read(&path).ok().as_deref(), Some(&b"ancien"[..]));
    assert_eq!(entries(&dir), ["contacts.vcf"]);
    let _ = std::fs::remove_dir_all(&dir);
}
