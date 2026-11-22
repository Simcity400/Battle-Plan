use job_priority::SpawnCarried as _;
use std::{
    borrow::Cow,
    ffi::OsString,
    fs::File,
    io::{self, BufWriter, Write},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const QUEUED_FRAMES: usize = 4096;

#[derive(Clone)]
pub struct Frame {
    pub t_ns: u128,
    pub epoch_us: Option<u128>,
    pub playback: Option<(replay_data::Hour, f32)>,
    pub drawn: bool,
    pub view: Cow<'static, str>,
}

impl Frame {
    fn write(self, out: &mut impl Write) -> io::Result<()> {
        match self.epoch_us {
            Some(epoch_us) => write!(out, "{} {epoch_us} ", self.t_ns)?,
            None => write!(out, "{} - ", self.t_ns)?,
        }
        let drawn = u8::from(self.drawn);
        match self.playback {
            Some((hour, fraction)) => writeln!(out, "{hour} {fraction} {drawn} {}", self.view),
            None => writeln!(out, "- - {drawn} {}", self.view),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Pass<'a> {
    recorder: &'a Recorder,
    t_ns: u128,
    epoch_us: Option<u128>,
}

impl Pass<'_> {
    pub fn record(
        self,
        playback: Option<(replay_data::Hour, f32)>,
        drawn: bool,
        view: impl Into<Cow<'static, str>>,
    ) {
        self.recorder.record(Frame {
            t_ns: self.t_ns,
            epoch_us: self.epoch_us,
            playback,
            drawn,
            view: view.into(),
        });
    }
}

enum Message {
    Frame(Frame),
    Dropped(u64),
    Flush(mpsc::Sender<()>),
}

pub struct Recorder {
    started: Instant,
    queue: SyncSender<Message>,
    dropped: AtomicU64,
}

impl Recorder {
    fn start<W: Write + Send + 'static>(
        path: Option<OsString>,
        started: Instant,
        capacity: usize,
        open: impl FnOnce(OsString) -> io::Result<W> + Send + 'static,
    ) -> Option<Self> {
        let path = path?;
        let (queue, frames) = mpsc::sync_channel(capacity);
        std::thread::Builder::new()
            .name("viewer frame writer".into())
            .spawn_carried(move || {
                let mut out = match open(path) {
                    Ok(out) => out,
                    Err(error) => {
                        eprintln!("BATTLEPLAN_VIEWER_FRAMES: {error}");
                        return;
                    }
                };
                for message in frames {
                    match message {
                        Message::Frame(frame) => {
                            let _ = frame.write(&mut out);
                        }
                        Message::Dropped(count) => {
                            let _ = writeln!(out, "# {count} frames dropped");
                        }
                        Message::Flush(done) => {
                            let _ = out.flush();
                            let _ = done.send(());
                        }
                    }
                }
                let _ = out.flush();
            })
            .ok()
            .map(|_| Self {
                started,
                queue,
                dropped: AtomicU64::new(0),
            })
    }

    pub fn pass(&self) -> Pass<'_> {
        Pass {
            recorder: self,
            t_ns: self.started.elapsed().as_nanos(),
            epoch_us: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_micros()),
        }
    }

    pub fn record(&self, frame: Frame) {
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            match self.queue.try_send(Message::Dropped(dropped)) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    self.dropped.fetch_add(dropped + 1, Ordering::Relaxed);
                    return;
                }
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
        if let Err(TrySendError::Full(_)) = self.queue.try_send(Message::Frame(frame)) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn flush(&self) {
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 && self.queue.send(Message::Dropped(dropped)).is_err() {
            return;
        }
        let (done, written) = mpsc::channel();
        if self.queue.send(Message::Flush(done)).is_ok() {
            let _ = written.recv();
        }
    }
}

static RECORDER: OnceLock<Option<Recorder>> = OnceLock::new();

pub fn initialize(started: Instant) {
    RECORDER.get_or_init(|| {
        Recorder::start(
            std::env::var_os("BATTLEPLAN_VIEWER_FRAMES").or_else(|| std::env::var_os("HOI4_VIEWER_FRAMES")),
            started,
            QUEUED_FRAMES,
            |path| File::create(path).map(BufWriter::new),
        )
    });
}

pub fn recorder() -> Option<&'static Recorder> {
    RECORDER.get().and_then(Option::as_ref)
}

pub fn flush() {
    if let Some(recorder) = recorder() {
        recorder.flush();
    }
}

#[cfg(test)]
mod tests;
