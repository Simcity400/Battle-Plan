use super::*;
use std::sync::{Arc, Mutex, mpsc::Receiver};

#[test]
fn lines_preserve_timestamps_playback_and_views_and_mark_missing_values() {
    let mut out = Vec::new();
    for frame in [
        Frame {
            t_ns: 12_345_678_901,
            epoch_us: Some(1_800_000_000_000_001),
            playback: Some((60_832_633, 0.25)),
            drawn: true,
            view: "campaign".into(),
        },
        Frame {
            t_ns: 12_345_678_902,
            epoch_us: Some(0),
            playback: Some((-1, 0.0)),
            drawn: false,
            view: "campaign".into(),
        },
        Frame {
            t_ns: 12_345_678_903,
            epoch_us: Some(1_800_000_000_000_003),
            playback: None,
            drawn: false,
            view: "campaign".into(),
        },
        Frame {
            t_ns: 12_345_678_904,
            epoch_us: None,
            playback: Some((60_832_634, 0.75)),
            drawn: true,
            view: "campaign".into(),
        },
        Frame {
            t_ns: 12_345_678_905,
            epoch_us: None,
            playback: None,
            drawn: false,
            view: "-".into(),
        },
    ] {
        frame.write(&mut out).unwrap();
    }
    assert_eq!(
        String::from_utf8(out).unwrap(),
        concat!(
            "12345678901 1800000000000001 60832633 0.25 1 campaign\n",
            "12345678902 0 -1 0 0 campaign\n",
            "12345678903 1800000000000003 - - 0 campaign\n",
            "12345678904 - 60832634 0.75 1 campaign\n",
            "12345678905 - - - 0 -\n",
        )
    );
}

#[test]
fn a_pass_queues_each_views_shown_playback_and_draw_result_with_the_same_timestamps() {
    let (queue, frames) = mpsc::sync_channel(3);
    let recorder = Recorder {
        started: Instant::now(),
        queue,
        dropped: AtomicU64::new(0),
    };
    let pass = Pass {
        recorder: &recorder,
        t_ns: 12_345_678_901,
        epoch_us: Some(1_800_000_000_000_001),
    };
    pass.record(Some((60_832_633, 0.25)), true, String::from("map-a"));
    pass.record(Some((60_832_634, 0.75)), false, String::from("map-b"));
    pass.record(None, false, String::from("map-loading"));
    let mut out = Vec::new();
    for message in frames.try_iter() {
        let Message::Frame(frame) = message else {
            panic!("a pass must queue a frame per view");
        };
        frame.write(&mut out).unwrap();
    }
    assert_eq!(
        String::from_utf8(out).unwrap(),
        concat!(
            "12345678901 1800000000000001 60832633 0.25 1 map-a\n",
            "12345678901 1800000000000001 60832634 0.75 0 map-b\n",
            "12345678901 1800000000000001 - - 0 map-loading\n",
        )
    );
}

#[test]
fn an_unset_flag_creates_no_recorder_and_never_opens_output() {
    let recorder = Recorder::start(None, Instant::now(), 1, |_| -> io::Result<Vec<u8>> {
        panic!("an unset frame flag must not open a file");
    });
    assert!(recorder.is_none());
}

struct Stalled {
    bytes: Arc<Mutex<Vec<u8>>>,
    release: Option<Receiver<()>>,
    started: mpsc::Sender<()>,
    lines: mpsc::Sender<()>,
    flushed: Arc<AtomicU64>,
}

impl Write for Stalled {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(release) = self.release.take() {
            self.started.send(()).unwrap();
            release.recv().unwrap();
        }
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        for _ in bytes.iter().filter(|&&byte| byte == b'\n') {
            self.lines.send(()).unwrap();
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

struct Run {
    recorder: Recorder,
    bytes: Arc<Mutex<Vec<u8>>>,
    release: mpsc::Sender<()>,
    lines: Receiver<()>,
    flushed: Arc<AtomicU64>,
}

fn frame(t_ns: u128) -> Frame {
    Frame {
        t_ns,
        epoch_us: Some(123_456),
        playback: Some((42, 0.5)),
        drawn: true,
        view: "campaign".into(),
    }
}

fn stalled() -> Run {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let flushed = Arc::new(AtomicU64::new(0));
    let (release, released) = mpsc::channel();
    let (started, write_started) = mpsc::channel();
    let (line_written, lines) = mpsc::channel();
    let out = Stalled {
        bytes: Arc::clone(&bytes),
        release: Some(released),
        started,
        lines: line_written,
        flushed: Arc::clone(&flushed),
    };
    let recorder = Recorder::start(Some("synthetic".into()), Instant::now(), 2, move |_| {
        Ok(out)
    })
    .unwrap();
    recorder.record(frame(0));
    write_started.recv().unwrap();
    for t_ns in 1..=5 {
        recorder.record(frame(t_ns));
    }
    Run {
        recorder,
        bytes,
        release,
        lines,
        flushed,
    }
}

fn text(bytes: &Mutex<Vec<u8>>) -> String {
    String::from_utf8(bytes.lock().unwrap().clone()).unwrap()
}

const QUEUED: &str = concat!(
    "0 123456 42 0.5 1 campaign\n",
    "1 123456 42 0.5 1 campaign\n",
    "2 123456 42 0.5 1 campaign\n",
);
const DROPPED: &str = "# 3 frames dropped\n";

#[test]
fn a_stalled_writer_drops_instead_of_blocking_and_reports_before_the_next_frame() {
    let run = stalled();
    assert_eq!(run.recorder.dropped.load(Ordering::Relaxed), 3);
    run.release.send(()).unwrap();
    for _ in 0..3 {
        run.lines.recv().unwrap();
    }
    run.recorder.record(frame(6));
    run.recorder.flush();
    assert_eq!(
        text(&run.bytes),
        format!("{QUEUED}{DROPPED}6 123456 42 0.5 1 campaign\n")
    );
}

#[test]
fn exit_flush_includes_pending_drop_counts_and_flushes_the_output() {
    let run = stalled();
    run.release.send(()).unwrap();
    run.recorder.flush();
    assert_eq!(text(&run.bytes), format!("{QUEUED}{DROPPED}"));
    assert!(run.flushed.load(Ordering::Relaxed) >= 1);
}
