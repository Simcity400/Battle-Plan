mod channel;
pub mod commands;
pub mod control;
mod feed;
mod install;
pub mod saving;
pub mod session;
mod label;

pub use channel::Channel;

use serde::Serialize;
use std::{
    sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};

const RELEASES: Option<&str> = option_env!("BATTLEPLAN_RELEASES");
const VERSION: &str = match option_env!("BATTLEPLAN_VERSION") {
    Some(version) if !version.is_empty() => version,
    _ => env!("CARGO_PKG_VERSION"),
};
pub const STATE_EVENT: &str = "app-update-state";
const FIRST_CHECK: Duration = Duration::from_secs(15);
const CHECK_EVERY: Duration = Duration::from_secs(4 * 60);
const BLOCKED_EVERY: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Dashboard,
    Recorder,
}

impl Role {
    fn pipe(self) -> &'static str {
        match self {
            Self::Dashboard => "dashboard",
            Self::Recorder => "recorder",
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Dashboard => Self::Recorder,
            Self::Recorder => Self::Dashboard,
        }
    }

    fn app(self) -> &'static str {
        match self {
            Self::Dashboard => app_name(),
            Self::Recorder => recorder_name(),
        }
    }

    fn installer_arguments(self) -> &'static str {
        match self {
            Self::Dashboard => "/P /UPDATE /R",
            Self::Recorder => "/P /UPDATE /BATTLEPLANRECORDER",
        }
    }
}

pub type Launch = Box<dyn FnOnce() -> Result<(), String> + Send>;

fn nightly() -> bool {
    VERSION.contains("-nightly.")
}

pub fn app_name() -> &'static str {
    static NAME: LazyLock<String> =
        LazyLock::new(|| format!("BattlePlan ({})", label::channel_label(VERSION)));
    &NAME
}

pub fn recorder_name() -> &'static str {
    static NAME: LazyLock<String> =
        LazyLock::new(|| format!("BattlePlan Recorder ({})", label::channel_label(VERSION)));
    &NAME
}

pub trait Host: Send + Sync + 'static {
    fn install_blocked(&self) -> Option<String> {
        None
    }
    fn busy(&self) -> Option<String> {
        self.install_blocked()
    }
    fn install(&self, launch: Launch) -> Result<(), String>;
    fn close_for_update(&self) -> Result<(), String>;
    fn write_preference(&self, path: &std::path::Path, bytes: &[u8]) -> std::io::Result<Option<String>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Build {
    Stable,
    Nightly,
    Preview,
    Development,
}

impl Build {
    fn current() -> Self {
        if VERSION.contains("-preview.") {
            Self::Preview
        } else if RELEASES.is_none() {
            Self::Development
        } else if nightly() {
            Self::Nightly
        } else {
            Self::Stable
        }
    }

    fn disabled(self) -> Option<&'static str> {
        match self {
            Self::Preview => {
                Some("Preview builds test the release pipeline and are never updated.")
            }
            Self::Development => {
                Some("Updates are off in builds made outside the release pipeline.")
            }
            Self::Stable | Self::Nightly => None,
        }
    }

    fn channel(self) -> Channel {
        if self == Self::Nightly {
            Channel::Nightly
        } else {
            Channel::Stable
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Disabled,
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Downloaded,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Check,
    Download,
    Install,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub enabled: bool,
    pub status: Status,
    pub channel: Channel,
    pub build: Build,
    pub current_version: &'static str,
    pub available_version: Option<String>,
    pub downloaded_version: Option<String>,
    pub releases: Vec<feed::ReleaseNotes>,
    pub unlisted: bool,
    pub download_percent: Option<f64>,
    pub checked_at: Option<u64>,
    pub message: Option<String>,
    pub error_context: Option<Action>,
    pub install_blocked: Option<String>,
    pub channel_warning: Option<String>,
    pub downgrade: bool,
}

struct Inner {
    state: UpdateState,
    update: Option<feed::Release>,
    package: Option<Arc<Vec<u8>>>,
    busy: bool,
    checking: bool,
}

pub struct Updates {
    role: Role,
    host: Arc<dyn Host>,
    control_error: Option<String>,
    check_now: std::sync::atomic::AtomicBool,
    client: Option<reqwest::Client>,
    current: semver::Version,
    inner: Mutex<Inner>,
    wake: Condvar,
}

fn now_ms() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_millis() as u64)
}

fn feed(channel: Channel) -> Result<String, String> {
    let releases = RELEASES.ok_or("This build has no update feed.")?;
    Ok(format!(
        "{releases}/download/{}/latest.json",
        channel.name()
    ))
}

const SETUP_GUARD: &str = "Local\\BattlePlan.Setup";

pub fn setup_running() -> bool {
    use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::OpenMutexW};
    const SYNCHRONIZE: u32 = 0x0010_0000;
    let name: Vec<u16> = SETUP_GUARD.encode_utf16().chain(Some(0)).collect();
    let guard = unsafe { OpenMutexW(SYNCHRONIZE, 0, name.as_ptr()) };
    if guard.is_null() {
        return false;
    }
    unsafe { CloseHandle(guard) };
    true
}

pub fn refused_during_setup(app: &str) -> bool {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if !setup_running() {
        return false;
    }
    let quoted = |text: &str| format!("'{}'", text.replace('\'', "''"));
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; [void][System.Windows.Forms.MessageBox]::Show({}, {}, 'OK', 'Information')",
        quoted(&format!(
            "{app} is being installed or updated. Open it again once setup has finished."
        )),
        quoted(app),
    );
    let encoded = STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<u8>>(),
    );
    let shown = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    if let Err(error) = shown {
        eprintln!("{app} did not start while setup runs, and could not say so: {error}");
    }
    true
}

pub fn init(app: &AppHandle, role: Role, host: Arc<dyn Host>) -> std::io::Result<()> {
    let build = Build::current();
    let saved = channel::load();
    let client = feed::client(VERSION);
    let disabled = build
        .disabled()
        .map(str::to_owned)
        .or_else(|| client.as_ref().err().cloned());
    let state = UpdateState {
        enabled: disabled.is_none(),
        status: if disabled.is_some() {
            Status::Disabled
        } else {
            Status::Idle
        },
        channel: saved
            .as_ref()
            .ok()
            .copied()
            .flatten()
            .unwrap_or(build.channel()),
        build,
        current_version: VERSION,
        available_version: None,
        downloaded_version: None,
        releases: Vec::new(),
        unlisted: false,
        download_percent: None,
        checked_at: None,
        message: disabled.clone().or(saved.err()),
        error_context: None,
        install_blocked: None,
        channel_warning: None,
        downgrade: false,
    };
    let control_error = control::serve(role, Arc::clone(&host))
        .err()
        .map(|error| {
            format!(
                "Setup couldn’t ask {} to close, so it can’t be updated now: {error}",
                role.app()
            )
        });
    let updates = Arc::new(Updates {
        role,
        host: Arc::clone(&host),
        control_error,
        check_now: std::sync::atomic::AtomicBool::new(false),
        client: client.ok(),
        current: semver::Version::parse(VERSION).expect("Cargo versions are semver"),
        inner: Mutex::new(Inner {
            state,
            update: None,
            package: None,
            busy: false,
            checking: false,
        }),
        wake: Condvar::new(),
    });
    app.manage(Arc::clone(&updates));
    if disabled.is_none() {
        let app = app.clone();
        std::thread::Builder::new()
            .name("update checks".into())
            .spawn(move || updates.poll(&app))?;
    }
    Ok(())
}

impl Updates {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, app: &AppHandle, state: UpdateState) {
        if let Err(error) = app.emit(STATE_EVENT, state) {
            eprintln!("The update state could not reach the window: {error}");
        }
    }

    fn change<T>(&self, app: &AppHandle, change: impl FnOnce(&mut Inner) -> T) -> T {
        let (result, state) = {
            let mut inner = self.lock();
            let result = change(&mut inner);
            (result, inner.state.clone())
        };
        self.emit(app, state);
        result
    }

    fn start<T>(
        &self,
        app: &AppHandle,
        checking: bool,
        start: impl FnOnce(&mut Inner) -> Result<T, String>,
    ) -> Result<T, String> {
        let (found, state) = {
            let mut inner = self.lock();
            while inner.checking {
                inner = self
                    .wake
                    .wait(inner)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            Self::available(&inner)?;
            let found = start(&mut inner);
            if found.is_ok() {
                inner.busy = true;
                inner.checking = checking;
            }
            (found, inner.state.clone())
        };
        self.emit(app, state);
        found
    }

    fn available(inner: &Inner) -> Result<(), String> {
        if !inner.state.enabled {
            return Err(inner.state.message.clone().unwrap_or_default());
        }
        if inner.busy {
            return Err(match inner.state.status {
                Status::Downloading => "An update is downloading.".into(),
                _ => "The update is being installed.".into(),
            });
        }
        Ok(())
    }

    fn finish(&self, app: &AppHandle, change: impl FnOnce(&mut Inner)) -> UpdateState {
        let state = self.change(app, |inner| {
            inner.busy = false;
            inner.checking = false;
            change(inner);
            inner.state.clone()
        });
        self.wake.notify_all();
        state
    }

    pub fn state(&self) -> UpdateState {
        self.lock().state.clone()
    }

    fn poll(self: Arc<Self>, app: &AppHandle) {
        let mut next_check = Instant::now() + FIRST_CHECK;
        loop {
            let now = Instant::now();
            let asked = self
                .check_now
                .swap(false, std::sync::atomic::Ordering::AcqRel);
            if now >= next_check || asked {
                next_check = now + CHECK_EVERY;
                let _ = tauri::async_runtime::block_on(self.check(app));
            }
            let waiting = self.lock().package.is_some();
            if waiting {
                self.refresh_blocked(app);
            }
            let until = next_check.saturating_duration_since(Instant::now());
            let inner = self.lock();
            if self.check_now.load(std::sync::atomic::Ordering::Acquire) {
                continue;
            }
            let _ = self
                .wake
                .wait_timeout(
                    inner,
                    if waiting {
                        until.min(BLOCKED_EVERY)
                    } else {
                        until
                    },
                )
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn refresh_blocked(&self, app: &AppHandle) {
        let blocked = self.blocked();
        let changed = {
            let mut inner = self.lock();
            let changed = inner.state.install_blocked != blocked;
            inner.state.install_blocked = blocked;
            changed.then(|| inner.state.clone())
        };
        if let Some(state) = changed {
            self.emit(app, state);
        }
    }

    pub async fn check(&self, app: &AppHandle) -> Result<UpdateState, String> {
        let saved = channel::load();
        let channel = self.start(app, true, |inner| {
            let channel = match &saved {
                Ok(saved) => saved.unwrap_or(inner.state.build.channel()),
                Err(_) => inner.state.channel,
            };
            if channel != inner.state.channel {
                inner.state.channel = channel;
                Self::forget_offer(inner);
            }
            if inner.update.is_none() {
                inner.state.status = Status::Checking;
                inner.state.error_context = None;
                inner.state.message = None;
            }
            Ok(channel)
        })?;
        let switching = channel != Build::current().channel();
        let result = match (saved, feed(channel), &self.client) {
            (Err(error), _, _) | (_, Err(error), _) => Err(error),
            (_, _, None) => Err("The update client didn’t start.".to_owned()),
            (Ok(_), Ok(url), Some(client)) => feed::offer(client, &url, &self.current, switching)
                .await
                .map_err(|error| {
                    format!(
                        "The {} update feed couldn’t be read: {error}.",
                        channel.name()
                    )
                }),
        };
        let state = self.finish(app, |inner| {
            inner.state.checked_at = now_ms();
            match result {
                Ok(Some(update)) => {
                    let kept = inner.package.is_some()
                        && inner.state.downloaded_version.as_deref()
                            == Some(update.version.as_str());
                    if kept {
                        inner.state.status = Status::Downloaded;
                    } else {
                        inner.package = None;
                        inner.state.downloaded_version = None;
                        inner.state.install_blocked = None;
                        inner.state.status = Status::Available;
                    }
                    inner.state.available_version = Some(update.version.clone());
                    inner.state.releases = update.releases.clone();
                    inner.state.unlisted = update.unlisted;
                    inner.state.downgrade = update.older;
                    inner.state.error_context = None;
                    inner.state.message = None;
                    inner.update = Some(update);
                }
                Ok(None) => {
                    Self::forget_offer(inner);
                    inner.state.status = Status::UpToDate;
                    inner.state.error_context = None;
                    inner.state.message = None;
                }
                Err(error) => {
                    if inner.update.is_none() {
                        inner.state.status = Status::Error;
                    }
                    inner.state.error_context = Some(Action::Check);
                    inner.state.message = Some(error);
                }
            }
        });
        match state.error_context {
            Some(Action::Check) => Err(state.message.clone().unwrap_or_default()),
            _ => Ok(state),
        }
    }

    fn blocked(&self) -> Option<String> {
        self.control_error
            .clone()
            .or_else(|| self.host.install_blocked())
            .or_else(|| control::install_blocked(self.role.other()))
    }

    fn reconcile_channel(&self, inner: &mut Inner) -> Result<(), String> {
        let channel = channel::load()?.unwrap_or(inner.state.build.channel());
        if channel == inner.state.channel {
            return Ok(());
        }
        inner.state.channel = channel;
        Self::forget_offer(inner);
        inner.state.status = Status::Idle;
        inner.state.error_context = None;
        inner.state.message = None;
        self.check_now
            .store(true, std::sync::atomic::Ordering::Release);
        self.wake.notify_all();
        Err(format!(
            "The update channel was changed to {} in the other app, so this update was dropped. \
             The {} feed is being checked now.",
            channel.name(),
            channel.name()
        ))
    }

    fn forget_offer(inner: &mut Inner) {
        inner.update = None;
        inner.package = None;
        inner.state.available_version = None;
        inner.state.downloaded_version = None;
        inner.state.releases.clear();
        inner.state.unlisted = false;
        inner.state.download_percent = None;
        inner.state.install_blocked = None;
        inner.state.downgrade = false;
    }

    pub async fn download(self: &Arc<Self>, app: &AppHandle) -> Result<UpdateState, String> {
        let update = self.start(app, false, |inner| {
            self.reconcile_channel(inner)?;
            let update = inner
                .update
                .clone()
                .filter(|_| inner.package.is_none())
                .ok_or("No update is waiting to be downloaded.")?;
            inner.state.status = Status::Downloading;
            inner.state.download_percent = None;
            inner.state.error_context = None;
            inner.state.message = None;
            Ok(update)
        })?;
        let progress = {
            let updates = Arc::clone(self);
            let app = app.clone();
            move |received: u64, total: Option<u64>| {
                let Some(total) = total.filter(|&total| total > 0) else {
                    return;
                };
                let percent = (received as f64 * 100.0 / total as f64).min(100.0);
                let shown = updates.lock().state.download_percent;
                if shown.is_none_or(|shown| percent.floor() > shown.floor()) {
                    updates.change(&app, |inner| inner.state.download_percent = Some(percent));
                }
            }
        };
        let result = match &self.client {
            Some(client) => feed::download(client, &update, progress).await,
            None => Err("the update client didn’t start".to_owned()),
        };
        let blocked = result.as_ref().ok().and_then(|_| self.blocked());
        let state = self.finish(app, |inner| {
            inner.state.download_percent = None;
            match result {
                Ok(package) => {
                    inner.package = Some(Arc::new(package));
                    inner.state.status = Status::Downloaded;
                    inner.state.downloaded_version = Some(update.version.clone());
                    inner.state.install_blocked = blocked;
                }
                Err(error) => {
                    inner.state.status = Status::Error;
                    inner.state.error_context = Some(Action::Download);
                    inner.state.message = Some(format!(
                        "Update {} couldn’t be downloaded: {error}.",
                        update.version
                    ));
                }
            }
        });
        match state.error_context {
            Some(Action::Download) => Err(state.message.clone().unwrap_or_default()),
            _ => Ok(state),
        }
    }

    pub fn install(&self, app: &AppHandle) -> Result<(), String> {
        let (package, version) = self.start(app, false, |inner| {
            self.reconcile_channel(inner)?;
            let found = inner
                .package
                .clone()
                .zip(inner.state.downloaded_version.clone())
                .ok_or("No update has been downloaded.")?;
            inner.state.error_context = None;
            inner.state.message = None;
            Ok(found)
        })?;
        if let Some(reason) = self.blocked() {
            self.finish(app, |inner| {
                inner.state.install_blocked = Some(reason.clone())
            });
            return Err(reason);
        }
        let arguments = self.role.installer_arguments();
        let launch: Launch = Box::new(move || install::launch(&version, &package, arguments));
        let result = self.host.install(launch);
        if let Err(error) = &result {
            self.finish(app, |inner| {
                inner.state.status = Status::Error;
                inner.state.error_context = Some(Action::Install);
                inner.state.message = Some(error.clone());
            });
        }
        result
    }

    pub fn set_channel(&self, app: &AppHandle, channel: Channel) -> Result<UpdateState, String> {
        self.change(app, |inner| {
            if inner.busy {
                return Err("Wait for the update to finish before changing the channel.".into());
            }
            Self::available(inner)?;
            inner.state.channel_warning = channel::save(channel, |path, bytes| self.host.write_preference(path, bytes))?;
            if inner.state.channel != channel {
                inner.state.channel = channel;
                Self::forget_offer(inner);
                inner.state.status = Status::Idle;
                inner.state.error_context = None;
                inner.state.message = None;
            }
            Ok(inner.state.clone())
        })
    }
}
