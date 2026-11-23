
pub(crate) mod driver;
mod window;

use crate::{Options, app, embedded};
use dashboard_core::protocol::{NativeCommand, NativeEvent};
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
pub use window::{Inputs, WM_TAKE_FOCUS, Window};

pub fn log_map_frames(prefix: &std::path::Path) {
    crate::perf_log::log_to(prefix);
}

pub const MAP_WINDOW_TITLE: &str = "BattlePlan map";

pub fn create_window(parent: usize) -> io::Result<(Window, Inputs)> {
    Window::create(Some(parent), [1, 1], MAP_WINDOW_TITLE)
}

const STACK_BYTES: usize = 64 << 20;

pub type Events = Arc<dyn Fn(NativeEvent) + Send + Sync>;

#[derive(Clone, Debug, PartialEq)]
pub enum Ending {
    Closed,
    DeviceLost(String),
    Failed(String),
}

impl Ending {
    pub(crate) fn into_error(self) -> io::Error {
        match self {
            Self::Closed => io::Error::other("The map closed"),
            Self::DeviceLost(reason) | Self::Failed(reason) => io::Error::other(reason),
        }
    }
}

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

    pub fn watch(&self, device: &wgpu::Device, wake: impl Fn() + Send + 'static) {
        let lost = self.clone();
        device.set_device_lost_callback(move |reason, message| {
            let detail = if message.is_empty() { String::new() } else { format!(": {message}") };
            lost.lose(format!("The map lost its graphics device: {reason:?}{detail}"));
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

thread_local! {
    static PANICKED_AT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn note_panic_locations() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let at = info.location().map(|at| format!("{}:{}", at.file(), at.line()));
            PANICKED_AT.with(|held| *held.borrow_mut() = at);
            previous(info);
        }));
    });
}

pub(crate) fn contain<T>(lost: &Lost, run: impl FnOnce() -> Result<T, String>) -> Result<T, Ending> {
    note_panic_locations();
    PANICKED_AT.with(|held| held.borrow_mut().take());
    let result = panic::catch_unwind(AssertUnwindSafe(run));
    if let Some(reason) = lost.reason() {
        return Err(Ending::DeviceLost(reason));
    }
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(reason)) => Err(Ending::Failed(reason)),
        Err(payload) => {
            let message = panic_message(&*payload);
            Err(Ending::Failed(match PANICKED_AT.with(|held| held.borrow_mut().take()) {
                Some(at) => format!("The map stopped: {message} (at {at})"),
                None => format!("The map stopped: {message}"),
            }))
        }
    }
}

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

pub(crate) fn run<T: Send + 'static>(
    name: &str,
    run: impl FnOnce(&Lost) -> Result<T, String> + Send + 'static,
) -> io::Result<Result<T, Ending>> {
    let (sent, ended) = mpsc::sync_channel(1);
    spawn(name, run, move |result| {
        let _ = sent.send(result);
    })?;
    ended.recv().map_err(|_| io::Error::other("The map’s thread ended without saying how"))
}

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

pub struct Launch {
    pub args: Vec<OsString>,
    pub window: Window,
    pub inputs: Inputs,
    pub events: Events,
    pub role: Role,
    pub service: Arc<dashboard::Service>,
}

pub const STOPPED: &str = "The map stopped before the export finished";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Saved {
    pub message: String,
    pub warning: Option<String>,
}

pub type Answer = Result<Saved, String>;

pub struct Reply(Option<Box<dyn FnOnce(Answer) + Send>>);

impl Reply {
    pub fn new(answer: impl FnOnce(Answer) + Send + 'static) -> Self {
        Self(Some(Box::new(answer)))
    }

    fn send(mut self, answer: Answer) {
        if let Some(reply) = self.0.take() {
            reply(answer);
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        if let Some(reply) = self.0.take() {
            reply(Err(STOPPED.to_owned()));
        }
    }
}

pub struct ExportJob {
    reply: Reply,
    pub path: String,
    pub view_id: Option<String>,
    pub receipt: replay_settings::exit::Receipt<'static>,
}

impl ExportJob {
    pub fn new(path: String, view_id: Option<String>, receipt: replay_settings::exit::Receipt<'static>, reply: Reply) -> Self {
        Self { reply, path, view_id, receipt }
    }

    pub fn answer(self, answer: Answer) {
        let Self { reply, receipt, .. } = self;
        reply.send(answer);
        drop(receipt);
    }
}

pub enum Command {
    Native(NativeCommand),
    Export(ExportJob),
}

impl From<NativeCommand> for Command {
    fn from(native: NativeCommand) -> Self {
        Self::Native(native)
    }
}

pub struct Map {
    commands: mpsc::Sender<Command>,
}

impl Map {
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

    pub fn send(&self, command: impl Into<Command>) -> bool {
        self.commands.send(command.into()).is_ok()
    }

    pub fn send_export(&self, job: ExportJob) -> Result<(), ExportJob> {
        self.commands.send(Command::Export(job)).map_err(|refused| match refused.0 {
            Command::Export(job) => job,
            Command::Native(_) => unreachable!("an export was sent"),
        })
    }
}

pub fn window_dump(args: impl Iterator<Item = OsString>) -> io::Result<()> {
    let options = Options::parse(args)?;
    if !options.window_dump || options.out.is_none() || options.replay.is_none() {
        return Err(io::Error::other("--window-dump needs a recording and --out"));
    }
    let (mut config, config_error) = match dashboard_core::config::Config::load() {
        Ok(config) => (config, None),
        Err(error) => (dashboard_core::config::Config::default(), Some(error.to_string())),
    };
    if let Some(path) = &options.game_dir {
        config.game_dir = path.clone();
    }
    if let Some(path) = &options.user_dir {
        config.user_dir = path.clone();
    }
    dump_window(config, options, config_error)
}

pub(crate) fn dump_window(
    config: dashboard_core::config::Config,
    options: Options,
    config_error: Option<String>,
) -> io::Result<()> {
    let (window, inputs) = Window::create(None, options.size, "BattlePlan")?;
    if crate::background_window(std::env::var_os("BATTLEPLAN_VIEWER_PROFILE_BACKGROUND").or_else(|| std::env::var_os("HOI4_VIEWER_PROFILE_BACKGROUND")).as_deref()) {
        window.send_to_back();
    }
    let (sent, ended) = mpsc::sync_channel(1);
    let closing = window.clone();
    let drawn = window.clone();
    spawn(
        "map (window dump)",
        move |lost| {
            viewer_runtime::worker_priority::frame_priority();
            let failure: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
            let reported = Arc::clone(&failure);
            driver::run(&drawn, inputs, lost, move |ctx, state| {
                let _generation = content_reader_layers::content::Generation::enter();
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
        .map_err(|_| io::Error::other("The map’s thread ended without saying how"))?
        .map_err(Ending::into_error)
}

pub(crate) fn paced_window(
    size: [u32; 2],
    job: impl FnOnce(&Window, Inputs) -> io::Result<()> + Send + 'static,
) -> io::Result<()> {
    let (window, inputs) = Window::create(None, size, "BattlePlan map (paced)")?;
    let (sent, ended) = mpsc::sync_channel(1);
    let closing = window.clone();
    let drawn = window.clone();
    spawn(
        "map (harness)",
        move |_lost| {
            viewer_runtime::worker_priority::frame_priority();
            job(&drawn, inputs).map_err(|error| error.to_string())
        },
        move |result| {
            closing.destroy();
            let _ = sent.send(result);
        },
    )?;
    window::pump();
    ended
        .recv()
        .map_err(|_| io::Error::other("The map’s thread ended without saying how"))?
        .map_err(Ending::into_error)
}

fn embedded_map(
    launch: Launch,
    commands: mpsc::Receiver<Command>,
    lost: &Lost,
) -> Result<(), String> {
    let Launch { args, window, inputs, events, role, service } = launch;
    let options = Options::parse(args.into_iter()).map_err(|error| error.to_string())?;
    if options.compare.is_some() || options.headless_dump || options.window_dump {
        return Err(
            "Desktop embedding requires the interactive viewer, without dump or compare options"
                .into(),
        );
    }
    crate::perf_log::enter(role.name());
    viewer_runtime::worker_priority::frame_priority();
    let mut config = dashboard_core::config::Config::default();
    if let Some(path) = &options.game_dir {
        config.game_dir = path.clone();
    }
    if let Some(path) = &options.user_dir {
        config.user_dir = path.clone();
    }
    let parent = embedded::Parent::new(window.clone(), commands, events, service);
    driver::run(&window, inputs, lost, move |ctx, state| {
        let _generation = content_reader_layers::content::Generation::enter();
        let embedding = parent.attach(ctx).map_err(|error| format!("Couldn’t show the map in its window: {error}"))?;
        let failure = Arc::new(Mutex::new(None));
        Ok(app::Viewer::new(ctx, Some(state), config, options, None, failure, Some(embedding)))
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) type Answers = Arc<Mutex<Vec<Answer>>>;

    pub(crate) fn job_under(exit: &'static replay_settings::exit::Exit, path: &str) -> (ExportJob, Answers) {
        let answers = Answers::default();
        let heard = Arc::clone(&answers);
        let receipt = exit.admit(replay_settings::exit::What::Export).unwrap();
        (ExportJob::new(path.into(), None, receipt, Reply::new(move |answer| heard.lock().unwrap().push(answer))), answers)
    }

    pub(crate) fn job(path: &str) -> (ExportJob, Answers) {
        job_under(Box::leak(Box::new(replay_settings::exit::Exit::new())), path)
    }

    pub(crate) fn taken(answers: &Answers) -> Vec<Result<String, String>> {
        taken_saved(answers).into_iter().map(|answer| answer.map(|saved| saved.message)).collect()
    }

    pub(crate) fn taken_saved(answers: &Answers) -> Vec<Answer> {
        std::mem::take(&mut *answers.lock().unwrap())
    }

    #[test]
    fn an_export_job_answers_once_and_settles_its_reply_before_it_lets_its_receipt_go() {
        let exit: &'static replay_settings::exit::Exit = Box::leak(Box::new(replay_settings::exit::Exit::new()));
        let answers = Answers::default();
        let heard = Arc::clone(&answers);
        let job = ExportJob::new("map.png".into(), None, exit.admit(replay_settings::exit::What::Export).unwrap(), Reply::new(move |answer| {
            assert_eq!(exit.count(replay_settings::exit::What::Export), 1, "the reply is settled while the receipt still holds the exit");
            heard.lock().unwrap().push(answer);
        }));
        job.answer(Ok(Saved { message: "Saved map.png".into(), warning: None }));
        assert_eq!(taken(&answers), [Ok("Saved map.png".to_owned())]);
        assert_eq!(exit.count(replay_settings::exit::What::Export), 0);
        let (dropped, answers) = job_under(exit, "lost.png");
        drop(dropped);
        assert_eq!(taken(&answers), [Err(STOPPED.to_owned())], "a job the map drops unanswered still answers, by name");
        assert_eq!(exit.count(replay_settings::exit::What::Export), 0);
    }


    #[test]
    fn a_panic_in_the_map_ends_it_by_name_and_where_and_the_next_map_starts() {
        let at = line!() + 2;
        let failed = run("map (test)", |_: &Lost| -> Result<(), String> {
            panic!("the picture’s layer list is empty")
        })
        .unwrap();
        assert_eq!(
            failed,
            Err(Ending::Failed(format!("The map stopped: the picture’s layer list is empty (at {}:{at})", file!())))
        );
        let reason = String::from("formatted: 3 layers");
        let at = line!() + 1;
        let failed = run("map (test)", move |_: &Lost| -> Result<(), String> { panic!("{reason}") }).unwrap();
        assert_eq!(failed, Err(Ending::Failed(format!("The map stopped: formatted: 3 layers (at {}:{at})", file!()))));
        assert_eq!(run("map (test)", |_: &Lost| Ok::<_, String>(7)).unwrap(), Ok(7));
        assert_eq!(
            run("map (test)", |_: &Lost| Err::<(), _>("no folders".to_owned())).unwrap(),
            Err(Ending::Failed("no folders".into()))
        );
    }

    #[test]
    fn a_map_whose_device_was_lost_ends_for_the_loss_whatever_followed_it() {
        let lost = Lost::default();
        lost.lose("The map lost its graphics device: Unknown".into());
        let expected = Err(Ending::DeviceLost("The map lost its graphics device: Unknown".into()));
        assert_eq!(contain(&lost, || -> Result<(), String> { panic!("wgpu error: Parent device is lost") }), expected);
        assert_eq!(contain(&lost, || Ok::<(), String>(())), expected);
        lost.lose("later".into());
        assert_eq!(contain(&lost, || Err::<(), _>("closed".to_owned())), expected);
    }

    #[test]
    fn a_map_started_after_another_ended_builds_on_a_device_of_its_own() {
        let _gpu = viewer_gpu::test_support::serial();
        let (first, _) = driver::map_device(None).unwrap_or_else(|reason| {
            panic!(
                "a_map_started_after_another_ended_builds_on_a_device_of_its_own requires a usable GPU or WARP adapter: {reason}"
            )
        });
        let kept = first.device.clone();
        let made = viewer_runtime::device_shared::shared(&first.device, "restart probe", || 1_u32);
        drop(first);
        let (second, _) = driver::map_device(None).unwrap();
        assert!(kept != second.device, "the second map's device compares equal to the first's");
        let mut built = false;
        let again = viewer_runtime::device_shared::shared(&second.device, "restart probe", || {
            built = true;
            2_u32
        });
        assert!(built && *again == 2, "the second map was handed the first map's object");
        drop(made);
    }

    #[test]
    fn a_destroyed_device_is_seen_as_lost_and_its_later_errors_do_not_panic() {
        let _gpu = viewer_gpu::test_support::serial();
        let instance = shader_pipeline::gpu::instance();
        let adapter = pollster::block_on(async {
            match instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
            {
                Ok(adapter) => Ok(adapter),
                Err(_) => {
                    instance
                        .request_adapter(&wgpu::RequestAdapterOptions {
                            force_fallback_adapter: true,
                            ..Default::default()
                        })
                        .await
                }
            }
        })
        .unwrap_or_else(|error| {
            panic!(
                "a_destroyed_device_is_seen_as_lost_and_its_later_errors_do_not_panic requires a GPU or WARP adapter: {error}"
            )
        });
        let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let lost = Lost::default();
        let (woke, woken) = mpsc::channel();
        lost.watch(&device, move || {
            let _ = woke.send(());
        });
        device.destroy();
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        woken.recv().unwrap();
        assert!(lost.reason().is_some_and(|reason| reason.starts_with("The map lost its graphics device: Destroyed")));
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
