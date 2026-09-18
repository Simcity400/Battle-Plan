//! Hosts the existing native viewer, with a bounded pipe for control messages.

use desktop_protocol::{
    EVENT_PREFIX, MAX_MESSAGE_BYTES, NativeCommand, NativeEvent, ViewBounds, ViewerState,
    replay_revision,
};
use serde::Serialize;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, WebviewWindow};

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    pub running: bool,
    pub ready: bool,
    pub visible: bool,
    pub viewer: ViewerState,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SeekTarget {
    path: String,
    revision: String,
    branch: String,
    hour: i32,
}

impl SeekTarget {
    fn command(self) -> NativeCommand {
        NativeCommand::Seek {
            path: self.path,
            revision: self.revision,
            branch: self.branch,
            hour: self.hour,
        }
    }

    fn matches(&self, state: &ViewerState) -> bool {
        state.path.as_ref() == Some(&self.path)
            && state.revision.as_ref() == Some(&self.revision)
            && state.branch.as_ref() == Some(&self.branch)
            && state.hour == Some(self.hour)
    }
}

#[derive(Default)]
struct Pending {
    bounds: Option<ViewBounds>,
    visibility: Option<bool>,
    open: Option<(String, String)>,
    seek: Option<SeekTarget>,
    close: bool,
}

impl Pending {
    fn can_send(&self, visible: bool) -> bool {
        self.close
            || self.bounds.is_some()
            || self.visibility.is_some()
            || (visible && (self.open.is_some() || self.seek.is_some()))
    }

    fn take_commands(&mut self, visible: bool) -> Vec<NativeCommand> {
        if self.close {
            *self = Self::default();
            return vec![NativeCommand::Close];
        }
        let mut commands = Vec::with_capacity(4);
        if let Some(bounds) = self.bounds.take() {
            commands.push(bounds.command());
        }
        // Analytics can open another recording without loading its map assets.
        // Keep only the latest document request until Map becomes visible.
        if visible && let Some((path, revision)) = self.open.take() {
            commands.push(NativeCommand::Open { path, revision });
        }
        if visible && let Some(target) = self.seek.take() {
            commands.push(target.command());
        }
        if let Some(visible) = self.visibility.take() {
            commands.push(NativeCommand::Visibility { visible });
        }
        commands
    }
}

struct Process {
    child: Child,
    _job: Job,
}

#[derive(Default)]
struct Inner {
    process: Option<Process>,
    generation: u64,
    status: HostStatus,
    bounds: Option<ViewBounds>,
    pending: Pending,
    requested_path: Option<String>,
    requested_revision: Option<String>,
    requested_seek: Option<SeekTarget>,
    shutting_down: bool,
    last_diagnostic: Option<String>,
}

#[derive(Clone, Default)]
pub struct NativeHost {
    shared: Arc<(Mutex<Inner>, Condvar)>,
}

impl NativeHost {
    pub fn status(&self) -> HostStatus {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status
            .clone()
    }

    pub fn set_view(
        &self,
        window: &WebviewWindow,
        visible: bool,
        bounds: ViewBounds,
    ) -> Result<HostStatus, String> {
        let bounds = bounds.validate().map_err(str::to_owned)?;
        {
            let mut inner = self.shared.0.lock().map_err(|e| e.to_string())?;
            inner.bounds = Some(bounds);
            inner.status.visible = visible;
            inner.pending.bounds = Some(bounds);
            inner.pending.visibility = Some(visible);
        }
        if visible {
            self.ensure_started(window)?;
        }
        self.shared.1.notify_all();
        Ok(self.status())
    }

    pub fn open(&self, path: String) -> Result<(), String> {
        if path.len() > 32_000 {
            return Err("Replay path is too long".into());
        }
        let revision = replay_revision(&path).map_err(|e| e.to_string())?;
        let mut inner = self.shared.0.lock().map_err(|e| e.to_string())?;
        inner.requested_path = Some(path.clone());
        inner.requested_revision = Some(revision.clone());
        inner.requested_seek = None;
        inner.pending.open = Some((path, revision));
        inner.pending.seek = None;
        drop(inner);
        self.shared.1.notify_all();
        Ok(())
    }

    pub fn seek(
        &self,
        path: String,
        revision: String,
        branch: String,
        hour: i32,
    ) -> Result<(), String> {
        branch.parse::<u64>().map_err(|_| "Invalid replay branch")?;
        if replay_revision(&path).map_err(|e| e.to_string())? != revision {
            return Err("The recording changed since this analysis. Refresh the analysis before viewing this sample on the map.".into());
        }
        let mut inner = self.shared.0.lock().map_err(|e| e.to_string())?;
        let current_matches = inner.status.viewer.path.as_ref() == Some(&path)
            && inner.status.viewer.revision.as_ref() == Some(&revision)
            && inner.requested_path.is_none();
        if !current_matches {
            inner.requested_path = Some(path.clone());
            inner.requested_revision = Some(revision.clone());
            inner.pending.open = Some((path.clone(), revision.clone()));
        }
        let target = SeekTarget {
            path,
            revision,
            branch,
            hour,
        };
        inner.requested_seek = Some(target.clone());
        inner.pending.seek = Some(target);
        drop(inner);
        self.shared.1.notify_all();
        Ok(())
    }

    fn ensure_started(&self, window: &WebviewWindow) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        use tauri::Manager;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        let mut inner = self.shared.0.lock().map_err(|e| e.to_string())?;
        if inner.process.is_some() {
            return Ok(());
        }
        if inner.shutting_down {
            return Err("The application is closing".into());
        }
        let executable = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name("viewer.exe");
        if !executable.is_file() {
            return Err(format!(
                "Native viewer is missing at {}. Run npm run dev or npm run build:desktop to build both executables.",
                executable.display()
            ));
        }
        let parent = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
        let mut child = Command::new(&executable)
            .args(std::env::args_os().skip(1))
            .env("BATTLE_PLAN_VIEWER_PARENT", parent.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Could not start {}: {e}", executable.display()))?;
        let job = match Job::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let input = child
            .stdin
            .take()
            .ok_or("Native viewer input pipe is missing")?;
        let output = child
            .stdout
            .take()
            .ok_or("Native viewer output pipe is missing")?;
        let errors = child
            .stderr
            .take()
            .ok_or("Native viewer error pipe is missing")?;
        if inner.generation > 0 {
            inner.restore_commands();
        }
        inner.generation += 1;
        let generation = inner.generation;
        inner.status.running = true;
        inner.status.ready = false;
        inner.status.error = None;
        inner.last_diagnostic = None;
        inner.pending.bounds = inner.bounds;
        inner.pending.visibility = Some(inner.status.visible);
        inner.process = Some(Process { child, _job: job });
        drop(inner);

        let writer = self.clone();
        let writer_app = window.app_handle().clone();
        thread::spawn(move || writer.write_commands(input, generation, &writer_app));
        let reader = self.clone();
        let reader_app = window.app_handle().clone();
        thread::spawn(move || {
            read_lines(output, |line| {
                if let Some(json) = line.strip_prefix(EVENT_PREFIX)
                    && let Ok(event) = serde_json::from_str::<NativeEvent>(json)
                {
                    reader.receive(event, generation, &reader_app);
                }
            });
        });
        let diagnostics = self.clone();
        thread::spawn(move || {
            read_lines(errors, |line| {
                let mut inner = diagnostics
                    .shared
                    .0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if inner.generation == generation && !line.trim().is_empty() {
                    inner.last_diagnostic = Some(line.chars().take(2000).collect());
                }
            });
        });
        let monitor = self.clone();
        let monitor_app = window.app_handle().clone();
        thread::spawn(move || monitor.monitor(generation, &monitor_app));
        Ok(())
    }

    fn write_commands(&self, mut input: impl Write, generation: u64, app: &AppHandle) {
        loop {
            let commands = {
                let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
                while !inner.pending.can_send(inner.status.visible)
                    && inner.generation == generation
                    && inner.process.is_some()
                {
                    inner = self.shared.1.wait(inner).unwrap_or_else(|e| e.into_inner());
                }
                if inner.generation != generation || inner.process.is_none() {
                    return;
                }
                let visible = inner.status.visible;
                inner.pending.take_commands(visible)
            };
            for command in commands {
                let close = matches!(command, NativeCommand::Close);
                let result = serde_json::to_writer(&mut input, &command)
                    .map_err(std::io::Error::other)
                    .and_then(|()| input.write_all(b"\n"))
                    .and_then(|()| input.flush());
                if let Err(error) = result {
                    self.receive(
                        NativeEvent::Error {
                            message: format!("Native viewer connection closed: {error}"),
                        },
                        generation,
                        app,
                    );
                    return;
                }
                if close {
                    return;
                }
            }
        }
    }

    fn receive(&self, event: NativeEvent, generation: u64, app: &AppHandle) {
        let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        if inner.generation != generation || inner.shutting_down || inner.process.is_none() {
            return;
        }
        match event {
            NativeEvent::Ready { .. } => inner.status.ready = true,
            NativeEvent::State {
                path,
                branch,
                hour,
                revision,
                error,
            } => {
                if inner.requested_path.as_ref() == path.as_ref()
                    && inner.requested_revision.as_ref() == revision.as_ref()
                {
                    inner.requested_path = None;
                    inner.requested_revision = None;
                }
                inner.status.viewer = ViewerState {
                    path,
                    branch,
                    hour,
                    revision,
                };
                if inner.requested_path.is_none()
                    && inner
                        .requested_seek
                        .as_ref()
                        .is_some_and(|target| target.matches(&inner.status.viewer))
                {
                    inner.requested_seek = None;
                }
                inner.status.error = error;
            }
            NativeEvent::Error { message } => {
                inner.last_diagnostic = Some(message.clone());
                inner.status.error = Some(message);
            }
        }
        // Publish while holding the state lock so reader and exit notifications
        // reach the frontend in mutation order.
        let _ = app.emit("viewer-state", inner.status.clone());
    }

    fn monitor(&self, generation: u64, app: &AppHandle) {
        loop {
            thread::sleep(Duration::from_millis(250));
            {
                let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
                if inner.generation != generation {
                    return;
                }
                let Some(process) = inner.process.as_mut() else {
                    return;
                };
                match process.child.try_wait() {
                    Ok(None) => continue,
                    result => {
                        inner.process = None;
                        inner.generation += 1;
                        inner.status.running = false;
                        inner.status.ready = false;
                        if !inner.shutting_down {
                            let reason = match result {
                                Ok(Some(exit)) => format!("Native viewer exited ({exit})"),
                                Err(error) => format!("Cannot observe native viewer: {error}"),
                                Ok(None) => unreachable!(),
                            };
                            inner.status.error = Some(match inner.last_diagnostic.as_ref() {
                                Some(detail) => format!("{reason}: {detail}"),
                                None => reason,
                            });
                        }
                        let _ = app.emit("viewer-state", inner.status.clone());
                    }
                }
            };
            self.shared.1.notify_all();
            return;
        }
    }

    pub fn shutdown(&self) {
        {
            let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
            if inner.shutting_down {
                return;
            }
            inner.shutting_down = true;
            inner.pending.close = true;
        }
        self.shared.1.notify_all();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
            let Some(process) = inner.process.as_mut() else {
                break;
            };
            if process.child.try_wait().ok().flatten().is_some() {
                inner.process = None;
                break;
            }
            if Instant::now() >= deadline {
                let _ = process.child.kill();
                let _ = process.child.wait();
                inner.process = None;
                break;
            }
            drop(inner);
            thread::sleep(Duration::from_millis(20));
        }
        self.shared.1.notify_all();
    }
}

impl Inner {
    /// Pending pipe slots may already have been sent when a process exits.
    /// Retain unacknowledged requests, otherwise reopen its last displayed hour.
    fn restore_commands(&mut self) {
        let target = self
            .requested_path
            .clone()
            .zip(self.requested_revision.clone())
            .or_else(|| {
                self.requested_seek
                    .as_ref()
                    .map(|target| (target.path.clone(), target.revision.clone()))
            })
            .or_else(|| {
                self.status
                    .viewer
                    .path
                    .clone()
                    .zip(self.status.viewer.revision.clone())
            });
        if let Some((path, revision)) = target {
            let last_position = (self.status.viewer.path.as_ref() == Some(&path)
                && self.status.viewer.revision.as_ref() == Some(&revision))
            .then(|| {
                self.status
                    .viewer
                    .branch
                    .clone()
                    .zip(self.status.viewer.hour)
                    .map(|(branch, hour)| SeekTarget {
                        path: path.clone(),
                        revision: revision.clone(),
                        branch,
                        hour,
                    })
            })
            .flatten();
            self.pending.open = Some((path, revision));
            self.pending.seek = self.requested_seek.clone().or(last_position);
        }
    }
}

/// Bound even a malformed diagnostic line; discard its remainder in chunks.
fn read_lines(input: impl Read, mut visit: impl FnMut(&str)) {
    let mut reader = BufReader::new(input);
    let mut bytes = Vec::new();
    let mut discarding = false;
    loop {
        bytes.clear();
        let count = match reader
            .by_ref()
            .take((MAX_MESSAGE_BYTES + 1) as u64)
            .read_until(b'\n', &mut bytes)
        {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        let complete = bytes.last() == Some(&b'\n');
        if !discarding
            && count <= MAX_MESSAGE_BYTES
            && complete
            && let Ok(line) = std::str::from_utf8(&bytes)
        {
            visit(line.trim_end());
        }
        discarding = !complete;
    }
}

struct Job(usize);

impl Job {
    fn attach(child: &Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        // The job contains only the viewer we just spawned. Closing the host
        // also closes this handle, preventing an orphaned renderer after a crash.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let job = Self(handle as usize);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0
            {
                return Err(format!(
                    "Cannot contain native viewer lifecycle: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(job)
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resized_commands_coalesce_and_close_takes_priority() {
        let mut pending = Pending {
            visibility: Some(true),
            open: Some(("x".into(), "v1".into())),
            close: true,
            ..Default::default()
        };
        assert!(matches!(
            pending.take_commands(false).as_slice(),
            [NativeCommand::Close]
        ));
        assert!(!pending.can_send(true));
    }

    #[test]
    fn oversized_lines_do_not_hide_the_next_message() {
        let mut input = vec![b'x'; MAX_MESSAGE_BYTES * 3];
        input.extend_from_slice(b"\nvalid\n");
        let mut lines = Vec::new();
        read_lines(input.as_slice(), |line| lines.push(line.to_owned()));
        assert_eq!(lines, ["valid"]);
    }

    #[test]
    fn restart_restores_unacknowledged_open_and_seek() {
        let requested = SeekTarget {
            path: "new.battleplan".into(),
            revision: "new-version".into(),
            branch: "9".into(),
            hour: 456,
        };
        let mut inner = Inner {
            requested_path: Some("new.battleplan".into()),
            requested_revision: Some("new-version".into()),
            requested_seek: Some(requested.clone()),
            status: HostStatus {
                viewer: ViewerState {
                    path: Some("old.battleplan".into()),
                    branch: Some("2".into()),
                    hour: Some(123),
                    revision: Some("old-version".into()),
                },
                ..Default::default()
            },
            ..Default::default()
        };
        inner.restore_commands();
        assert_eq!(
            inner.pending.open,
            Some(("new.battleplan".into(), "new-version".into()))
        );
        assert_eq!(inner.pending.seek, Some(requested));
        inner.requested_path = None;
        inner.requested_revision = None;
        inner.requested_seek = None;
        inner.restore_commands();
        assert_eq!(
            inner.pending.open,
            Some(("old.battleplan".into(), "old-version".into()))
        );
        assert_eq!(
            inner.pending.seek,
            Some(SeekTarget {
                path: "old.battleplan".into(),
                revision: "old-version".into(),
                branch: "2".into(),
                hour: 123,
            })
        );
    }

    #[test]
    fn seek_acknowledgement_requires_the_same_recording_and_revision() {
        let target = SeekTarget {
            path: "first.battleplan".into(),
            revision: "v1".into(),
            branch: "3".into(),
            hour: 42,
        };
        let mut state = ViewerState {
            path: Some("second.battleplan".into()),
            revision: Some("v1".into()),
            branch: Some("3".into()),
            hour: Some(42),
        };
        assert!(!target.matches(&state));
        state.path = Some(target.path.clone());
        state.revision = Some("v2".into());
        assert!(!target.matches(&state));
        state.revision = Some(target.revision.clone());
        assert!(target.matches(&state));
    }

    #[test]
    fn hidden_map_defers_document_work_without_spinning() {
        let mut pending = Pending {
            open: Some(("next.battleplan".into(), "v2".into())),
            seek: Some(SeekTarget {
                path: "next.battleplan".into(),
                revision: "v2".into(),
                branch: "3".into(),
                hour: 42,
            }),
            visibility: Some(false),
            ..Default::default()
        };
        assert!(matches!(
            pending.take_commands(false).as_slice(),
            [NativeCommand::Visibility { visible: false }]
        ));
        assert!(!pending.can_send(false));
        assert!(pending.can_send(true));
        assert!(matches!(
            pending.take_commands(true).as_slice(),
            [NativeCommand::Open { .. }, NativeCommand::Seek { .. }]
        ));
    }
}
