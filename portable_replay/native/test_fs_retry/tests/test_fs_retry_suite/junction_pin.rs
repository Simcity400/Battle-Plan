//! kills: lane QI console-window defect class. Every former junction fixture
//! shelled out to the cmd link builtin (`cmd /C mklink /J`), a console
//! program, so each call from a console-less harness allocated a fresh,
//! visible terminal window on the desktop. The native helper below replaces
//! those spawns; this pin holds it to mklink's observable contract in-process:
//! the junction is created, reads back as a mount-point reparse point,
//! redirects traversal to the target, refuses a missing target, refuses an
//! occupied link path, and never silently repoints an existing junction --
//! exercised specifically against an alias whose current target is EMPTY,
//! the exact case the repoint defect lived in.

#![cfg(windows)]

use portable_test_fs_retry::{create_junction, reparse_tag_of, IO_REPARSE_TAG_MOUNT_POINT};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

struct Temp(std::path::PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "test_fs_retry_junction_{tag}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn creates_a_junction_that_reads_back_as_a_mount_point_and_redirects() {
    let temp = Temp::new("roundtrip");
    // The target starts EMPTY: an alias over an empty directory is exactly
    // the shape a naive implementation would happily repoint, because
    // following the link sees no entries.
    let target = temp.0.join("real");
    fs::create_dir_all(&target).unwrap();

    let link = temp.0.join("alias");
    create_junction(&link, &target).expect("the native junction must be created");

    assert!(
        fs::symlink_metadata(&link)
            .expect("the junction exists on disk")
            .file_type()
            .is_symlink(),
        "the created entry must be a reparse point"
    );
    assert_eq!(
        reparse_tag_of(&link).expect("read the junction back"),
        IO_REPARSE_TAG_MOUNT_POINT,
        "the tag must be exactly the mount-point tag"
    );

    // An existing junction in the link position is occupied even when its
    // TARGET is empty: the same-tag reparse submission would silently repoint
    // it, so the helper must refuse before opening the link.
    let other_empty_target = temp.0.join("other_empty");
    fs::create_dir_all(&other_empty_target).unwrap();
    let error = create_junction(&link, &other_empty_target)
        .expect_err("an existing junction is occupied and must never be repointed");
    assert!(error.contains("occupied"), "{error}");
    assert_eq!(
        reparse_tag_of(&link).expect("the original junction survives"),
        IO_REPARSE_TAG_MOUNT_POINT,
        "the refused attempt must leave the original junction untouched"
    );
    assert!(
        fs::read_dir(&other_empty_target).unwrap().next().is_none(),
        "the refused attempt must not have written through the alias"
    );

    // The surviving junction still redirects to ITS ORIGINAL target: content
    // written there after the refused attempt is visible through the alias,
    // which a mere unchanged-tag check could not prove.
    fs::write(target.join("payload.txt"), b"through the alias").unwrap();
    assert_eq!(
        fs::read(link.join("payload.txt")).unwrap(),
        b"through the alias",
        "traversal through the junction must land in the original target directory"
    );

    // Idempotence at the same paths is refused, not silently duplicated.
    let error = create_junction(&link, &target).expect_err("an existing junction is occupied");
    assert!(error.contains("occupied"), "{error}");
}

#[test]
fn refuses_to_point_a_junction_at_a_missing_target() {
    let temp = Temp::new("missing_target");
    let link = temp.0.join("alias");
    let missing = temp.0.join("never_created");

    let error = create_junction(&link, &missing)
        .expect_err("a junction into the void must be refused, not fabricated");
    assert!(error.contains("does not name an existing directory"), "{error}");
    assert!(
        !link.exists(),
        "a refused creation must not leave a link behind"
    );
}

#[test]
fn refuses_an_occupied_link_path() {
    let temp = Temp::new("occupied_link");
    let target = temp.0.join("real");
    fs::create_dir_all(&target).unwrap();
    let link = temp.0.join("alias");
    fs::create_dir_all(&link).unwrap();
    fs::write(link.join("resident.txt"), b"do not clobber").unwrap();

    let error = create_junction(&link, &target)
        .expect_err("a populated directory must never be converted or replaced");
    assert!(error.contains("occupied"), "{error}");
    assert_eq!(
        fs::read(link.join("resident.txt")).unwrap(),
        b"do not clobber",
        "the refusal must leave the resident content untouched"
    );
}

#[test]
fn converts_only_an_empty_directory_left_in_the_link_position_like_the_shell_did() {
    // The retired shell form accepted an empty directory in the link position
    // (it tagged it with the mount-point reparse data). Fixtures rely on that
    // tolerance; the native helper keeps it -- but only for a PLAIN empty
    // directory, never for an existing junction.
    let temp = Temp::new("empty_link_dir");
    let target = temp.0.join("real");
    fs::create_dir_all(target.join("nested")).unwrap();
    let link = temp.0.join("alias");
    fs::create_dir_all(&link).unwrap();

    create_junction(&link, &target).expect("an empty link-position directory is convertible");
    assert!(std::path::Path::new(&link).join("nested").is_dir());
}
