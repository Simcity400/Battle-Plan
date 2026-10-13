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

pub(crate) struct Progress {
    file: Option<File>,
    opened: Instant,
}

impl Progress {
    /// Opens (truncating) the session's progress file; without one, every
    /// step is dropped.
    pub(crate) fn open(directory: &Path) -> Self {
        Self {
            file: File::create(directory.join(FILE)).ok(),
            opened: Instant::now(),
        }
    }

    /// Appends `step`, with `count` when it has one.
    pub(crate) fn step(&self, step: impl std::fmt::Display, count: Option<u64>) {
        let Some(mut file) = self.file.as_ref() else {
            return;
        };
        let mut buffer = [0_u8; 160];
        let length = line(&mut buffer, self.opened.elapsed().as_millis(), step, count);
        let _ = file.write_all(&buffer[..length]);
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
