//! Prepare published library revisions while the game is absent.
use crate::{dashboard_service::DashboardService, game_process, replay_library};
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use tauri::{AppHandle, Manager};

/// The preparation revision, whether the prepared-data volume has room for
/// library preparation, and each library recording with its replay revision.
type Observation = (u64, bool, Vec<(PathBuf, Option<String>)>);

/// One admission attempt per observed input/worker change. A busy or unchanged
/// reply ends retries until new external inputs or completed work wake them.
#[derive(Default)]
struct LibraryAdmission {
    observed: Option<Observation>,
    #[cfg(test)]
    requests: usize,
}
impl LibraryAdmission {
    fn prepare_changed(
        &mut self,
        runtime: &dashboard::Service,
        paths: &[PathBuf],
    ) -> io::Result<bool> {
        let current: Observation = (
            runtime.preparation_revision(),
            runtime.background_room(),
            paths
                .iter()
                .map(|path| {
                    (
                        path.clone(),
                        dashboard::protocol::replay_revision(path).ok(),
                    )
                })
                .collect(),
        );
        if self.observed.as_ref() == Some(&current) {
            return Ok(false);
        }
        // Save the observation even if admission returns busy or fails. Repeating
        // that request cannot make progress until an input/worker changes.
        self.observed = Some(current);
        #[cfg(test)]
        {
            self.requests += 1;
        }
        runtime.prepare_library(paths).map(|()| true)
    }
}

pub struct PreparationMonitor {
    stopped: Arc<AtomicBool>,
    game_present: Arc<AtomicBool>,
}
impl Default for PreparationMonitor {
    fn default() -> Self {
        Self {
            stopped: Arc::new(AtomicBool::new(false)),
            // No automatic admission until the first successful process snapshot.
            game_present: Arc::new(AtomicBool::new(true)),
        }
    }
}
impl PreparationMonitor {
    pub fn start(&self, app: &AppHandle) -> io::Result<()> {
        let stopped = Arc::clone(&self.stopped);
        let game_present = Arc::clone(&self.game_present);
        let guard_app = app.clone();
        let guard = thread::Builder::new()
            .name("preparation game guard".into())
            .spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    let running = game_process::game_running().unwrap_or(true);
                    if game_present.swap(running, Ordering::AcqRel) != running {
                        guard_app
                            .state::<DashboardService>()
                            .suspend_preparation(running);
                    }
                    thread::sleep(Duration::from_millis(250));
                }
            })?;
        let stopped = Arc::clone(&self.stopped);
        let game_present = Arc::clone(&self.game_present);
        let library_app = app.clone();
        let library = dashboard::prepared::runner::worker_thread("preparation library monitor")
            .spawn(move || {
                let mut admission = LibraryAdmission::default();
                while !stopped.load(Ordering::Acquire) {
                    if !game_present.load(Ordering::Acquire) {
                        let service = library_app.state::<DashboardService>();
                        if let Ok(runtime) = service.runtime(&library_app)
                            && runtime.preparation_preferences().background_enabled
                        {
                            let paths = replay_library::configured_folder()
                                .and_then(|folder| {
                                    replay_library::scan_with(&folder, || {
                                        if stopped.load(Ordering::Acquire)
                                            || game_present.load(Ordering::Acquire)
                                        {
                                            Err("Background library scan interrupted".into())
                                        } else {
                                            Ok(())
                                        }
                                    })
                                })
                                .map(|library| {
                                    library
                                        .files
                                        .into_iter()
                                        .map(|file| file.path.into())
                                        .collect::<Vec<_>>()
                                });
                            // Interrupted opened sources can lie outside the current
                            // library folder. The service includes those retained paths.
                            if !stopped.load(Ordering::Acquire)
                                && !game_present.load(Ordering::Acquire)
                            {
                                let _ =
                                    admission.prepare_changed(&runtime, &paths.unwrap_or_default());
                            }
                        }
                    }
                    thread::sleep(Duration::from_secs(1));
                }
            });
        if let Err(error) = library {
            self.stopped.store(true, Ordering::Release);
            let _ = guard.join();
            return Err(error);
        }
        // Shutdown signals both monitors; a filesystem operation need not delay
        // closing the app. They check stop before admitting more work.
        Ok(())
    }

    pub fn shutdown(&self, app: &AppHandle) {
        self.stopped.store(true, Ordering::Release);
        self.game_present.store(true, Ordering::Release);
        app.state::<DashboardService>().suspend_preparation(true);
    }
}
impl Drop for PreparationMonitor {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercise the real service's failed-open path: no work completes, so
    /// repeated replies cannot be used as an excuse to request again.
    #[test]
    fn library_admission_stops_unchanged_requests() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "preparation-no-progress-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("unreadable.battleplan");
        std::fs::write(&path, b"synthetic invalid recording").unwrap();
        let runtime = dashboard::Service::new(Arc::new(|_| true)).unwrap();
        let mut admission = LibraryAdmission::default();
        let paths = vec![path.clone(); 3];
        assert!(admission.prepare_changed(&runtime, &paths).unwrap());
        for _ in 0..5 {
            assert!(!admission.prepare_changed(&runtime, &paths).unwrap());
        }
        assert_eq!(
            admission.requests, 1,
            "no-progress replies must end retries"
        );
        // An actual recording revision change permits one new request.
        std::fs::write(&path, b"a different synthetic invalid recording").unwrap();
        assert!(admission.prepare_changed(&runtime, &paths).unwrap());
        for _ in 0..5 {
            assert!(!admission.prepare_changed(&runtime, &paths).unwrap());
        }
        assert_eq!(
            admission.requests, 2,
            "requests are bounded by changed inputs"
        );
        // Likewise, a preference/game change can wake admission exactly once.
        runtime.set_preparation_suspended(true);
        assert!(admission.prepare_changed(&runtime, &paths).unwrap());
        for _ in 0..5 {
            assert!(!admission.prepare_changed(&runtime, &paths).unwrap());
        }
        assert_eq!(admission.requests, 3);
        runtime.shutdown();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    /// A session ends when the recorder publishes its recording into the
    /// library folder by renaming a finished temporary file; the next scan
    /// requests admission for it once.
    #[test]
    fn a_published_session_wakes_library_admission_once() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "preparation-session-end-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("earlier.battleplan"), b"synthetic recording").unwrap();
        let scan = || {
            replay_library::scan(&root)
                .unwrap()
                .files
                .into_iter()
                .map(|file| PathBuf::from(file.path))
                .collect::<Vec<_>>()
        };
        let runtime = dashboard::Service::new(Arc::new(|_| true)).unwrap();
        let mut admission = LibraryAdmission::default();
        assert!(admission.prepare_changed(&runtime, &scan()).unwrap());
        assert!(!admission.prepare_changed(&runtime, &scan()).unwrap());
        // Finalization writes the recording beside its final name first.
        let temporary = root.join("session.battleplan.tmp");
        std::fs::write(&temporary, b"synthetic session").unwrap();
        assert!(
            !admission.prepare_changed(&runtime, &scan()).unwrap(),
            "an unfinished recording is not a session end"
        );
        std::fs::rename(&temporary, root.join("session.battleplan")).unwrap();
        assert!(admission.prepare_changed(&runtime, &scan()).unwrap());
        assert!(!admission.prepare_changed(&runtime, &scan()).unwrap());
        assert_eq!(admission.requests, 2);
        runtime.shutdown();
        std::fs::remove_dir_all(root).unwrap();
    }
}
