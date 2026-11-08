//! Writes a campaign's recording: created once, then each save added as a revision. A save
//! copies the pages its captures' files published since the last one, byte for byte, into the
//! runs they continue or new ones: each page and run directory is written again for its place
//! in this file, every block and restart as it is, so their digests and the run's chain hold.
//! The records the save adds and a game facts pack go in with them, and a new root names it all.

use super::{Replay, Revision, records::Records, records::RunSource};
use crate::{columnar_frames::Recording, format::FormatVersion};
use replay_columnar::{
    container::{ContainerWriter, SectionKey, Tags, schema},
    envelope::{self, Extent, Kind, RunDirectory},
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// One run of the file, as its directory names it.
struct HeldRun {
    directory: Option<Extent>,
    branch: u64,
    epoch: u8,
    first_hour: i32,
    ordinals: u32,
    chain: [u8; 32],
    pages: Vec<Extent>,
    /// Pages were added since its directory was written.
    dirty: bool,
}

/// What [`CampaignWriter::ingest`] added from one capture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ingest {
    pub runs: u32,
    pub pages: u32,
    pub hours: u64,
    pub stored: u64,
}

pub struct CampaignWriter<W: Write + Seek> {
    container: ContainerWriter<W>,
    level: i32,
    runs: Vec<HeldRun>,
    /// Where each run came from: the file's, then those this save adds.
    sources: Vec<RunSource>,
    pending: Records,
    /// A write failed partway: what the file holds past its last revision is not to be named.
    poisoned: bool,
}

impl<W: Write + Seek> CampaignWriter<W> {
    /// A new recording at `file`'s start, of `identity`, its extents compressed at `level`.
    pub fn create(file: W, identity: envelope::Identity, level: i32) -> io::Result<Self> {
        Self::with(ContainerWriter::create(file, identity, level, None)?, level, Vec::new(), Vec::new())
    }

    fn with(mut container: ContainerWriter<W>, level: i32, runs: Vec<HeldRun>, sources: Vec<RunSource>) -> io::Result<Self> {
        container.section(SectionKey::WORLD_LOG, schema::WORLD_LOG)?;
        container.section(SectionKey::RECORDS, schema::RECORDS)?;
        container.section(SectionKey::FACTS, schema::FACTS)?;
        Ok(Self {
            container,
            level,
            runs,
            sources,
            pending: Records::default(),
            poisoned: false,
        })
    }

    /// The file it writes.
    pub fn into_inner(self) -> W {
        self.container.into_inner()
    }

    /// Whether this save adds anything yet.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty() && !self.runs.iter().any(|run| run.dirty)
    }

    /// Each run the file holds, this save's pages included: its branch, first hour and hours.
    pub fn runs(&self) -> impl Iterator<Item = (u64, i32, u32)> + '_ {
        self.runs.iter().map(|run| (run.branch, run.first_hour, run.ordinals))
    }

    /// The game facts packs the file carries, these saves' included.
    pub fn facts(&self) -> usize {
        self.container.section_of(SectionKey::FACTS).map_or(0, |section| section.len() as usize)
    }

    /// Adds `records` to the save.
    pub fn add_records(&mut self, records: &mut Records) -> io::Result<()> {
        self.pending.append(records)
    }

    /// Adds game facts pack `pack` to the save.
    pub fn add_facts(&mut self, pack: &[u8]) -> io::Result<()> {
        self.usable()?;
        let result = (|| {
            // Stored as it is: a pack is compressed within, and read at its offsets.
            let extent = self.append(Kind::Facts, 0, 0, 0, 0, pack.len(), pack)?;
            self.container.append(SectionKey::FACTS, extent)
        })();
        self.poison(result)
    }

    /// Adds every page of `capture`'s runs of the branches `keep` keeps that the file does not
    /// hold yet: capture `number` written in session spool `spool`. A run it continues must
    /// hold exactly the pages the file holds of it, first.
    pub fn ingest(
        &mut self,
        capture: &Recording,
        spool: &str,
        number: u32,
        keep: &dyn Fn(u64) -> bool,
    ) -> io::Result<Ingest> {
        self.usable()?;
        let result = self.copy(capture, spool, number, keep);
        self.poison(result)
    }

    fn copy(
        &mut self,
        capture: &Recording,
        spool: &str,
        number: u32,
        keep: &dyn Fn(u64) -> bool,
    ) -> io::Result<Ingest> {
        if let Some(damage) = capture.damage().first() {
            return Err(invalid(format!(
                "A capture that cannot be read whole is not saved: {damage}"
            )));
        }
        let identity = self.container.identity();
        if capture.root_identity().schema != identity.schema
            || !capture.campaign().eq_ignore_ascii_case(&identity.campaign)
            || capture.root_identity().capture_mode != identity.capture_mode
        {
            return Err(invalid(
                "A capture of another campaign, capture mode or schema than its recording's",
            ));
        }
        let mut added = Ingest::default();
        for at in 0..capture.run_count() {
            let run = capture.run(at)?;
            if run.pages() == 0 || !keep(run.branch) {
                continue;
            }
            let held = self
                .sources
                .iter()
                .find(|source| source.spool == spool && source.capture == number && source.capture_run == at)
                .map(|source| source.run as usize);
            let target = match held {
                Some(target) => {
                    let into = self
                        .runs
                        .get(target)
                        .ok_or_else(|| invalid("A run source names a run the recording does not hold"))?;
                    if into.branch != run.branch || into.first_hour != run.first_hour {
                        return Err(invalid("A capture's run differs from the run it continues"));
                    }
                    let copied = into.pages.len();
                    if copied > run.pages() {
                        return Err(invalid("A capture holds fewer pages than its recording holds of it"));
                    }
                    if copied > 0 && capture.page(at, copied - 1)?.chain != into.chain {
                        return Err(invalid("A capture's pages differ from those its recording holds"));
                    }
                    target
                }
                None => {
                    let target = self.runs.len();
                    let run_index = u32::try_from(target).map_err(|_| invalid("Runs pass 2^32"))?;
                    engine_allocation::reserve(&mut self.runs, 1)?;
                    self.runs.push(HeldRun {
                        directory: None,
                        branch: run.branch,
                        epoch: run.epoch,
                        first_hour: run.first_hour,
                        ordinals: 0,
                        chain: [0; 32],
                        pages: Vec::new(),
                        dirty: true,
                    });
                    let source = RunSource {
                        run: run_index,
                        spool: engine_allocation::string(spool)?,
                        capture: number,
                        capture_run: at,
                    };
                    engine_allocation::reserve(&mut self.sources, 1)?;
                    self.sources.push(source.clone());
                    engine_allocation::reserve(&mut self.pending.sources, 1)?;
                    self.pending.sources.push(source);
                    added.runs += 1;
                    target
                }
            };
            let run_index = target as u32;
            let from = self.runs[target].pages.len();
            for page_at in from..run.pages() {
                let mut page = capture.page(at, page_at)?;
                let mut copy = |extent: &mut Extent| -> io::Result<()> {
                    let stored = capture.stored_bytes(extent)?;
                    let raw = extent.raw as usize;
                    *extent = self.append(extent.kind, run_index, extent.stream, extent.first, extent.hours, raw, &stored)?;
                    added.stored += extent.stored;
                    Ok(())
                };
                page.run = run_index;
                copy(&mut page.records)?;
                for table in &mut page.tables {
                    copy(table)?;
                }
                if let Some(restart) = &mut page.restart {
                    copy(&mut restart.chunk)?;
                    for table in &mut restart.tables {
                        copy(&mut table.extent)?;
                    }
                }
                let bytes = page.write()?;
                let extent = self.extent(Kind::IndexPage, run_index, page.first, page.hours, &bytes)?;
                let held = &mut self.runs[target];
                engine_allocation::reserve(&mut held.pages, 1)?;
                held.pages.push(extent);
                held.chain = page.chain;
                held.ordinals = page.first + page.hours;
                held.dirty = true;
                added.pages += 1;
                added.hours += u64::from(page.hours);
            }
            let held = &self.runs[target];
            if held.ordinals != run.ordinals || held.chain != run.chain {
                return Err(invalid("A capture's run directory differs from its pages"));
            }
        }
        Ok(added)
    }

    /// Publishes the save as the next revision: the directories of the runs it added to, its
    /// records, and a root naming everything the file holds. `sync` makes what is written
    /// durable before what names it. A failure leaves the file at its last revision.
    pub fn revise(&mut self, sync: impl FnMut(&mut W) -> io::Result<()>) -> io::Result<Revision> {
        self.revise_with(sync, |_| Ok(()))
    }

    /// [`Self::revise`], `prepare` told the revision once everything but its footer is durable,
    /// before the footer names it: a crash after `prepare` leaves the file at either revision.
    pub fn revise_with(
        &mut self,
        mut sync: impl FnMut(&mut W) -> io::Result<()>,
        prepare: impl FnOnce(Revision) -> io::Result<()>,
    ) -> io::Result<Revision> {
        self.usable()?;
        let result = self.publish(&mut sync, prepare);
        self.poison(result)
    }

    fn publish(
        &mut self,
        sync: &mut dyn FnMut(&mut W) -> io::Result<()>,
        prepare: impl FnOnce(Revision) -> io::Result<()>,
    ) -> io::Result<Revision> {
        if !self.pending.is_empty() {
            let bytes = self.pending.encode(FormatVersion::CURRENT)?;
            let extent = self.extent(Kind::Controls, 0, 0, 0, &bytes)?;
            self.container.append(SectionKey::RECORDS, extent)?;
        }
        let mut directories = engine_allocation::vec(self.runs.len())?;
        for at in 0..self.runs.len() {
            let held = &self.runs[at];
            let directory = match held.directory {
                Some(extent) if !held.dirty => extent,
                _ => {
                    let bytes = RunDirectory {
                        branch: held.branch,
                        epoch: held.epoch,
                        first_hour: held.first_hour,
                        ordinals: held.ordinals,
                        chain: held.chain,
                        pages: engine_allocation::copy_vec(&held.pages)?,
                    }
                    .write(at as u32)?;
                    self.extent(Kind::RunDirectory, at as u32, 0, 0, &bytes)?
                }
            };
            directories.push(directory);
        }
        self.container.set_open(SectionKey::WORLD_LOG, &directories)?;
        let revision = self.container.commit_with(None, sync, prepare)?;
        // Named now: the next save writes only what changes after this.
        for (held, directory) in self.runs.iter_mut().zip(directories) {
            held.directory = Some(directory);
            held.dirty = false;
        }
        self.pending = Records::default();
        Ok(revision)
    }

    fn usable(&self) -> io::Result<()> {
        if self.poisoned {
            return Err(io::Error::other(
                "A write to the recording failed partway; it is rolled back to its last revision",
            ));
        }
        Ok(())
    }

    fn poison<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    /// `raw`, compressed, appended as an extent.
    fn extent(&mut self, kind: Kind, run: u32, first: u32, hours: u32, raw: &[u8]) -> io::Result<Extent> {
        let tags = Tags {
            run,
            first,
            hours,
            ..Tags::default()
        };
        self.container.put_zstd(kind, tags, raw, self.level)
    }

    #[allow(clippy::too_many_arguments)]
    fn append(
        &mut self,
        kind: Kind,
        run: u32,
        stream: u32,
        first: u32,
        hours: u32,
        raw: usize,
        stored: &[u8],
    ) -> io::Result<Extent> {
        self.container.put(kind, Tags { run, stream, first, hours }, raw as u64, stored)
    }
}

impl<W: Write + Seek> CampaignWriter<W> {
    /// Adds saves after `replay`'s last revision, which `file` holds through its end, positioned
    /// there.
    pub(crate) fn resumed(replay: &Replay, file: W, level: i32) -> io::Result<Self> {
        replay.require_undamaged()?;
        let recording = replay.recording();
        let root = recording.root();
        let directories = recording.run_directories();
        let mut runs = engine_allocation::vec(directories.len())?;
        for (at, directory) in (0..).zip(directories) {
            let run = recording.run(at)?;
            runs.push(HeldRun {
                directory: Some(*directory),
                branch: run.branch,
                epoch: run.epoch,
                first_hour: run.first_hour,
                ordinals: run.ordinals,
                chain: run.chain,
                pages: engine_allocation::copy_vec(run.page_extents())?,
                dirty: false,
            });
        }
        let container = ContainerWriter::resume(file, root.clone(), replay.saved_length(), level, None)?;
        Self::with(container, level, runs, replay.sources.clone())
    }
}

impl CampaignWriter<File> {
    /// Takes up the recording `replay` opened at `path`, at its revision: what follows it is cut.
    pub fn resume(replay: &Replay, path: &Path, level: i32) -> io::Result<Self> {
        replay.require_undamaged()?;
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        if file.seek(SeekFrom::End(0))? < replay.saved_length() {
            return Err(invalid("The recording is shorter than its last revision"));
        }
        check_revision(&mut file, replay.revision())?;
        file.set_len(replay.saved_length())?;
        file.seek(SeekFrom::Start(replay.saved_length()))?;
        Self::resumed(replay, file, level)
    }
}

/// Whether the file at `path` holds `revision`: its footer there names that root.
pub fn holds_revision(path: &Path, revision: Revision) -> io::Result<bool> {
    let mut file = File::open(path)?;
    if file.seek(SeekFrom::End(0))? < revision.length {
        return Ok(false);
    }
    match check_revision(&mut file, revision) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => Ok(false),
        Err(error) => Err(error),
    }
}

/// Refuses a file whose footer ending at `revision.length` does not name its root.
fn check_revision(file: &mut File, revision: Revision) -> io::Result<()> {
    let start = revision
        .length
        .checked_sub(envelope::FOOTER_BYTES as u64)
        .ok_or_else(|| invalid("A revision ends before its footer"))?;
    let mut footer = [0; envelope::FOOTER_BYTES];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut footer)?;
    if &footer[32..] != envelope::FOOTER_MAGIC || footer[16..32] != revision.root {
        return Err(invalid("The recording does not hold the revision"));
    }
    Ok(())
}

/// Cuts the recording at `path` back to `revision`, which it must hold: every later save goes.
pub fn roll_back(path: &Path, revision: Revision) -> io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    check_revision(&mut file, revision)?;
    file.set_len(revision.length)?;
    file.sync_all()
}

/// Copies the recording `replay` opened through its revision to `destination`, a new file.
pub fn copy_saved(replay: &Replay, source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::copy(source, destination)?;
    let file = OpenOptions::new().read(true).write(true).open(destination)?;
    let mut held = file;
    check_revision(&mut held, replay.revision())?;
    held.set_len(replay.saved_length())?;
    held.sync_all()
}
