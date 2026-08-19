use std::fmt::Write as _;
use std::fs;
use std::path::Path;

pub fn publish_replacing(temp: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(temp, destination).map_err(|error| retained(error.to_string(), temp))
}

pub fn publish_no_replace(temp: &Path, destination: &Path) -> Result<(), String> {
    no_replace_rename(temp, destination).map_err(|error| {
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

fn is_destination_occupied(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::AlreadyExists
        || matches!(error.raw_os_error(), Some(183) | Some(80))
}

fn retained(mut message: String, temp: &Path) -> String {
    if let Err(error) = write!(
        message,
        "; the written document is retained at {}",
        temp.display()
    ) {
        let _ = error;
    }
    message
}

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
    let ok = unsafe { MoveFileExW(existing.as_ptr(), replacement.as_ptr(), 0) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

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

    fn immovable_destination(root: &Path, name: &str) -> std::path::PathBuf {
        let destination = root.join(name);
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("occupant"), b"not a rename target").unwrap();
        destination
    }

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
        assert!(
            error.contains("is already published"),
            "the refusal must name the cause: {error}"
        );
        assert_eq!(fs::read(&temp).unwrap(), b"ours");
        portable_test_fs_retry::remove_dir_all(&root).unwrap();
    }

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
        let unrelated =
            no_replace_rename(&root.join("absent.tmp"), &root.join("elsewhere.json")).unwrap_err();
        assert!(
            !is_destination_occupied(&unrelated),
            "a missing source is not an occupied destination: {unrelated:?}"
        );
        portable_test_fs_retry::remove_dir_all(&root).unwrap();
    }

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
