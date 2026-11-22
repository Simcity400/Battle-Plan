use std::{
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

pub const CLOSED: &str = "BattlePlan is closing, so nothing more is written.";

#[derive(Default)]
struct Count {
    running: usize,
    closed: bool,
}

pub struct Writes {
    count: Mutex<Count>,
    idle: Condvar,
}

pub struct Writing<'a>(&'a Writes);

impl Drop for Writing<'_> {
    fn drop(&mut self) {
        self.0.lock().running -= 1;
        self.0.idle.notify_all();
    }
}

impl Default for Writes {
    fn default() -> Self {
        Self::new()
    }
}

static STAGED: AtomicU64 = AtomicU64::new(0);

impl Writes {
    pub const fn new() -> Self {
        Self { count: Mutex::new(Count { running: 0, closed: false }), idle: Condvar::new() }
    }

    fn lock(&self) -> MutexGuard<'_, Count> {
        self.count.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn begin(&self) -> io::Result<Writing<'_>> {
        let mut count = self.lock();
        if count.closed {
            return Err(io::Error::new(io::ErrorKind::Interrupted, CLOSED));
        }
        count.running += 1;
        Ok(Writing(self))
    }

    pub fn admitted(&self) -> Writing<'_> {
        self.lock().running += 1;
        Writing(self)
    }

    pub fn under_lock<T>(&self, lock: &Path, work: impl FnOnce(&Writing<'_>) -> io::Result<T>) -> io::Result<T> {
        if let Some(folder) = lock.parent() {
            fs::create_dir_all(folder)?;
        }
        let file = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock)?;
        file.lock()?;
        let writing = self.begin()?;
        work(&writing)
    }

    pub fn close_admission(&self) -> bool {
        let mut count = self.lock();
        count.closed = true;
        count.running > 0
    }

    pub fn drained_within(&self, limit: std::time::Duration) -> bool {
        let count = self.lock();
        let (count, _) = self.idle.wait_timeout_while(count, limit, |count| count.running > 0).unwrap_or_else(PoisonError::into_inner);
        count.running == 0
    }

    pub fn close(&self) {
        let mut count = self.lock();
        count.closed = true;
        while count.running > 0 {
            count = self.idle.wait(count).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Writing<'_> {
    pub fn replace(&self, path: &Path, write: impl FnOnce(&mut BufWriter<&File>) -> io::Result<()>) -> io::Result<()> {
        if let Some(folder) = path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
            fs::create_dir_all(folder)?;
        }
        let mut name = path.as_os_str().to_owned();
        name.push(format!(".{}.{}.tmp", std::process::id(), STAGED.fetch_add(1, Ordering::Relaxed)));
        let staged = PathBuf::from(name);
        let file = fs::OpenOptions::new().write(true).create_new(true).open(&staged)?;
        let result = (|| {
            let mut out = BufWriter::new(&file);
            write(&mut out)?;
            out.flush()?;
            drop(out);
            file.sync_all()?;
            drop(file);
            #[cfg(windows)]
            let replaced = crate::publication::replace_file(&staged, path).map_err(io::Error::from);
            #[cfg(not(windows))]
            let replaced = fs::rename(&staged, path);
            replaced
        })();
        let _ = fs::remove_file(&staged);
        result
    }
}

pub static WRITES: Writes = Writes::new();

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("battleplan-writes-{name}-{}-{}", std::process::id(), STAGED.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn closing_waits_for_the_writes_under_way_and_refuses_new_ones() {
        let writes = Writes::new();
        let writing = writes.begin().unwrap();
        std::thread::scope(|scope| {
            let (closing, closed) = std::sync::mpsc::channel();
            let writes = &writes;
            scope.spawn(move || {
                writes.close();
                closing.send(()).unwrap();
            });
            while !writes.lock().closed {
                std::thread::yield_now();
            }
            assert!(closed.try_recv().is_err(), "the write under way holds the exit");
            assert_eq!(writes.begin().err().map(|error| error.to_string()), Some(CLOSED.to_owned()), "no write starts once BattlePlan closes");
            drop(writing);
            closed.recv().unwrap();
        });
        writes.close();
    }

    #[test]
    fn a_write_waiting_on_another_instances_file_lock_is_not_admitted_so_closing_never_waits_on_it() {
        let folder = scratch("lock");
        let lock = folder.join("settings.lock");
        let held = fs::OpenOptions::new().write(true).create(true).truncate(false).open(&lock).unwrap();
        held.lock().unwrap();
        let writes = Writes::new();
        std::thread::scope(|scope| {
            let waiting = scope.spawn(|| writes.under_lock(&lock, |_| Ok(())));
            assert!(!writes.close_admission(), "a write blocked on the file lock holds no permit, so the exit has nothing to wait for");
            writes.close();
            held.unlock().unwrap();
            assert_eq!(waiting.join().unwrap().map_err(|error| error.to_string()), Err(CLOSED.to_owned()), "and once the lock frees it is refused before it writes");
        });
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_replacement_that_fails_part_way_keeps_the_file_it_would_replace() {
        let folder = scratch("replace");
        let path = folder.join("campaign-map.png");
        fs::write(&path, b"the earlier image").unwrap();
        let writes = Writes::new();
        let writing = writes.begin().unwrap();
        let failed = writing.replace(&path, |out| {
            out.write_all(b"half an ima")?;
            Err(io::Error::other("the encoder stopped"))
        });
        assert!(failed.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"the earlier image", "the destination is replaced only once the whole file is written");
        writing.replace(&path, |out| out.write_all(b"the new image")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"the new image");
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 1, "no staged file is left beside it");
        drop(writing);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_file_named_without_a_folder_is_written_in_the_current_one() {
        let name = PathBuf::from(format!("battleplan-writes-here-{}-{}.png", std::process::id(), STAGED.fetch_add(1, Ordering::Relaxed)));
        let writes = Writes::new();
        let written = writes.begin().unwrap().replace(&name, |out| out.write_all(b"frame"));
        let read = fs::read(&name);
        let _ = fs::remove_file(&name);
        written.unwrap();
        assert_eq!(read.unwrap(), b"frame", "viewer --dump --out frame.png writes beside the shell, as it did before files were staged");
    }

    #[test]
    fn a_write_another_permit_admitted_goes_ahead_after_the_close_and_the_exit_waits_for_it() {
        let writes = Writes::new();
        let accepted = writes.begin().unwrap();
        assert!(writes.close_admission());
        let folder = scratch("admitted");
        let path = folder.join("campaign-map.png");
        let writing = writes.admitted();
        drop(accepted);
        assert!(!writes.drained_within(std::time::Duration::ZERO), "the export the host accepted is still being written");
        writing.replace(&path, |out| out.write_all(b"the image")).unwrap();
        drop(writing);
        assert!(writes.drained_within(std::time::Duration::ZERO));
        assert_eq!(fs::read(&path).unwrap(), b"the image");
        fs::remove_dir_all(folder).unwrap();
    }
}
