//! Prepare published library revisions.
use crate::{dashboard_service::DashboardService, replay_library};
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
/// library preparation, and each library recording with its replay revision
/// and its native recordings' stamp.
type Observation = (u64, bool, Vec<(PathBuf, Option<String>, Option<String>)>);

/// One admission attempt per observed input/worker change. A busy or unchanged
/// reply ends retries until new external inputs or completed work wake them.
#[derive(Default)]
struct LibraryAdmission {
    observed: Option<Observation>,
    #[cfg(test)]
    requests: usize,
}
impl LibraryAdmission {
    /// Scan the library `folder` and admit what changed, unless the app is
    /// closing.
    fn scan(
        &mut self,
        runtime: &dashboard::Service,
        folder: Result<PathBuf, String>,
        stopped: &AtomicBool,
    ) {
        let paths = folder
            .and_then(|folder| {
                replay_library::scan_with(&folder, || {
                    if stopped.load(Ordering::Acquire) {
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
        // Retained sources can lie outside the current library folder. The
        // service includes those paths.
        if !stopped.load(Ordering::Acquire) {
            let _ = self.prepare_changed(runtime, &paths.unwrap_or_default());
        }
    }

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
                        dashboard::prepared::change_runs::native_revision(path).ok(),
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

#[derive(Default)]
pub struct PreparationMonitor {
    stopped: Arc<AtomicBool>,
}
impl PreparationMonitor {
    pub fn start(&self, app: &AppHandle) -> io::Result<()> {
        let stopped = Arc::clone(&self.stopped);
        let app = app.clone();
        dashboard::prepared::runner::worker_thread("preparation library monitor").spawn(
            move || {
                let mut admission = LibraryAdmission::default();
                while !stopped.load(Ordering::Acquire) {
                    let service = app.state::<DashboardService>();
                    if let Ok(runtime) = service.runtime(&app)
                        && runtime.preparation_preferences().background_enabled
                    {
                        admission.scan(&runtime, replay_library::configured_folder(), &stopped);
                    }
                    thread::sleep(Duration::from_secs(1));
                }
            },
        )?;
        // Shutdown signals the monitor; a filesystem operation need not delay
        // closing the app. It checks stop before admitting more work.
        Ok(())
    }

    /// Stop admitting library work; closing the dashboard service stops the
    /// work already running.
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
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
        // Likewise, a preference change can wake admission exactly once.
        runtime.set_background_preparation(false);
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

    /// A native recording of a recording's campaign landing wakes the next
    /// scan's admission once more, as one replaced does.
    #[test]
    fn a_native_recording_landing_wakes_library_admission_once() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "preparation-native-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        // A recording's header up to its campaign id, all the scan reads of it.
        let mut header = b"H4RP".to_vec();
        let format = replay_data::format::FormatVersion::CURRENT.version();
        header.extend_from_slice(&format.to_le_bytes());
        header.extend_from_slice(&[0; 4]);
        header.extend_from_slice(&8_u32.to_le_bytes());
        header.extend_from_slice(b"campaign");
        std::fs::write(root.join("session.battleplan"), header).unwrap();
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
        let folder = replay::columnar_file::campaign_folder(&root, b"campaign");
        std::fs::create_dir(&folder).unwrap();
        let native = folder.join("session-1.0.h4col");
        std::fs::write(&native, b"synthetic native recording").unwrap();
        assert!(admission.prepare_changed(&runtime, &scan()).unwrap());
        assert!(!admission.prepare_changed(&runtime, &scan()).unwrap());
        std::fs::write(&native, b"another synthetic native recording").unwrap();
        assert!(admission.prepare_changed(&runtime, &scan()).unwrap());
        // Another campaign's native recording is not this one's.
        let other = replay::columnar_file::campaign_folder(&root, b"other");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("session-1.0.h4col"), b"synthetic").unwrap();
        assert!(!admission.prepare_changed(&runtime, &scan()).unwrap());
        assert_eq!(admission.requests, 3);
        runtime.shutdown();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Only closing the app holds a library scan back: with nothing else to
    /// wait for, it requests admission of what was published.
    #[test]
    fn a_library_scan_requests_admission_unless_the_app_is_closing() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "preparation-ungated-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("session.battleplan"), b"synthetic recording").unwrap();
        let runtime = dashboard::Service::new(Arc::new(|_| true)).unwrap();
        let mut admission = LibraryAdmission::default();
        admission.scan(&runtime, Ok(root.clone()), &AtomicBool::new(true));
        assert_eq!(admission.requests, 0, "a closing app admits nothing");
        admission.scan(&runtime, Ok(root.clone()), &AtomicBool::new(false));
        assert_eq!(admission.requests, 1);
        assert_eq!(
            admission
                .observed
                .as_ref()
                .map(|(_, _, recordings)| recordings.len()),
            Some(1),
            "the published recording is requested"
        );
        runtime.shutdown();
        std::fs::remove_dir_all(root).unwrap();
    }
}
