
use job_priority::SpawnCarried as _;
use crate::{
    Frame, Mix,
    engine::{CALLBACK_FRAMES, Content, Engine, Load, RATE, Samples},
    sdl::{DeviceFormat, Stream},
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    io::Write as _,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const AHEAD: Duration = Duration::from_millis(60);
const RETAIN: Duration = Duration::from_secs(60);
const STEP: usize = CALLBACK_FRAMES;
const CATCH_UP: usize = 5;
const REOPEN: Duration = Duration::from_secs(3);
const STALLED: Duration = Duration::from_secs(1);
const MUSIC_OPEN: Duration = Duration::from_secs(2);

pub struct Source {
    pub content: Arc<Content>,
    pub layers: content_reader_layers::ResolvedLayers,
}

type Wake = Arc<dyn Fn() + Send + Sync>;

enum Command {
    Frame(u64, Arc<Source>, Frame, Option<Wake>),
    Mix(u64, Mix),
    Claim(u64),
    Release(u64),
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub device: Option<Result<String, String>>,
    pub notes: Vec<String>,
}

pub struct System {
    commands: Mutex<mpsc::Sender<Command>>,
    owner: AtomicU64,
    next: AtomicU64,
    status: Arc<Mutex<Status>>,
}

pub struct Owner {
    wake: Option<Wake>,
    wake_enabled: bool,
    token: u64,
    system: Arc<System>,
    last: Option<Instant>,
}

static SILENT: AtomicBool = AtomicBool::new(false);

impl System {
    pub fn silent() {
        SILENT.store(true, Ordering::Release);
    }

    pub fn shared() -> Arc<Self> {
        static SHARED: std::sync::OnceLock<Arc<System>> = std::sync::OnceLock::new();
        Arc::clone(SHARED.get_or_init(Self::start))
    }

    fn start() -> Arc<Self> {
        Self::spawn(SILENT.load(Ordering::Acquire))
    }

    fn spawn(silent: bool) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let system = Arc::new(Self {
            commands: Mutex::new(sender),
            owner: AtomicU64::new(0),
            next: AtomicU64::new(1),
            status: Arc::clone(&status),
        });
        let spawned = thread::Builder::new()
            .name("map audio".into())
            .spawn_carried(move || control(receiver, status, silent));
        if let Err(error) = spawned {
            system
                .status
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .device = Some(Err(format!("Map audio thread did not start: {error}")));
        }
        system
    }

    pub fn owner(self: &Arc<Self>) -> Owner {
        Owner {
            wake: None,
            wake_enabled: true,
            token: self.next.fetch_add(1, Ordering::Relaxed),
            system: Arc::clone(self),
            last: None,
        }
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn send(&self, command: Command) {
        let sent = self
            .commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(command);
        if sent.is_err() {
            self.status.lock().unwrap_or_else(|e| e.into_inner()).device =
                Some(Err("The map audio thread stopped".into()));
        }
    }
}

impl Owner {
    pub fn claim(&mut self) {
        if self.system.owner.swap(self.token, Ordering::AcqRel) != self.token {
            self.last = None;
            self.system.send(Command::Claim(self.token));
        }
    }

    pub fn set_wake(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.wake = Some(Arc::new(wake));
    }

    pub fn set_wake_enabled(&mut self, enabled: bool) {
        self.wake_enabled = enabled;
    }

    pub fn owns(&self) -> bool {
        self.system.owner.load(Ordering::Acquire) == self.token
    }

    pub fn release(&mut self) {
        if self
            .system
            .owner
            .compare_exchange(self.token, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.last = None;
            self.system.send(Command::Release(self.token));
        }
    }

    pub fn set_mix(&self, mix: Mix) {
        if self.owns() {
            self.system.send(Command::Mix(self.token, mix));
        }
    }

    pub fn frame(&mut self, source: &Arc<Source>, mut frame: Frame) {
        if !self.owns() {
            return;
        }
        let now = Instant::now();
        frame.elapsed = self
            .last
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        self.last = Some(now);
        self.system.send(Command::Frame(
            self.token,
            Arc::clone(source),
            frame,
            self.wake_enabled.then(|| self.wake.clone()).flatten(),
        ));
    }

    pub fn status(&self) -> Status {
        self.system.status()
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.release();
    }
}

struct Ring {
    slots: Box<[AtomicU32]>,
    channels: usize,
    read: AtomicUsize,
    write: AtomicUsize,
}

impl Ring {
    fn new(frames: usize, channels: usize) -> Self {
        let channels = channels.max(1);
        Self {
            slots: (0..frames.max(1) * channels)
                .map(|_| AtomicU32::new(0))
                .collect(),
            channels,
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
        }
    }

    fn len(&self) -> usize {
        let read = self.read.load(Ordering::Acquire);
        self.write.load(Ordering::Acquire).wrapping_sub(read)
    }

    fn consumed(&self) -> usize {
        self.read.load(Ordering::Acquire)
    }

    fn push_frame(&self, frame: &[f32]) -> bool {
        debug_assert_eq!(frame.len(), self.channels);
        let write = self.write.load(Ordering::Relaxed);
        if write.wrapping_sub(self.read.load(Ordering::Acquire)) + self.channels > self.slots.len()
        {
            return false;
        }
        for (offset, value) in frame.iter().enumerate() {
            self.slots[write.wrapping_add(offset) % self.slots.len()]
                .store(value.to_bits(), Ordering::Relaxed);
        }
        self.write
            .store(write.wrapping_add(self.channels), Ordering::Release);
        true
    }

    fn pop_frame(&self, mut take: impl FnMut(usize, f32)) -> bool {
        let read = self.read.load(Ordering::Relaxed);
        if self.write.load(Ordering::Acquire).wrapping_sub(read) < self.channels {
            return false;
        }
        for channel in 0..self.channels {
            let slot = &self.slots[read.wrapping_add(channel) % self.slots.len()];
            take(channel, f32::from_bits(slot.load(Ordering::Relaxed)));
        }
        self.read
            .store(read.wrapping_add(self.channels), Ordering::Release);
        true
    }
}

const STREAM_CONTINUES: [cpal::ErrorKind; 3] = [
    cpal::ErrorKind::DeviceChanged,
    cpal::ErrorKind::Xrun,
    cpal::ErrorKind::RealtimeDenied,
];

const STREAM_FAILURES: [cpal::ErrorKind; 11] = [
    cpal::ErrorKind::DeviceBusy,
    cpal::ErrorKind::DeviceNotAvailable,
    cpal::ErrorKind::HostUnavailable,
    cpal::ErrorKind::InvalidInput,
    cpal::ErrorKind::PermissionDenied,
    cpal::ErrorKind::ResourceExhausted,
    cpal::ErrorKind::StreamInvalidated,
    cpal::ErrorKind::UnsupportedConfig,
    cpal::ErrorKind::UnsupportedOperation,
    cpal::ErrorKind::BackendError,
    cpal::ErrorKind::Other,
];

fn record_failure(failure: &AtomicU8, kind: cpal::ErrorKind) {
    if STREAM_CONTINUES.contains(&kind) {
        return;
    }
    let code = STREAM_FAILURES
        .iter()
        .position(|known| *known == kind)
        .unwrap_or(STREAM_FAILURES.len() - 1) as u8
        + 1;
    let _ = failure.compare_exchange(0, code, Ordering::AcqRel, Ordering::Acquire);
}

struct Output {
    ring: Arc<Ring>,
    rate: u32,
    channels: usize,
    format: DeviceFormat,
    alive: Arc<AtomicBool>,
    failure: Arc<AtomicU8>,
    progress: (usize, Instant),
}

impl Output {
    fn lost(&mut self, now: Instant) -> Option<String> {
        let code = self.failure.swap(0, Ordering::AcqRel);
        if code != 0 {
            let kind = STREAM_FAILURES
                .get(usize::from(code) - 1)
                .copied()
                .unwrap_or(cpal::ErrorKind::Other);
            return Some(kind.to_string());
        }
        let consumed = self.ring.consumed();
        if consumed != self.progress.0 || self.ring.len() == 0 {
            self.progress = (consumed, now);
            return None;
        }
        (now.saturating_duration_since(self.progress.1) > STALLED)
            .then(|| "it stopped taking samples".into())
    }
}

fn open_output() -> Result<(Output, String), String> {
    let (ready, opened) = mpsc::channel();
    let alive = Arc::new(AtomicBool::new(true));
    let failure = Arc::new(AtomicU8::new(0));
    let (thread_alive, thread_failure) = (Arc::clone(&alive), Arc::clone(&failure));
    thread::Builder::new()
        .name("map audio output".into())
        .spawn_carried(move || {
            let result = (|| {
                let host = cpal::default_host();
                let device = host
                    .default_output_device()
                    .ok_or("No audio output device")?;
                let name = device
                    .description()
                    .map(|description| description.name().to_owned())
                    .unwrap_or_else(|_| "audio output".into());
                let config = device
                    .default_output_config()
                    .map_err(|error| error.to_string())?;
                let channels = usize::from(config.channels());
                let rate = config.sample_rate();
                let format = config.sample_format();
                let config = config.config();
                let ring = Arc::new(Ring::new(rate as usize, channels));
                let error = move |error: cpal::Error| record_failure(&thread_failure, error.kind());
                let stream = match format {
                    cpal::SampleFormat::F32 => {
                        let ring = Arc::clone(&ring);
                        device.build_output_stream(
                            config,
                            move |out: &mut [f32], _| drain(&ring, out, |v| v),
                            error,
                            None,
                        )
                    }
                    cpal::SampleFormat::I16 => {
                        let ring = Arc::clone(&ring);
                        device.build_output_stream(
                            config,
                            move |out: &mut [i16], _| drain(&ring, out, |v| (v * 32_768.0) as i16),
                            error,
                            None,
                        )
                    }
                    other => return Err(format!("Unsupported output sample format {other}")),
                }
                .map_err(|error| error.to_string())?;
                stream.play().map_err(|error| error.to_string())?;
                let format = match format {
                    cpal::SampleFormat::I16 => DeviceFormat::S16,
                    _ => DeviceFormat::F32,
                };
                Ok((stream, ring, name, rate, channels, format))
            })();
            match result {
                Ok((stream, ring, name, rate, channels, format)) => {
                    let _ = ready.send(Ok((ring, name, rate, channels, format)));
                    while thread_alive.load(Ordering::Acquire) {
                        thread::park_timeout(Duration::from_millis(500));
                    }
                    drop(stream);
                }
                Err(error) => {
                    let _ = ready.send(Err(error));
                }
            }
        })
        .map_err(|error| error.to_string())?;
    let (ring, name, rate, channels, format) = opened
        .recv()
        .map_err(|_| "The audio output thread ended".to_owned())??;
    Ok((
        Output {
            ring,
            rate,
            channels,
            format,
            alive,
            failure,
            progress: (0, Instant::now()),
        },
        name,
    ))
}

fn open_device(status: &Mutex<Status>) -> Option<(Output, Stream)> {
    let opened = open_output().and_then(|(device, name)| {
        match Stream::new(device.rate, device.channels, device.format) {
            Some(stream) => Ok((device, stream, name)),
            None => Err(format!(
                "Unsupported output channel count {} at {} Hz",
                device.channels, device.rate
            )),
        }
    });
    let mut status = status.lock().unwrap_or_else(|e| e.into_inner());
    match opened {
        Ok((device, stream, name)) => {
            status.device = Some(Ok(name));
            Some((device, stream))
        }
        Err(error) => {
            status.device = Some(Err(error));
            None
        }
    }
}

fn drain<T: Default + Copy>(ring: &Ring, out: &mut [T], convert: impl Fn(f32) -> T) {
    for frame in out.chunks_mut(ring.channels) {
        let complete = frame.len() == ring.channels
            && ring.pop_frame(|channel, value| frame[channel] = convert(value));
        if !complete {
            frame.fill(T::default());
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

struct Loaded {
    generation: u64,
    load: Load,
    payload: Result<Payload, String>,
}

enum Payload {
    Sound(Arc<Samples>),
    Track(crate::music::TrackStream),
}

type Request = (u64, Arc<Source>, Load);

pub(crate) fn sample_path(source: &Source, load: Load) -> Result<&str, String> {
    match load {
        Load::Sound(sound) => source
            .content
            .catalog
            .sounds
            .get(sound)
            .map(|sound| sound.path.as_str())
            .ok_or_else(|| "unknown sound".into()),
        Load::Track(track) => source
            .content
            .music
            .tracks
            .get(track)
            .map(|track| track.path.as_str())
            .ok_or_else(|| "unknown track".into()),
    }
}

fn read_bytes(source: &Source, load: Load) -> Result<content_reader_layers::content::Bytes, String> {
    let path = sample_path(source, load)?;
    source
        .layers
        .read_uncovered(path)
        .map_err(|error| format!("{path}: {error}"))
}

pub(crate) fn decode(source: &Source, load: Load) -> Result<Arc<Samples>, String> {
    let path = sample_path(source, load)?;
    let pcm = content_reader_sound::wav::decode(&read_bytes(source, load)?)
        .map_err(|why| format!("{path}: {why}"))?;
    Ok(Arc::new(Samples::from_pcm(&pcm)))
}

pub(crate) fn open_track(source: &Source, load: Load) -> Result<content_reader_sound::ogg::Stream, String> {
    let path = sample_path(source, load)?;
    content_reader_sound::ogg::open(read_bytes(source, load)?).map_err(|why| format!("{path}: {why}"))
}

pub(crate) fn guarded<T>(
    source: &Source,
    load: Load,
    decode: impl FnOnce(&Source, Load) -> Result<T, String>,
) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode(source, load))).unwrap_or_else(
        |_| {
            let path = sample_path(source, load).unwrap_or("an unknown file");
            Err(format!("{path}: the decoder failed on it"))
        },
    )
}

fn loader(name: &str, requests: mpsc::Receiver<Request>, done: mpsc::Sender<Loaded>) {
    let _ = thread::Builder::new().name(name.into()).spawn_carried(move || {
        for (generation, source, load) in requests {
            let send = |payload| {
                done.send(Loaded {
                    generation,
                    load,
                    payload,
                })
                .is_ok()
            };
            let sent = match load {
                Load::Sound(_) => send(guarded(&source, load, decode).map(Payload::Sound)),
                Load::Track(_) => match guarded(&source, load, open_track) {
                    Err(why) => send(Err(why)),
                    Ok(mut stream) => {
                        let (sender, track) = crate::music::track_channel();
                        let sent = send(Ok(Payload::Track(track)));
                        if sent {
                            let path = sample_path(&source, load).unwrap_or("an unknown file");
                            crate::music::pump(
                                || {
                                    stream
                                        .decode_next()
                                        .map(|packet| {
                                            packet.map(|packet| {
                                                (packet.rate, packet.channels, packet.samples)
                                            })
                                        })
                                        .map_err(|why| format!("{path}: {why}"))
                                },
                                &sender,
                            );
                        }
                        sent
                    }
                },
            };
            if !sent {
                break;
            }
        }
    });
}

fn control(commands: mpsc::Receiver<Command>, status: Arc<Mutex<Status>>, silent: bool) {
    let log_path = std::env::var_os("BATTLEPLAN_VIEWER_AUDIO_LOG").or_else(|| std::env::var_os("HOI4_VIEWER_AUDIO_LOG")).map(PathBuf::from);
    let log_file = log_path.and_then(|path| std::fs::File::create(path).ok());
    Control::new(status, log_file, silent).run(commands);
}

struct Control {
    wake: Option<Wake>,
    engine: Engine,
    status: Arc<Mutex<Status>>,
    log_file: Option<std::fs::File>,
    source: Option<Arc<Source>>,
    owner: u64,
    silent: bool,
    output: Option<(Output, Stream)>,
    next_open: Instant,
    generation: u64,
    done: mpsc::Receiver<Loaded>,
    sounds: mpsc::Sender<Request>,
    music: mpsc::Sender<Request>,
    block: Vec<i16>,
    handed_off: Option<(u64, Option<usize>)>,
    last_evict: Instant,
    unplayed: Instant,
}

impl Control {
    fn new(status: Arc<Mutex<Status>>, log_file: Option<std::fs::File>, silent: bool) -> Self {
        let mut engine = Engine::new(0x4155_4449);
        if log_file.is_some() {
            engine.log = Some(Vec::new());
        }
        let (done_send, done) = mpsc::channel::<Loaded>();
        let (sounds, sound_requests) = mpsc::channel::<Request>();
        let (music, music_requests) = mpsc::channel::<Request>();
        loader("map audio loader", sound_requests, done_send.clone());
        loader("map music loader", music_requests, done_send);
        Self {
            engine,
            wake: None,
            status,
            log_file,
            source: None,
            owner: 0,
            silent,
            output: None,
            next_open: Instant::now(),
            generation: 0,
            done,
            sounds,
            music,
            block: vec![0i16; STEP * 2],
            handed_off: None,
            last_evict: Instant::now(),
            unplayed: Instant::now(),
        }
    }

    fn run(mut self, commands: mpsc::Receiver<Command>) {
        loop {
            let wait = if self.output.is_some() {
                Duration::from_millis(5)
            } else {
                Duration::from_millis(50)
            };
            match commands.recv_timeout(wait) {
                Ok(command) => {
                    self.apply(command);
                    for command in commands.try_iter() {
                        self.apply(command);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            self.step();
        }
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Claim(token) => {
                self.wake = None;
                self.owner = token;
                self.engine.silence();
            }
            Command::Release(token) => {
                if self.owner == token {
                    self.wake = None;
                    self.owner = 0;
                    self.engine.silence();
                }
            }
            Command::Mix(token, mix) => {
                if token == self.owner {
                    self.engine.set_mix(mix);
                }
            }
            Command::Frame(token, source, frame, wake) => {
                if token != self.owner {
                    return;
                }
                if self
                    .source
                    .as_ref()
                    .is_none_or(|known| !Arc::ptr_eq(known, &source))
                {
                    self.engine.set_content(Some(Arc::clone(&source.content)));
                    self.status
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .notes
                        .clear();
                    self.source = Some(source);
                    self.generation += 1;
                }
                self.wake = wake;
                self.engine.frame(&frame);
            }
        }
    }

    fn step(&mut self) {
        self.step_at(Instant::now());
    }

    fn step_at(&mut self, now: Instant) {
        self.receive_loads();
        self.request_loads();
        self.keep_device(now);
        self.mix(now);
        if self.wake.is_some()
            && self.engine.needs_frame()
            && let Some(wake) = self.wake.take()
        {
            wake();
        }
        self.write_log(now);
    }

    fn receive_loads(&mut self) {
        while let Ok(loaded) = self.done.try_recv() {
            self.handle_loaded(loaded);
        }
    }

    fn handle_loaded(&mut self, loaded: Loaded) {
        {
            if loaded.generation != self.generation {
                return;
            }
            let payload = match loaded.payload {
                Ok(payload) => Some(payload),
                Err(error) => {
                    self.engine.notes.note(format!("Unreadable sample {error}"));
                    None
                }
            };
            match (loaded.load, payload) {
                (Load::Sound(sound), Some(Payload::Sound(samples))) => {
                    self.engine.provide(sound, Some(samples))
                }
                (Load::Track(track), Some(Payload::Track(stream))) => {
                    self.engine.provide_track(track, Some(stream))
                }
                (Load::Sound(sound), _) => self.engine.provide(sound, None),
                (Load::Track(track), _) => self.engine.provide_track(track, None),
            }
        }
    }

    fn request_loads(&mut self) {
        let Some(source) = &self.source else {
            return;
        };
        for load in self.engine.take_wanted() {
            let queue = match load {
                Load::Sound(_) => &self.sounds,
                Load::Track(_) => &self.music,
            };
            if queue
                .send((self.generation, Arc::clone(source), load))
                .is_err()
            {
                let path = sample_path(source, load).unwrap_or("an unknown file");
                self.engine
                    .notes
                    .note(format!("Unreadable sample {path}: its loader stopped"));
                match load {
                    Load::Sound(sound) => self.engine.provide(sound, None),
                    Load::Track(track) => self.engine.provide_track(track, None),
                }
            }
        }
    }

    fn keep_device(&mut self, now: Instant) {
        if self.silent {
            return;
        }
        if let Some((device, _)) = &mut self.output
            && let Some(why) = device.lost(now)
        {
            self.output = None;
            self.status.lock().unwrap_or_else(|e| e.into_inner()).device =
                Some(Err(format!("The audio output was lost ({why})")));
            self.next_open = now + REOPEN;
            self.unplayed = now;
        }
        if self.output.is_none() && self.owner != 0 && now >= self.next_open {
            self.output = open_device(&self.status);
            let opened = Instant::now();
            if self.output.is_none() {
                self.next_open = opened + REOPEN;
            }
            self.unplayed = opened;
        }
    }

    fn mix(&mut self, now: Instant) {
        if self.output.is_none() {
            let due = (now.saturating_duration_since(self.unplayed).as_secs_f64() * f64::from(RATE))
                as usize;
            let steps = due / STEP;
            for _ in 0..steps.min(CATCH_UP) {
                self.hand_off_music();
                self.engine.render(&mut self.block);
            }
            if steps > CATCH_UP {
                self.unplayed = Instant::now();
            } else {
                self.unplayed += Duration::from_secs_f64((steps * STEP) as f64 / f64::from(RATE));
            }
            return;
        }
        for _ in 0..CATCH_UP {
            self.hand_off_music();
            let Some((device, stream)) = &mut self.output else {
                return;
            };
            let target = (device.rate as f64 * AHEAD.as_secs_f64()) as usize * device.channels;
            if device.ring.len() >= target {
                break;
            }
            self.engine.render(&mut self.block);
            stream.put(&self.block);
            let queue = stream.queue();
            let frames = queue.len() / device.channels;
            let samples = frames * device.channels;
            let converted = queue.make_contiguous();
            for frame in converted[..samples].chunks_exact(device.channels) {
                if !device.ring.push_frame(frame) {
                    break;
                }
            }
            queue.drain(..samples);
        }
    }

    fn hand_off_music(&mut self) {
        let drawn = (self.engine.music_draws(), self.engine.music_playing());
        if !self.engine.music_waiting() || self.handed_off == Some(drawn) {
            return;
        }
        self.handed_off = Some(drawn);
        self.request_loads();
        let deadline = Instant::now() + MUSIC_OPEN;
        loop {
            self.engine
                .prime_music(deadline.saturating_duration_since(Instant::now()));
            if !self.engine.music_waiting() {
                return;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.done.recv_timeout(left) {
                Ok(loaded) => self.handle_loaded(loaded),
                Err(_) => return,
            }
        }
    }

    fn write_log(&mut self, now: Instant) {
        if let (Some(log), Some(file)) = (&mut self.engine.log, &mut self.log_file) {
            for started in log.drain(..) {
                let _ = writeln!(
                    file,
                    "{:.3}\t{}\t{}\t{}\t{:.1},{:.1},{:.1}\t{:.3}",
                    started.at,
                    started
                        .hour
                        .map_or_else(String::new, |hour| hour.to_string()),
                    started.effect,
                    started.sound,
                    started.position[0],
                    started.position[1],
                    started.position[2],
                    started.gain
                );
            }
        }
        if now.saturating_duration_since(self.last_evict) <= Duration::from_secs(5) {
            return;
        }
        self.last_evict = now;
        self.engine.evict(RETAIN.as_secs_f64());
        let notes = self.engine.notes.lines();
        if let Some(file) = &mut self.log_file {
            let (hour, emitters, instances, voices) = self.engine.census();
            let device = match &self.status.lock().unwrap_or_else(|e| e.into_inner()).device {
                Some(Ok(name)) => name.clone(),
                Some(Err(error)) => format!("none ({error})"),
                None => "not opened".into(),
            };
            let _ = writeln!(
                file,
                "# hour {} emitters {emitters} instances {instances} voices {voices} device {device}; {}",
                hour.map_or_else(String::new, |hour| hour.to_string()),
                notes.join("; ")
            );
        }
        self.status.lock().unwrap_or_else(|e| e.into_inner()).notes = notes;
    }
}

#[cfg(test)]
mod tests;
