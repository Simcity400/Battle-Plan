//! The runtime thread's attach and detach progress, appended to `progress`
//! in the session directory as one line per step: milliseconds since the
//! collector opened it, the step and an optional count. The recorder waits
//! for readiness and for detach while this file grows, and names its last
//! line when it stops growing; the lines also time each step for diagnosis.
//! Writing formats into a stack buffer and appends it, with no allocation, so
//! detach can record its steps after the allocation gate closes. A failed
//! write loses only that line.
use std::{fs::File, io::Write, path::Path, time::Instant};

pub(crate) const FILE: &str = "progress";

#[cfg(test)]
type TestWriter = Box<dyn Fn(&[u8])>;

pub(crate) struct Progress {
    file: Option<File>,
    opened: Instant,
    #[cfg(test)]
    writer: Option<TestWriter>,
}

impl Progress {
    /// Opens (truncating) the session's progress file; without one, every
    /// step is dropped. Writes append, so lines other handles append (a
    /// worker panic) are never overwritten.
    pub(crate) fn open(directory: &Path) -> Self {
        let path = directory.join(FILE);
        Self {
            file: File::create(&path)
                .and_then(|_| std::fs::OpenOptions::new().append(true).open(&path))
                .ok(),
            opened: Instant::now(),
            #[cfg(test)]
            writer: None,
        }
    }

    /// When the file was opened, the zero of its step times.
    pub(crate) fn opened(&self) -> Instant {
        self.opened
    }

    /// Appends `step`, with `count` when it has one.
    pub(crate) fn step(&self, step: impl std::fmt::Display, count: Option<u64>) {
        let mut buffer = [0_u8; 160];
        let length = line(&mut buffer, self.opened.elapsed().as_millis(), step, count);
        self.append(&buffer[..length]);
    }

    fn publication(&self, family: &str, summary: crate::hook::publication::Summary) {
        let mut buffer = [0_u8; 512];
        let length = line(
            &mut buffer,
            self.opened.elapsed().as_millis(),
            format_args!(
                "publication {family} attempts={} total_hold_ns={} longest_hold_ns={} longest_synchronization_ns={} slowest_context_ns={} slowest_context_thread_id={} slowest_context_priority={} priority_restore_failures={}",
                summary.attempts,
                summary.total_hold_ns,
                summary.longest_hold_ns,
                summary.longest_synchronization_ns,
                summary.slowest_context_ns,
                summary.slowest_context_thread_id,
                summary.slowest_context_priority,
                summary.priority_restore_failures,
            ),
            None,
        );
        self.append(&buffer[..length]);
    }

    fn append(&self, bytes: &[u8]) {
        #[cfg(test)]
        if let Some(writer) = &self.writer {
            writer(bytes);
            return;
        }
        if let Some(mut file) = self.file.as_ref() {
            let _ = file.write_all(bytes);
        }
    }
}

/// The family attach is installing, as the `Cell` it replaces, appending a
/// step whenever an attempt reaches a family no earlier attempt reached:
/// retries of the same families add no lines.
pub(crate) struct Stage<'a> {
    name: std::cell::Cell<&'static str>,
    first: &'static str,
    reached: std::cell::Cell<u32>,
    furthest: std::cell::Cell<u32>,
    progress: &'a Progress,
}

impl<'a> Stage<'a> {
    pub(crate) fn new(progress: &'a Progress, first: &'static str) -> Self {
        progress.step(format_args!("install {first}"), None);
        crate::hook::publication::select_family(Some(first));
        Self {
            name: std::cell::Cell::new(first),
            first,
            reached: std::cell::Cell::new(0),
            furthest: std::cell::Cell::new(0),
            progress,
        }
    }

    pub(crate) fn set(&self, name: &'static str) {
        self.name.set(name);
        crate::hook::publication::select_family(Some(name));
        let reached = if name == self.first {
            0
        } else {
            self.reached.get() + 1
        };
        self.reached.set(reached);
        if reached > self.furthest.get() {
            self.furthest.set(reached);
            self.progress.step(format_args!("install {name}"), None);
        }
    }

    pub(crate) fn get(&self) -> &'static str {
        self.name.get()
    }

    pub(crate) fn finish(&self) {
        crate::hook::publication::select_family(None);
        let (families, available) = crate::hook::publication::snapshot();
        for (name, summary) in families {
            self.progress.publication(name, summary);
        }
        if !available {
            self.progress.step("publication unavailable", None);
        }
    }
}

/// `<ms> <step>[ <count>]` and a line feed in `buffer`, truncated to fit;
/// the bytes written. Steps are the collector's own names, without line
/// breaks.
fn line(
    buffer: &mut [u8],
    millis: u128,
    step: impl std::fmt::Display,
    count: Option<u64>,
) -> usize {
    let end = buffer.len() - 1;
    let mut cursor = std::io::Cursor::new(&mut buffer[..end]);
    let _ = write!(cursor, "{millis} {step}");
    if let Some(count) = count {
        let _ = write!(cursor, " {count}");
    }
    let length = cursor.position() as usize;
    buffer[length] = b'\n';
    length + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_emit_one_cumulative_summary_only_after_the_deadline_decision() {
        use std::{
            cell::{Cell, RefCell},
            rc::Rc,
        };
        const FAMILY: &str = "synthetic_slow_progress";
        let time = Rc::new(Cell::new(0));
        let lines = Rc::new(RefCell::new(Vec::new()));
        let (written, clock) = (lines.clone(), time.clone());
        let progress = Progress {
            file: None,
            opened: Instant::now(),
            writer: Some(Box::new(move |bytes| {
                let text = String::from_utf8(bytes.to_vec()).unwrap();
                if text.contains(" publication ") {
                    clock.set(clock.get() + 20_000);
                }
                written.borrow_mut().push(text);
            })),
        };
        let stage = Stage::new(&progress, FAMILY);
        // Same ready/expired ordering as the runtime; slow diagnostic writes
        // would stop this not-ready/not-ready/ready sequence on its second result.
        let mut decision = false;
        for ready in [false, false, true] {
            stage.set(FAMILY);
            crate::hook::publication::complete_test_attempt();
            time.set(time.get() + 1);
            if ready || time.get() >= 15_000 {
                decision = ready;
                break;
            }
        }
        assert!(decision);
        assert_eq!(time.get(), 3);
        assert_eq!(lines.borrow().len(), 1, "retries add no diagnostic I/O");
        stage.finish();
        let lines = lines.borrow();
        let summaries: Vec<_> = lines
            .iter()
            .filter(|line| line.contains(FAMILY) && line.contains(" publication "))
            .collect();
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].contains("attempts=3 total_hold_ns=0 longest_hold_ns=0 longest_synchronization_ns=0 slowest_context_ns=0 slowest_context_thread_id=0"));
        assert!(time.get() > 15_000, "the slow writer runs after success");
    }

    #[test]
    fn lines_carry_time_step_and_count_and_stay_in_their_buffer() {
        let mut buffer = [0_u8; 32];
        let length = line(&mut buffer, 1234, "restore Raid", Some(3));
        assert_eq!(&buffer[..length], b"1234 restore Raid 3\n");
        let length = line(&mut buffer, 7, "spool_joined", None);
        assert_eq!(&buffer[..length], b"7 spool_joined\n");
        let long = "x".repeat(100);
        let length = line(&mut buffer, 5, &long, Some(9));
        assert_eq!(length, 32);
        assert_eq!(buffer[31], b'\n');
        assert!(buffer[..31].iter().all(|&byte| byte != b'\n'));
    }

    #[test]
    fn steps_append_to_the_session_file() {
        let directory = std::env::temp_dir().join(format!(
            "hoi4-progress-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let progress = Progress::open(&directory);
        progress.step("detach_requested", None);
        progress.step(format_args!("restore {:?}", "Hourly"), Some(1));
        let text = std::fs::read_to_string(directory.join(FILE)).unwrap();
        let steps: Vec<&str> = text
            .lines()
            .map(|line| line.split_once(' ').unwrap().1)
            .collect();
        assert_eq!(steps, ["detach_requested", "restore \"Hourly\" 1"]);
        drop(progress);
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
