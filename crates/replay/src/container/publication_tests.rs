use super::*;
use crate::publication::{ReplaceError, posix_replace, posix_replace_with, replace_file_with};
use std::{
    fs,
    os::windows::fs::OpenOptionsExt,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "battle-plan-publication-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn revisions() -> (Replay, Replay) {
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2])]).unwrap();
    let new = old.append(&[records(2, &[3, 4])]).unwrap();
    (old, new)
}

fn reader(path: &Path, share_delete: bool) -> File {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | if share_delete { FILE_SHARE_DELETE } else { 0 })
        .open(path)
        .unwrap()
}

fn verify_revision(bytes: Vec<u8>, expected: &Replay) {
    assert_eq!(bytes, expected.bytes());
    let replay = Replay::open(bytes).unwrap();
    assert_eq!(replay.index, expected.index);
    for block in &replay.index {
        assert_eq!(
            replay.read_block(block).unwrap(),
            expected.read_block(block).unwrap(),
        );
    }
}

#[test]
fn replace_while_reader_is_midway_keeps_old_bytes_and_new_opens_see_append() {
    let scratch = Scratch::new();
    let path = scratch.0.join("recording.battleplan");
    let (old, new) = revisions();
    old.write_atomic(&path).unwrap();
    let mut held = reader(&path, true);
    let (midway_tx, midway_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let (result, new_open, crossed, reread) = std::thread::scope(|scope| {
        let old_size = old.bytes().len();
        let reading = scope.spawn(move || {
            let mut crossed = vec![0; old_size];
            held.read_exact(&mut crossed[..old_size / 2]).unwrap();
            midway_tx.send(()).unwrap();
            resume_rx.recv_timeout(Duration::from_secs(30)).unwrap();
            held.read_exact(&mut crossed[old_size / 2..]).unwrap();
            held.rewind().unwrap();
            let mut reread = Vec::new();
            held.read_to_end(&mut reread).unwrap();
            (crossed, reread)
        });
        midway_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        // Publication must finish while the reader is paused, without closing it.
        let result = new.write_atomic(&path);
        let new_open = fs::read(&path);
        resume_tx.send(()).unwrap();
        let (crossed, reread) = reading.join().unwrap();
        (result, new_open, crossed, reread)
    });
    result.unwrap();
    verify_revision(crossed, &old);
    verify_revision(reread, &old);
    verify_revision(new_open.unwrap(), &new);
    assert!(!temporary_path(&path).exists());
}

#[test]
fn reader_without_delete_sharing_returns_sharing_error_and_preserves_old_file() {
    let scratch = Scratch::new();
    let path = scratch.0.join("recording.battleplan");
    let (old, new) = revisions();
    old.write_atomic(&path).unwrap();
    let mut held = reader(&path, false);
    let error = new.write_atomic(&path).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(32));
    verify_revision(fs::read(&path).unwrap(), &old);
    let mut bytes = Vec::new();
    held.read_to_end(&mut bytes).unwrap();
    verify_revision(bytes, &old);
    verify_revision(fs::read(temporary_path(&path)).unwrap(), &new);
    drop(held);
    new.write_atomic(&path).unwrap();
    verify_revision(fs::read(&path).unwrap(), &new);
}

// Replay still performs every byte write and the temp sync. Only the rename
// syscall result and the immediate post-rename checkpoint are injectable.
struct RenamePublisher<'a> {
    inner: FilePublisher,
    rename: &'a mut dyn FnMut(&File, &[u16]) -> io::Result<bool>,
    after_rename: &'a mut dyn FnMut() -> io::Result<()>,
}

impl AtomicPublisher for RenamePublisher<'_> {
    type Writer = <FilePublisher as AtomicPublisher>::Writer;

    fn create(&mut self) -> io::Result<Self::Writer> {
        self.inner.create()
    }

    fn sync(&mut self, writer: &mut Self::Writer) -> io::Result<()> {
        self.inner.sync(writer)
    }

    fn replace(&mut self, writer: Self::Writer) -> io::Result<()> {
        drop(writer);
        Ok(replace_file_with(
            &self.inner.temporary,
            &self.inner.destination,
            &mut self.rename,
            &mut self.after_rename,
        )?)
    }
}

fn publisher<'a>(
    path: &Path,
    rename: &'a mut dyn FnMut(&File, &[u16]) -> io::Result<bool>,
    after_rename: &'a mut dyn FnMut() -> io::Result<()>,
) -> RenamePublisher<'a> {
    RenamePublisher {
        inner: FilePublisher {
            temporary: temporary_path(path),
            destination: path.into(),
        },
        rename,
        after_rename,
    }
}

#[test]
fn unsupported_rename_codes_use_classic_replace_but_access_and_sharing_errors_do_not() {
    let (old, new) = revisions();
    for code in [1, 50, 87, 5, 32] {
        let scratch = Scratch::new();
        let path = scratch.0.join("recording.battleplan");
        old.write_atomic(&path).unwrap();
        let held = reader(&path, true);
        let unsupported = matches!(code, 1 | 50 | 87);
        let mut fell_back = false;
        let mut rename = |_: &File, target: &[u16]| {
            let supported =
                posix_replace_with(target, |_, _| Err(io::Error::from_raw_os_error(code)))?;
            fell_back = !supported;
            Ok(supported)
        };
        let mut after_rename = || panic!("injected rename must not succeed");
        let error = new
            .publish_with(
                &mut publisher(&path, &mut rename, &mut after_rename),
                |_| Ok(()),
            )
            .unwrap_err();
        // Unsupported codes run real MoveFileExW, refused by the held reader.
        assert_eq!(
            error.raw_os_error(),
            Some(if unsupported { 5 } else { code })
        );
        verify_revision(fs::read(&path).unwrap(), &old);
        drop(held);
        let retry = new.publish_with(
            &mut publisher(&path, &mut rename, &mut after_rename),
            |_| Ok(()),
        );
        assert_eq!(fell_back, unsupported);
        if unsupported {
            retry.unwrap();
            verify_revision(fs::read(&path).unwrap(), &new);
            assert!(!temporary_path(&path).exists());
        } else {
            assert_eq!(retry.unwrap_err().raw_os_error(), Some(code));
            verify_revision(fs::read(&path).unwrap(), &old);
        }
    }
}

#[test]
fn errors_before_rename_keep_old_file_and_errors_after_rename_report_new_file() {
    let scratch = Scratch::new();
    let path = scratch.0.join("recording.battleplan");
    let (old, new) = revisions();
    old.write_atomic(&path).unwrap();
    let error = new
        .write_atomic_with(&path, |stage| {
            if stage == PublishStage::Flushed {
                Err(io::Error::other("injected after temp sync"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error.to_string(), "injected after temp sync");
    verify_revision(fs::read(&path).unwrap(), &old);
    let mut rename = posix_replace;
    let mut after_rename = || Err(io::Error::other("injected after rename"));
    let error = new
        .publish_with(
            &mut publisher(&path, &mut rename, &mut after_rename),
            |_| Ok(()),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "injected after rename");
    verify_revision(fs::read(&path).unwrap(), &new);
    assert!(!temporary_path(&path).exists());
}

#[test]
fn replacement_errors_tell_a_refused_rename_from_a_failed_flush() {
    let scratch = Scratch::new();
    let path = scratch.0.join("published.bin");
    let staged = scratch.0.join("published.bin.tmp");
    fs::write(&path, b"old").unwrap();
    fs::write(&staged, b"new").unwrap();
    let refused = replace_file_with(
        &staged,
        &path,
        |_, _| Err(io::Error::from_raw_os_error(5)),
        || panic!("a refused rename has nothing to flush"),
    );
    assert!(matches!(refused, Err(ReplaceError::Unrenamed(_))));
    assert_eq!(fs::read(&path).unwrap(), b"old");
    assert!(staged.exists());
    let unflushed = replace_file_with(&staged, &path, posix_replace, || {
        Err(io::Error::other("flush failed"))
    });
    assert!(matches!(unflushed, Err(ReplaceError::Unflushed(_))));
    assert_eq!(fs::read(&path).unwrap(), b"new");
    assert!(!staged.exists());
}

const CRASH_DESTINATION: &str = "battle-plan_PUBLICATION_CRASH_DESTINATION";
const CRASH_CHECKPOINT: &str = "battle-plan_PUBLICATION_CRASH_CHECKPOINT";
const CRASH_CHILD: &str = "container::tests::publication_tests::publication_crash_child";

#[test]
#[ignore = "launched by publication_crashes_leave_complete_old_or_new_destination"]
fn publication_crash_child() {
    let path = PathBuf::from(std::env::var_os(CRASH_DESTINATION).unwrap());
    let stop = std::env::var(CRASH_CHECKPOINT).unwrap();
    let (_, new) = revisions();
    let abort = || {
        fs::write(path.with_extension("checkpoint"), &stop).unwrap();
        std::process::abort();
    };
    let mut rename = posix_replace;
    let mut after_rename = || {
        if stop == "renamed" {
            abort();
        }
        Ok(())
    };
    new.publish_with(
        &mut publisher(&path, &mut rename, &mut after_rename),
        |stage| {
            if stop == "synced" && stage == PublishStage::Flushed {
                abort();
            }
            Ok(())
        },
    )
    .unwrap();
    panic!("publisher did not reach crash checkpoint");
}

#[test]
fn publication_crashes_leave_complete_old_or_new_destination() {
    let (old, new) = revisions();
    for stop in ["synced", "renamed"] {
        let scratch = Scratch::new();
        let path = scratch.0.join("recording.battleplan");
        old.write_atomic(&path).unwrap();
        let mut held = reader(&path, true);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", CRASH_CHILD, "--nocapture"])
            .env(CRASH_DESTINATION, &path)
            .env(CRASH_CHECKPOINT, stop)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let started = Instant::now();
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() >= Duration::from_secs(30) {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!("crash child timed out: {output:?}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        assert_eq!(
            fs::read_to_string(path.with_extension("checkpoint")).unwrap_or_else(|error| {
                panic!("checkpoint missing: {error}; child output: {output:?}")
            }),
            stop,
        );
        verify_revision(
            fs::read(&path).unwrap(),
            if stop == "synced" { &old } else { &new },
        );
        let mut held_bytes = Vec::new();
        held.read_to_end(&mut held_bytes).unwrap();
        verify_revision(held_bytes, &old);
        if stop == "synced" {
            verify_revision(fs::read(temporary_path(&path)).unwrap(), &new);
        } else {
            assert!(!temporary_path(&path).exists());
        }
    }
}
