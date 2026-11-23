use super::*;
use replay_recording::columnar_frames::{Frames, Recording};
use replay_journal::fixtures;
use replay_data::rows::TableId;
use std::{fs, sync::atomic::{AtomicBool, Ordering}};

const RUNS: [(u64, i32, u32); 2] = [(1, 100, SPAN_BLOCKS + 1), (2, 900_000, 2)];

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("recompress-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn recording(&self, bytes: &[u8]) -> PathBuf {
        let path = self.0.join("Germany 1936.battleplan");
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_reads_alike(a: &Path, b: &Path) {
    let (a, b) = (Recording::open_path(a).unwrap(), Recording::open_path(b).unwrap());
    assert_eq!((a.run_count(), a.hours()), (b.run_count(), b.hours()));
    assert_eq!(a.root_identity(), b.root_identity());
    let (mut left, mut right) = (Frames::new(&a, &TableId::ALL).unwrap(), Frames::new(&b, &TableId::ALL).unwrap());
    for run in 0..a.run_count() {
        let (held, other) = (a.run(run).unwrap(), b.run(run).unwrap());
        assert_eq!((held.branch, held.epoch, held.first_hour, held.ordinals), (other.branch, other.epoch, other.first_hour, other.ordinals));
        for ordinal in 0..held.ordinals {
            left.go_to(run, ordinal).unwrap();
            right.go_to(run, ordinal).unwrap();
            assert!(left.frame(held.branch).unwrap() == right.frame(held.branch).unwrap(), "run {run} ordinal {ordinal} reads otherwise");
        }
        for block in 0..held.ordinals.div_ceil(envelope::UNIT_ORDINALS) {
            assert_eq!(a.clock(run, block).unwrap(), b.clock(run, block).unwrap(), "run {run} block {block}'s clock");
        }
    }
    let records = |file: &Recording| file.records_extents().iter().map(|extent| file.raw(extent).unwrap()).collect::<Vec<_>>();
    assert_eq!(records(&a), records(&b));
}

fn assert_exercises_every_link(path: &Path) {
    let file = Recording::open_path(path).unwrap();
    let (mut chained, mut again, mut prefixed) = (0, 0, 0);
    let mut wholes: Vec<HashSet<u64>> = vec![HashSet::new(); TABLE_COUNT];
    for run in 0..file.run_count() {
        for block in 0..file.run(run).unwrap().ordinals.div_ceil(envelope::UNIT_ORDINALS) {
            for (table, entry) in file.block(run, block).unwrap().tables.iter().enumerate() {
                chained += entry.unit.parts.iter().flat_map(|part| &part.frames).filter(|frame| frame.chained).count();
                for restore in [&entry.unit.first, &entry.unit.restore].into_iter().flatten() {
                    let chunks = restore.streams.iter().flat_map(|(_, chunks)| chunks);
                    if restore.whole {
                        wholes[table] = chunks.map(|chunk| chunk.frame.offset).collect();
                        continue;
                    }
                    for chunk in chunks {
                        again += usize::from(wholes[table].contains(&chunk.frame.offset));
                        prefixed += usize::from(chunk.prefix.is_some());
                    }
                }
            }
        }
    }
    assert!(chained > 0 && again > 0 && prefixed > 0, "chained {chained}, named again {again}, prefixed {prefixed}");
    assert!(!file.run(0).unwrap().directory().spans.is_empty(), "the first run closes a span");
}

#[test]
fn a_recording_compressed_again_reads_as_it_did_and_is_smaller() {
    let folder = Folder::new("again");
    let bytes = fixtures::restored(recorder_derive::writers(), 7, &RUNS);
    let path = folder.recording(&bytes);
    let before = folder.0.join("before.battleplan");
    fs::write(&before, &bytes).unwrap();
    assert_exercises_every_link(&before);
    assert_eq!(level_of(&path).unwrap(), Some(7));
    let mut told = Vec::new();
    let outcome = recompress_file(&path, 19, false, |progress| {
        told.push(*progress);
        true
    })
    .unwrap();
    let Outcome::Replaced { before: length, after, unflushed: None } = outcome else { panic!("{outcome:?}") };
    assert_eq!(length, bytes.len() as u64);
    assert!(after < length, "{after} bytes against {length}");
    assert_eq!(fs::metadata(&path).unwrap().len(), after);
    assert_eq!(level_of(&path).unwrap(), Some(19));
    assert!(partials(&path).unwrap().is_empty());
    let last = told.last().unwrap();
    assert_eq!(last.units, last.total);
    assert_eq!(last.total, u64::from(RUNS.iter().map(|run| run.2).sum::<u32>()) * TABLE_COUNT as u64);
    assert!(told.windows(2).all(|pair| pair[0].units < pair[1].units && pair[0].written <= pair[1].written));
    assert_reads_alike(&before, &path);
}

#[test]
fn a_recompress_stopped_midway_leaves_the_recording_whole_and_its_partial_unlisted() {
    let folder = Folder::new("stopped");
    let bytes = fixtures::restored(recorder_derive::writers(), 7, &RUNS[1..]);
    let path = folder.recording(&bytes);
    let staged = Staged::beside(&path).unwrap();
    let partial = staged.path.clone();
    staged.keep();
    let mut units = 0;
    let stopped = write_paced(&path, || Ok(File::create(&partial).unwrap()), 19, false, |progress| {
        units = progress.units;
        progress.units < 3
    }, Pace::here());
    assert!(matches!(stopped, Err(Kept::Stopped)), "{stopped:?}");
    assert_eq!(units, 3);
    assert!(fs::metadata(&partial).unwrap().len() > 0, "the stopped write is left as a killed process leaves it");
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(Recording::open_path(&path).unwrap().hours(), u64::from(RUNS[1].2 * envelope::UNIT_ORDINALS));
    let listed: Vec<PathBuf> = crate::recording_files::listing(&folder.0).unwrap().into_iter().map(|(path, ..)| path).collect();
    assert_eq!(listed, [path.clone()]);
    let before = folder.0.join("before.battleplan");
    fs::write(&before, &bytes).unwrap();
    assert!(matches!(recompress_file(&path, 19, false, |_| true), Ok(Outcome::Replaced { .. })));
    assert!(partials(&path).unwrap().is_empty(), "the retry reclaims what the killed attempt left");
    let digest = partial.file_name().unwrap().to_string_lossy().split(".recompress.").next().unwrap().to_owned();
    let legacy = partial.with_file_name(format!("{digest}.recompress.tmp"));
    let someone_elses = partial.with_file_name(format!("{digest}.recompress.backup.tmp"));
    fs::write(&legacy, b"left by an older build").unwrap();
    fs::write(&someone_elses, b"not a recompression").unwrap();
    assert_eq!(partials(&path).unwrap(), [legacy], "only names a recompression writes are its leftovers");
    assert!(someone_elses.exists());
    assert_reads_alike(&before, &path);
}

#[test]
fn a_finished_partial_belongs_to_its_attempt_until_the_destination_is_checked() {
    let folder = Folder::new("interleaved");
    let bytes = fixtures::restored(recorder_derive::writers(), 7, &RUNS[1..]);
    let path = folder.recording(&bytes);
    let before = folder.0.join("before.battleplan");
    fs::write(&before, &bytes).unwrap();
    let Written::Whole(original, first) = write_paced(&path, || Staged::beside(&path), 19, false, |_| true, Pace::here()).unwrap() else { panic!("the first attempt wrote nothing") };
    let replaced = recompress_file(&path, 3, false, |_| true);
    assert!(matches!(replaced, Ok(Outcome::Replaced { .. })), "{replaced:?}");
    assert_eq!(partials(&path).unwrap(), [first.path.clone()], "the second attempt staged apart and left the first's");
    let published = fs::read(&path).unwrap();
    let kept = publish(&path, first, original);
    assert!(matches!(kept, Err(Kept::Changed)), "{kept:?}");
    assert_eq!(fs::read(&path).unwrap(), published);
    assert_eq!(level_of(&path).unwrap(), Some(3));
    assert!(partials(&path).unwrap().is_empty());
    assert_reads_alike(&before, &path);

    let Written::Whole(original, staged) = write_paced(&path, || Staged::beside(&path), 19, false, |_| true, Pace::here()).unwrap() else { panic!("the attempt wrote nothing") };
    let mut grown = published.clone();
    grown.extend_from_slice(&[0; 16]);
    fs::write(&path, &grown).unwrap();
    let kept = publish(&path, staged, original);
    assert!(matches!(kept, Err(Kept::Changed)), "{kept:?}");
    assert_eq!(fs::read(&path).unwrap(), grown);
    assert!(partials(&path).unwrap().is_empty());
}

#[test]
fn a_recording_already_at_the_level_is_left_alone() {
    let folder = Folder::new("already");
    let bytes = fixtures::restored(recorder_derive::writers(), 19, &[(1, 100, 1)]);
    let path = folder.recording(&bytes);
    assert_eq!(recompress_file(&path, 19, false, |_| true).unwrap(), Outcome::AlreadyAt(19));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(partials(&path).unwrap().is_empty());
}

#[test]
fn a_recording_still_being_written_is_refused_by_name() {
    let folder = Folder::new("written");
    let bytes = fixtures::restored(recorder_derive::writers(), 7, &[(1, 100, 1)]);
    let path = folder.recording(&bytes);
    let writer = OpenOptions::new().read(true).write(true).share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE).open(&path).unwrap();
    let held = recompress_file(&path, 19, false, |_| true);
    assert!(matches!(held, Err(Kept::Recording)), "{held:?}");
    assert!(held.unwrap_err().to_string().contains("still being written"));
    drop(writer);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let (root, length) = container::locate(bytes.as_slice(), Open::Newest).unwrap().unwrap();
    let read = container::read_root(bytes.as_slice(), &root).unwrap();
    let at = container::Revision { number: read.revision, length, root: root.digest };
    let mut appending = ContainerWriter::resume(io::Cursor::new(bytes.clone()), read, at, 7, None).unwrap();
    appending.commit(None, false, |_| Ok(())).unwrap();
    let unsaved = appending.into_inner().into_inner();
    fs::write(&path, &unsaved).unwrap();
    assert!(matches!(recompress_file(&path, 19, false, |_| true), Err(Kept::Recording)));
    assert_eq!(fs::read(&path).unwrap(), unsaved);
    assert!(partials(&path).unwrap().is_empty());
}

#[test]
fn a_recording_opened_elsewhere_while_compressed_is_kept_as_it_was() {
    let folder = Folder::new("elsewhere");
    let bytes = fixtures::restored(recorder_derive::writers(), 7, &[(1, 100, 1)]);
    let path = folder.recording(&bytes);
    let mut reader = None;
    let kept = recompress_file(&path, 19, false, |_| {
        reader.get_or_insert_with(|| File::open(&path).unwrap());
        true
    });
    assert!(matches!(kept, Err(Kept::OpenElsewhere)), "{kept:?}");
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(partials(&path).unwrap().is_empty());
    drop(reader);
    assert!(matches!(recompress_file(&path, 19, false, |_| true), Ok(Outcome::Replaced { .. })));
}

#[test]
fn a_background_recompress_runs_below_normal_priority() {
    use windows_sys::Win32::System::Threading::{BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, NORMAL_PRIORITY_CLASS, SetPriorityClass, THREAD_PRIORITY_BELOW_NORMAL};
    let folder = Folder::new("priority");
    let path = folder.recording(&fixtures::restored(recorder_derive::writers(), 7, &[(1, 100, 1)]));
    let seen = std::thread::spawn(move || {
        let mut seen = None;
        let outcome = recompress_file(&path, 19, true, |_| {
            seen = Some(priority());
            true
        });
        (outcome, seen)
    })
    .join()
    .unwrap();
    unsafe { SetPriorityClass(GetCurrentProcess(), NORMAL_PRIORITY_CLASS) };
    assert!(matches!(seen.0, Ok(Outcome::Replaced { .. })), "{:?}", seen.0);
    assert_eq!(seen.1, Some((BELOW_NORMAL_PRIORITY_CLASS, THREAD_PRIORITY_BELOW_NORMAL)));
}

#[derive(Clone, Copy, Debug)]
struct Held {
    units: usize,
    stalled: bool,
}

pub(super) struct Hold {
    above: usize,
    units: usize,
    taken: AtomicBool,
    seen: Mutex<Option<Held>>,
}

impl Queue {
    pub(super) fn hold(&self, seq: u64, raw: usize) {
        let Some(hold) = &self.hold else { return };
        if raw <= hold.above || hold.taken.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut state = self.lock();
        let held = loop {
            let units = state.units.range(seq + 1..).count();
            let stalled = state.tasks.is_empty() && state.given.is_empty() && state.waiting && state.idle + 1 == self.workers;
            if units >= hold.units || stalled || state.closed {
                break Held { units, stalled };
            }
            state = self.given.wait(state).unwrap_or_else(PoisonError::into_inner);
        };
        drop(state);
        *hold.seen.lock().unwrap() = Some(held);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PartsHeld {
    written: bool,
    stalled: bool,
}

pub(super) struct PartsHold {
    taken: AtomicBool,
    seen: Mutex<Option<PartsHeld>>,
}

impl Queue {
    pub(super) fn hold_parts(&self, seq: u64) {
        let Some(hold) = &self.parts else { return };
        if hold.taken.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut state = self.lock();
        let held = loop {
            let written = state.applied > seq;
            let stalled = state.tasks.is_empty() && state.given.is_empty() && state.waiting && state.idle + 1 == self.workers;
            if written || stalled || state.closed {
                break PartsHeld { written, stalled };
            }
            state = self.given.wait(state).unwrap_or_else(PoisonError::into_inner);
        };
        drop(state);
        *hold.seen.lock().unwrap() = Some(held);
    }

    pub(super) fn applied(&self, applied: u64) {
        self.lock().applied = applied;
        self.given.notify_all();
    }
}

impl State {
    pub(super) fn gave(&mut self, message: &Message) {
        if let Message::Done { seq, slot: Some(_), unit: true, .. } = message {
            self.units.insert(*seq);
        }
    }
}

#[test]
fn a_very_large_unit_keeps_every_worker_busy_and_writes_the_same_bytes() {
    let folder = Folder::new("large");
    let path = folder.recording(&fixtures::one_large_unit(recorder_derive::writers(), 1));
    let file = Recording::open_path(&path).unwrap();
    let mut units = Vec::new();
    let mut heads = 0;
    for run in 0..file.run_count() {
        for block in 0..file.run(run).unwrap().ordinals.div_ceil(envelope::UNIT_ORDINALS) {
            let read = file.block(run, block).unwrap();
            let head = std::iter::once(&read.head.clock).chain(read.head.strings.iter().map(|chunk| &chunk.frame));
            heads = head.map(|frame| frame.raw).fold(heads, u32::max);
            for entry in &read.tables {
                let chunks = [&entry.unit.first, &entry.unit.restore].into_iter().flatten().flat_map(|restore| restore.streams.iter().flat_map(|(_, chunks)| chunks));
                let frames: Vec<u32> = entry.unit.parts.iter().flat_map(|part| &part.frames).map(|frame| frame.raw).chain(chunks.map(|chunk| chunk.frame.raw)).collect();
                let work = entry.unit.parts.iter().any(|part| !part.frames.is_empty()) || entry.summary.is_some();
                units.push((frames.iter().copied().max().unwrap_or(0), frames.iter().map(|&raw| u64::from(raw)).sum::<u64>(), work));
            }
        }
    }
    let large = (0..units.len()).max_by_key(|&at| units[at].1).unwrap();
    let above = (0..units.len()).filter(|&at| at != large).map(|at| units[at].0).fold(heads, u32::max);
    assert!(units[large].0 > above, "the large unit's largest frame holds {} bytes, against {above} elsewhere", units[large].0);
    let later = units[large + 1..].iter().filter(|unit| unit.2).count();
    assert!(later >= 6, "{later} units after the large one hold work");
    let hold = Arc::new(Hold { above: above as usize, units: later, taken: AtomicBool::new(false), seen: Mutex::new(None) });
    let paced = |name: &str, pace: Pace| {
        let partial = folder.0.join(name);
        let written = write_paced(&path, || Ok(File::create(&partial).unwrap()), 3, false, |_| true, pace);
        assert!(matches!(written, Ok(Written::Whole(..))), "{written:?}");
        fs::read(&partial).unwrap()
    };
    let held = paced("held.tmp", Pace { workers: 4, window: 2 * units[large].1, hold: Some(Arc::clone(&hold)), parts: None });
    let seen = hold.seen.lock().unwrap().expect("a frame of the large unit was held");
    assert!(seen.units >= later, "{} of {later} later units compressed before the large one's held frame (the others left without work: {})", seen.units, seen.stalled);
    assert_eq!(held, paced("one.tmp", Pace { workers: 1, window: 0, hold: None, parts: None }));
    assert_eq!(held, paced("here.tmp", Pace::here()));
    assert_reads_alike(&path, &folder.0.join("held.tmp"));
}

#[test]
fn a_unit_waits_for_its_part_frames_task_to_say_it_is_done() {
    let folder = Folder::new("parts-late");
    let path = folder.recording(&fixtures::restored(recorder_derive::writers(), 7, &RUNS));
    assert_exercises_every_link(&path);
    let hold = Arc::new(PartsHold { taken: AtomicBool::new(false), seen: Mutex::new(None) });
    let paced = |name: &str, pace: Pace| {
        let partial = folder.0.join(name);
        let written = write_paced(&path, || Ok(File::create(&partial).unwrap()), 3, false, |_| true, pace);
        assert!(matches!(written, Ok(Written::Whole(..))), "{written:?}");
        fs::read(&partial).unwrap()
    };
    let held = paced("held.tmp", Pace { workers: 2, window: 0, hold: None, parts: Some(Arc::clone(&hold)) });
    let seen = *hold.seen.lock().unwrap();
    assert_eq!(seen, Some(PartsHeld { written: false, stalled: true }), "the unit was written before its part frames task was done");
    assert_eq!(held, paced("one.tmp", Pace { workers: 1, window: 0, hold: None, parts: None }));
    assert_reads_alike(&path, &folder.0.join("held.tmp"));
}
