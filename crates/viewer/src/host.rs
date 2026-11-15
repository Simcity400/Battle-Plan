//! The map inside its host's process. Each map draws on a thread of its own into a window of its
//! own ([`window::Window`]), which the thread that owns the host's window makes and pumps: the
//! map runs no event loop. It talks to its host through in-process channels that carry the
//! desktop protocol's commands and events. A panic in the map stays on its thread and ends the map
//! by name; a lost GPU device ends it so that its host starts it again on a new device and surface.
//! The development harness runs its frames on a thread from the same [`spawn`].

mod driver;
mod window;

use crate::{Options, app, embedded};
use dashboard::protocol::{NativeCommand, NativeEvent};
use eframe::wgpu;
use job_priority::SpawnCarried as _;
use std::{
    any::Any,
    ffi::OsString,
    io,
    panic::{self, AssertUnwindSafe},
    sync::{Arc, Mutex, mpsc},
    thread,
};

pub use embedded::install_follow;
pub use window::{Inputs, Window};

/// Writes each map's frame timings to `<prefix>.<map>.jsonl`.
pub fn log_map_frames(prefix: &std::path::Path) {
    crate::perf_log::log_to(prefix);
}

/// The embedded map's own window's title, by which tools find it among the app's windows.
pub const MAP_WINDOW_TITLE: &str = "Battle Plan map";

/// Makes a map's window, a child of `parent`, on the calling thread, which owns `parent` and pumps
/// its messages.
pub fn create_window(parent: usize) -> io::Result<(Window, Inputs)> {
    Window::create(Some(parent), [1, 1], MAP_WINDOW_TITLE)
}

/// A map thread's stack.
const STACK_BYTES: usize = 64 << 20;

/// Where a map's events go, from threads of the map's own, never its frame thread.
pub type Events = Arc<dyn Fn(NativeEvent) + Send + Sync>;

/// How a map ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Ending {
    /// Its host or its window closed it.
    Closed,
    /// Its GPU device was lost, and why: a new map makes a new device and surface.
    DeviceLost(String),
    /// It could not start, or it stopped: why, by name.
    Failed(String),
}

impl Ending {
    /// A map that did not close, as an error naming why.
    pub(crate) fn into_error(self) -> io::Error {
        match self {
            Self::Closed => io::Error::other("The map closed"),
            Self::DeviceLost(reason) | Self::Failed(reason) => io::Error::other(reason),
        }
    }
}

/// Why a map's GPU device was lost, once it was.
#[derive(Clone, Default)]
pub struct Lost(Arc<Mutex<Option<String>>>);

impl Lost {
    pub fn reason(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).clone()
    }

    fn lose(&self, reason: String) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_or_insert(reason);
    }

    /// Watches `device`: its loss is kept by name and `wake` runs, and the errors its work
    /// raises after the loss are the loss's own; before it, an error panics as wgpu's own
    /// handler does.
    pub fn watch(&self, device: &wgpu::Device, wake: impl Fn() + Send + 'static) {
        let lost = self.clone();
        device.set_device_lost_callback(move |reason, message| {
            let detail = if message.is_empty() { String::new() } else { format!(": {message}") };
            lost.lose(format!("The map's GPU device was lost ({reason:?}{detail})"));
            wake();
        });
        let lost = self.clone();
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            if lost.reason().is_none() {
                panic!("wgpu error: {error}\n");
            }
        }));
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic without a message")
}

/// Runs a map, a panic in it included, to how it ended. A map whose device was lost ended for
/// that, whatever followed the loss.
pub(crate) fn contain<T>(lost: &Lost, run: impl FnOnce() -> Result<T, String>) -> Result<T, Ending> {
    let result = panic::catch_unwind(AssertUnwindSafe(run));
    if let Some(reason) = lost.reason() {
        return Err(Ending::DeviceLost(reason));
    }
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(reason)) => Err(Ending::Failed(reason)),
        Err(payload) => Err(Ending::Failed(format!("The map stopped: {}", panic_message(&*payload)))),
    }
}

/// Runs `run` on a map thread of its own and hands how it ended to `ended` on that thread.
pub fn spawn<T: 'static>(
    name: &str,
    run: impl FnOnce(&Lost) -> Result<T, String> + Send + 'static,
    ended: impl FnOnce(Result<T, Ending>) + Send + 'static,
) -> io::Result<()> {
    thread::Builder::new()
        .name(name.into())
        .stack_size(STACK_BYTES)
        .spawn_carried(move || {
            let lost = Lost::default();
            ended(contain(&lost, || run(&lost)));
        })?;
    Ok(())
}

/// [`spawn`], waiting for how the map ended.
pub(crate) fn run<T: Send + 'static>(
    name: &str,
    run: impl FnOnce(&Lost) -> Result<T, String> + Send + 'static,
) -> io::Result<Result<T, Ending>> {
    let (sent, ended) = mpsc::sync_channel(1);
    spawn(name, run, move |result| {
        let _ = sent.send(result);
    })?;
    ended.recv().map_err(|_| io::Error::other("The map's thread ended without saying how"))
}

/// Which of the host's maps this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Main,
    PopOut,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::PopOut => "popout",
        }
    }
}

/// What a map starts with.
pub struct Launch {
    /// Viewer options as a command line names them (the recording, an hour, a camera, the
    /// folders).
    pub args: Vec<OsString>,
    /// The map's own window, a child of the host's ([`create_window`]), and its input.
    pub window: Window,
    pub inputs: Inputs,
    pub events: Events,
    pub role: Role,
}

/// A map running in this process. Dropping it closes the map.
pub struct Map {
    commands: mpsc::Sender<NativeCommand>,
}

impl Map {
    /// Starts a map on a thread of its own; `ended` hears how it ended, on that thread, once the
    /// map has let go of its surface and asked for its window to be destroyed.
    pub fn start(launch: Launch, ended: impl FnOnce(Ending) + Send + 'static) -> io::Result<Self> {
        let (commands, received) = mpsc::channel();
        let name = format!("map ({})", launch.role.name());
        let window = launch.window.clone();
        spawn(&name, move |lost| embedded_map(launch, received, lost), move |result| {
            window.destroy();
            ended(result.err().unwrap_or(Ending::Closed));
        })?;
        Ok(Self { commands })
    }

    /// Sends `command` to the map; false once the map has gone.
    pub fn send(&self, command: NativeCommand) -> bool {
        self.commands.send(command).is_ok()
    }
}

/// `--window-dump` from the desktop app's own command line: the recording's map drawn through its
/// window from the installed layout, written to `--out`. Folders are its options, else the app's
/// saved ones.
pub fn window_dump(args: impl Iterator<Item = OsString>) -> io::Result<()> {
    let options = Options::parse(args)?;
    if !options.window_dump || options.out.is_none() || options.replay.is_none() {
        return Err(io::Error::other("--window-dump needs a recording and --out"));
    }
    let (mut config, config_error) = match dashboard::Config::load() {
        Ok(config) => (config, None),
        Err(error) => (dashboard::Config::default(), Some(error.to_string())),
    };
    if let Some(path) = &options.game_dir {
        config.game_dir = path.clone();
    }
    if let Some(path) = &options.user_dir {
        config.user_dir = path.clone();
    }
    dump_window(config, options, config_error)
}

/// A `--window-dump` draws into a window of its own, made and pumped by this thread, from a map
/// thread as the desktop's maps do.
pub(crate) fn dump_window(
    config: dashboard::Config,
    options: Options,
    config_error: Option<String>,
) -> io::Result<()> {
    let (window, inputs) = Window::create(None, options.size, "Battle Plan")?;
    if crate::background_window(std::env::var_os("BATTLE_PLAN_VIEWER_PROFILE_BACKGROUND").or_else(|| std::env::var_os("HOI4_VIEWER_PROFILE_BACKGROUND")).as_deref()) {
        window.send_to_back();
    }
    let (sent, ended) = mpsc::sync_channel(1);
    let closing = window.clone();
    let drawn = window.clone();
    spawn(
        "map (window dump)",
        move |lost| {
            viewer_foundation::worker_priority::frame_priority();
            let failure: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
            let reported = Arc::clone(&failure);
            driver::run(&drawn, inputs, lost, move |ctx, state| {
                let _generation = installed_content_core::content::Generation::enter();
                Ok(app::Viewer::new(ctx, Some(state), config, options, config_error, reported, None))
            })?;
            match failure.lock().unwrap_or_else(|error| error.into_inner()).take() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        },
        move |result| {
            closing.destroy();
            let _ = sent.send(result);
        },
    )?;
    window::pump();
    ended
        .recv()
        .map_err(|_| io::Error::other("The map's thread ended without saying how"))?
        .map_err(Ending::into_error)
}

fn embedded_map(
    launch: Launch,
    commands: mpsc::Receiver<NativeCommand>,
    lost: &Lost,
) -> Result<(), String> {
    let Launch { args, window, inputs, events, role } = launch;
    let options = Options::parse(args.into_iter()).map_err(|error| error.to_string())?;
    if options.compare.is_some() || options.headless_dump || options.window_dump {
        return Err(
            "Desktop embedding requires the interactive viewer, without dump or compare options"
                .into(),
        );
    }
    crate::perf_log::enter(role.name());
    // This thread draws the map.
    viewer_foundation::worker_priority::frame_priority();
    // The host owns the content folders and sends them; the map reads none of its own.
    let mut config = dashboard::Config::default();
    if let Some(path) = &options.game_dir {
        config.game_dir = path.clone();
    }
    if let Some(path) = &options.user_dir {
        config.user_dir = path.clone();
    }
    let parent = embedded::Parent::new(window.clone(), commands, events);
    driver::run(&window, inputs, lost, move |ctx, state| {
        let _generation = installed_content_core::content::Generation::enter();
        let embedding = parent.attach(ctx).map_err(|error| format!("The map's window: {error}"))?;
        let failure = Arc::new(Mutex::new(None));
        Ok(app::Viewer::new(ctx, Some(state), config, options, None, failure, Some(embedding)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_in_the_map_ends_it_by_name_and_the_next_map_starts() {
        let failed = run("map (test)", |_: &Lost| -> Result<(), String> {
            panic!("the picture's layer list is empty")
        })
        .unwrap();
        assert_eq!(
            failed,
            Err(Ending::Failed("The map stopped: the picture's layer list is empty".into()))
        );
        let reason = String::from("formatted: 3 layers");
        let failed = run("map (test)", move |_: &Lost| -> Result<(), String> { panic!("{reason}") }).unwrap();
        assert_eq!(failed, Err(Ending::Failed("The map stopped: formatted: 3 layers".into())));
        assert_eq!(run("map (test)", |_: &Lost| Ok::<_, String>(7)).unwrap(), Ok(7));
        assert_eq!(
            run("map (test)", |_: &Lost| Err::<(), _>("no folders".to_owned())).unwrap(),
            Err(Ending::Failed("no folders".into()))
        );
    }

    #[test]
    fn a_map_whose_device_was_lost_ends_for_the_loss_whatever_followed_it() {
        let lost = Lost::default();
        lost.lose("The map's GPU device was lost (Unknown: removed)".into());
        let expected = Err(Ending::DeviceLost("The map's GPU device was lost (Unknown: removed)".into()));
        assert_eq!(contain(&lost, || -> Result<(), String> { panic!("wgpu error: Parent device is lost") }), expected);
        assert_eq!(contain(&lost, || Ok::<(), String>(())), expected);
        // The first reason stays.
        lost.lose("later".into());
        assert_eq!(contain(&lost, || Err::<(), _>("closed".to_owned())), expected);
    }

    /// A map started after another ended (the "Retry map" after a panic) builds on a device of its
    /// own: wgpu tells devices apart by an id, and the device-keyed caches the map's passes share
    /// (pipelines, layouts, the null cube) must never hand the new map the old map's objects.
    // quarantine: #1453 needs a GPU adapter
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn a_map_started_after_another_ended_builds_on_a_device_of_its_own() {
        let _gpu = viewer_foundation::map::test_support::serial();
        let (first, _) = driver::map_device(None).unwrap();
        // The caches keep the first map's device and what was made on it.
        let kept = first.device.clone();
        let made = viewer_foundation::map::device_shared::shared(&first.device, "restart probe", || 1_u32);
        drop(first);
        let (second, _) = driver::map_device(None).unwrap();
        assert!(kept != second.device, "the second map's device compares equal to the first's");
        let mut built = false;
        let again = viewer_foundation::map::device_shared::shared(&second.device, "restart probe", || {
            built = true;
            2_u32
        });
        assert!(built && *again == 2, "the second map was handed the first map's object");
        drop(made);
    }

    /// A real device destroyed under the watcher (the loss a driver reset causes, faked) is kept by
    /// name, wakes the map, and makes the device's later errors the loss's own.
    // quarantine: #1453 needs a GPU adapter
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn a_destroyed_device_is_seen_as_lost_and_its_later_errors_do_not_panic() {
        let _gpu = viewer_foundation::map::test_support::serial();
        let instance = shader_pipeline::gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
        let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let lost = Lost::default();
        let (woke, woken) = mpsc::channel();
        lost.watch(&device, move || {
            let _ = woke.send(());
        });
        device.destroy();
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        woken.recv().unwrap();
        assert!(lost.reason().is_some_and(|reason| reason.starts_with("The map's GPU device was lost (Destroyed")));
        // Work on a lost device raises errors, which the watcher keeps from panicking.
        let _ = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 4,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        assert_eq!(
            contain(&lost, || Ok::<(), String>(())),
            Err(Ending::DeviceLost(lost.reason().unwrap()))
        );
    }
}
