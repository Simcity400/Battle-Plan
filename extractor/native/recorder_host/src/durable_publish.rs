//! Publishing a written temporary at its final name, without destroying it on
//! failure.
//!
//! # The defect this closes
//!
//! Every durable document the recorder writes — session manifest, diagnostic
//! receipt, sink-failure receipt, raw-debug startup-failure receipt, the
//! finalize manifest rewrite, and the create-new receipts — is written to a
//! same-directory temporary, `sync_all`ed, closed, and then renamed into place
//! so a reader sees the old bytes or the new bytes and never a partial file.
//! That part was already right.
//!
//! What was wrong is the failure path. Five of the six did
//! `let _ = fs::remove_file(&temp)` when the rename failed, which DELETES the
//! only copy of a document that had just been produced and durably written. The
//! rename is precisely the operation most likely to fail transiently on
//! Windows: a real-time antimalware scanner takes a handle on the freshly
//! written temporary, and a rename whose source is held returns
//! `ERROR_SHARING_VIOLATION` (32). A momentary scanner handle therefore
//! destroyed evidence that a human could otherwise have picked up off the disk.
//!
//! Two standing rules say what to do instead. Partial data still seals: missing
//! data never destroys a capture, it is excluded and accounted for. And a
//! degraded outcome names its cause. So a failed publication now KEEPS the
//! written document and NAMES where it is, in the error string the caller
//! surfaces.
//!
//! # No retry here, deliberately
//!
//! A bounded retry around the rename would reduce how often this happens, and
//! it is affordable on the finalize publications. It is NOT affordable on all
//! six: [`publish_no_replace`] is reached from `CAPTURE_READY_V2.json` and
//! `TARGET_LEASE_V1.json`, which are published while the target process is
//! alive or attaching, and a stall on that path is the class of change that has
//! to be decided against "never make the engine wait" rather than inside a
//! lane. Retention costs nothing and stalls nothing, so it lands on its own.
//!
//! # Replace versus create-new
//!
//! Windows `MoveFileExW` is the primitive under both, and the two differ by one
//! flag:
//!
//! * [`publish_replacing`] is for a document that OVERWRITES a known previous
//!   version of itself — the manifest as finalize adds to it. Rust's
//!   `fs::rename` already passes `MOVEFILE_REPLACE_EXISTING` on Windows, so
//!   this is `fs::rename` plus the retention contract.
//! * [`publish_no_replace`] is for a document that must be published exactly
//!   once. An `exists()` check followed by a REPLACING rename is not that: it
//!   is a check with a window after it, and anything that appears at the
//!   destination inside that window is silently overwritten. Here the refusal
//!   is the operating system's, taken atomically with the publication, so a
//!   competing publisher's document survives.
//!
//! Neither call ever leaves the destination half-written: `MoveFileExW` on one
//! volume either performs the rename or does not.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

/// Publish `temp` at `destination`, replacing whatever is there.
///
/// On success the temporary is gone — it IS the destination now. On failure the
/// temporary is left on disk and named in the returned message, so the document
/// can be recovered by hand.
pub fn publish_replacing(temp: &Path, destination: &Path) -> Result<(), String> {
    // `fs::rename` is `MoveFileExW(.., MOVEFILE_REPLACE_EXISTING)` on Windows
    // and `rename(2)` elsewhere: on both, the destination goes from its old
    // bytes to its new bytes with no interval of neither.
    fs::rename(temp, destination).map_err(|error| retained(error.to_string(), temp))
}

/// Publish `temp` at `destination` only if nothing is there.
///
/// The refusal is atomic with the publication, so a document that appeared at
/// the destination after the caller last looked is NOT overwritten. On failure
/// — including that refusal — the temporary is retained and named.
pub fn publish_no_replace(temp: &Path, destination: &Path) -> Result<(), String> {
    no_replace_rename(temp, destination).map_err(|error| {
        // Name the cause rather than leaving the operator to decode a raw
        // errno: "somebody else already published this" and "the filesystem
        // refused the move" are different situations with different next steps,
        // and only the first one means this run is not the sole publisher.
        let cause = if is_destination_occupied(&error) {
            format!(
                "{} is already published, so it was NOT replaced ({error})",
                destination.display()
            )
        } else {
            error.to_string()
        };
        retained(cause, temp)
    })
}

/// Does this error mean the destination was already taken?
///
/// `ERROR_ALREADY_EXISTS` (183) and `ERROR_FILE_EXISTS` (80) are both spelled
/// by `MoveFileExW` for an occupied destination; `AlreadyExists` covers the
/// `hard_link` path off Windows.
fn is_destination_occupied(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::AlreadyExists
        || matches!(error.raw_os_error(), Some(183) | Some(80))
}

/// The retention clause. One shape, so a caller's prefix is the only thing that
/// differs between the six publications and the operator always reads the same
/// sentence.
fn retained(mut message: String, temp: &Path) -> String {
    if let Err(error) = write!(
        message,
        "; the written document is retained at {}",
        temp.display()
    ) {
        // `String`'s `fmt::Write` is infallible; this arm exists so the failure
        // is not silently swallowed if that ever stops being true.
        let _ = error;
    }
    message
}

/// `MoveFileExW` WITHOUT `MOVEFILE_REPLACE_EXISTING`.
///
/// `MOVEFILE_WRITE_THROUGH` is not passed: it asks the rename itself to be
/// flushed before returning, and every caller here has already `sync_all`ed the
/// bytes. The directory-entry durability that flag buys is not what this
/// function is for, and it is not what the replacing path does either, so the
/// two stay comparable.
#[cfg(windows)]
fn no_replace_rename(temp: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    fn wide(path: &Path) -> std::io::Result<Vec<u16>> {
        let mut out: Vec<u16> = path.as_os_str().encode_wide().collect();
        if out.contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "publication path contains an embedded NUL",
            ));
        }
        out.push(0);
        Ok(out)
    }

    let existing = wide(temp)?;
    let replacement = wide(destination)?;
    // Flags 0: move, and refuse if the destination is taken.
    let ok = unsafe { MoveFileExW(existing.as_ptr(), replacement.as_ptr(), 0) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// `hard_link` is the portable create-new publication: it fails with
/// `AlreadyExists` rather than replacing, which is the property this function is
/// named for. The temporary is then unlinked, leaving one name for the inode.
///
/// This arm is NOT link-and-unlink-atomic: if the unlink fails the document is
/// published AND the temporary is still there, so the "success consumes the
/// temporary" half of [`publish_no_replace`]'s contract does not hold here. It
/// is stated rather than hidden, and it is loud: publication succeeded, so
/// returning an error would be the worse lie. `recorder_host` only ships on
/// Windows, where [`no_replace_rename`] is a single `MoveFileExW` and the
/// question does not arise; this arm exists so the crate still builds and its
/// tests still run on a developer's non-Windows checkout.
#[cfg(not(windows))]
fn no_replace_rename(temp: &Path, destination: &Path) -> std::io::Result<()> {
    fs::hard_link(temp, destination)?;
    if let Err(error) = fs::remove_file(temp) {
        eprintln!(
            "recorder note: published {} but could not unlink its temporary {}: {error}",
            destination.display(),
            temp.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ls_durable_publish_{label}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// A rename onto a NON-EMPTY DIRECTORY fails on every target this builds
    /// for, which is the deterministic failure lever these tests need. It is a
    /// stand-in for the transient one that actually happens (a scanner holding
    /// the temporary): the failure path under test does not read the error, it
    /// only decides what to do with the temporary.
    fn immovable_destination(root: &Path, name: &str) -> std::path::PathBuf {
        let destination = root.join(name);
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("occupant"), b"not a rename target").unwrap();
        destination
    }

    /// kills: `let _ = fs::remove_file(&temp)` on the rename failure path of any
    /// of the six publications, which is what deleted the only copy of a
    /// document that had just been durably written.
    #[test]
    fn a_failed_replacing_publication_keeps_the_document_and_names_it() {
        let root = root("replace_fail");
        let temp = root.join(".doc.tmp");
        fs::write(&temp, b"the whole document").unwrap();
        let destination = immovable_destination(&root, "doc.json");

        let error = publish_replacing(&temp, &destination).unwrap_err();
        assert!(
            error.contains(&temp.display().to_string()),
            "the error must name the retained temporary: {error}"
        );
        assert_eq!(
            fs::read(&temp).unwrap(),
            b"the whole document",
            "the written document must survive a failed publication"
        );
        portable_test_fs_retry::remove_dir_all(&root).unwrap();
    }

    /// kills: the same removal on the create-new publication.
    ///
    /// The failure here is an ABSENT parent, deliberately not an occupied
    /// destination: that keeps this test about retention on a failure the
    /// create-new refusal below is not already covering.
    #[test]
    fn a_failed_create_new_publication_keeps_the_document_and_names_it() {
        let root = root("create_new_fail");
        let temp = root.join(".doc.tmp");
        fs::write(&temp, b"the whole document").unwrap();
        let destination = root.join("no_such_directory").join("doc.json");

        let error = publish_no_replace(&temp, &destination).unwrap_err();
        assert!(
            error.contains(&temp.display().to_string()),
            "the error must name the retained temporary: {error}"
        );
        assert_eq!(fs::read(&temp).unwrap(), b"the whole document");
        portable_test_fs_retry::remove_dir_all(&root).unwrap();
    }

    /// The create-new semantic itself.
    ///
    /// kills: implementing `publish_no_replace` with `fs::rename` (or
    /// `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`), which on Windows
    /// silently overwrites the destination. With a replacing rename the
    /// destination would read `"ours"` and the call would return `Ok`.
    #[test]
    fn a_create_new_publication_refuses_an_occupied_destination_without_touching_it() {
        let root = root("create_new_occupied");
        let temp = root.join(".doc.tmp");
        fs::write(&temp, b"ours").unwrap();
        let destination = root.join("doc.json");
        fs::write(&destination, b"a competing publisher got here first").unwrap();

        let error = publish_no_replace(&temp, &destination).unwrap_err();
        assert_eq!(
            fs::read(&destination).unwrap(),
            b"a competing publisher got here first",
            "the competing document must survive: {error}"
        );
        assert!(
            error.contains(&temp.display().to_string()),
            "the refusal must still name the retained temporary: {error}"
        );
        // kills: reporting an occupied destination as a generic IO failure,
        // which leaves the operator unable to tell "somebody else published
        // this" from "the filesystem refused the move".
        assert!(
            error.contains("is already published"),
            "the refusal must name the cause: {error}"
        );
        assert_eq!(fs::read(&temp).unwrap(), b"ours");
        portable_test_fs_retry::remove_dir_all(&root).unwrap();
    }

    /// kills: classifying an occupied destination as anything else, which is
    /// what would let a caller report "publish failed" for a refusal that means
    /// somebody else already published.
    #[test]
    fn an_occupied_destination_is_recognisable_as_occupied() {
        let root = root("occupied_class");
        let temp = root.join(".doc.tmp");
        fs::write(&temp, b"ours").unwrap();
        let destination = root.join("doc.json");
        fs::write(&destination, b"theirs").unwrap();

        let error = no_replace_rename(&temp, &destination).unwrap_err();
        assert!(
            is_destination_occupied(&error),
            "an occupied destination must classify as occupied: {error:?} ({:?})",
            error.raw_os_error()
        );
        // And an unrelated failure must NOT.
        let unrelated = no_replace_rename(&root.join("absent.tmp"), &root.join("elsewhere.json"))
            .unwrap_err();
        assert!(
            !is_destination_occupied(&unrelated),
            "a missing source is not an occupied destination: {unrelated:?}"
        );
        portable_test_fs_retry::remove_dir_all(&root).unwrap();
    }

    /// kills: a publication that leaves its temporary behind on SUCCESS, which
    /// would litter every run directory with one file per published document.
    #[test]
    fn a_successful_publication_consumes_the_temporary() {
        let root = root("success");
        for (temp_name, destination_name, publish) in [
            (
                ".replace.tmp",
                "replace.json",
                publish_replacing as fn(&Path, &Path) -> Result<(), String>,
            ),
            (".create.tmp", "create.json", publish_no_replace),
        ] {
            let temp = root.join(temp_name);
            let destination = root.join(destination_name);
            fs::write(&temp, b"published bytes").unwrap();
            publish(&temp, &destination).unwrap();
            assert!(!temp.exists(), "{temp_name} must not survive publication");
            assert_eq!(fs::read(&destination).unwrap(), b"published bytes");
        }
        portable_test_fs_retry::remove_dir_all(&root).unwrap();
    }

    /// kills: making `publish_replacing` refuse an existing destination, which
    /// would break the manifest updates that legitimately overwrite themselves.
    #[test]
    fn a_replacing_publication_replaces() {
        let root = root("replace_ok");
        let temp = root.join(".manifest.tmp");
        fs::write(&temp, b"new manifest").unwrap();
        let destination = root.join("manifest.json");
        fs::write(&destination, b"old manifest").unwrap();

        publish_replacing(&temp, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"new manifest");
        assert!(!temp.exists());
        portable_test_fs_retry::remove_dir_all(&root).unwrap();
    }
}
