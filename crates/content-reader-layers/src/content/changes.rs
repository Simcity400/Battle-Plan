use std::{
    io,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[cfg(windows)]
mod windows;

const PERMANENT: u64 = 1 << 63;

struct State {
    dirty: AtomicU64,
    #[cfg(windows)]
    stopping: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    notification: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl State {
    fn changed(&self) {
        let _ = self
            .dirty
            .fetch_update(Ordering::Release, Ordering::Relaxed, |value| {
                Some(value.saturating_add(1))
            });
        #[cfg(test)]
        if let Some(sender) = self.notification.lock().unwrap().as_ref() {
            let _ = sender.send(());
        }
    }

    fn failed(&self) {
        self.dirty.fetch_or(PERMANENT, Ordering::Release);
    }
}

pub(super) struct Changes {
    state: Arc<State>,
    sweep: Mutex<()>,
    pub(super) validations: AtomicU64,
    #[cfg(windows)]
    owner: Mutex<Option<windows::Owner>>,
}

impl Default for Changes {
    fn default() -> Self {
        Self {
            state: Arc::new(State {
                dirty: AtomicU64::new(if cfg!(windows) { 1 } else { PERMANENT }),
                #[cfg(windows)]
                stopping: std::sync::atomic::AtomicBool::new(false),
                #[cfg(test)]
                notification: Mutex::new(None),
            }),
            sweep: Mutex::new(()),
            validations: AtomicU64::new(0),
            #[cfg(windows)]
            owner: Mutex::new(None),
        }
    }
}

impl Changes {
    pub(super) fn changed(&self) {
        self.state.changed();
    }

    pub(super) fn watch(&self, path: &Path, subtree: bool) {
        if self.failed() {
            return;
        }
        #[cfg(windows)]
        {
            let mut owner = self.owner.lock().unwrap_or_else(|error| error.into_inner());
            let result = (|| {
                if owner.is_none() {
                    *owner = Some(windows::Owner::new(Arc::clone(&self.state))?);
                }
                owner
                    .as_mut()
                    .expect("watch owner created")
                    .watch(path, subtree)
            })();
            if let Err(error) = result {
                self.unwatchable(path, &error);
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (path, subtree);
            self.state.failed();
        }
    }

    pub(super) fn unwatchable(&self, path: &Path, error: &io::Error) {
        self.state.failed();
        eprintln!("Content change watch {}: {error}; validating witnesses on every admission", path.display());
    }

    pub(super) fn watch_root(&self, root: &Path, subtree: bool) {
        self.watch(root, subtree);
        if let Some(parent) = root.parent() {
            self.watch(parent, false);
        }
    }

    pub(super) fn watch_listing(&self, path: &Path) {
        let mut ancestor = path;
        loop {
            match ancestor.canonicalize() {
                Ok(root) => {
                    self.watch_root(&root, true);
                    return;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if let Some(parent) = ancestor.parent() {
                        ancestor = parent;
                        continue;
                    }
                }
                Err(_) => {}
            }
            self.watch_root(ancestor, true);
            return;
        }
    }

    pub(super) fn validate(
        &self,
        force: bool,
        work: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<()> {
        if !force && self.state.dirty.load(Ordering::Acquire) == 0 {
            return Ok(());
        }
        let _sweep = self.sweep.lock().unwrap_or_else(|error| error.into_inner());
        let ticket = self.state.dirty.load(Ordering::Acquire);
        if !force && ticket == 0 {
            return Ok(());
        }
        self.validations.fetch_add(1, Ordering::Relaxed);
        work()?;
        if ticket & PERMANENT == 0 {
            let _ =
                self.state
                    .dirty
                    .compare_exchange(ticket, 0, Ordering::AcqRel, Ordering::Acquire);
        }
        Ok(())
    }

    pub(super) fn failed(&self) -> bool {
        self.state.dirty.load(Ordering::Acquire) & PERMANENT != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_admission_skips_sweeps_and_a_change_during_a_sweep_is_kept() {
        let changes = Changes::default();
        changes.state.dirty.store(1, Ordering::Release);
        changes.validate(false, || Ok(())).unwrap();
        for _ in 0..100 {
            changes
                .validate(false, || panic!("clean admission must not sweep"))
                .unwrap();
        }
        changes.state.changed();
        changes
            .validate(false, || {
                changes.state.changed();
                Ok(())
            })
            .unwrap();
        assert_ne!(changes.state.dirty.load(Ordering::Acquire), 0);
        changes.validate(false, || Ok(())).unwrap();
        assert_eq!(changes.validations.load(Ordering::Relaxed), 3);
        assert_eq!(changes.state.dirty.load(Ordering::Acquire), 0);
    }

    #[test]
    fn watch_failure_never_allows_a_clean_admission() {
        let changes = Changes::default();
        changes.state.failed();
        for _ in 0..3 {
            changes.validate(false, || Ok(())).unwrap();
        }
        assert!(changes.failed());
        assert_eq!(changes.validations.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn concurrent_admissions_share_one_sweep() {
        let changes = Changes::default();
        changes.state.dirty.store(1, Ordering::Release);
        let ready = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let changes = &changes;
                let ready = &ready;
                scope.spawn(move || {
                    ready.wait();
                    changes.validate(false, || Ok(())).unwrap();
                });
            }
        });
        assert_eq!(changes.validations.load(Ordering::Relaxed), 1);
    }

    #[cfg(windows)]
    #[test]
    fn entered_junction_targets_signal_the_generation() {
        use super::super::{entry_kind, generation, tests::junction, walk_folder};
        use crate::test_support::change_directory_with_new_stamp;
        let root = std::env::temp_dir().join(format!("content-watch-links-{}", std::process::id()));
        let layer = root.join("layer");
        let target = root.join("outside/nested/art");
        std::fs::create_dir_all(layer.join("gfx")).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        junction(&layer.join("gfx/linked"), &target);
        let scope = generation::Generation::enter();
        let graph = generation::current().unwrap();
        graph.changes.watch_root(&layer, true);
        walk_folder(&layer, &layer.join("gfx"), true, &entry_kind, &mut |_, _| Ok(())).unwrap();
        assert!(!graph.changes.failed());
        graph.changes.validate(false, || Ok(())).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        *graph.changes.state.notification.lock().unwrap() = Some(sender);
        change_directory_with_new_stamp(&target, || {
            std::fs::write(target.join("added.dds"), b"art").unwrap();
            receiver.recv().unwrap();
            assert_ne!(graph.changes.state.dirty.load(Ordering::Acquire), 0);
        });
        assert!(graph.changes.validate(false, || graph.witnesses.validate()).is_err());
        drop(scope);
        drop(graph);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn subtree_writes_signal_admission_and_pending_watches_retire() {
        let root = std::env::temp_dir().join(format!("content-watch-{}", std::process::id()));
        std::fs::create_dir_all(root.join("nested")).unwrap();
        let changes = Changes::default();
        changes.watch(&root, true);
        assert!(!changes.failed());
        changes.validate(false, || Ok(())).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        *changes.state.notification.lock().unwrap() = Some(sender);
        let path = root.join("nested/input");
        std::fs::write(&path, b"changed").unwrap();
        receiver.recv().unwrap();
        assert_ne!(changes.state.dirty.load(Ordering::Acquire), 0);
        changes
            .validate(false, || {
                assert_eq!(std::fs::read(&path)?, b"changed");
                Ok(())
            })
            .unwrap();
        drop(changes);
        std::fs::remove_dir_all(root).unwrap();
    }
}
