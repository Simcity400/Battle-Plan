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
    panic::AssertUnwindSafe,
    sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, PoisonError, atomic::AtomicU64},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};

type Publish = Box<dyn Fn(UpdateState) + Send + Sync>;
type LoadChannel = fn() -> Result<Option<Channel>, String>;

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
    pub revision: u64,
}

struct Inner {
    state: UpdateState,
    update: Option<feed::Release>,
    package: Option<Arc<Vec<u8>>>,
    busy: bool,
    checking: bool,
}

struct Slot<'a> {
    updates: &'a Updates,
    action: Action,
    held: bool,
}

impl Slot<'_> {
    fn release(&mut self, change: impl FnOnce(&mut Inner)) -> UpdateState {
        let state = {
            let mut inner = self.updates.lock();
            inner.busy = false;
            inner.checking = false;
            change(&mut inner);
            self.held = false;
            Updates::snapshot(&mut inner)
        };
        self.updates.wake.notify_all();
        state
    }

    fn finish(mut self, change: impl FnOnce(&mut Inner)) -> UpdateState {
        let state = self.release(change);
        self.updates.emit(state.clone());
        state
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        if !self.held || !std::thread::panicking() {
            return;
        }
        let action = self.action;
        let state = self.release(|inner| {
            inner.state.download_percent = None;
            if action != Action::Check || inner.update.is_none() {
                inner.state.status = Status::Error;
            }
            inner.state.error_context = Some(action);
            inner.state.message = Some(
                match action {
                    Action::Check => "The update check stopped unexpectedly.",
                    Action::Download => "The update download stopped unexpectedly.",
                    Action::Install => "The update install stopped unexpectedly.",
                }
                .into(),
            );
        });
        let updates = self.updates;
        if std::panic::catch_unwind(AssertUnwindSafe(|| updates.emit(state))).is_err() {
            eprintln!("The window couldn’t be told that the update stopped unexpectedly.");
        }
    }
}

pub struct Updates {
    role: Role,
    host: Arc<dyn Host>,
    control_error: Option<String>,
    check_now: std::sync::atomic::AtomicBool,
    client: Option<reqwest::Client>,
    current: semver::Version,
    load_channel: LoadChannel,
    publish: Publish,
    sent: AtomicU64,
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
        revision: 0,
    };
    let control_error = control::serve(role, Arc::clone(&host))
        .err()
        .map(|error| {
            format!(
                "Setup couldn’t ask {} to close, so it can’t be updated now: {error}",
                role.app()
            )
        });
    let publish = {
        let app = app.clone();
        Box::new(move |state: UpdateState| {
            if let Err(error) = app.emit(STATE_EVENT, state) {
                eprintln!("The update state could not reach the window: {error}");
            }
        })
    };
    let updates = Arc::new(Updates::new(role, host, control_error, client.ok(), state, channel::load, publish));
    app.manage(Arc::clone(&updates));
    if disabled.is_none() {
        std::thread::Builder::new()
            .name("update checks".into())
            .spawn(move || updates.poll())?;
    }
    Ok(())
}

impl Updates {
    fn new(
        role: Role,
        host: Arc<dyn Host>,
        control_error: Option<String>,
        client: Option<reqwest::Client>,
        state: UpdateState,
        load_channel: LoadChannel,
        publish: Publish,
    ) -> Self {
        Self {
            role,
            host,
            control_error,
            check_now: std::sync::atomic::AtomicBool::new(false),
            client,
            current: semver::Version::parse(VERSION).expect("Cargo versions are semver"),
            load_channel,
            publish,
            sent: AtomicU64::new(0),
            inner: Mutex::new(Inner {
                state,
                update: None,
                package: None,
                busy: false,
                checking: false,
            }),
            wake: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn snapshot(inner: &mut Inner) -> UpdateState {
        inner.state.revision += 1;
        inner.state.clone()
    }

    fn emit(&self, state: UpdateState) {
        if self.sent.fetch_max(state.revision, std::sync::atomic::Ordering::AcqRel) < state.revision {
            (self.publish)(state);
        }
    }

    fn change<T>(&self, change: impl FnOnce(&mut Inner) -> T) -> T {
        let (result, snapshot) = {
            let mut inner = self.lock();
            let result = change(&mut inner);
            (result, Self::snapshot(&mut inner))
        };
        self.emit(snapshot);
        result
    }

    fn start<T>(
        &self,
        action: Action,
        start: impl FnOnce(&mut Inner) -> Result<T, String>,
    ) -> Result<(T, Slot<'_>), String> {
        let (found, snapshot) = {
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
                inner.checking = action == Action::Check;
            }
            (found, Self::snapshot(&mut inner))
        };
        let found = found.map(|found| (found, Slot { updates: self, action, held: true }));
        self.emit(snapshot);
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

    pub fn state(&self) -> UpdateState {
        self.lock().state.clone()
    }

    fn poll(self: Arc<Self>) {
        let mut next_check = Instant::now() + FIRST_CHECK;
        loop {
            let now = Instant::now();
            let asked = self
                .check_now
                .swap(false, std::sync::atomic::Ordering::AcqRel);
            let due = now >= next_check || asked;
            if due {
                next_check = now + CHECK_EVERY;
            }
            self.scheduled(due);
            let until = next_check.saturating_duration_since(Instant::now());
            let inner = self.lock();
            if self.check_now.load(std::sync::atomic::Ordering::Acquire) {
                continue;
            }
            let limit = if inner.package.is_some() {
                until.min(BLOCKED_EVERY)
            } else {
                until
            };
            let _ = self
                .wake
                .wait_timeout(inner, limit)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn scheduled(&self, due: bool) {
        let contained = |what: &str, work: &dyn Fn()| {
            if std::panic::catch_unwind(AssertUnwindSafe(work)).is_err() {
                eprintln!("The automatic {what} stopped unexpectedly, and runs again at its next turn.");
            }
        };
        if due {
            contained("update check", &|| {
                let _ = tauri::async_runtime::block_on(self.check());
            });
        }
        if self.lock().package.is_some() {
            contained("install-block refresh", &|| self.refresh_blocked());
        }
    }

    fn refresh_blocked(&self) {
        let blocked = self.blocked();
        let changed = {
            let mut inner = self.lock();
            if inner.package.is_none() || inner.state.install_blocked == blocked {
                return;
            }
            inner.state.install_blocked = blocked;
            Self::snapshot(&mut inner)
        };
        self.emit(changed);
    }

    pub async fn check(&self) -> Result<UpdateState, String> {
        let ((saved, channel), slot) = self.start(Action::Check, |inner| {
            let saved = (self.load_channel)();
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
            Ok((saved, channel))
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
        let state = slot.finish(|inner| {
            if inner.state.channel != channel {
                return;
            }
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
        let channel = (self.load_channel)()?.unwrap_or(inner.state.build.channel());
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

    pub async fn download(self: &Arc<Self>) -> Result<UpdateState, String> {
        let (update, slot) = self.start(Action::Download, |inner| {
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
            move |received: u64, total: Option<u64>| {
                let Some(total) = total.filter(|&total| total > 0) else {
                    return;
                };
                let percent = (received as f64 * 100.0 / total as f64).min(100.0);
                let shown = updates.lock().state.download_percent;
                if shown.is_none_or(|shown| percent.floor() > shown.floor()) {
                    updates.change(|inner| inner.state.download_percent = Some(percent));
                }
            }
        };
        let result = match &self.client {
            Some(client) => feed::download(client, &update, progress).await,
            None => Err("the update client didn’t start".to_owned()),
        };
        let blocked = result.as_ref().ok().and_then(|_| self.blocked());
        let state = slot.finish(|inner| {
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

    pub fn install(&self) -> Result<(), String> {
        let ((package, version), slot) = self.start(Action::Install, |inner| {
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
            slot.finish(|inner| {
                inner.state.install_blocked = Some(reason.clone())
            });
            return Err(reason);
        }
        let arguments = self.role.installer_arguments();
        let launch: Launch = Box::new(move || install::launch(&version, &package, arguments));
        let result = self.host.install(launch);
        if let Err(error) = &result {
            slot.finish(|inner| {
                inner.state.status = Status::Error;
                inner.state.error_context = Some(Action::Install);
                inner.state.message = Some(error.clone());
            });
        }
        result
    }

    pub fn set_channel(&self, channel: Channel) -> Result<UpdateState, String> {
        self.change(|inner| {
            if !inner.state.enabled {
                return Err(inner.state.message.clone().unwrap_or_default());
            }
            if inner.busy && !inner.checking {
                return Err("Wait for the update to finish before changing the channel.".into());
            }
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

#[cfg(test)]
mod tests {
    use super::{Action, Arc, Build, Channel, Host, Launch, Mutex, Role, Status, UpdateState, Updates, VERSION};
    use std::{
        panic::AssertUnwindSafe,
        sync::{
            OnceLock, Weak,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
    };

    const RECORDING: &str = "BattlePlan Recorder is recording.";

    #[derive(Default)]
    struct App {
        while_blocked: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    impl Host for App {
        fn install_blocked(&self) -> Option<String> {
            let meanwhile = self.while_blocked.lock().unwrap().take();
            if let Some(meanwhile) = meanwhile {
                meanwhile();
            }
            Some(RECORDING.to_owned())
        }

        fn install(&self, _: Launch) -> Result<(), String> {
            unreachable!("no install in these tests")
        }

        fn close_for_update(&self) -> Result<(), String> {
            unreachable!("no close in these tests")
        }

        fn write_preference(&self, _: &std::path::Path, _: &[u8]) -> std::io::Result<Option<String>> {
            Ok(None)
        }
    }

    fn first(call: impl FnOnce() + Send + 'static) -> impl Fn(UpdateState) + Send + Sync + 'static {
        let call = Mutex::new(Some(call));
        move |_| {
            let call = call.lock().unwrap().take();
            if let Some(call) = call {
                call();
            }
        }
    }

    fn updates(host: Arc<App>, publish: impl Fn(UpdateState) + Send + Sync + 'static) -> Updates {
        let state = UpdateState {
            enabled: true,
            status: Status::Idle,
            channel: Channel::Stable,
            build: Build::Stable,
            current_version: VERSION,
            available_version: None,
            downloaded_version: None,
            releases: Vec::new(),
            unlisted: false,
            download_percent: None,
            checked_at: None,
            message: None,
            error_context: None,
            install_blocked: None,
            channel_warning: None,
            downgrade: false,
            revision: 0,
        };
        Updates::new(Role::Dashboard, host, None, None, state, || Ok(Some(Channel::Stable)), Box::new(publish))
    }

    #[test]
    fn a_channel_chosen_during_a_background_check_is_kept_and_the_old_feeds_answer_is_dropped() {
        let (entered, checking) = mpsc::channel();
        let (go, held) = mpsc::channel::<()>();
        let updates = updates(
            Arc::default(),
            first(move || {
                entered.send(()).unwrap();
                held.recv().unwrap();
            }),
        );
        std::thread::scope(|scope| {
            let check = scope.spawn(|| tauri::async_runtime::block_on(updates.check()));
            checking.recv().unwrap();
            let chosen = updates.set_channel(Channel::Nightly);
            go.send(()).unwrap();
            assert!(chosen.is_ok(), "a background check doesn’t refuse a channel change");
            let _ = check.join().unwrap();
        });
        let state = updates.state();
        assert_eq!(state.channel, Channel::Nightly, "the check doesn’t put back the channel it started on");
        assert_eq!((state.status, state.message), (Status::Idle, None), "the stable feed’s answer isn’t shown as nightly’s");
    }

    #[test]
    fn an_older_state_published_after_a_newer_one_isnt_delivered() {
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let updates = updates(Arc::default(), {
            let delivered = Arc::clone(&delivered);
            move |state: UpdateState| delivered.lock().unwrap().push(state.revision)
        });
        let (older, newer) = {
            let mut inner = updates.lock();
            (Updates::snapshot(&mut inner), Updates::snapshot(&mut inner))
        };
        let revision = newer.revision;
        updates.emit(newer);
        updates.emit(older);
        assert_eq!(*delivered.lock().unwrap(), [revision]);
    }

    #[test]
    fn an_install_block_read_while_the_download_was_dropped_isnt_written_back() {
        let host = Arc::new(App::default());
        let updates = Arc::new(updates(Arc::clone(&host), |_| {}));
        {
            let mut inner = updates.lock();
            inner.package = Some(Arc::new(vec![0]));
            inner.state.downloaded_version = Some("9.0.0".into());
            inner.state.status = Status::Downloaded;
        }
        let dropping = Arc::downgrade(&updates);
        *host.while_blocked.lock().unwrap() = Some(Box::new(move || {
            dropping.upgrade().unwrap().set_channel(Channel::Nightly).unwrap();
        }));
        updates.refresh_blocked();
        let state = updates.state();
        assert_eq!(state.downloaded_version, None, "changing the channel dropped the download");
        assert_eq!(state.install_blocked, None, "no install is blocked once nothing is waiting to install");
    }

    #[test]
    fn a_scheduled_check_whose_window_panics_says_it_stopped_and_install_blocks_are_still_refreshed() {
        let updates = updates(Arc::default(), |_| panic!("the window went away mid-update"));
        {
            let mut inner = updates.lock();
            inner.package = Some(Arc::new(vec![0]));
            inner.state.downloaded_version = Some("9.0.0".into());
        }
        updates.scheduled(true);
        let state = updates.state();
        assert_eq!((state.status, state.error_context), (Status::Error, Some(Action::Check)), "the check is shown as stopped, not still checking");
        assert_eq!(state.install_blocked.as_deref(), Some(RECORDING), "polling carries on after the panic");
        let inner = updates.lock();
        assert!(!inner.busy && !inner.checking, "a later check, download or install isn’t refused or left waiting");
    }

    #[test]
    fn a_window_that_panics_on_a_checks_answer_doesnt_free_a_slot_taken_since() {
        let taken = Arc::new(OnceLock::<Weak<Updates>>::new());
        let publishes = AtomicUsize::new(0);
        let updates = Arc::new(updates(Arc::default(), {
            let taken = Arc::clone(&taken);
            move |_| {
                if publishes.fetch_add(1, Ordering::SeqCst) == 1 {
                    taken.get().unwrap().upgrade().unwrap().lock().busy = true;
                    panic!("the window went away mid-update");
                }
            }
        }));
        taken.set(Arc::downgrade(&updates)).unwrap();
        let panicked = std::panic::catch_unwind(AssertUnwindSafe(|| tauri::async_runtime::block_on(updates.check())));
        assert!(panicked.is_err());
        assert!(updates.lock().busy, "the action that took the freed slot keeps it");
    }
}
