use super::*;
use crate::{
    branches::{BranchReason, Parentage},
    campaign::{fixtures, writer},
    columnar_journal::Journal,
    frame::{Frame, Metrics, TableUpdate, TableView},
    identity::{CaptureMode, SampleBoundary, Verification},
    rows::{CampaignRow, ClockRow, TableId, TableRows},
    spool::Record as Spooled,
};
use std::collections::{BTreeMap, BTreeSet};

const CAMPAIGN: &[u8; 36] = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
const SESSION: u64 = 7;
const SOURCE: &str = "session-a";

/// A directory of its own, removed even when an assertion fails: the recording, its writer's
/// resume state and the session's journal; and whether its writers include the log's reader
/// ([`push_ledger`]).
struct Folder(PathBuf, bool);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("hoi4-live-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path, false)
    }

    /// Its writers also the log's reader.
    fn with_ledgers(mut self) -> Self {
        self.1 = true;
        self
    }

    /// Capture `capture`'s writer on the recording.
    fn open(&self, capture: u32) -> LiveWriter {
        let mut writers = recorder_derive::writers();
        if self.1 {
            push_ledger(&mut writers);
        }
        LiveWriter::open_with(&self.recording(), &self.state(), SOURCE, capture, Settings::LOW, Vec::new(), writers).unwrap()
    }

    fn recording(&self) -> PathBuf {
        self.0.join("recording.battleplan")
    }

    fn state(&self) -> PathBuf {
        self.journal().join("recording.state")
    }

    fn journal(&self) -> PathBuf {
        self.0.join(SOURCE)
    }

    fn segments(&self) -> Vec<(u32, u32)> {
        crate::columnar_journal::segments(&self.journal())
            .unwrap()
            .into_iter()
            .map(|(capture, segment, _)| (capture, segment))
            .collect()
    }

    /// Opens capture `capture`'s writer on the recording and its journal where the writer reads
    /// on, and takes what the journal holds.
    fn write(&self, capture: u32, host: &mut Deciding) -> (LiveWriter, Journal, Progress) {
        self.write_until(capture, host, &|| false)
    }

    fn write_until(
        &self,
        capture: u32,
        host: &mut Deciding,
        stop: &dyn Fn() -> bool,
    ) -> (LiveWriter, Journal, Progress) {
        let mut writer = self.open(capture);
        let mut journal = Journal::open(&self.journal(), capture);
        journal.start(writer.segments()).unwrap();
        let progress = writer.advance(&mut journal, host, stop).unwrap();
        (writer, journal, progress)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn session() -> Spooled {
    Spooled::SessionStart(SessionStart {
        id: SESSION,
        process_id: 2,
        process_start_time: 13,
        qpc_frequency: 100,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Unchecked,
        },
    })
}

fn branch(id: BranchId, first_hour: Hour) -> Spooled {
    Spooled::BranchStart(BranchStart {
        id,
        session: SESSION,
        loaded_hour: Some(first_hour - 1),
        first_hour,
        parent: Parentage::Unknown,
        // A session starts one branch; each later one is a load.
        reason: match id {
            1 => BranchReason::SessionStart,
            _ => BranchReason::LoadSignal,
        },
    })
}

/// One complete hour of `branch` under `load`: its clock, the campaign, and every other table
/// empty or one default row.
fn frame(branch: BranchId, hour: Hour, load: i64) -> Frame {
    Frame {
        branch,
        hour,
        keyframe: true,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            let rows = match TableId::ALL[index] {
                TableId::Clock => TableRows::Clock(vec![ClockRow {
                    hour,
                    play_session_started_at: load,
                    ..Default::default()
                }]),
                TableId::Campaign => TableRows::Campaign(vec![CampaignRow {
                    id: CAMPAIGN.to_vec(),
                }]),
                table => TableRows::empty(table),
            };
            match crate::frame::current_absence(rows.table()) {
                Some(TableView::NotRecorded { mode }) => TableUpdate::NotRecorded { mode },
                Some(TableView::Unavailable { reason }) => TableUpdate::Unavailable { reason },
                None => TableUpdate::Snapshot(rows),
                _ => unreachable!("absence is an omission or a named refusal"),
            }
        }),
    }
}

/// Each hour of `hours` of `branch` under `load`: its clock witness, then its frame.
fn hours(branch: BranchId, hours: std::ops::RangeInclusive<Hour>, load: i64) -> Vec<Spooled> {
    hours
        .flat_map(|hour| {
            [
                Spooled::Control(ControlRecord::ClockWitness {
                    session: SESSION,
                    qpc: 1_000 + hour as u64,
                    hour,
                    load_signal: Some(load),
                }),
                Spooled::Frame(frame(branch, hour, load)),
            ]
        })
        .collect()
}

fn detach(last_hour: Hour) -> Spooled {
    Spooled::Control(ControlRecord::Detach {
        session: SESSION,
        last_hour: Some(last_hour),
    })
}

/// A host that writes the loads it admits, passes over those it refuses and leaves the rest
/// waiting; `all` writes every branch, as the command line does.
#[derive(Default)]
struct Deciding {
    all: bool,
    admitted: BTreeSet<i64>,
    refused: BTreeSet<i64>,
    saves: bool,
    asked: Vec<Asked>,
    skipped: Vec<(Option<i64>, u64)>,
    committed: Vec<Committed>,
    hours: u64,
    /// The session's game facts pack it offers, as a session folder's file is read each time.
    facts: Option<Vec<u8>>,
    /// A session facts file it offers the pack of at its place, as the recorder's host does.
    facts_file: Option<PathBuf>,
    /// It offers either only once it was told of this hour.
    facts_after: Option<Hour>,
    /// The last hour it was told of.
    last: Option<Hour>,
}

impl Deciding {
    /// Whether it offers the session's facts yet.
    fn offers(&self) -> bool {
        self.facts_after.is_none_or(|after| self.last.is_some_and(|last| last >= after))
    }

    fn everything(saves: bool) -> Self {
        Self {
            all: true,
            saves,
            ..Self::default()
        }
    }
}

impl Host for Deciding {
    fn admit(&mut self, asked: &Asked) -> Admit {
        self.asked.push(*asked);
        match asked.load {
            _ if self.all => Admit::Write,
            Some(load) if self.admitted.contains(&load) => Admit::Write,
            Some(load) if self.refused.contains(&load) => Admit::Skip,
            _ => Admit::Wait,
        }
    }

    fn saves(&mut self) -> bool {
        self.saves
    }

    fn facts(&mut self, _: &Identity) -> Option<Vec<u8>> {
        self.offers().then(|| self.facts.clone()).flatten()
    }

    fn facts_file(&mut self, identity: &Identity) -> Option<FactsFile> {
        if !self.offers() {
            return None;
        }
        let mut file = File::open(self.facts_file.as_ref()?).ok()?;
        let (offset, length) = crate::campaign::session_facts_place(&mut file, identity).unwrap()?;
        Some(FactsFile { file, offset, length })
    }

    fn hour(&mut self, applied: &Applied<'_>) {
        self.hours += 1;
        self.last = Some(applied.hour);
    }

    fn skipped(&mut self, asked: &Asked, hours: u64) {
        self.skipped.push((asked.load, hours));
    }

    fn committed(&mut self, committed: &Committed) {
        self.committed.push(*committed);
    }
}

/// A source that fails to let go of its segments when told to, as a recorder killed between a
/// commit and the journal's cleanup leaves them.
struct Kept<'a> {
    journal: &'a mut Journal,
    fail_at: u32,
    retired: u32,
}

impl Source for Kept<'_> {
    fn start(&mut self, segment: u32) -> io::Result<()> {
        self.journal.start(segment)
    }

    fn record(&mut self) -> io::Result<Option<Record<'_>>> {
        self.journal.record()
    }

    fn advance(&mut self) {
        self.journal.advance();
    }

    fn retire(&mut self) -> io::Result<()> {
        self.retired += 1;
        if self.retired == self.fail_at {
            return Err(io::Error::other("killed before the journal was let go of"));
        }
        self.journal.retire()
    }

    fn finish(&mut self) -> io::Result<()> {
        self.journal.finish()
    }

    fn producing(&mut self) -> io::Result<bool> {
        self.journal.producing()
    }
}

/// The newest saved revision of the recording at `path`.
fn saved(path: &Path) -> Recording {
    Recording::open_path_as(path, Open::Saved).unwrap()
}

/// The newest revision of the recording at `path`, saved or not.
fn newest(path: &Path) -> Recording {
    Recording::open_path_as(path, Open::Newest).unwrap()
}

/// Hours its runs hold.
fn recorded(log: &Recording) -> u64 {
    log.directories().unwrap().iter().map(|run| u64::from(run.hours)).sum()
}

/// The hours its runs of `branch` hold, in order.
fn recorded_hours(log: &Recording, branch: BranchId) -> Vec<Hour> {
    log.directories().unwrap()
        .iter()
        .filter(|run| run.branch == branch)
        .flat_map(|run| run.first_hour..run.first_hour + run.hours as i32)
        .collect()
}

/// A capture's hours and records go straight into its recording: created at its first hour,
/// which is its run's first restore, one revision a segment, each segment let go of once its
/// revision is committed, and the capture's end leaving nothing beside the file. What it holds
/// is what the records hold.
#[test]
fn a_capture_is_written_once_into_its_recording() {
    let folder = Folder::new("once");
    let mut records = vec![session(), branch(1, 100)];
    records.extend(hours(1, 100..=829, 19));
    records.push(Spooled::Gap(Gap {
        branch: 1,
        first_hour: 830,
        last_hour: 899,
        reason: "synthetic loss".into(),
    }));
    records.extend(hours(1, 900..=905, 19));
    records.push(detach(905));
    fixtures::write_journal(&folder.journal(), 0, CAMPAIGN, &[records.clone()], true).unwrap();
    assert_eq!(folder.segments(), [(0, 0), (0, 1), (0, 2), (0, 3)]);

    let mut host = Deciding::everything(true);
    let (writer, _, progress) = folder.write(0, &mut host);
    assert_eq!(progress, Progress::Finished);
    assert!(writer.finished());
    assert!(folder.segments().is_empty(), "every segment is let go of");
    assert!(!folder.state().exists(), "a finished capture leaves no resume state");
    // A full block, the run's short last block, and the run after the gap: one revision each.
    let added: Vec<_> = host.committed.iter().map(|committed| (committed.hours, committed.finished)).collect();
    assert_eq!(added, [(720, false), (10, false), (6, true)]);
    assert_eq!(host.hours, 736);

    let written = saved(&folder.recording());
    assert_eq!(written.revision().number, 3);
    let places: Vec<_> = written.directories().unwrap().iter().map(|run| (run.branch, run.first_hour, run.hours)).collect();
    assert_eq!(places, [(1, 100, 730), (1, 900, 6)]);
    let held = written.records().unwrap();
    assert_eq!(held.sessions.len(), 1);
    assert_eq!(held.branches.iter().map(|branch| branch.id).collect::<Vec<_>>(), [1]);
    assert_eq!(held.gaps.len(), 1);
    let witnessed: Vec<Hour> = held
        .controls
        .iter()
        .filter_map(|control| match control {
            ControlRecord::ClockWitness { hour, .. } => Some(*hour),
            _ => None,
        })
        .collect();
    assert_eq!(witnessed, (100..=829).chain(900..=905).collect::<Vec<_>>());
    let cursor = held.cursors.iter().rev().find(|cursor| cursor.source == SOURCE && cursor.capture == 0).unwrap();
    assert!(cursor.finished && cursor.session == SESSION);
    // The capture is held whole: its writer has nothing more to take.
    let (again, ..) = folder.write(0, &mut Deciding::everything(true));
    assert!(again.finished());
}

/// A run's first hour is its first restore: every table's entry of its first block names it,
/// and no part of that block starts before the run's second hour. A load starts a run, whose
/// first block holds a first restore of its own.
#[test]
fn a_run_s_first_hour_is_its_first_restore_and_a_load_starts_a_run() {
    let folder = Folder::new("first");
    let mut records = vec![session(), branch(1, 100)];
    records.extend(hours(1, 100..=900, 19));
    records.push(branch(2, 2_000));
    records.extend(hours(2, 2_000..=2_010, 23));
    records.push(detach(2_010));
    fixtures::write_journal(&folder.journal(), 0, CAMPAIGN, &[records], true).unwrap();
    let (_, _, progress) = folder.write(0, &mut Deciding::everything(true));
    assert_eq!(progress, Progress::Finished);
    let written = saved(&folder.recording());
    assert_eq!(written.directories().unwrap().len(), 2, "the load starts a run");
    for run in 0..2 {
        let first = written.block(run, 0).unwrap();
        for (table, entry) in first.tables.iter().enumerate() {
            assert!(entry.unit.first.is_some(), "run {run} table {table} has its first restore");
            assert!(entry.unit.parts.iter().all(|part| part.first >= 1), "run {run} table {table} has no part of the run's first hour");
        }
    }
    let later = written.block(0, 1).unwrap();
    assert!(later.tables.iter().all(|entry| entry.unit.first.is_none()));
}

/// The strings a block's hours appended are a frame of the block, read back from the file by
/// the store's source for a string it let go of.
#[test]
fn a_block_s_strings_are_read_back_from_its_file() {
    let folder = Folder::new("strings");
    let mut records = vec![session(), branch(1, 100)];
    records.extend(hours(1, 100..=1_600, 19));
    records.push(detach(1_600));
    fixtures::write_journal(&folder.journal(), 0, CAMPAIGN, &[records], true).unwrap();
    let (_, _, progress) = folder.write(0, &mut Deciding::everything(true));
    assert_eq!(progress, Progress::Finished);
    let written = saved(&folder.recording());
    let heads = written.heads(0).unwrap();
    assert_eq!(heads.len(), 3);
    let frames: Frames = Arc::new(Mutex::new(heads.iter().flat_map(|head| head.strings.iter().map(|chunk| (chunk.first, chunk.frame.clone()))).collect()));
    let mut source = FileStrings::new(File::open(folder.recording()).unwrap(), frames);
    let mut seen = 0;
    for head in &heads {
        for (id, text) in written.strings(head).unwrap() {
            assert_eq!(replay_columnar::store::StringSource::text(&mut source, id).unwrap(), Some(text));
            seen += 1;
        }
    }
    assert!(seen > 0, "the campaign's identity is a string the first hour appends");
}

/// Killed at any point, the writer goes on from the recording's newest committed revision and
/// the journal segments after it: before its first commit (the file holds no revision, and is
/// made again), in the middle of a block, after a commit whose segment was not let go of yet,
/// and with a block's frames written but no root naming them. The recording ends up byte for
/// byte the one an unbroken write makes, the log's reader among its writers.
#[test]
fn a_killed_writer_goes_on_from_its_newest_revision_to_the_unbroken_file() {
    let whole = Folder::new("unbroken").with_ledgers();
    let folder = Folder::new("killed").with_ledgers();
    let mut records = vec![session(), branch(1, 100)];
    // Three full blocks and a short one.
    records.extend(hours(1, 100..=2_269, 19));
    records.push(detach(2_269));
    for target in [&whole, &folder] {
        fixtures::write_journal(&target.journal(), 0, CAMPAIGN, &[records.clone()], true).unwrap();
    }
    let (_, _, progress) = whole.write(0, &mut Deciding::everything(true));
    assert_eq!(progress, Progress::Finished);
    let expected = fs::read(whole.recording()).unwrap();
    assert_eq!(recorded(&saved(&whole.recording())), 2_170);

    let taken = std::cell::Cell::new(0_u32);
    let taken = &taken;
    let after = |records: u32| {
        taken.set(0);
        move || {
            taken.set(taken.get() + 1);
            taken.get() >= records
        }
    };
    // Killed before the first commit: the file holds no revision, and the journal is whole; its
    // session makes it again from the journal.
    let (writer, _, progress) = folder.write_until(0, &mut Deciding::everything(true), &after(300));
    assert_eq!(progress, Progress::Paused);
    drop(writer);
    assert!(container::locate(&File::open(folder.recording()).unwrap(), Open::Newest).is_err());
    assert_eq!(folder.segments().len(), 5);
    fs::remove_file(folder.recording()).unwrap();

    // Killed in the middle of the second block: the first is committed, and what was written
    // after its revision is torn.
    let mut host = Deciding::everything(true);
    let (writer, _, progress) = folder.write_until(0, &mut host, &after(1_000));
    assert_eq!(progress, Progress::Paused);
    assert_eq!(writer.segments(), 2);
    drop(writer);
    let first = host.committed[0].revision;
    let mut torn = fs::OpenOptions::new().append(true).open(folder.recording()).unwrap();
    io::Write::write_all(&mut torn, &[0xa5; 4_000]).unwrap();
    drop(torn);
    assert_eq!(recorded(&saved(&folder.recording())), 720);

    // Killed after the third block's commit, before its segment was let go of; then, as if
    // killed a moment earlier, with that block's frames written and no root naming them.
    let mut host = Deciding::everything(true);
    let mut writer = folder.open(0);
    assert_eq!(writer.segments(), 2);
    let mut journal = Journal::open(&folder.journal(), 0);
    journal.start(writer.segments()).unwrap();
    let mut kept = Kept { journal: &mut journal, fail_at: 2, retired: 0 };
    assert!(writer.advance(&mut kept, &mut host, &|| false).is_err());
    drop(writer);
    let revisions: Vec<_> = host.committed.iter().map(|committed| committed.revision).collect();
    assert_eq!(revisions.len(), 2);
    assert!(revisions[0].length > first.length);
    assert_eq!(recorded(&saved(&folder.recording())), 2_160);
    assert!(folder.segments().contains(&(0, 3)), "the third block's segment was not let go of");
    fs::OpenOptions::new().write(true).open(folder.recording()).unwrap().set_len(revisions[0].length + 1_000).unwrap();
    assert_eq!(recorded(&saved(&folder.recording())), 1_440);
    // And killed after its next resume state slot was written, before a root named it: the
    // newest root names the other slot, which that write never touched.
    let named = newest(&folder.recording()).root().state.expect("the newest root names a slot");
    let mut state = StateFile::open(&folder.state()).unwrap();
    let next = state.newest().unwrap() + 1;
    state.write(next, b"a slot no root names", Some(named)).unwrap();
    drop(state);

    let mut host = Deciding::everything(true);
    let (writer, _, progress) = folder.write(0, &mut host);
    assert_eq!(progress, Progress::Finished);
    assert!(writer.finished() && folder.segments().is_empty());
    let resumed = fs::read(folder.recording()).unwrap();
    let differs = resumed.iter().zip(&expected).position(|(a, b)| a != b);
    if let Some(at) = differs {
        // The first revision whose root differs, read from each file cut at its end.
        let length = revisions.iter().map(|revision| revision.length as usize).find(|&length| length > at).unwrap_or(resumed.len());
        let root = |bytes: &[u8]| {
            let (extent, _) = container::locate(&bytes[..length], Open::Newest).unwrap().unwrap();
            format!("{:?}", container::read_root(&bytes[..length], &extent).unwrap())
        };
        eprintln!("resumed root: {}
unbroken root: {}", root(&resumed), root(&expected));
    }
    assert!(
        resumed == expected,
        "the resumed file is the unbroken one, byte for byte: {} bytes against {}, first differing at {differs:?}; revisions {revisions:?}",
        resumed.len(),
        expected.len()
    );
}

/// A note added mid-run is in the next committed revision, once; a writer killed after that
/// revision and taken up holds it once too, and ends byte for byte the unbroken file.
#[test]
fn a_note_added_mid_run_is_committed_once_and_survives_a_kill() {
    let whole = Folder::new("noted-unbroken");
    let folder = Folder::new("noted-killed");
    let mut records = vec![session(), branch(1, 100)];
    records.extend(hours(1, 100..=2_269, 19));
    records.push(detach(2_269));
    for target in [&whole, &folder] {
        fixtures::write_journal(&target.journal(), 0, CAMPAIGN, &[records.clone()], true).unwrap();
    }
    let taken = std::cell::Cell::new(0_u32);
    let taken = &taken;
    let after = |records: u32| {
        taken.set(0);
        move || {
            taken.set(taken.get() + 1);
            taken.get() >= records
        }
    };
    let note = |segment: u32| RecoveryNote {
        session: Some(SESSION),
        segment,
        valid_bytes: 0,
        reason: "the recorder restarted: recorder 1 stopped while the game ran".into(),
    };

    let mut host = Deciding::everything(true);
    let (mut writer, mut journal, progress) = whole.write_until(0, &mut host, &after(1_000));
    assert_eq!(progress, Progress::Paused);
    let noted = note(writer.segments());
    writer.note(noted.clone()).unwrap();
    assert_eq!(writer.advance(&mut journal, &mut host, &|| false).unwrap(), Progress::Finished);
    drop(writer);
    assert_eq!(newest(&whole.recording()).records().unwrap().notes, [noted.clone()]);
    let expected = fs::read(whole.recording()).unwrap();

    let mut host = Deciding::everything(true);
    let (mut writer, mut journal, progress) = folder.write_until(0, &mut host, &after(1_000));
    assert_eq!(progress, Progress::Paused);
    assert_eq!(note(writer.segments()), noted);
    writer.note(noted.clone()).unwrap();
    let committed = host.committed.len();
    // Killed after the next commit, before its segment was let go of.
    let mut kept = Kept { journal: &mut journal, fail_at: 1, retired: 0 };
    assert!(writer.advance(&mut kept, &mut host, &|| false).is_err());
    drop(writer);
    assert!(host.committed.len() > committed);
    assert_eq!(newest(&folder.recording()).records().unwrap().notes, [noted.clone()]);
    let (writer, _, progress) = folder.write(0, &mut Deciding::everything(true));
    assert_eq!(progress, Progress::Finished);
    drop(writer);
    assert_eq!(newest(&folder.recording()).records().unwrap().notes, [noted]);
    assert!(fs::read(folder.recording()).unwrap() == expected, "the resumed file is the unbroken one");
}

/// A reader of a recording being written reads the revision it opened while blocks are appended
/// after it, and a reader opening it again reads the new blocks too, the earlier ones unchanged.
#[test]
fn a_reader_while_its_recording_is_appended_reads_its_own_revision() {
    let folder = Folder::new("appended");
    let mut records = vec![session(), branch(1, 100)];
    records.extend(hours(1, 100..=1_539, 19));
    records.push(detach(1_539));
    fixtures::write_journal(&folder.journal(), 0, CAMPAIGN, &[records.clone()], true).unwrap();
    let taken = std::cell::Cell::new(0_u32);
    let stop = || {
        taken.set(taken.get() + 1);
        taken.get() >= 900
    };
    let mut host = Deciding::everything(true);
    let (writer, _, progress) = folder.write_until(0, &mut host, &stop);
    assert_eq!(progress, Progress::Paused);
    drop(writer);
    let early = saved(&folder.recording());
    assert_eq!(recorded(&early), 720);
    let first_block = early.block(0, 0).unwrap();
    let (_, _, progress) = folder.write(0, &mut Deciding::everything(true));
    assert_eq!(progress, Progress::Finished);
    // The reader opened before keeps its revision: one block, read as before.
    assert_eq!(early.run(0).unwrap().directory().blocks(), 1);
    assert_eq!(early.block(0, 0).unwrap(), first_block);
    let later = saved(&folder.recording());
    assert_eq!(recorded(&later), 1_440);
    assert_eq!(later.block(0, 0).unwrap(), first_block);
}

/// A branch is decided before anything of it is written. One whose load waits is not written:
/// its records are kept with the writer's resume state and its hours stay in the journal. Once
/// admitted it is written whole; one refused is never written, its hours are counted, and its
/// journal goes. A branch the recording already names is written without asking.
#[test]
fn a_branch_waits_for_its_load_to_be_decided_and_a_refused_one_is_never_written() {
    let folder = Folder::new("admission");
    let mut records = vec![session(), branch(1, 100)];
    records.extend(hours(1, 100..=105, 19));
    records.push(branch(2, 200));
    records.extend(hours(2, 200..=203, 23));
    records.push(branch(3, 300));
    records.extend(hours(3, 300..=302, 29));
    records.push(detach(302));
    fixtures::write_journal(&folder.journal(), 0, CAMPAIGN, &[records], true).unwrap();

    let mut host = Deciding { admitted: [19].into(), refused: [29].into(), saves: true, ..Deciding::default() };
    let (writer, _, progress) = folder.write(0, &mut host);
    let Progress::Undecided(asked) = progress else {
        panic!("{progress:?}");
    };
    assert_eq!((asked.branch, asked.load, asked.qpc, asked.last), (2, Some(23), Some(1_200), false));
    let held = saved(&folder.recording());
    assert_eq!(held.records().unwrap().branches.iter().map(|branch| branch.id).collect::<Vec<_>>(), [1]);
    assert_eq!(recorded_hours(&held, 1), (100..=105).collect::<Vec<_>>());
    assert_eq!(folder.segments(), [(0, 2), (0, 3)], "the waiting branch's hours stay in the journal");
    // The writer is killed while it waits: the branch's start, kept aside, comes back with the
    // resume state.
    drop(writer);

    host.admitted.insert(23);
    host.asked.clear();
    let (writer, _, progress) = folder.write(0, &mut host);
    assert_eq!(progress, Progress::Finished);
    assert!(writer.finished() && folder.segments().is_empty());
    assert_eq!(host.skipped, [(Some(29), 3)]);
    let written = saved(&folder.recording());
    let records = written.records().unwrap();
    assert_eq!(records.branches.iter().map(|branch| branch.id).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(recorded_hours(&written, 2), (200..=203).collect::<Vec<_>>());
    assert_eq!(recorded(&written), 10);
    let witnessed: Vec<Hour> = records
        .controls
        .iter()
        .filter_map(|control| match control {
            ControlRecord::ClockWitness { hour, .. } => Some(*hour),
            _ => None,
        })
        .collect();
    assert_eq!(witnessed, (100..=105).chain(200..=203).collect::<Vec<_>>());

    // The same branch in a later capture is the recording's own: it is written unasked.
    let mut more = vec![session(), branch(1, 100)];
    more.extend(hours(1, 106..=108, 31));
    fixtures::write_journal(&folder.journal(), 1, CAMPAIGN, &[more], true).unwrap();
    let mut host = Deciding { saves: true, ..Deciding::default() };
    let (_, _, progress) = folder.write(1, &mut host);
    assert_eq!(progress, Progress::Finished);
    assert!(host.asked.is_empty(), "{:?}", host.asked);
    assert_eq!(recorded_hours(&saved(&folder.recording()), 1), (100..=108).collect::<Vec<_>>());
}

/// An hour whose complete Campaign table names another campaign than its recording's is refused
/// as it is written, so no reader checks it.
#[test]
fn an_hour_naming_another_campaign_is_refused_as_it_is_written() {
    let folder = Folder::new("campaign");
    let mut records = vec![session(), branch(1, 100)];
    records.extend(hours(1, 100..=103, 19));
    let mut other = frame(1, 104, 19);
    other.tables[TableId::Campaign.index()] = TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
        id: b"another campaign".to_vec(),
    }]));
    records.push(Spooled::Frame(other));
    records.push(detach(104));
    fixtures::write_journal(&folder.journal(), 0, CAMPAIGN, &[records], true).unwrap();
    let mut writer = LiveWriter::open(&folder.recording(), &folder.state(), SOURCE, 0, Settings::LOW, Vec::new()).unwrap();
    let mut journal = Journal::open(&folder.journal(), 0);
    journal.start(writer.segments()).unwrap();
    let error = writer.advance(&mut journal, &mut Deciding::everything(true), &|| false).unwrap_err();
    assert!(error.to_string().contains("campaign identity differs"), "{error}");
}

/// A capture none of whose branches is written leaves no recording, and its journal goes.
#[test]
fn a_capture_of_refused_loads_alone_leaves_no_recording() {
    let folder = Folder::new("refused");
    let mut records = vec![session(), branch(1, 100)];
    records.extend(hours(1, 100..=103, 19));
    records.push(detach(103));
    fixtures::write_journal(&folder.journal(), 0, CAMPAIGN, &[records], true).unwrap();
    let mut host = Deciding { refused: [19].into(), ..Deciding::default() };
    let (writer, _, progress) = folder.write(0, &mut host);
    assert_eq!(progress, Progress::Finished);
    assert!(writer.finished() && !writer.created());
    assert!(!folder.recording().exists() && folder.segments().is_empty());
    assert_eq!(host.skipped, [(Some(19), 4)]);
}

/// Hours that are not saved are in the recording and do not read as it: a reader opens the
/// newest saved revision whatever follows it, a later capture appends after it, and cutting the
/// file back to the saved revision discards what was not.
#[test]
fn unsaved_hours_are_appended_in_place_and_readers_open_the_saved_revision() {
    let folder = Folder::new("saved");
    let path = folder.recording();
    let mut records = vec![session(), branch(1, 100)];
    records.extend(hours(1, 100..=105, 19));
    fixtures::write_journal(&folder.journal(), 0, CAMPAIGN, &[records], true).unwrap();
    let (_, _, progress) = folder.write(0, &mut Deciding::everything(true));
    assert_eq!(progress, Progress::Finished);
    let saved_revision = saved(&path).revision();

    // A later capture goes on in the same file, unsaved.
    let mut more = vec![session(), branch(1, 100)];
    more.extend(hours(1, 106..=109, 19));
    fixtures::write_journal(&folder.journal(), 1, CAMPAIGN, &[more], true).unwrap();
    let (_, _, progress) = folder.write(1, &mut Deciding::everything(false));
    assert_eq!(progress, Progress::Finished);
    // And a block being appended is no revision at all.
    let mut appending = fs::OpenOptions::new().append(true).open(&path).unwrap();
    io::Write::write_all(&mut appending, &[0x5a; 70_000]).unwrap();
    drop(appending);
    let opened = saved(&path);
    assert_eq!((recorded(&opened), opened.revision()), (6, saved_revision));
    let held = newest(&path);
    assert_eq!((recorded(&held), held.saved()), (10, Some(saved_revision)));
    assert_eq!(held.records().unwrap().sessions.len(), 1);
    drop((opened, held));

    // Discarded: the file is its saved revision again, to the byte.
    writer::cut_back(&path, saved_revision).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), saved_revision.length);
    let held = newest(&path);
    assert_eq!((recorded(&held), held.revision()), (6, saved_revision));
}

/// A capture whose producer stopped without finishing it ends where its journal does: its
/// hours are written, the recording says where and why they stop, and its journal goes.
#[test]
fn a_capture_its_producer_left_unfinished_ends_where_its_journal_does() {
    let folder = Folder::new("stopped");
    let path = folder.recording();
    let mut records = vec![session(), branch(1, 100)];
    records.extend(hours(1, 100..=102, 19));
    records.push(branch(2, 200));
    records.extend(hours(2, 200..=201, 19));
    fixtures::write_journal(&folder.journal(), 0, CAMPAIGN, &[records], false).unwrap();
    let mut host = Deciding::everything(false);
    let (writer, _, progress) = folder.write(0, &mut host);
    assert_eq!(progress, Progress::Waiting);
    drop(writer);
    // The first run is committed; the second's hours wait for their block.
    let held = newest(&path);
    assert_eq!(recorded(&held), 3);
    assert!(held.root().state.is_some());
    drop(held);

    let (mut writer, mut journal, progress) = folder.write(0, &mut host);
    assert_eq!(progress, Progress::Waiting);
    assert!(!journal.producing().unwrap());
    writer.finish_stopped(&mut journal, &mut host, "the collector ended before finishing this file").unwrap();
    assert!(writer.finished() && folder.segments().is_empty());
    let written = newest(&path);
    assert_eq!(recorded_hours(&written, 2), [200, 201]);
    let records = written.records().unwrap();
    let [stop] = records.stops.as_slice() else {
        panic!("{:?}", records.stops);
    };
    assert_eq!((stop.branch, stop.last_hour, stop.capture), (2, Some(201), 0));
    assert!(written.root().state.is_none());
}

/// A recording of another format is refused by name by its readers: format 11 by the format-12
/// reader, and format 12 by the format-11 one.
#[test]
fn a_recording_of_another_format_is_refused_by_name() {
    let folder = Folder::new("formats");
    let path = folder.recording();
    let mut older = Vec::new();
    older.extend_from_slice(envelope::MAGIC);
    older.extend_from_slice(&11_u32.to_le_bytes());
    older.extend_from_slice(&[0; 64]);
    fs::write(&path, &older).unwrap();
    let error = Recording::open_path_as(&path, Open::Newest).unwrap_err();
    assert!(error.to_string().contains("format 11"), "{error}");
    fs::remove_file(&path).unwrap();
    let mut records = vec![session(), branch(1, 100)];
    records.extend(hours(1, 100..=103, 19));
    records.push(detach(103));
    fixtures::write_journal(&folder.journal(), 0, CAMPAIGN, &[records], true).unwrap();
    folder.write(0, &mut Deciding::everything(true));
    assert_eq!(crate::columnar_frames::Recording::open_path(&path).unwrap().hours(), 4, "a recording of format 12 reads");
}

/// Another owner's part of each revision: its section's extents and its entry of the resume
/// state are committed with the block's, and the entry comes back when the recording is taken
/// up.
#[test]
fn another_owner_adds_its_extents_and_resume_state_to_each_revision() {
    const KEY: SectionKey = SectionKey::new(SectionKind::FamilyAnswers, 9);

    #[derive(Default)]
    struct Seen {
        hours: u32,
        resumed: Vec<Option<Vec<u8>>>,
    }
    struct Counting(Arc<Mutex<Seen>>);
    impl Part for Counting {
        fn open(&mut self, container: &mut ContainerWriter<File>, state: Option<&StateEntry>) -> io::Result<()> {
            let mut seen = self.0.lock().unwrap();
            seen.resumed.push(state.map(|entry| entry.bytes.clone()));
            if let Some(entry) = state {
                seen.hours = u32::from_le_bytes(entry.bytes[..4].try_into().unwrap());
            }
            container.section(KEY, 1)
        }

        fn hour(&mut self, _: &Applied<'_>) -> io::Result<()> {
            self.0.lock().unwrap().hours += 1;
            Ok(())
        }

        fn revise(&mut self, container: &mut ContainerWriter<File>) -> io::Result<Option<StateEntry>> {
            let hours = self.0.lock().unwrap().hours.to_le_bytes();
            let extent = container.put(Kind::Stored, Tags::default(), 4, &hours)?;
            container.append(KEY, extent)?;
            Ok(Some(StateEntry { owner: KEY, version: 1, bytes: hours.to_vec() }))
        }
    }

    let folder = Folder::new("parts");
    let mut records = vec![session(), branch(1, 100)];
    records.extend(hours(1, 100..=102, 19));
    records.push(branch(2, 200));
    records.extend(hours(2, 200..=201, 19));
    fixtures::write_journal(&folder.journal(), 0, CAMPAIGN, &[records], true).unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let open = || LiveWriter::open(&folder.recording(), &folder.state(), SOURCE, 0, Settings::LOW, vec![Box::new(Counting(Arc::clone(&seen)))]).unwrap();
    let mut host = Deciding::everything(true);
    // Stopped after the first run's revision, then taken up again.
    let mut writer = open();
    let mut journal = Journal::open(&folder.journal(), 0);
    // The capture's first ten records end with its second run's start.
    let commits = std::cell::Cell::new(0);
    let progress = writer
        .advance(&mut journal, &mut host, &|| {
            commits.set(commits.get() + 1);
            commits.get() >= 10
        })
        .unwrap();
    assert_eq!(progress, Progress::Paused);
    assert_eq!(host.committed.len(), 1);
    drop(writer);
    let mut writer = open();
    let mut journal = Journal::open(&folder.journal(), 0);
    journal.start(writer.segments()).unwrap();
    assert_eq!(writer.advance(&mut journal, &mut host, &|| false).unwrap(), Progress::Finished);
    assert_eq!(seen.lock().unwrap().resumed, [None, Some(3_u32.to_le_bytes().to_vec())]);
    let written = saved(&folder.recording());
    let section = written.root().section(KEY).unwrap();
    assert_eq!((section.version, section.len()), (1, 2));
}

/// What a derived writer reading a table [`recorder_derive::framework::Watch::Dropped`] was
/// handed in an hour: each deleted row's key and cells, and each written cell's value before
/// the hour by (key, column).
type Handed = (Vec<(u128, Vec<String>)>, Vec<((u128, u16), String)>);

/// A derived writer of no table, reading one as [`recorder_derive::framework::Watch::Dropped`]:
/// it notes by hour the kept cells it was handed.
struct Probe {
    reads: &'static [recorder_derive::framework::Read],
    seen: Arc<Mutex<BTreeMap<Hour, Handed>>>,
}

impl Probe {
    fn new(table: TableId, seen: &Arc<Mutex<BTreeMap<Hour, Handed>>>) -> Self {
        use recorder_derive::framework::{Columns, Read, Watch};
        let reads = Box::leak(Box::new([Read { table, columns: Columns::Every, watch: Watch::Dropped, required: false }]));
        Self { reads, seen: Arc::clone(seen) }
    }
}

impl recorder_derive::framework::Writer for Probe {
    fn name(&self) -> &'static str {
        "Probe"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [recorder_derive::framework::Output] {
        &[]
    }

    fn reads(&self) -> &'static [recorder_derive::framework::Read] {
        self.reads
    }

    fn start(&mut self) {}

    fn hour(&mut self, hour: &recorder_derive::framework::Hour<'_>, _: &mut recorder_derive::framework::Rows) -> io::Result<()> {
        use replay_columnar::store::{Kept as Cell, Value};
        let table = self.reads[0].table;
        let shown = |kept: &Cell| match kept.value() {
            Value::Text(id) => format!("{:?}", hour.text(id)),
            other => format!("{other:?}"),
        };
        let mut dropped: Vec<(u128, Vec<String>)> = hour.dropped(table).iter().map(|(key, cells)| (*key, cells.iter().map(shown).collect())).collect();
        dropped.sort();
        let deleted: std::collections::HashMap<u32, u128> = hour.changes(table).deleted.iter().copied().collect();
        let view = hour.table(table);
        let mut before = BTreeMap::new();
        for (slot, column, kept) in hour.previous(table) {
            let key = deleted.get(slot).copied().or_else(|| view.key(*slot)).expect("a written cell's row");
            before.entry((key, *column)).or_insert_with(|| shown(kept));
        }
        self.seen.lock().unwrap().insert(hour.hour, (dropped, before.into_iter().collect()));
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

/// One complete hour whose CountryLinks holds three rows from `hour / 10`, each linked to the
/// slot `hour`: every hour writes their cells, every tenth deletes one.
fn linked(branch: BranchId, hour: Hour) -> Frame {
    let mut frame = frame(branch, hour, 19);
    let first = (hour / 10) as u32;
    let links = (first..first + 3).map(|id| crate::rows::CountryLinkRow { id, canonical_slot: hour as u32 }).collect();
    frame.tables[TableId::CountryLinks.index()] = TableUpdate::Snapshot(TableRows::CountryLinks(links));
    frame
}

/// The journal door's writer with a [`Probe`] of CountryLinks, taking records until `stop`.
fn probed(folder: &Folder, seen: &Arc<Mutex<BTreeMap<Hour, Handed>>>, stop: &dyn Fn() -> bool) -> Progress {
    let mut writers = recorder_derive::writers();
    writers.push(Box::new(Probe::new(TableId::CountryLinks, seen)));
    let mut writer = LiveWriter::open_with(&folder.recording(), &folder.state(), SOURCE, 0, Settings::LOW, Vec::new(), writers).unwrap();
    let mut journal = Journal::open(&folder.journal(), 0);
    journal.start(writer.segments()).unwrap();
    writer.advance(&mut journal, &mut Deciding::everything(true), stop).unwrap()
}

/// A writer reading kept cells, stopped in a block and taken up at the commit before, is handed
/// in every hour the cells a writer never stopped is: they are the hour log's, cleared at each
/// hour's start, kept in no resume state, and none is kept in a run's first hour.
#[test]
fn a_writer_reading_kept_cells_taken_up_at_a_commit_is_handed_what_an_unbroken_one_is() {
    let mut records = vec![session(), branch(1, 100)];
    for hour in 100..=1_000 {
        records.push(Spooled::Control(ControlRecord::ClockWitness { session: SESSION, qpc: 1_000 + hour as u64, hour, load_signal: Some(19) }));
        records.push(Spooled::Frame(linked(1, hour)));
    }
    records.push(detach(1_000));
    let (whole, folder) = (Folder::new("kept-unbroken"), Folder::new("kept-killed"));
    for target in [&whole, &folder] {
        fixtures::write_journal(&target.journal(), 0, CAMPAIGN, &[records.clone()], true).unwrap();
    }
    let unbroken = Arc::new(Mutex::new(BTreeMap::new()));
    assert_eq!(probed(&whole, &unbroken, &|| false), Progress::Finished);

    let taken_up = 100 + UNIT_ORDINALS as Hour;
    let seen = Arc::new(Mutex::new(BTreeMap::new()));
    assert_eq!(probed(&folder, &seen, &|| seen.lock().unwrap().contains_key(&(taken_up + 30))), Progress::Paused);
    assert_eq!(recorded(&saved(&folder.recording())), UNIT_ORDINALS as u64, "the first block is committed");
    seen.lock().unwrap().retain(|&hour, _| hour < taken_up);
    assert_eq!(probed(&folder, &seen, &|| false), Progress::Finished);

    let (unbroken, seen) = (unbroken.lock().unwrap(), seen.lock().unwrap());
    assert_eq!(unbroken.len(), 901, "every hour handed");
    assert_eq!(unbroken[&100], (Vec::new(), Vec::new()), "a run's first hour keeps nothing");
    let first = &unbroken[&taken_up];
    assert!(!first.0.is_empty() && !first.1.is_empty(), "the hour taken up at deletes a row and writes others: {first:?}");
    assert_eq!(*seen, *unbroken, "the writer taken up is handed what the unbroken one is");
}

/// A ledger's row counting the rows its run's hours appended as events.
const EVENTS_ROW: u128 = u128::MAX;

/// A reader of the log, kept in its table alone: by ordinal each row it is handed, its key's low
/// bits in its column, a row the log forgot deleted; at [`EVENTS_ROW`] how many rows the run's
/// hours appended as events, none counted of the list at the run's start.
struct Ledger {
    table: &'static [recorder_derive::framework::Output],
    column: usize,
    columns: usize,
}

impl recorder_derive::framework::Writer for Ledger {
    fn name(&self) -> &'static str {
        "Ledger"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [recorder_derive::framework::Output] {
        self.table
    }

    fn reads(&self) -> &'static [recorder_derive::framework::Read] {
        use recorder_derive::framework::{Columns, Read, Watch};
        &[Read { table: TableId::NuclearStrikes, columns: Columns::Every, watch: Watch::Appended, required: false }]
    }

    fn start(&mut self) {}

    fn hour(&mut self, hour: &recorder_derive::framework::Hour<'_>, out: &mut recorder_derive::framework::Rows) -> io::Result<()> {
        use recorder_derive::framework::Value as Set;
        let (log, table) = (TableId::NuclearStrikes, self.table[0].table());
        let forgotten = hour.forgotten(log);
        let gone = |ordinal: u64| forgotten.iter().any(|range| range.contains(&ordinal));
        let cells = |value: u64| {
            let mut cells = vec![Set::Unrecorded; self.columns];
            cells[self.column] = Set::U64(value);
            cells
        };
        let appended = hour.appended(log);
        for row in appended.rows().filter(|row| !gone(row.ordinal)) {
            out.set(table, u128::from(row.ordinal), cells(row.key as u64));
        }
        for ordinal in forgotten.iter().flat_map(Clone::clone) {
            out.delete(table, u128::from(ordinal));
        }
        if !appended.backlog() && !appended.is_empty() {
            let own = out.table(table);
            let counted = own.slot(EVENTS_ROW).and_then(|slot| own.number(self.column, slot)).map_or(0, |count| count as u64);
            out.set(table, EVENTS_ROW, cells(counted + appended.len() as u64));
        }
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

/// Adds the log's reader to `writers`, writing the first state table nothing else of the recorder
/// reads, writes or summarizes that has a U64 column.
fn push_ledger(writers: &mut recorder_derive::Writers) {
    let kinds = schema().unwrap();
    let derived = writers.keyed();
    let summarized: Vec<usize> = crate::summaries::registry().unwrap().iter().map(|(table, _)| *table).collect();
    let logs = crate::rows::logs();
    let (table, column) = (0..kinds.len())
        .filter(|&table| {
            ![TableId::Campaign.index(), TableId::Clock.index()].contains(&table)
                && !derived.contains(&table)
                && !summarized.contains(&table)
                && !logs[table]
                && !keeps_last()[table]
        })
        .find_map(|table| kinds[table].iter().position(|&kind| kind == Column::U64).map(|column| (table, column)))
        .expect("a state table with a U64 column");
    let table: &'static [recorder_derive::framework::Output] = Box::leak(Box::new([recorder_derive::framework::Output::State(TableId::ALL[table])]));
    writers.push(Box::new(Ledger { table, column, columns: kinds[table[0].table().index()].len() }));
}

/// `journal-encode` gives a recording its session's facts as the recorder's publish does: the
/// session's `facts.bin`, found where the recorder's layout puts it beside the journal, gives the
/// pack it holds verbatim when it is the journal's content's; another content's is refused by
/// name; a bare pack named by hand is given as it is.
#[test]
fn an_encoded_journal_carries_its_sessions_facts() {
    use crate::{campaign::{SESSION_FACTS_FILE, encode_session_facts}, encode_report};
    let folder = Folder::new("facts");
    let session_dir = folder.0.join("session");
    let journal = session_dir.join(std::str::from_utf8(CAMPAIGN).unwrap());
    let mut records = vec![session(), branch(1, 100)];
    records.extend(hours(1, 100..=105, 19));
    records.push(detach(105));
    fixtures::write_journal(&journal, 0, CAMPAIGN, &[records], true).unwrap();
    let (campaign, start) = encode_report::journal_start(&journal, 0).unwrap();
    assert_eq!(campaign, CAMPAIGN);
    let Spooled::SessionStart(written) = session() else { unreachable!("a session start") };
    assert_eq!(start, written);

    let file = session_dir.join(SESSION_FACTS_FILE);
    assert_eq!(encode_report::session_facts_file(&journal, &campaign), None, "the session has no facts file yet");
    let pack = b"H4FP the session's pack".to_vec();
    fs::write(&file, encode_session_facts(&start.identity, &pack).unwrap()).unwrap();
    assert_eq!(encode_report::session_facts_file(&journal, &campaign).as_ref(), Some(&file));
    assert_eq!(encode_report::session_facts_file(&journal.join("kept-journal"), &campaign).as_ref(), Some(&file));
    assert_eq!(encode_report::session_facts_file(&session_dir.join("another"), &campaign), None, "a folder not named for the campaign");

    let given = encode_report::facts_pack(&file, fs::read(&file).unwrap(), &start.identity, true).unwrap();
    assert_eq!(given, pack, "the session's pack, verbatim");
    let other = Identity { game_version: "another build".into(), ..start.identity.clone() };
    let refused = encode_report::facts_pack(&file, encode_session_facts(&other, &pack).unwrap(), &start.identity, false).unwrap_err();
    assert!(refused.to_string().ends_with("facts.bin is another content's facts"), "{refused}");
    assert_eq!(encode_report::facts_pack(&file, pack.clone(), &start.identity, false).unwrap(), pack, "a bare pack, as it is");
    let not_session = encode_report::facts_pack(&file, pack.clone(), &start.identity, true).unwrap_err();
    assert!(not_session.to_string().ends_with("facts.bin is not a session's facts file"), "{not_session}");

    let mut writer = LiveWriter::open_with(&folder.recording(), &folder.0.join("recording.state"), SOURCE, 0, Settings::LOW, Vec::new(), recorder_derive::writers()).unwrap();
    let mut source = Journal::open_read_only(&journal, 0);
    source.start(writer.segments()).unwrap();
    let mut host = encode_report::Everything::new(Some(given), None);
    assert_eq!(writer.advance(&mut source, &mut host, &|| false).unwrap(), Progress::Finished);
    let log = saved(&folder.recording());
    let [extent] = log.facts_extents() else { panic!("one facts pack, not {}", log.facts_extents().len()) };
    assert_eq!(log.stored_bytes(extent).unwrap(), pack, "the recording's facts are the session's pack, byte for byte");
}

/// Writers reading definitions of the recorded content, through the journal door.
mod definitions {
    use super::*;
    use replay_columnar::store::Value;

    /// The runs' first hour.
    const FIRST: Hour = 100;

    /// The define the leveled writer requires.
    const LEVEL: recorder_derive::framework::definitions::Define =
        recorder_derive::framework::definitions::Define { section: "NTest", name: "LEVEL", form: recorder_derive::framework::definitions::Form::Int32 };

    /// The test's facts pack: the level's digits; read by [`levels`] in place of the game facts'
    /// readers.
    const PACK: &[u8] = b"7";

    fn levels(facts: &recorder_derive::Facts, _: &Identity, _: &[recorder_derive::framework::Definition]) -> recorder_derive::Definitions {
        let mut pack = vec![0; facts.length as usize];
        let level = facts.read_at(0, &mut pack).ok().and_then(|()| std::str::from_utf8(&pack).ok()?.parse::<i64>().ok()).ok_or_else(|| "not a level".to_owned());
        recorder_derive::Definitions::read_none().with_define(LEVEL, level)
    }

    /// The table the leveled writer writes and its U64 column: the last state table nothing else
    /// of the recorder reads, writes or summarizes that has one.
    fn level_table() -> (usize, usize) {
        let kinds = schema().unwrap();
        let derived = recorder_derive::writers().keyed();
        let summarized: Vec<usize> = crate::summaries::registry().unwrap().iter().map(|(table, _)| *table).collect();
        let logs = crate::rows::logs();
        (0..kinds.len())
            .rev()
            .filter(|&table| {
                ![TableId::Campaign.index(), TableId::Clock.index()].contains(&table) && !derived.contains(&table) && !summarized.contains(&table) && !logs[table] && !keeps_last()[table]
            })
            .find_map(|table| kinds[table].iter().position(|&kind| kind == Column::U64).map(|column| (table, column)))
            .expect("a state table with a U64 column")
    }

    /// A writer requiring the define [`LEVEL`]: each hour one row holding the level in millions
    /// plus the hour, the other columns not recorded.
    struct Leveled {
        table: &'static [recorder_derive::framework::Output],
        column: usize,
        columns: usize,
    }

    impl recorder_derive::framework::Writer for Leveled {
        fn name(&self) -> &'static str {
            "Leveled"
        }

        fn version(&self) -> u32 {
            1
        }

        fn tables(&self) -> &'static [recorder_derive::framework::Output] {
            self.table
        }

        fn reads(&self) -> &'static [recorder_derive::framework::Read] {
            &[]
        }

        // Its row holds the hour.
        fn hourly(&self) -> bool {
            true
        }

        fn definitions(&self) -> &'static [recorder_derive::framework::Uses] {
            &[recorder_derive::framework::Uses { definition: recorder_derive::framework::Definition::Define(LEVEL), required: true }]
        }

        fn start(&mut self) {}

        fn hour(&mut self, hour: &recorder_derive::framework::Hour<'_>, out: &mut recorder_derive::framework::Rows) -> io::Result<()> {
            use recorder_derive::framework::Value as Set;
            let level = hour.definitions().define(LEVEL.section, LEVEL.name).map_err(|why| invalid(why.to_owned()))?;
            let mut cells = vec![Set::Unrecorded; self.columns];
            cells[self.column] = Set::U64(level as u64 * 1_000_000 + hour.hour as u64);
            out.set(self.table[0].table(), 0, cells);
            Ok(())
        }

        fn save(&self, _: &mut Bytes) -> io::Result<()> {
            Ok(())
        }

        fn load(&mut self, _: &[u8]) -> io::Result<()> {
            Ok(())
        }
    }

    /// `writers` with the leveled writer after them, reading definitions with [`levels`].
    fn leveled(mut writers: recorder_derive::Writers) -> recorder_derive::Writers {
        let (table, column) = level_table();
        let columns = schema().unwrap()[table].len();
        writers.push(Box::new(Leveled { table: Box::leak(Box::new([recorder_derive::framework::Output::State(TableId::ALL[table])])), column, columns }));
        writers.read_definitions_with(levels);
        writers
    }

    /// A host writing everything that offers the session's facts pack or none.
    fn offering(facts: bool) -> Deciding {
        Deciding { facts: facts.then(|| PACK.to_vec()), ..Deciding::everything(true) }
    }

    /// The leveled writer's row at each block's end of the recording at `path`: complete, the
    /// level from the facts at every hour.
    fn assert_leveled(path: &Path, blocks: u32) {
        let written = saved(path);
        let run = written.directories().unwrap()[0].clone();
        let pages = written.unit_pages(0, 0, run.blocks()).unwrap();
        let kinds = schema().unwrap();
        let (table, column) = level_table();
        for block in 0..blocks {
            let end = FIRST + ((block + 1) * UNIT_ORDINALS) as i32 - 1;
            let (cells, _) = reader::rebuild(written.extents(), &pages, &kinds[table], table, block as usize, FIRST).unwrap();
            let read: Vec<Value<'_>> = cells.rows().map(|slot| cells.value(column, slot)).collect();
            assert_eq!(read, [Value::Number(7_000_000 + end as u64)], "the leveled row at block {block}'s end");
        }
    }

    /// Through the journal door, a writer requiring a definition killed in the middle of its
    /// second block is taken up with its definitions from the recording's own facts (its host
    /// offers none any more), and the recording ends byte for byte the unbroken one.
    #[test]
    fn a_killed_writer_reading_a_definition_takes_it_up_from_its_recording() {
        let whole = Folder::new("defined-unbroken");
        let folder = Folder::new("defined-killed");
        let last = FIRST + 3 * UNIT_ORDINALS as i32 + 49;
        let mut records = vec![session(), branch(1, FIRST)];
        records.extend(hours(1, FIRST..=last, 19));
        records.push(detach(last));
        for target in [&whole, &folder] {
            fixtures::write_journal(&target.journal(), 0, CAMPAIGN, &[records.clone()], true).unwrap();
        }
        let write = |folder: &Folder, host: &mut Deciding, stop: &dyn Fn() -> bool| {
            let mut writer = LiveWriter::open_with(&folder.recording(), &folder.state(), SOURCE, 0, Settings::LOW, Vec::new(), leveled(recorder_derive::writers())).unwrap();
            let mut journal = Journal::open(&folder.journal(), 0);
            journal.start(writer.segments()).unwrap();
            writer.advance(&mut journal, host, stop).unwrap()
        };
        assert_eq!(write(&whole, &mut offering(true), &|| false), Progress::Finished);
        let expected = fs::read(whole.recording()).unwrap();
        assert_leveled(&whole.recording(), 3);

        let taken = std::cell::Cell::new(0_u32);
        let stop = || {
            taken.set(taken.get() + 1);
            taken.get() >= 1_000
        };
        assert_eq!(write(&folder, &mut offering(true), &stop), Progress::Paused);
        assert_eq!(recorded(&saved(&folder.recording())), UNIT_ORDINALS as u64, "killed in the second block");
        assert_eq!(write(&folder, &mut offering(false), &|| false), Progress::Finished);
        let resumed = fs::read(folder.recording()).unwrap();
        let differs = resumed.iter().zip(&expected).position(|(a, b)| a != b);
        assert!(resumed == expected, "the resumed file is the unbroken one: {} bytes against {}, first differing at {differs:?}", resumed.len(), expected.len());
    }

    /// A session folder's facts file in `folder` holding `pack`, as the recorder packs it.
    fn facts_file(folder: &Folder, pack: &[u8]) -> PathBuf {
        let Spooled::SessionStart(start) = session() else { unreachable!("the session's start") };
        let path = folder.0.join(crate::campaign::SESSION_FACTS_FILE);
        fs::write(&path, crate::campaign::encode_session_facts(&start.identity, pack).unwrap()).unwrap();
        path
    }

    /// A recorder restarted while the game runs takes the session over: it opens the recording
    /// at its newest revision as a killed one does, and packs the session's facts file again.
    /// A writer reading a definition, taken over, ends byte for byte the unbroken file both
    /// ways: when the recording carries facts, the definitions come from the recording's own
    /// pack, read from its file, though the repacked file differs; when it carries none yet,
    /// from the repacked file, read at its place a piece at a time, from the hour it is there.
    #[test]
    fn a_writer_reading_a_definition_taken_over_reads_the_recording_s_facts_or_the_repacked_file() {
        let last = FIRST + 3 * UNIT_ORDINALS as i32 + 49;
        let mut records = vec![session(), branch(1, FIRST)];
        records.extend(hours(1, FIRST..=last, 19));
        records.push(detach(last));
        let write = |folder: &Folder, host: &mut Deciding, stop: &dyn Fn() -> bool| {
            let mut writer = LiveWriter::open_with(&folder.recording(), &folder.state(), SOURCE, 0, Settings::LOW, Vec::new(), leveled(recorder_derive::writers())).unwrap();
            let mut journal = Journal::open(&folder.journal(), 0);
            journal.start(writer.segments()).unwrap();
            writer.advance(&mut journal, host, stop).unwrap()
        };
        let taken = std::cell::Cell::new(0_u32);
        let stop = || {
            taken.set(taken.get() + 1);
            taken.get() >= 1_000
        };
        // The facts arrive in the third block, after the take-over's commit, or from the start.
        let late = FIRST + 2 * UNIT_ORDINALS as i32 + 100;
        for arrives in [None, Some(late)] {
            let (whole, folder) = (Folder::new("taken-over-unbroken"), Folder::new("taken-over"));
            for target in [&whole, &folder] {
                fixtures::write_journal(&target.journal(), 0, CAMPAIGN, &[records.clone()], true).unwrap();
            }
            let by_file = |folder: &Folder, pack: &[u8]| Deciding { facts_file: Some(facts_file(folder, pack)), facts_after: arrives, ..Deciding::everything(true) };
            assert_eq!(write(&whole, &mut by_file(&whole, PACK), &|| false), Progress::Finished);
            let expected = fs::read(whole.recording()).unwrap();

            taken.set(0);
            assert_eq!(write(&folder, &mut by_file(&folder, PACK), &stop), Progress::Paused);
            assert_eq!(recorded(&saved(&folder.recording())), UNIT_ORDINALS as u64, "stopped in the second block");
            assert_eq!(saved(&folder.recording()).facts_extents().is_empty(), arrives.is_some());
            // Repacked: when the recording carries facts, a file holding another pack is not read.
            let repacked = if arrives.is_some() { PACK } else { b"9".as_slice() };
            assert_eq!(write(&folder, &mut by_file(&folder, repacked), &|| false), Progress::Finished);
            let resumed = fs::read(folder.recording()).unwrap();
            let differs = resumed.iter().zip(&expected).position(|(a, b)| a != b);
            assert!(resumed == expected, "facts arriving {arrives:?}: the taken-over file is the unbroken one: {} bytes against {}, first differing at {differs:?}", resumed.len(), expected.len());
            if arrives.is_none() {
                assert_leveled(&folder.recording(), 3);
            }
        }
    }
}
