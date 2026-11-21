use super::*;
use std::{
    io,
    sync::{Arc, Mutex, mpsc::Receiver},
};

struct Stalled {
    written: Arc<Mutex<Vec<u8>>>,
    release: Option<Receiver<()>>,
    started: mpsc::Sender<()>,
    lines: mpsc::Sender<()>,
}

impl Write for Stalled {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(release) = self.release.take() {
            let _ = self.started.send(());
            let _ = release.recv();
        }
        self.written.lock().unwrap().extend_from_slice(bytes);
        for _ in bytes.iter().filter(|&&byte| byte == b'\n') {
            let _ = self.lines.send(());
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Run {
    sink: Sink,
    written: Arc<Mutex<Vec<u8>>>,
    release: mpsc::Sender<()>,
    lines: Receiver<()>,
}

fn stalled_with_ten_behind() -> Run {
    let written = Arc::new(Mutex::new(Vec::new()));
    let (release, released) = mpsc::channel();
    let (started, write_started) = mpsc::channel();
    let (line_written, lines) = mpsc::channel();
    let out = Stalled {
        written: Arc::clone(&written),
        release: Some(released),
        started,
        lines: line_written,
    };
    let sink = Sink::start(out, 4).unwrap();
    sink.print("line 0".into()).unwrap();
    write_started.recv().unwrap();
    for line in 1..=10 {
        sink.print(format!("line {line}")).unwrap();
    }
    Run {
        sink,
        written,
        release,
        lines,
    }
}

fn text(written: &Mutex<Vec<u8>>) -> String {
    String::from_utf8(written.lock().unwrap().clone()).unwrap()
}

const FIRST_FIVE: &str = "line 0\nline 1\nline 2\nline 3\nline 4\n";
const SIX_DROPPED: &str = "profile writer: 6 lines dropped while the output was behind\n";

#[test]
fn a_stalled_output_keeps_the_queue_bounded_and_reports_the_gap_where_it_was() {
    let run = stalled_with_ten_behind();
    run.release.send(()).unwrap();
    for _ in 0..5 {
        run.lines.recv().unwrap();
    }
    run.sink.print("line 11".into()).unwrap();
    run.sink.flush();
    assert_eq!(
        text(&run.written),
        format!("{FIRST_FIVE}{SIX_DROPPED}line 11\n")
    );
}

#[test]
fn flush_writes_the_drop_count_after_the_queued_lines() {
    let run = stalled_with_ten_behind();
    run.release.send(()).unwrap();
    run.sink.flush();
    assert_eq!(text(&run.written), format!("{FIRST_FIVE}{SIX_DROPPED}"));
}
