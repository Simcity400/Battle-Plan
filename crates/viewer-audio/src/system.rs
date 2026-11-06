//! The process's audio threads and output device.
//!
//! One [`System`] serves every map view of a process. Exactly one view owns
//! it at a time ([`Owner::claim`]); frames from other views are dropped
//! before they are sent. The control thread applies the owner's frames to
//! the [`Engine`], asks the loader thread for samples, and renders about
//! [`AHEAD`] of audio into the queue the device callback drains. Decoded
//! samples stay while voices use them and for [`RETAIN`] after their last
//! use.

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

/// Audio rendered ahead of the device.
const AHEAD: Duration = Duration::from_millis(60);
/// How long an unused decoded sample stays.
const RETAIN: Duration = Duration::from_secs(60);
/// Frames rendered per control step: one callback of the game's mixer.
const STEP: usize = CALLBACK_FRAMES;
/// Steps one control iteration renders at most without a device (116 ms).
const CATCH_UP: usize = 5;
/// The wait before the default device is opened again after it failed to
/// open or was lost.
const REOPEN: Duration = Duration::from_secs(3);
/// A device that takes no samples for this long while some wait is lost.
const STALLED: Duration = Duration::from_secs(1);
/// The longest a song's handoff waits for the next track to open.
const MUSIC_OPEN: Duration = Duration::from_secs(2);

/// One content stack's audio: its definitions and the layers its samples
/// are read from.
pub struct Source {
    pub content: Arc<Content>,
    pub layers: installed_content_core::ResolvedLayers,
}

enum Command {
    Frame(u64, Arc<Source>, Frame),
    /// The owner's volumes and mute, applied at once rather than with its
    /// next frame.
    Mix(u64, Mix),
    Claim(u64),
    Release(u64),
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    /// The output device's name, or why none is open.
    pub device: Option<Result<String, String>>,
    /// Named, counted refusals: missing effects, unreadable samples.
    pub notes: Vec<String>,
}

pub struct System {
    commands: Mutex<mpsc::Sender<Command>>,
    owner: AtomicU64,
    next: AtomicU64,
    status: Arc<Mutex<Status>>,
}

/// A view's handle on the process audio.
pub struct Owner {
    token: u64,
    system: Arc<System>,
    last: Option<Instant>,
}

/// Set before the first [`System::shared`]: the process never opens an
/// output device and only logs (window captures with `BATTLE_PLAN_VIEWER_AUDIO_LOG`).
static SILENT: AtomicBool = AtomicBool::new(false);

impl System {
    /// Makes this process's audio a silent trace: voices start, advance and
    /// log, but no device opens. It must precede the first [`System::shared`].
    pub fn silent() {
        SILENT.store(true, Ordering::Release);
    }

    /// The process's audio, started on first use.
    pub fn shared() -> Arc<Self> {
        static SHARED: std::sync::OnceLock<Arc<System>> = std::sync::OnceLock::new();
        Arc::clone(SHARED.get_or_init(Self::start))
    }

    fn start() -> Arc<Self> {
        Self::spawn(SILENT.load(Ordering::Acquire))
    }

    /// A system whose control thread never opens a device when `silent`.
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
    /// Makes this view the one that sounds; the previous owner falls silent.
    pub fn claim(&mut self) {
        if self.system.owner.swap(self.token, Ordering::AcqRel) != self.token {
            self.last = None;
            self.system.send(Command::Claim(self.token));
        }
    }

    pub fn owns(&self) -> bool {
        self.system.owner.load(Ordering::Acquire) == self.token
    }

    /// Gives the audio up when this view owns it: everything it played,
    /// music included, stops. A later [`Owner::claim`] takes it again.
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

    /// Applies volumes and mute now, between frames: a view that stops
    /// drawing still silences what it plays.
    pub fn set_mix(&self, mix: Mix) {
        if self.owns() {
            self.system.send(Command::Mix(self.token, mix));
        }
    }

    /// Sends one presented frame when this view owns the audio.
    pub fn frame(&mut self, source: &Arc<Source>, mut frame: Frame) {
        if !self.owns() {
            return;
        }
        let now = Instant::now();
        frame.elapsed = self
            .last
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        self.last = Some(now);
        self.system
            .send(Command::Frame(self.token, Arc::clone(source), frame));
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

/// A single-producer, single-consumer queue of whole frames: the control
/// thread pushes and the device callback pops, and neither ever waits for
/// the other. Both ends move a frame's samples at once, so the channels
/// never shift, whatever the callback interrupts or finds missing.
struct Ring {
    slots: Box<[AtomicU32]>,
    channels: usize,
    /// Samples ever popped and pushed, always whole frames; their difference
    /// is the length.
    read: AtomicUsize,
    write: AtomicUsize,
}

impl Ring {
    /// Room for `frames` frames of `channels` samples.
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

    /// Queued samples, a whole number of frames.
    fn len(&self) -> usize {
        let read = self.read.load(Ordering::Acquire);
        self.write.load(Ordering::Acquire).wrapping_sub(read)
    }

    /// Samples the consumer has taken so far.
    fn consumed(&self) -> usize {
        self.read.load(Ordering::Acquire)
    }

    /// The producer side: one frame of exactly `channels` samples, published
    /// at once; `false` when the queue has no room for it.
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

    /// The consumer side: hands one whole frame to `take` (channel, sample);
    /// `false`, taking nothing, when no whole frame is queued.
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

/// Stream errors that leave the stream playing: a reroute to the new default
/// device, an underrun and a refused real-time priority.
const STREAM_CONTINUES: [cpal::ErrorKind; 3] = [
    cpal::ErrorKind::DeviceChanged,
    cpal::ErrorKind::Xrun,
    cpal::ErrorKind::RealtimeDenied,
];

/// The stream error kinds a failure code names, by code minus one; a kind
/// not listed is reported as [`cpal::ErrorKind::Other`].
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

/// The stream error callback: it runs on the device's thread, so it only
/// records the first failure's kind in an atomic, without locking or
/// allocating. Errors after which the stream plays on are ignored.
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

/// The device side: a queue of interleaved frames at the device's rate and
/// channel count, and the stream's reported failure.
struct Output {
    ring: Arc<Ring>,
    rate: u32,
    channels: usize,
    format: DeviceFormat,
    alive: Arc<AtomicBool>,
    /// The first stream failure's code ([`record_failure`]); 0 for none.
    failure: Arc<AtomicU8>,
    /// The consumed count last seen and when it last moved.
    progress: (usize, Instant),
}

impl Output {
    /// Why the device can no longer play: a failure the stream reported, or
    /// samples waiting that nothing has taken for [`STALLED`].
    fn lost(&mut self) -> Option<String> {
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
            self.progress = (consumed, Instant::now());
            return None;
        }
        (self.progress.1.elapsed() > STALLED).then(|| "it stopped taking samples".into())
    }
}

fn open_output() -> Result<(Output, String), String> {
    let (ready, opened) = mpsc::channel();
    let alive = Arc::new(AtomicBool::new(true));
    let failure = Arc::new(AtomicU8::new(0));
    let (thread_alive, thread_failure) = (Arc::clone(&alive), Arc::clone(&failure));
    // The stream stays on its own thread: it is not sent between threads.
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
                // A second of audio: far more than is ever rendered ahead.
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
                            // The stream's 16-bit samples, held as steps of 1/32768.
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

/// Opens the default device, recording the outcome in the status. The
/// status lock is not held while the device opens, which can take seconds.
fn open_device(status: &Mutex<Status>) -> Option<(Output, Stream)> {
    let opened = open_output().and_then(|(device, name)| {
        // The mixer's stereo converts to the device as SDL's stream does.
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

/// The device callback: it never blocks, and takes whole frames only; a
/// frame not yet rendered, or a partial frame at the end of `out`, is
/// silence.
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

/// What a loader hands over: a sound's decoded samples, or a track's stream
/// that keeps filling as the music loader decodes it.
enum Payload {
    Sound(Arc<Samples>),
    Track(crate::music::TrackStream),
}

type Request = (u64, Arc<Source>, Load);

/// The content path of a requested sample file.
fn sample_path(source: &Source, load: Load) -> Result<&str, String> {
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

/// Reads a requested file's bytes through the content stack.
fn read_bytes(source: &Source, load: Load) -> Result<Vec<u8>, String> {
    let path = sample_path(source, load)?;
    source
        .layers
        .read_uncovered(path)
        .map_err(|error| format!("{path}: {error}"))
}

/// Reads and decodes one sound's WAVE file whole.
fn decode(source: &Source, load: Load) -> Result<Arc<Samples>, String> {
    let path = sample_path(source, load)?;
    let pcm = installed_content_sound::wav::decode(&read_bytes(source, load)?)
        .map_err(|why| format!("{path}: {why}"))?;
    Ok(Arc::new(Samples::from_pcm(&pcm)))
}

/// Opens one music track to stream it.
fn open_track(source: &Source, load: Load) -> Result<installed_content_sound::ogg::Stream, String> {
    let path = sample_path(source, load)?;
    installed_content_sound::ogg::open(read_bytes(source, load)?).map_err(|why| format!("{path}: {why}"))
}

/// Runs `decode` on one file; a decoder that panics refuses that file by
/// name and leaves the loader running.
fn guarded<T>(
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

/// A loader thread; sounds and music load on separate ones, so a long track
/// never delays a sound effect. A sound decodes whole; a track is handed
/// over as a stream once it opens, then decoded into it a packet at a time
/// until it ends or the player lets go of it, before the next request.
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
    let log_path = std::env::var_os("BATTLE_PLAN_VIEWER_AUDIO_LOG").or_else(|| std::env::var_os("HOI4_VIEWER_AUDIO_LOG")).map(PathBuf::from);
    let log_file = log_path.and_then(|path| std::fs::File::create(path).ok());
    Control::new(status, log_file, silent).run(commands);
}

/// The control thread's state: the engine, the owner's content, the loaders
/// and the device.
struct Control {
    engine: Engine,
    status: Arc<Mutex<Status>>,
    log_file: Option<std::fs::File>,
    source: Option<Arc<Source>>,
    owner: u64,
    /// No device is ever opened (window captures).
    silent: bool,
    output: Option<(Output, Stream)>,
    /// When the default device may be opened (again).
    next_open: Instant,
    generation: u64,
    done: mpsc::Receiver<Loaded>,
    sounds: mpsc::Sender<Request>,
    music: mpsc::Sender<Request>,
    block: Vec<i16>,
    /// The draw count and track the last music handoff waited for.
    handed_off: Option<(u64, Option<usize>)>,
    last_evict: Instant,
    /// Without a device, voices still advance in time and finish.
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

    /// One command, in arrival order: a reset is never coalesced away.
    fn apply(&mut self, command: Command) {
        match command {
            Command::Claim(token) => {
                self.owner = token;
                self.engine.silence();
            }
            Command::Release(token) => {
                if self.owner == token {
                    self.owner = 0;
                    self.engine.silence();
                }
            }
            Command::Mix(token, mix) => {
                if token == self.owner {
                    self.engine.set_mix(mix);
                }
            }
            Command::Frame(token, source, frame) => {
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
                self.engine.frame(&frame);
            }
        }
    }

    /// Everything between commands: loads, the device, mixing and the log.
    fn step(&mut self) {
        self.receive_loads();
        self.request_loads();
        self.keep_device();
        self.mix();
        self.write_log();
    }

    fn receive_loads(&mut self) {
        while let Ok(loaded) = self.done.try_recv() {
            self.handle_loaded(loaded);
        }
    }

    fn handle_loaded(&mut self, loaded: Loaded) {
        {
            // Samples of a replaced content are dropped.
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

    /// Asks the loaders for what the engine wants. A loader that is gone
    /// refuses the file by name, so no voice waits for it forever.
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

    /// Opens the default device while a view owns the audio, and drops a
    /// device that failed: the status says why, mixing goes on by the clock
    /// and the default device is opened again after [`REOPEN`].
    fn keep_device(&mut self) {
        if self.silent {
            return;
        }
        if let Some((device, _)) = &mut self.output
            && let Some(why) = device.lost()
        {
            self.output = None;
            self.status.lock().unwrap_or_else(|e| e.into_inner()).device =
                Some(Err(format!("The audio output was lost ({why})")));
            self.next_open = Instant::now() + REOPEN;
            self.unplayed = Instant::now();
        }
        if self.output.is_none() && self.owner != 0 && Instant::now() >= self.next_open {
            self.output = open_device(&self.status);
            if self.output.is_none() {
                self.next_open = Instant::now() + REOPEN;
            }
            self.unplayed = Instant::now();
        }
    }

    fn mix(&mut self) {
        if self.output.is_none() {
            // Mixing that cannot keep up drops the backlog, as a device
            // underrun does, so the thread keeps taking frames.
            let due = (self.unplayed.elapsed().as_secs_f64() * f64::from(RATE)) as usize;
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
        // At most a bounded amount per pass: frames are taken between passes
        // even when mixing falls behind the device.
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

    /// A song drawn when the last one ended opens and decodes its first
    /// samples before the next callback mixes, as the game's player opens
    /// its next track at once, so the handoff adds no wait for a load.
    fn hand_off_music(&mut self) {
        // One wait per drawn track: a slow decoder then plays in when it can.
        let drawn = (self.engine.music_draws(), self.engine.music_playing());
        if !self.engine.music_waiting() || self.handed_off == Some(drawn) {
            return;
        }
        self.handed_off = Some(drawn);
        self.request_loads();
        let deadline = Instant::now() + MUSIC_OPEN;
        loop {
            // An opened track waits for its first samples; otherwise the
            // loader's answer is awaited.
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

    fn write_log(&mut self) {
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
        if self.last_evict.elapsed() <= Duration::from_secs(5) {
            return;
        }
        self.last_evict = Instant::now();
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
