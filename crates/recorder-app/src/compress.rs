use replay::{compression::Compression, recompress, recorder_settings::RecorderSettings};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    ffi::OsString,
    io::{self, BufRead, BufReader, Write},
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Condvar, Mutex, MutexGuard},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
        Threading::{BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW},
    },
};

pub const LEVEL: i32 = replay::columnar_file::Settings::HIGH.level;

pub const ARGUMENT: &str = "--recompress";

pub fn auto(settings: &RecorderSettings) -> bool {
    settings.compression == Compression::Low && settings.auto_compress
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Job {
    Queued,
    Compressing { units: u64, total: u64 },
    Kept { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    Progress { units: u64, total: u64 },
    Done,
    Kept(String),
}

impl Line {
    fn text(&self) -> String {
        match self {
            Self::Progress { units, total } => format!("progress {units} {total}"),
            Self::Done => "done".into(),
            Self::Kept(reason) => format!("kept {reason}"),
        }
    }

    fn read(text: &str) -> Option<Self> {
        if text == "done" {
            return Some(Self::Done);
        }
        if let Some(reason) = text.strip_prefix("kept ") {
            return Some(Self::Kept(reason.into()));
        }
        let (units, total) = text.strip_prefix("progress ")?.split_once(' ')?;
        Some(Self::Progress {
            units: units.parse().ok()?,
            total: total.parse().ok()?,
        })
    }
}

pub fn job(path: &Path, below_normal: bool, mut tell: impl FnMut(Line)) -> Result<(), String> {
    let mut shown = None;
    let result = recompress::recompress_file(path, LEVEL, below_normal, |progress| {
        let percent = progress.units * 100 / progress.total.max(1);
        if shown != Some(percent) {
            shown = Some(percent);
            tell(Line::Progress {
                units: progress.units,
                total: progress.total,
            });
        }
        true
    });
    match result {
        Ok(_) => {
            tell(Line::Done);
            Ok(())
        }
        Err(kept) => {
            let reason = match kept {
                recompress::Kept::Failed(why) => format!("compression couldn’t finish: {why}"),
                kept => kept.to_string(),
            };
            tell(Line::Kept(reason.clone()));
            Err(reason)
        }
    }
}

pub fn child(path: Option<OsString>) -> i32 {
    let Some(path) = path else { return 2 };
    let mut out = io::stdout();
    let result = job(Path::new(&path), true, |line| {
        let _ = writeln!(out, "{}", line.text()).and_then(|()| out.flush());
    });
    i32::from(result.is_err())
}

struct Bound(HANDLE);

impl Bound {
    fn new() -> io::Result<Self> {
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let bound = Self(job);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set = unsafe {
            SetInformationJobObject(
                bound.0,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(bound)
    }

    fn bind(&self, child: &std::process::Child) -> io::Result<()> {
        if unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for Bound {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn run(path: &Path, bound: &Bound, mut tell: impl FnMut(Line)) -> Result<(), String> {
    let program = std::env::current_exe().map_err(|error| {
        format!("compression couldn’t start: BattlePlan Recorder couldn’t find itself: {error}")
    })?;
    let mut child = Command::new(program)
        .arg(ARGUMENT)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS)
        .spawn()
        .map_err(|error| format!("compression couldn’t start: {error}"))?;
    if let Err(error) = bound.bind(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!(
            "compression couldn’t start: it couldn’t be tied to BattlePlan Recorder: {error}"
        ));
    }
    let mut ended = None;
    if let Some(output) = child.stdout.take() {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            match Line::read(&line) {
                Some(progress @ Line::Progress { .. }) => tell(progress),
                Some(end) => ended = Some(end),
                None => {}
            }
        }
    }
    let status = child
        .wait()
        .map_err(|error| format!("compression couldn’t finish: {error}"))?;
    match ended {
        Some(Line::Done) if status.success() => Ok(()),
        Some(Line::Kept(reason)) => Err(reason),
        _ => Err(format!(
            "compression couldn’t finish: the compressor stopped early ({status})"
        )),
    }
}

#[derive(Default)]
pub struct Jobs {
    state: Mutex<State>,
    wake: Condvar,
}

#[derive(Default)]
struct State {
    queue: VecDeque<PathBuf>,
    running: Option<(PathBuf, u64, u64)>,
    kept: HashMap<PathBuf, String>,
}

impl Jobs {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn request(&self, path: &Path) -> Result<(), String> {
        let path = path.canonicalize().map_err(|error| {
            format!(
                "Couldn’t compress that recording: {}: {error}",
                path.display()
            )
        })?;
        if !path.is_file() || !replay::recording_files::is_recording(&path) {
            return Err("Choose a BattlePlan recording to compress.".into());
        }
        let mut state = self.lock();
        state.kept.remove(&path);
        if state
            .running
            .as_ref()
            .is_some_and(|(running, ..)| *running == path)
            || state.queue.contains(&path)
        {
            return Ok(());
        }
        state.queue.push_back(path);
        self.wake.notify_one();
        Ok(())
    }

    pub fn saved(&self, path: &Path) {
        if RecorderSettings::load().is_ok_and(|settings| auto(&settings))
            && let Err(error) = self.request(path)
        {
            eprintln!("Auto Compress: {error}");
        }
    }

    pub fn job(&self, path: &Path) -> Option<Job> {
        let state = self.lock();
        if let Some((running, units, total)) = &state.running
            && running == path
        {
            return Some(Job::Compressing {
                units: *units,
                total: *total,
            });
        }
        if state.queue.iter().any(|queued| queued == path) {
            return Some(Job::Queued);
        }
        state.kept.get(path).map(|reason| Job::Kept {
            reason: reason.clone(),
        })
    }

    fn next(&self) -> PathBuf {
        let mut state = self.lock();
        loop {
            if let Some(path) = state.queue.pop_front() {
                state.running = Some((path.clone(), 0, 0));
                return path;
            }
            state = self
                .wake
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn told(&self, path: &Path, line: &Line) {
        if let Line::Progress { units, total } = *line {
            self.lock().running = Some((path.to_owned(), units, total));
        }
    }

    fn ended(&self, path: &Path, result: Result<(), String>) {
        let mut state = self.lock();
        state.running = None;
        if let Err(reason) = result {
            state.kept.insert(path.to_owned(), reason);
        }
    }

    pub fn take_up(&self, root: &Path, settings: &RecorderSettings) {
        for path in crate::recordings::saved_files(root, true).0 {
            let partial = recompress::partial_path(&path);
            if !partial.exists() {
                continue;
            }
            if auto(settings) {
                let _ = self.request(&path);
            } else {
                let _ = std::fs::remove_file(&partial);
            }
        }
    }

    pub fn serve(self: &Arc<Self>, changed: impl Fn() + Send + 'static) -> io::Result<()> {
        let jobs = Arc::clone(self);
        std::thread::Builder::new()
            .name("recording-compress".into())
            .spawn(move || {
                let bound = match Bound::new() {
                    Ok(bound) => bound,
                    Err(error) => {
                        eprintln!("Recordings cannot be compressed: {error}");
                        return;
                    }
                };
                loop {
                    let path = jobs.next();
                    changed();
                    let result = run(&path, &bound, |line| {
                        jobs.told(&path, &line);
                        changed();
                    });
                    jobs.ended(&path, result);
                    changed();
                }
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn auto_compress_does_nothing_while_compression_is_high() {
        let with = |compression, auto_compress| RecorderSettings {
            compression,
            auto_compress,
            ..RecorderSettings::default()
        };
        assert!(auto(&with(Compression::Low, true)));
        assert!(!auto(&with(Compression::High, true)));
        assert!(!auto(&with(Compression::Low, false)));
    }

    #[test]
    fn a_compress_child_s_lines_read_back_as_told() {
        for line in [
            Line::Progress {
                units: 3,
                total: 400,
            },
            Line::Done,
            Line::Kept("open in the desktop app or another program".into()),
        ] {
            assert_eq!(Line::read(&line.text()), Some(line));
        }
        assert_eq!(Line::read("progress 3"), None);
        assert_eq!(Line::read("anything else"), None);
    }

    #[test]
    fn a_recording_open_elsewhere_when_its_job_finishes_is_kept_as_recorded() {
        let folder =
            std::env::temp_dir().join(format!("recorder-app-compress-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("Germany 1936.battleplan");
        let bytes = replay::columnar_frames::fixtures::restored(7, &[(1, 100, 1)]);
        fs::write(&path, &bytes).unwrap();
        let jobs = Jobs::default();
        jobs.request(&path).unwrap();
        let path = jobs.next();
        assert_eq!(
            jobs.job(&path),
            Some(Job::Compressing { units: 0, total: 0 })
        );
        let mut desktop = None;
        let result = job(&path, false, |line| {
            if matches!(line, Line::Progress { .. }) {
                desktop.get_or_insert_with(|| fs::File::open(&path).unwrap());
            }
            jobs.told(&path, &line);
        });
        jobs.ended(&path, result);
        assert_eq!(
            jobs.job(&path),
            Some(Job::Kept {
                reason: "open in the desktop app or another program".into()
            })
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            recompress::level_of(&path).unwrap(),
            Some(7),
            "kept at its live level, so Compress is offered"
        );
        assert!(!recompress::partial_path(&path).exists());
        drop(desktop);
        jobs.request(&path).unwrap();
        assert_eq!(jobs.job(&path), Some(Job::Queued));
        fs::remove_dir_all(&folder).unwrap();
    }
}
