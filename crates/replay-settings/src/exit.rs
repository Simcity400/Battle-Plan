use std::{
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

pub const CLOSED: &str = "BattlePlan is closing, so it starts nothing more.";
pub const UPDATING: &str = "An update is starting, so nothing more is changed until it starts or fails.";
pub const BUSY: &str = "Changes are still being saved. Restart to update once they have saved.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    Change,
    Export,
    Install,
    Recording,
    WaitingRecording,
    Recovery,
}

impl What {
    const ALL: [Self; 6] = [Self::Change, Self::Export, Self::Install, Self::Recording, Self::WaitingRecording, Self::Recovery];

    fn at(self) -> usize {
        self as usize
    }

    fn excludes_install(self) -> bool {
        matches!(self, Self::Change | Self::Recording | Self::WaitingRecording | Self::Recovery)
    }
}

#[must_use]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied<V = (), R = ()> {
    pub value: V,
    pub result: R,
    pub warning: Option<String>,
}

impl Applied {
    pub fn confirmed() -> Self {
        Self { value: (), result: (), warning: None }
    }

    pub fn unconfirmed(error: &io::Error) -> Self {
        Self { value: (), result: (), warning: Some(format!("the change was made, but Windows didn’t confirm it reached the disk: {error}")) }
    }
}

impl<V, R> Applied<V, R> {
    pub fn with<W, S>(self, value: W, result: S) -> Applied<W, S> {
        Applied { value, result, warning: self.warning }
    }

    pub fn map<W>(self, map: impl FnOnce(V) -> W) -> Applied<W, R> {
        Applied { value: map(self.value), result: self.result, warning: self.warning }
    }

    pub fn warned(mut self, warning: Option<String>) -> Self {
        self.warning = self.warning.or(warning);
        self
    }

    pub fn logged(self, what: &str) -> Self {
        if let Some(warning) = &self.warning {
            eprintln!("{what}: {warning}");
        }
        self
    }
}

#[derive(Default)]
struct State {
    sealed: bool,
    counts: [usize; What::ALL.len()],
    notices: Vec<String>,
}

pub struct Exit {
    state: Mutex<State>,
    changed: Condvar,
}

pub struct Receipt<'a> {
    exit: &'a Exit,
    what: What,
}

impl Drop for Receipt<'_> {
    fn drop(&mut self) {
        self.exit.lock().counts[self.what.at()] -= 1;
        self.exit.changed.notify_all();
    }
}

impl Default for Exit {
    fn default() -> Self {
        Self::new()
    }
}

static STAGED: AtomicU64 = AtomicU64::new(0);

impl Exit {
    pub const fn new() -> Self {
        Self { state: Mutex::new(State { sealed: false, counts: [0; What::ALL.len()], notices: Vec::new() }), changed: Condvar::new() }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn admit(&self, what: What) -> io::Result<Receipt<'_>> {
        if let Some(carried) = CARRIED.with_borrow(|carried| carried.last().filter(|carried| std::ptr::eq(carried.exit, self)).map(Receipt::split)) {
            return Ok(Receipt { exit: self, what: carried.what_and_forget() });
        }
        let mut state = self.lock();
        if state.sealed {
            return Err(io::Error::new(io::ErrorKind::Interrupted, CLOSED));
        }
        let installing = state.counts[What::Install.at()] > 0;
        if what.excludes_install() && installing {
            return Err(io::Error::new(io::ErrorKind::ResourceBusy, UPDATING));
        }
        if what == What::Install && What::ALL.iter().any(|other| other.excludes_install() && state.counts[other.at()] > 0) {
            return Err(io::Error::new(io::ErrorKind::ResourceBusy, BUSY));
        }
        state.counts[what.at()] += 1;
        Ok(Receipt { exit: self, what })
    }

    pub fn under_lock<T>(&self, lock: &Path, what: What, work: impl FnOnce(&Receipt<'_>) -> io::Result<T>) -> io::Result<T> {
        let file = lock_file(lock)?;
        let receipt = self.admit(what)?;
        let result = work(&receipt);
        drop(file);
        result
    }

    pub fn seal(&self) -> bool {
        self.seal_with(Vec::new())
    }

    pub fn seal_with(&self, notices: Vec<String>) -> bool {
        let mut state = self.lock();
        state.notices.extend(notices);
        let first = !std::mem::replace(&mut state.sealed, true);
        drop(state);
        self.changed.notify_all();
        first
    }

    pub fn sealed(&self) -> bool {
        self.lock().sealed
    }

    pub fn count(&self, what: What) -> usize {
        self.lock().counts[what.at()]
    }

    pub fn held(&self) -> Option<What> {
        let state = self.lock();
        [What::Install, What::Recording, What::WaitingRecording, What::Recovery, What::Export, What::Change].into_iter().find(|what| state.counts[what.at()] > 0)
    }

    pub fn notices(&self) -> Vec<String> {
        self.lock().notices.clone()
    }

    pub fn wait_sealed(&self) {
        let state = self.lock();
        drop(self.changed.wait_while(state, |state| !state.sealed).unwrap_or_else(PoisonError::into_inner));
    }

    pub fn empty_within(&self, limit: std::time::Duration) -> bool {
        let state = self.lock();
        let (state, _) = self.changed.wait_timeout_while(state, limit, |state| state.counts.iter().any(|&count| count > 0)).unwrap_or_else(PoisonError::into_inner);
        state.counts.iter().all(|&count| count == 0)
    }
}

pub fn lock_file(lock: &Path) -> io::Result<File> {
    if let Some(folder) = lock.parent() {
        fs::create_dir_all(folder)?;
    }
    let file = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock)?;
    file.lock()?;
    Ok(file)
}

thread_local! {
    static CARRIED: std::cell::RefCell<Vec<Receipt<'static>>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub struct Carry(Option<Receipt<'static>>);

impl Carry {
    pub fn current() -> Self {
        Self(CARRIED.with_borrow(|carried| carried.last().map(Receipt::split)))
    }

    pub fn run<T>(self, work: impl FnOnce() -> T) -> T {
        let Some(receipt) = self.0 else { return work() };
        struct Installed;
        impl Drop for Installed {
            fn drop(&mut self) {
                drop(CARRIED.with_borrow_mut(Vec::pop));
            }
        }
        CARRIED.with_borrow_mut(|carried| carried.push(receipt));
        let _installed = Installed;
        work()
    }
}

impl Receipt<'static> {
    pub fn carry<T>(&self, work: impl FnOnce() -> T) -> T {
        Carry(Some(self.split())).run(work)
    }
}

impl Receipt<'_> {
    pub fn what(&self) -> What {
        self.what
    }

    pub fn split(&self) -> Self {
        self.exit.lock().counts[self.what.at()] += 1;
        Self { exit: self.exit, what: self.what }
    }

    fn what_and_forget(self) -> What {
        let what = self.what;
        std::mem::forget(self);
        what
    }

    pub fn replace(&self, path: &Path, write: impl FnOnce(&mut BufWriter<&File>) -> io::Result<()>) -> io::Result<Applied> {
        let staged = stage_path(path)?;
        let file = fs::OpenOptions::new().write(true).create_new(true).open(&staged)?;
        let result = (|| {
            let mut out = BufWriter::new(&file);
            write(&mut out)?;
            out.flush()?;
            drop(out);
            file.sync_all()?;
            drop(file);
            replace_staged_inner(&staged, path)
        })();
        let _ = fs::remove_file(&staged);
        result
    }

    pub fn replace_staged(&self, staged: &Path, path: &Path) -> io::Result<Applied> {
        if let Some(folder) = path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
            fs::create_dir_all(folder)?;
        }
        let result = (|| {
            let file = fs::OpenOptions::new().read(true).write(true).open(staged)?;
            file.sync_all()?;
            drop(file);
            replace_staged_inner(staged, path)
        })();
        let _ = fs::remove_file(staged);
        result
    }

    pub fn publish_distinct(&self, staged: &Path, preferred: &Path) -> io::Result<Applied<PathBuf>> {
        if let Some(folder) = preferred.parent().filter(|folder| !folder.as_os_str().is_empty()) {
            fs::create_dir_all(folder)?;
        }
        let file = fs::OpenOptions::new().read(true).write(true).open(staged)?;
        file.sync_all()?;
        drop(file);
        let stem = preferred.file_stem().unwrap_or_default().to_string_lossy();
        let ext = preferred.extension().map(|ext| ext.to_string_lossy());
        let mut attempt = 0u32;
        loop {
            let candidate = if attempt == 0 {
                preferred.to_path_buf()
            } else {
                let name = match &ext {
                    Some(ext) => format!("{stem} ({attempt}).{ext}"),
                    None => format!("{stem} ({attempt})"),
                };
                preferred.with_file_name(name)
            };
            #[cfg(windows)]
            let moved = crate::publication::rename_new(staged, &candidate);
            #[cfg(not(windows))]
            let moved = if candidate.exists() {
                Err(io::Error::new(io::ErrorKind::AlreadyExists, "destination exists"))
            } else {
                fs::rename(staged, &candidate)
            };
            match moved {
                Ok(()) => {
                    #[cfg(windows)]
                    let warning = crate::publication::sync_file(&candidate).err().map(|error| {
                        format!("the change was made, but Windows didn’t confirm it reached the disk: {error}")
                    });
                    #[cfg(not(windows))]
                    let warning = None;
                    return Ok(Applied { value: candidate, result: (), warning });
                }
                Err(error)
                    if error.kind() == io::ErrorKind::AlreadyExists
                        || matches!(error.raw_os_error(), Some(80 | 183)) =>
                {
                    attempt = attempt.checked_add(1).ok_or_else(|| io::Error::other("could not reserve distinct file name"))?;
                }
                Err(error) => {
                    return Err(error);
                }
            }
        }
    }
}

pub fn stage_path(path: &Path) -> io::Result<PathBuf> {
    if let Some(folder) = path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        fs::create_dir_all(folder)?;
    }
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{}.{}.tmp", std::process::id(), STAGED.fetch_add(1, Ordering::Relaxed)));
    Ok(PathBuf::from(name))
}

fn replace_staged_inner(staged: &Path, path: &Path) -> io::Result<Applied> {
    #[cfg(windows)]
    let replaced = match crate::publication::replace_file(staged, path) {
        Ok(()) => Ok(Applied::confirmed()),
        Err(crate::publication::ReplaceError::Unrenamed(error)) => Err(error),
        Err(crate::publication::ReplaceError::Unflushed(error)) => Ok(Applied::unconfirmed(&error)),
    };
    #[cfg(not(windows))]
    let replaced = fs::rename(staged, path).map(|()| Applied::confirmed());
    replaced
}

pub static EXIT: Exit = Exit::new();

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("battleplan-exit-{name}-{}-{}", std::process::id(), STAGED.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn nothing_is_admitted_once_sealed_and_the_set_empties_as_each_receipt_drops_on_any_path() {
        let exit = Exit::new();
        let saving = exit.admit(What::Change).unwrap();
        let carried = saving.split();
        let never_ran: Box<dyn FnOnce() + Send> = {
            let export = exit.admit(What::Export).unwrap();
            Box::new(move || drop(export))
        };
        assert!(exit.seal());
        assert!(!exit.seal(), "only the first seal commits");
        assert_eq!(exit.admit(What::Change).err().map(|error| error.to_string()), Some(CLOSED.to_owned()));
        let continued = carried.split();
        assert!(!exit.empty_within(Duration::ZERO));
        drop((saving, carried, continued));
        assert_eq!(exit.held(), Some(What::Export));
        drop(never_ran);
        assert!(exit.empty_within(Duration::ZERO), "a receipt moved into work that never ran still ends");
    }

    #[test]
    fn work_carried_on_a_thread_continues_under_its_receipt_after_the_seal_and_nowhere_else() {
        let exit: &'static Exit = Box::leak(Box::new(Exit::new()));
        let recording = exit.admit(What::Recording).unwrap();
        let carried = recording.carry(Carry::current);
        exit.seal();
        recording.carry(|| {
            let nested = exit.admit(What::Change).expect("a settings write inside the capture continues it");
            assert_eq!(nested.what(), What::Recording);
        });
        assert!(exit.admit(What::Change).is_err(), "outside the carried work the seal holds");
        let continued = std::thread::spawn(move || carried.run(|| exit.admit(What::Change).map(|nested| nested.what()))).join().unwrap();
        assert_eq!(continued.ok(), Some(What::Recording), "a thread the capture spawns carries it too");
        drop(recording);
        assert!(exit.empty_within(Duration::ZERO));
    }

    #[test]
    fn an_install_and_the_work_it_would_cut_short_exclude_each_other_whichever_comes_first() {
        let exit = Exit::new();
        let saving = exit.admit(What::Recording).unwrap();
        assert_eq!(exit.admit(What::Install).err().map(|error| error.to_string()), Some(BUSY.to_owned()));
        drop(saving);
        let installing = exit.admit(What::Install).unwrap();
        for what in [What::Change, What::Recording, What::WaitingRecording, What::Recovery] {
            assert_eq!(exit.admit(what).err().map(|error| error.to_string()), Some(UPDATING.to_owned()), "{what:?}");
        }
        assert!(exit.admit(What::Export).is_ok(), "an export doesn’t touch the installed files");
        drop(installing);
        assert!(exit.admit(What::Change).is_ok());
    }

    #[test]
    fn a_write_waiting_on_another_instances_file_lock_is_not_admitted_so_sealing_never_waits_on_it() {
        let folder = scratch("lock");
        let lock = folder.join("settings.lock");
        let held = lock_file(&lock).unwrap();
        let exit = Exit::new();
        std::thread::scope(|scope| {
            let waiting = scope.spawn(|| exit.under_lock(&lock, What::Change, |_| Ok(())));
            exit.seal();
            assert!(exit.empty_within(Duration::ZERO), "a write blocked on the file lock holds no receipt");
            drop(held);
            assert_eq!(waiting.join().unwrap().map_err(|error| error.to_string()), Err(CLOSED.to_owned()));
        });
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_replacement_that_fails_part_way_keeps_the_file_it_would_replace() {
        let folder = scratch("replace");
        let path = folder.join("campaign-map.png");
        fs::write(&path, b"the earlier image").unwrap();
        let exit = Exit::new();
        let receipt = exit.admit(What::Export).unwrap();
        let failed = receipt.replace(&path, |out| {
            out.write_all(b"half an ima")?;
            Err(io::Error::other("the encoder stopped"))
        });
        assert!(failed.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"the earlier image");
        let _ = receipt.replace(&path, |out| out.write_all(b"the new image")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"the new image");
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 1, "no staged file is left beside it");
        drop(receipt);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_file_named_without_a_folder_is_written_in_the_current_one() {
        let name = PathBuf::from(format!("battleplan-exit-here-{}-{}.png", std::process::id(), STAGED.fetch_add(1, Ordering::Relaxed)));
        let exit = Exit::new();
        let written = exit.admit(What::Export).unwrap().replace(&name, |out| out.write_all(b"frame"));
        let read = fs::read(&name);
        let _ = fs::remove_file(&name);
        let _ = written.unwrap();
        assert_eq!(read.unwrap(), b"frame");
    }

    #[test]
    fn a_replacement_whose_flush_was_not_confirmed_is_applied_and_says_why() {
        let applied = Applied::unconfirmed(&io::Error::other("the disk didn’t answer")).with(7, "kept");
        assert_eq!((applied.value, applied.result), (7, "kept"), "the change and its caller’s result go on as though confirmed");
        assert!(applied.warning.is_some_and(|warning| warning.contains("didn’t confirm") && warning.contains("the disk didn’t answer")));
        assert_eq!(Applied::confirmed().with(7, ()).warned(None).warning, None);
    }

    #[test]
    fn staged_replacement_and_distinct_partial_publication_preserve_existing_files() {
        let folder = scratch("staged");
        let dest = folder.join("campaign.mp4");
        fs::write(&dest, b"original video").unwrap();
        let exit = Exit::new();
        let receipt = exit.admit(What::Export).unwrap();

        let missing = stage_path(&dest).unwrap();
        assert!(receipt.replace_staged(&missing, &dest).is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"original video");

        let partial = stage_path(&dest).unwrap();
        fs::write(&partial, b"partial clip").unwrap();
        let published = receipt.publish_distinct(&partial, &dest).unwrap();
        assert_ne!(published.value, dest);
        assert_eq!(fs::read(&dest).unwrap(), b"original video");
        assert_eq!(fs::read(&published.value).unwrap(), b"partial clip");

        let complete = stage_path(&dest).unwrap();
        fs::write(&complete, b"finished clip").unwrap();
        let _ = receipt.replace_staged(&complete, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"finished clip");
        assert!(!complete.exists());
        drop(receipt);
        fs::remove_dir_all(folder).unwrap();
    }
}
