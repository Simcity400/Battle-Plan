//! A columnar recording file's world log (format 12, [`replay_columnar::pages`]) and records, in
//! its container ([`replay_columnar::container`]), written run by run as the recorder's store
//! closes them ([`replay_columnar::unit`]): each table part's frames and each restore's chunks the
//! moment they are compressed ([`PartSink`]), then at each block's end its clock, the strings its
//! hours appended, its tables' summary pieces and its page; every [`SPAN_BLOCKS`] blocks a span's
//! directory and index naming them table by table. A revision publishes everything written so far
//! under a new root; the file ends with its root and footer.

use engine_allocation::zstd::{Compressor, Holder};
use replay_columnar::{
    container::{ContainerWriter, Framing, Root, SectionKey, StateEntry, StateFile, Tags, schema},
    envelope::{self, Extent, RowPlace, SPAN_BLOCKS, SpanPages, UNIT_ORDINALS},
    leaf::{Reader, Writer as Bytes},
    pages::{Block, BlockHead, Entry, RunDirectory, STRING_CHUNK, SpanDirectory, SpanRow, StringChunk},
    state::{Column, Table},
    unit::{BlockPage, FrameEntry, PartSink},
};
use std::{
    ffi::{OsStr, OsString},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

/// The longest name [`capture_name`] writes.
pub const CAPTURE_NAME_BYTES: usize = "columnar.4294967295.h4col.partial".len();

/// The name in its campaign's spool directory of the session's `n`th capture's columnar
/// recording, counting from 0: `columnar.<n>.h4col`, `.partial` appended while the recorder
/// writes it. Written into `out`, so it is named without allocating.
pub fn capture_name(n: u32, partial: bool, out: &mut [u8; CAPTURE_NAME_BYTES]) -> &str {
    let mut cursor = io::Cursor::new(&mut out[..]);
    let suffix = if partial { ".partial" } else { "" };
    write!(cursor, "columnar.{n}.h4col{suffix}").expect("the longest name fits");
    let length = cursor.position() as usize;
    std::str::from_utf8(&out[..length]).expect("ASCII")
}

/// The capture a spool directory's file `name` holds ([`capture_name`]), and whether it is
/// still partial.
pub fn capture_of(name: &OsStr) -> Option<(u32, bool)> {
    let name = name.to_str()?.strip_prefix("columnar.")?;
    let (number, partial) = match name.strip_suffix(".h4col.partial") {
        Some(number) => (number, true),
        None => (name.strip_suffix(".h4col")?, false),
    };
    Some((counted(number)?, partial))
}

/// The folder of `replay_dir` holding `campaign`'s published columnar recordings: its id's
/// bytes, ASCII lower-cased as recordings are matched, in hex.
pub fn campaign_folder(replay_dir: &Path, campaign: &[u8]) -> PathBuf {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut name = String::with_capacity(2 * campaign.len());
    for byte in campaign.iter().map(u8::to_ascii_lowercase) {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 15)]));
    }
    replay_dir.join(name)
}

/// The published name of `session`'s `n`th capture in its campaign's folder, counting from 0:
/// `<session dir>.<n>.h4col`.
pub fn published_name(session: &OsStr, n: u32) -> OsString {
    let mut name = session.to_os_string();
    name.push(format!(".{n}.h4col"));
    name
}

/// The session dir and capture a published name holds ([`published_name`]).
pub fn published_of(name: &OsStr) -> Option<(&str, u32)> {
    let (session, number) = name.to_str()?.strip_suffix(".h4col")?.rsplit_once('.')?;
    Some((session, counted(number)?)).filter(|(session, _)| !session.is_empty())
}

/// Whether the recordings at `a` and `b` are one: of one length and ending in one footer, which
/// names a root whose digest covers everything it names.
pub fn same_recording(a: &Path, b: &Path) -> io::Result<bool> {
    let end = |path: &Path| -> io::Result<(u64, [u8; envelope::FOOTER_BYTES])> {
        let mut file = std::fs::File::open(path)?;
        let length = file.seek(SeekFrom::End(0))?;
        let mut footer = [0; envelope::FOOTER_BYTES];
        let held = length.min(envelope::FOOTER_BYTES as u64) as usize;
        file.seek(SeekFrom::Start(length - held as u64))?;
        file.read_exact(&mut footer[..held])?;
        Ok((length, footer))
    };
    Ok(end(a)? == end(b)?)
}

/// A capture number as names write it: decimal, without leading zeros.
fn counted(number: &str) -> Option<u32> {
    let valid = number.bytes().all(|byte| byte.is_ascii_digit())
        && (number == "0" || !number.starts_with('0'));
    number.parse().ok().filter(|_| valid)
}

/// The compression a writer stores a recording with ([`crate::compression`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub level: i32,
}

impl Settings {
    /// Smaller files, more of the recorder's CPU while it writes: zstd 19.
    pub const HIGH: Self = Self { level: 19 };
    /// Larger files, less of the recorder's CPU while it writes: zstd 7, the last level before
    /// bytes stop falling much for their CPU.
    pub const LOW: Self = Self { level: 7 };
}

static WRITER: Holder = Holder::new("zstd: columnar writer");

/// One table's restart state as the collector's frame writer keeps it.
pub struct RestartState {
    pub bytes: Vec<u8>,
    /// The run's dictionary ids the state references.
    pub ids: Vec<u32>,
}

/// Stored bytes a writer wrote, by what they hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Written {
    /// Table parts' frames and restores' chunks, as the unit's encoder handed them.
    pub frames: u64,
    pub clock: u64,
    pub strings: u64,
    pub summaries: u64,
    /// Block pages, span directories and indexes, run directories.
    pub pages: u64,
    /// Roots.
    pub roots: u64,
}

/// The run being written.
struct Run {
    run: u32,
    branch: u64,
    epoch: u8,
    first_hour: i32,
    hours: u32,
    chain: [u8; 32],
    spans: Vec<SpanPages>,
    /// The open span's block pages and the blocks they hold.
    open: Vec<Extent>,
    blocks: Vec<Block>,
    /// The summary pieces covering its last span when not whole, given as it ends.
    tail: Vec<(u32, Extent)>,
}

/// The run under way, as its resume state names it ([`FileWriter::open_run`]).
#[derive(Clone, Copy, Debug)]
pub struct RunCursor {
    pub run: u32,
    pub hours: u32,
    pub first_hour: i32,
    pub branch: u64,
}

/// Where a writer takes up a file it wrote before ([`FileWriter::resume`]): the root it takes up
/// and, when its last run may go on, that run.
pub struct Resumed {
    pub root: Root,
    /// That root's revision: its number, the file's length through its footer and its digest.
    pub at: replay_columnar::container::Revision,
    /// The branch and hour of the last hour the file holds.
    pub last: Option<(u64, i32)>,
    pub open: Option<OpenRun>,
}

/// A run a writer takes up: its directory and its open span's blocks.
pub struct OpenRun {
    pub run: u32,
    pub directory: RunDirectory,
    pub blocks: Vec<Block>,
}

/// What a block's end gives the file besides the parts its encoder wrote: its clock, the strings
/// its hours appended, each summarized table's piece of it and, when it closes its span, the
/// span's pieces of the levels above (table, level, piece).
pub struct BlockEnd<'a> {
    pub clock: &'a [u8],
    /// `count | per string: length | bytes`, and the first string's id.
    pub strings: (&'a [u8], u32),
    pub summaries: Vec<(u32, Vec<u8>)>,
    pub span: Vec<(u32, u8, Vec<u8>)>,
}

/// Writes a columnar recording's world log into its container: the frames of the recorder's
/// unit as its encoder hands them, and the pages naming them.
pub struct FileWriter<W: Write + Seek + Read> {
    container: ContainerWriter<W>,
    /// Its own frames' context, used only while no encoder's thread is given.
    compressor: Compressor,
    frames: Option<replay_columnar::unit::FrameCompressor>,
    tables: usize,
    run: Option<Run>,
    /// The finished runs' directories.
    runs: Vec<Extent>,
    /// The encoder's page of the block under way, waiting for the block's end.
    page: Option<BlockPage>,
    /// A write failed partway: what the file holds no longer matches what was given.
    poisoned: bool,
    written: Written,
    /// The branch and hour of the last hour written.
    last: Option<(u64, i32)>,
    /// Changes each block's page before it is written, for tests of damage.
    #[cfg(any(test, feature = "test-support"))]
    tamper: Option<Box<dyn Fn(&mut Block) + Send>>,
}

impl<W: Write + Seek + Read> PartSink for FileWriter<W> {
    fn frame(&mut self, stored: &[u8]) -> io::Result<u64> {
        self.usable()?;
        let run = self.run.as_ref().map_or(0, |run| run.run);
        let tags = Tags { run, ..Tags::default() };
        let result = self.container.put(envelope::Kind::Block, tags, 0, stored).map(|extent| extent.offset);
        self.written.frames += stored.len() as u64;
        self.poison(result)
    }

    fn block(&mut self, page: BlockPage) -> io::Result<()> {
        if self.page.is_some() {
            return Err(invalid("A block's page comes once its last one was written"));
        }
        self.page = Some(page);
        Ok(())
    }

    fn read_back(&mut self, offset: u64, stored: u32, out: &mut Vec<u8>) -> io::Result<()> {
        let file = self.container.file_mut();
        let end = file.stream_position()?;
        out.clear();
        out.resize(stored as usize, 0);
        let read = file.seek(SeekFrom::Start(offset)).and_then(|_| file.read_exact(out));
        file.seek(SeekFrom::Start(end))?;
        read
    }
}

impl<W: Write + Seek + Read> FileWriter<W> {
    /// Starts the file at its header, its revisions' resume state kept in `state`.
    pub fn create(
        file: W,
        identity: envelope::Identity,
        tables: usize,
        settings: Settings,
        state: Option<StateFile>,
    ) -> io::Result<Self> {
        let mut container = ContainerWriter::create(file, identity, settings.level, state)?;
        container.section(SectionKey::WORLD_LOG, schema::WORLD_LOG)?;
        container.section(SectionKey::RECORDS, schema::RECORDS)?;
        Self::with(container, tables, settings)
    }

    fn with(container: ContainerWriter<W>, tables: usize, settings: Settings) -> io::Result<Self> {
        Ok(Self {
            container,
            compressor: Compressor::new(&WRITER, settings.level)?,
            frames: None,
            tables,
            run: None,
            runs: Vec::new(),
            page: None,
            poisoned: false,
            written: Written::default(),
            last: None,
            #[cfg(any(test, feature = "test-support"))]
            tamper: None,
        })
    }

    /// A writer changing each block's page with `tamper` before it is written, for tests of
    /// damage.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_tamper(mut self, tamper: impl Fn(&mut Block) + Send + 'static) -> Self {
        self.tamper = Some(Box::new(tamper));
        self
    }

    /// Takes up `file` after the revision `resumed` names, its resume state kept in `state`: what
    /// follows the revision is cut by the caller, and its last run goes on when `resumed` holds it
    /// open.
    pub fn resume(file: W, resumed: Resumed, tables: usize, settings: Settings, state: Option<StateFile>) -> io::Result<Self> {
        let mut runs = match resumed.root.open(SectionKey::WORLD_LOG, schema::WORLD_LOG)? {
            Some(section) if section.frozen.is_empty() => engine_allocation::copy_vec(&section.open)?,
            Some(_) => return Err(invalid("A columnar file's world log names its runs in its root")),
            None => Vec::new(),
        };
        let mut container = ContainerWriter::resume(file, resumed.root, resumed.at, settings.level, state)?;
        container.section(SectionKey::WORLD_LOG, schema::WORLD_LOG)?;
        container.section(SectionKey::RECORDS, schema::RECORDS)?;
        let mut writer = Self::with(container, tables, settings)?;
        writer.last = resumed.last;
        if let Some(open) = resumed.open {
            if runs.len() != open.run as usize + 1 {
                return Err(invalid("A run taken up is the file's last"));
            }
            if open.blocks.len() != open.directory.open.len() {
                return Err(invalid("A run is taken up with its open span's blocks"));
            }
            runs.pop();
            let directory = open.directory;
            writer.run = Some(Run {
                run: open.run,
                branch: directory.branch,
                epoch: directory.epoch,
                first_hour: directory.first_hour,
                hours: directory.hours,
                chain: directory.chain,
                spans: directory.spans,
                open: directory.open,
                blocks: open.blocks,
                tail: Vec::new(),
            });
        }
        writer.runs = runs;
        Ok(writer)
    }

    fn usable(&self) -> io::Result<()> {
        if self.poisoned {
            Err(invalid("A failed write ended this columnar file"))
        } else {
            Ok(())
        }
    }

    /// `result`, ending the file when it failed partway.
    fn poison<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        self.poisoned |= result.is_err();
        result
    }

    /// Whether a run is under way.
    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    /// The run under way once it holds an hour.
    pub fn open_run(&self) -> Option<RunCursor> {
        self.run.as_ref().filter(|run| run.hours > 0).map(|run| RunCursor {
            run: run.run,
            hours: run.hours,
            first_hour: run.first_hour,
            branch: run.branch,
        })
    }

    /// The bytes it holds past the file: the open span's pages.
    pub fn held_bytes(&self) -> usize {
        self.run.as_ref().map_or(0, |run| run.blocks.iter().map(block_bytes).sum())
    }

    /// The stored bytes written so far.
    pub fn written(&self) -> Written {
        self.written
    }

    /// The stored bytes its container wrote of its own.
    pub fn framing(&self) -> Framing {
        self.container.framing()
    }

    /// The file it writes, for tests reading it between revisions.
    #[cfg(any(test, feature = "test-support"))]
    pub fn file_mut(&mut self) -> &mut W {
        self.container.file_mut()
    }

    /// The branch and hour of the last hour written.
    pub fn last(&self) -> Option<(u64, i32)> {
        self.last
    }

    /// The newest saved revision of the file it writes.
    pub fn saved(&self) -> Option<replay_columnar::container::Revision> {
        self.container.saved()
    }

    /// The container it writes into: whoever owns the recording declares its other sections
    /// there, and puts and names their extents in the revision under way.
    pub fn container(&mut self) -> &mut ContainerWriter<W> {
        &mut self.container
    }

    /// Starts a run of `branch` at `first_hour`, finishing the run under way first.
    pub fn start_run(&mut self, branch: u64, epoch: u8, first_hour: i32) -> io::Result<()> {
        self.usable()?;
        self.finish_run()?;
        let run = u32::try_from(self.runs.len()).map_err(|_| invalid("Runs pass 2^32"))?;
        self.run = Some(Run {
            run,
            branch,
            epoch,
            first_hour,
            hours: 0,
            chain: [0; 32],
            spans: Vec::new(),
            open: Vec::new(),
            blocks: Vec::new(),
            tail: Vec::new(),
        });
        Ok(())
    }

    /// `raw` as one frame, written: what names it.
    fn frame_of(&mut self, raw: &[u8]) -> io::Result<FrameEntry> {
        let stored = self.compressed(raw)?;
        let run = self.run.as_ref().map_or(0, |run| run.run);
        let extent = self.container.put(envelope::Kind::Block, Tags { run, ..Tags::default() }, raw.len() as u64, &stored)?;
        Ok(FrameEntry {
            streams: Vec::new(),
            offset: extent.offset,
            stored: u32::try_from(stored.len()).map_err(|_| invalid("A frame passes 2^32 stored bytes"))?,
            raw: u32::try_from(raw.len()).map_err(|_| invalid("A frame passes 2^32 raw bytes"))?,
            digest: extent.digest,
            chained: false,
            small: false,
        })
    }

    /// Its frames are compressed from now on by the unit encoder's thread, `frames`: one context
    /// for the whole recording.
    pub fn compress_on(&mut self, frames: replay_columnar::unit::FrameCompressor) {
        self.frames = Some(frames);
    }

    fn compressed(&mut self, raw: &[u8]) -> io::Result<Vec<u8>> {
        if let Some(frames) = &self.frames {
            return frames.compress(raw);
        }
        let needed = raw.len().max(2);
        let window = (usize::BITS - (needed - 1).leading_zeros()).clamp(10, 27);
        self.compressor.window_log(window)?;
        let mut stored = engine_allocation::vec(engine_allocation::zstd::bound(raw.len()))?;
        self.compressor
            .compress_append(raw, &mut stored)
            .map_err(|error| error.invalid())?;
        Ok(stored)
    }

    /// One extent of `raw` compressed as one frame, described by its envelope.
    fn extent(&mut self, kind: envelope::Kind, tags: Tags, raw: &[u8]) -> io::Result<Extent> {
        let stored = self.compressed(raw)?;
        let extent = self.container.put(kind, tags, raw.len() as u64, &stored)?;
        self.written.pages += extent.stored;
        Ok(extent)
    }

    /// The block under way ends: its page, which its encoder gave ([`PartSink::block`]), written
    /// with `end`; its span's pages once it closes the span. The block's head as written, and
    /// where its span's higher summary pieces went (table, level, extent).
    #[allow(clippy::type_complexity, reason = "the block's head and its span's pieces, returned together")]
    pub fn end_block(&mut self, end: BlockEnd<'_>) -> io::Result<(BlockHead, Vec<(u32, u8, Extent)>)> {
        self.usable()?;
        let result = self.write_block(end);
        self.poison(result)
    }

    /// Table `table`'s summary pieces of the open span's blocks, read back in order.
    pub fn span_summaries(&mut self, table: u32) -> io::Result<Vec<Vec<u8>>> {
        let extents: Vec<Extent> = match &self.run {
            Some(run) => run.blocks.iter().filter_map(|block| block.tables.get(table as usize).and_then(|entry| entry.summary)).collect(),
            None => Vec::new(),
        };
        extents.iter().map(|extent| self.summary(extent.offset, extent.stored, extent.digest)).collect()
    }

    /// The summary piece of `stored` bytes the file holds at `offset`, read back and checked
    /// against its `digest`.
    pub fn summary(&mut self, offset: u64, stored: u64, digest: [u8; 16]) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        let stored = u32::try_from(stored).map_err(|_| invalid("A summary piece passes 2^32 bytes"))?;
        self.read_back(offset, stored, &mut out)?;
        if envelope::digest(&out) != digest {
            return Err(invalid("A summary piece read back is not the one written"));
        }
        Ok(out)
    }

    #[allow(clippy::type_complexity, reason = "the block's head and its span's pieces, returned together")]
    fn write_block(&mut self, end: BlockEnd<'_>) -> io::Result<(BlockHead, Vec<(u32, u8, Extent)>)> {
        let page = self.page.take().ok_or_else(|| invalid("A block ends once its encoder gave its page"))?;
        let run = self.run.as_ref().ok_or_else(|| invalid("A block needs a run under way"))?;
        let (index, first) = (run.run, run.hours);
        if first % UNIT_ORDINALS != 0 || page.block != first / UNIT_ORDINALS || page.hours == 0 || u32::from(page.hours) > UNIT_ORDINALS {
            return Err(invalid("A block follows its run's last whole block"));
        }
        if page.tables.len() != self.tables {
            return Err(invalid("A block's page names every table"));
        }
        let hours = u32::from(page.hours);
        let clock = self.frame_of(end.clock)?;
        self.written.clock += u64::from(clock.stored);
        let (raw, first_string) = end.strings;
        let mut strings = Vec::new();
        for (first, chunk) in string_chunks(raw, first_string)? {
            let frame = self.frame_of(&chunk)?;
            self.written.strings += u64::from(frame.stored);
            strings.push(StringChunk { first, frame });
        }
        let mut summaries = engine_allocation::filled(self.tables, None)?;
        for (table, piece) in end.summaries {
            let tags = Tags { run: index, stream: table, first, hours };
            let extent = self.container.put(envelope::Kind::Summary, tags, piece.len() as u64, &piece)?;
            self.written.summaries += extent.stored;
            *summaries.get_mut(table as usize).ok_or_else(|| invalid("A summary piece of a table past the schema"))? = Some(extent);
        }
        let tables = page.tables.into_iter().zip(summaries).map(|(unit, summary)| Entry { unit, summary }).collect();
        let run = self.run.as_mut().expect("a run under way");
        let mut block = Block {
            run: index,
            block: page.block,
            head: BlockHead { hours: page.hours, clock, strings, first_string, chain: [0; 32] },
            tables,
        };
        block.head.chain = Block::chain_after(&run.chain, &block.body()?);
        run.chain = block.head.chain;
        #[cfg(any(test, feature = "test-support"))]
        if let Some(tamper) = &self.tamper {
            tamper(&mut block);
        }
        let bytes = block.write()?;
        let extent = self.extent(envelope::Kind::BlockPage, Tags { run: index, stream: 0, first, hours }, &bytes)?;
        let head = block.head.clone();
        let run = self.run.as_mut().expect("a run under way");
        run.open.push(extent);
        run.blocks.push(block);
        run.hours += hours;
        let last = run.first_hour + (run.hours - 1) as i32;
        self.last = Some((run.branch, last));
        let span = match run.open.len() == SPAN_BLOCKS as usize {
            true => self.close_span(&end.span)?,
            false => Vec::new(),
        };
        Ok((head, span))
    }

    /// Writes the open span's pages: every table's row with its higher summary pieces
    /// `summaries` (table, level, piece), and its directory; and closes the span. Where the pieces
    /// went.
    fn close_span(&mut self, summaries: &[(u32, u8, Vec<u8>)]) -> io::Result<Vec<(u32, u8, Extent)>> {
        let run = self.run.as_mut().expect("a run under way");
        let index = run.run;
        let first_block = run.spans.len() as u32 * SPAN_BLOCKS;
        let blocks = std::mem::take(&mut run.blocks);
        run.open.clear();
        let chain = run.chain;
        let first = first_block * UNIT_ORDINALS;
        let hours = SPAN_BLOCKS * UNIT_ORDINALS;
        let mut pieces: Vec<Vec<(u8, Extent)>> = (0..self.tables).map(|_| Vec::new()).collect();
        let mut written = Vec::with_capacity(summaries.len());
        for (table, level, piece) in summaries {
            let tags = Tags { run: index, stream: *table, first, hours };
            let extent = self.container.put(envelope::Kind::Summary, tags, piece.len() as u64, piece)?;
            self.written.summaries += extent.stored;
            pieces.get_mut(*table as usize).ok_or_else(|| invalid("A summary piece of a table past the schema"))?.push((*level, extent));
            written.push((*table, *level, extent));
        }
        let mut raw = Vec::new();
        let mut rows = Vec::with_capacity(self.tables);
        for (table, summaries) in pieces.into_iter().enumerate() {
            let row = SpanRow { blocks: blocks.iter().map(|block| block.tables[table].clone()).collect(), summaries };
            let start = raw.len();
            row.write(&mut raw)?;
            let bytes = &raw[start..];
            rows.push(RowPlace {
                table: table as u32,
                offset: start as u64,
                length: u32::try_from(bytes.len()).map_err(|_| invalid("A span row passes 2^32 bytes"))?,
                digest: envelope::digest(bytes),
            });
        }
        let tags = Tags { run: index, stream: 0, first, hours };
        let span_index = self.container.put(envelope::Kind::SpanIndex, tags, raw.len() as u64, &raw)?;
        self.written.pages += span_index.stored;
        let directory = SpanDirectory { run: index, first_block, blocks: blocks.into_iter().map(|block| block.head).collect(), chain, rows }.write();
        let directory = self.extent(envelope::Kind::SpanDirectory, tags, &directory)?;
        self.run.as_mut().expect("a run under way").spans.push(SpanPages { directory, index: span_index });
        Ok(written)
    }

    /// Puts `pieces` (table, piece), each covering the run's last span, which is not whole, for
    /// its directory to name when the run ends.
    pub fn tail(&mut self, pieces: &[(u32, Vec<u8>)]) -> io::Result<()> {
        self.usable()?;
        let Some(run) = self.run.as_ref() else { return Ok(()) };
        let first = run.spans.len() as u32 * SPAN_BLOCKS * UNIT_ORDINALS;
        let (index, hours) = (run.run, run.hours - first);
        let mut tail = Vec::with_capacity(pieces.len());
        for (table, piece) in pieces {
            let tags = Tags { run: index, stream: *table, first, hours };
            let extent = self.container.put(envelope::Kind::Summary, tags, piece.len() as u64, piece)?;
            self.written.summaries += extent.stored;
            tail.push((*table, extent));
        }
        self.run.as_mut().expect("a run under way").tail = tail;
        Ok(())
    }

    /// Ends the run under way with its directory.
    pub fn finish_run(&mut self) -> io::Result<()> {
        self.usable()?;
        if self.page.is_some() {
            return Err(invalid("A run ends once its last block's page was written"));
        }
        // A run that took no hour wrote nothing and is none.
        let Some(run) = self.run.take().filter(|run| run.hours > 0) else {
            return Ok(());
        };
        let result = self.directory(&run).map(|directory| self.runs.push(directory));
        self.poison(result)
    }

    fn directory(&mut self, run: &Run) -> io::Result<Extent> {
        let directory = RunDirectory {
            branch: run.branch,
            epoch: run.epoch,
            first_hour: run.first_hour,
            hours: run.hours,
            chain: run.chain,
            spans: engine_allocation::copy_vec(&run.spans)?,
            open: engine_allocation::copy_vec(&run.open)?,
            tail: engine_allocation::copy_vec(&run.tail)?,
        }
        .write(run.run)?;
        engine_allocation::reserve(&mut self.runs, 1)?;
        self.extent(envelope::Kind::RunDirectory, Tags { run: run.run, ..Tags::default() }, &directory)
    }

    /// Publishes everything written and named so far as the next revision, saved or not: the
    /// finished runs' directories and the directory of the run under way, with resume state
    /// `state`. `sync` makes what is written durable before what names it. A failure leaves the
    /// file at its last published revision.
    pub fn revise_runs(
        &mut self,
        state: Option<&[StateEntry]>,
        saved: bool,
        mut sync: impl FnMut(&mut W) -> io::Result<()>,
    ) -> io::Result<replay_columnar::container::Revision> {
        self.usable()?;
        let result = (|| {
            let mut runs = engine_allocation::copy_vec(&self.runs)?;
            if let Some(run) = self.run.take() {
                let directory = if run.hours > 0 { Some(self.directory(&run)) } else { None };
                self.run = Some(run);
                if let Some(directory) = directory {
                    runs.push(directory?);
                }
            }
            self.container.set_open(SectionKey::WORLD_LOG, &runs)?;
            let before = self.container.framing().roots;
            let revision = self.container.commit(state, saved, &mut sync)?;
            self.written.roots += self.container.framing().roots - before;
            Ok(revision)
        })();
        self.poison(result)
    }

    /// Finishes the run under way and publishes the last revision, saved or not: it names no
    /// resume state, and the state file goes. `final_state` is kept as
    /// [`ContainerWriter::finish`] keeps it. The file, the revision and what was stored.
    pub fn finish_runs(
        mut self,
        final_state: &[StateEntry],
        saved: bool,
        sync: impl FnMut(&mut W) -> io::Result<()>,
    ) -> io::Result<(W, replay_columnar::container::Revision, Written)> {
        self.usable()?;
        self.finish_run()?;
        let runs = std::mem::take(&mut self.runs);
        self.container.set_open(SectionKey::WORLD_LOG, &runs)?;
        let before = self.container.framing().roots;
        let Self { container, mut written, .. } = self;
        let (file, revision, framing) = container.finish(final_state, saved, sync)?;
        written.roots += framing.roots - before;
        Ok((file, revision, written))
    }
}

/// A block's appended strings, `count | per string: length | bytes`, cut into chunks of that
/// form of about [`STRING_CHUNK`] raw bytes: each with its first string's id.
fn string_chunks(raw: &[u8], first: u32) -> io::Result<Vec<(u32, Vec<u8>)>> {
    let mut reader = replay_columnar::leaf::Reader::new(raw);
    let count = reader.count()?;
    if count > raw.len() {
        return Err(invalid("A block's strings count more than they hold"));
    }
    let mut chunks = Vec::new();
    let (mut body, mut held, mut chunk_first) = (Vec::new(), 0_u64, first);
    for at in 0..count as u32 {
        let length = reader.count()?;
        let text = reader.raw(length)?;
        if held > 0 && body.len() + text.len() > STRING_CHUNK {
            let mut chunk = Vec::with_capacity(body.len() + 10);
            replay_columnar::unit::put_varint(&mut chunk, held);
            chunk.extend_from_slice(&body);
            chunks.push((chunk_first, chunk));
            (body, held, chunk_first) = (Vec::new(), 0, first + at);
        }
        replay_columnar::unit::put_varint(&mut body, length as u64);
        body.extend_from_slice(text);
        held += 1;
    }
    reader.finish()?;
    if held > 0 {
        let mut chunk = Vec::with_capacity(body.len() + 10);
        replay_columnar::unit::put_varint(&mut chunk, held);
        chunk.extend_from_slice(&body);
        chunks.push((chunk_first, chunk));
    }
    Ok(chunks)
}

/// Roughly what a block held in the open span takes.
fn block_bytes(block: &Block) -> usize {
    block
        .tables
        .iter()
        .map(|entry| {
            entry.unit.status.capacity()
                + entry.unit.parts.iter().map(|part| part.frames.iter().map(|frame| 48 + frame.streams.capacity() * 4).sum::<usize>()).sum::<usize>()
                + [&entry.unit.first, &entry.unit.restore].into_iter().flatten().map(|restore| restore.streams.iter().map(|(_, chunks)| chunks.len() * 96).sum::<usize>()).sum::<usize>()
                + 160
        })
        .sum()
}

/// The capture a file's runs belong to, which the root's control records hold: enough to
/// pair the file with exactly the row recording of the same capture and the hours of it the
/// file covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    pub campaign_id: Vec<u8>,
    /// The content identity, as the row recording encodes it.
    pub content: Vec<u8>,
    pub capture_mode: u8,
    /// The row recording's sessions the file's runs belong to, in recording order, each as
    /// the row recording encodes it.
    pub sessions: Vec<Vec<u8>>,
    /// Each run's index in `sessions`.
    pub run_sessions: Vec<u32>,
    /// Where writing stopped short of the rows, when it did.
    pub stop: Option<Stop>,
    /// The row recording's format version.
    pub format: u16,
    /// The journal segments ([`crate::columnar_journal`]) the file holds all of.
    pub journaled: u32,
    /// The capture's branch starts, gaps and control records but its clock witnesses, in
    /// recording order, each as the row recording encodes it (`spool::encode_record`).
    pub controls: Vec<Vec<u8>>,
}

/// The columnar writer stopped while the rows went on: the hours after `last_hour` of
/// `branch` are not in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stop {
    pub branch: u64,
    /// The last hour the file holds on `branch`; none when it holds none of it.
    pub last_hour: Option<i32>,
    pub reason: String,
}

impl Capture {
    const VERSION: u64 = 3;

    pub fn write(&self) -> io::Result<Vec<u8>> {
        self.write_with(&self.controls)
    }

    /// The record holding `controls` in place of the capture's own.
    pub(crate) fn write_with(&self, controls: &[Vec<u8>]) -> io::Result<Vec<u8>> {
        let mut out = Bytes::new();
        out.varint(Self::VERSION)?;
        for bytes in [&self.campaign_id, &self.content] {
            out.varint(bytes.len() as u64)?;
            out.raw(bytes)?;
        }
        out.varint(u64::from(self.capture_mode))?;
        out.varint(self.sessions.len() as u64)?;
        for session in &self.sessions {
            out.varint(session.len() as u64)?;
            out.raw(session)?;
        }
        out.varint(self.run_sessions.len() as u64)?;
        for &session in &self.run_sessions {
            out.varint(u64::from(session))?;
        }
        match &self.stop {
            None => out.varint(0)?,
            Some(stop) => {
                out.varint(1)?;
                out.varint(stop.branch)?;
                out.varint(stop.last_hour.map_or(0, |hour| (hour as u32 as u64) + 1))?;
                out.varint(stop.reason.len() as u64)?;
                out.raw(stop.reason.as_bytes())?;
            }
        }
        out.varint(u64::from(self.format))?;
        out.varint(u64::from(self.journaled))?;
        write_controls(&mut out, controls)?;
        Ok(out.into_bytes())
    }

    pub fn read(bytes: &[u8]) -> io::Result<Self> {
        let mut reader = Reader::new(bytes);
        if reader.varint()? != Self::VERSION {
            return Err(invalid("Unknown capture record version"));
        }
        let owned = |reader: &mut Reader<'_>| -> io::Result<Vec<u8>> {
            let length = usize::try_from(reader.varint()?)
                .map_err(|_| invalid("A capture field passes the address space"))?;
            engine_allocation::copy_vec(reader.raw(length)?)
        };
        let campaign_id = owned(&mut reader)?;
        let content = owned(&mut reader)?;
        let capture_mode = u8::try_from(reader.varint()?)
            .map_err(|_| invalid("A capture mode passes a byte"))?;
        let count = |reader: &mut Reader<'_>| -> io::Result<usize> {
            let count = usize::try_from(reader.varint()?)
                .map_err(|_| invalid("A capture count passes the address space"))?;
            if count > bytes.len() {
                return Err(invalid("A capture count passes its record"));
            }
            Ok(count)
        };
        let sessions_count = count(&mut reader)?;
        let mut sessions = engine_allocation::vec(sessions_count)?;
        for _ in 0..sessions_count {
            sessions.push(owned(&mut reader)?);
        }
        let runs_count = count(&mut reader)?;
        let mut run_sessions = engine_allocation::vec(runs_count)?;
        for _ in 0..runs_count {
            let session = u32::try_from(reader.varint()?)
                .map_err(|_| invalid("A run's session passes 2^32"))?;
            if session as usize >= sessions.len() {
                return Err(invalid("A run names no session of the capture"));
            }
            run_sessions.push(session);
        }
        let stop = match reader.varint()? {
            0 => None,
            1 => {
                let branch = reader.varint()?;
                let last_hour = match reader.varint()? {
                    0 => None,
                    hour => Some(
                        u32::try_from(hour - 1)
                            .map_err(|_| invalid("A stop's hour passes 2^32"))? as i32,
                    ),
                };
                let reason = String::from_utf8(owned(&mut reader)?)
                    .map_err(|_| invalid("A stop's reason is not UTF-8"))?;
                Some(Stop {
                    branch,
                    last_hour,
                    reason,
                })
            }
            _ => return Err(invalid("Unknown capture stop marker")),
        };
        let format = u16::try_from(reader.varint()?)
            .map_err(|_| invalid("A capture's format passes 2^16"))?;
        let journaled = u32::try_from(reader.varint()?)
            .map_err(|_| invalid("A capture's journal passes 2^32 segments"))?;
        let mut controls = Vec::new();
        read_controls(&mut reader, bytes.len(), &mut controls)?;
        reader.finish()?;
        Ok(Self {
            campaign_id,
            content,
            capture_mode,
            sessions,
            run_sessions,
            stop,
            format,
            journaled,
            controls,
        })
    }
}

fn write_controls(out: &mut Bytes, controls: &[Vec<u8>]) -> io::Result<()> {
    out.varint(controls.len() as u64)?;
    for control in controls {
        out.varint(control.len() as u64)?;
        out.raw(control)?;
    }
    Ok(())
}

fn read_controls(reader: &mut Reader<'_>, limit: usize, into: &mut Vec<Vec<u8>>) -> io::Result<()> {
    let count = usize::try_from(reader.varint()?)
        .ok()
        .filter(|&count| count <= limit)
        .ok_or_else(|| invalid("A control count passes its record"))?;
    engine_allocation::reserve(into, count)?;
    for _ in 0..count {
        let length = usize::try_from(reader.varint()?)
            .map_err(|_| invalid("A control record passes the address space"))?;
        into.push(engine_allocation::copy_vec(reader.raw(length)?)?);
    }
    Ok(())
}

/// The capture a root's records extents hold: each revision's newly added control records, in
/// order, then the capture record, which every revision replaces.
pub fn read_capture(
    extents: &[Extent],
    mut raw: impl FnMut(&Extent) -> io::Result<Vec<u8>>,
) -> io::Result<Option<Capture>> {
    let Some((last, chunks)) = extents.split_last() else {
        return Ok(None);
    };
    let mut capture = Capture::read(&raw(last)?)?;
    for extent in chunks {
        let bytes = raw(extent)?;
        let mut reader = Reader::new(&bytes);
        read_controls(&mut reader, bytes.len(), &mut capture.controls)?;
        reader.finish()?;
    }
    Ok(Some(capture))
}

/// The dictionary ids a table's state references: its refusal, and its recorded text cells
/// at live slots.
pub fn referenced_ids(table: &Table, refusal: Option<u32>) -> io::Result<Vec<u32>> {
    let mut ids = Vec::new();
    if let Some(id) = refusal {
        engine_allocation::reserve(&mut ids, 1)?;
        ids.push(id);
    }
    for column in 0..table.kinds().count() {
        if let Some(Column::Bytes(texts)) = table.column(column) {
            for slot in table.live().ones() {
                if table.recorded(column, slot)
                    && let Some(&id) = texts.get(slot as usize)
                {
                    engine_allocation::reserve(&mut ids, 1)?;
                    ids.push(id);
                }
            }
        }
    }
    Ok(ids)
}

#[cfg(any(test, feature = "test-support"))]
pub use hours::HourWriter;

/// Hours written into a file as the recorder writes them, without its records: for tests that
/// make recordings of synthetic hours.
#[cfg(any(test, feature = "test-support"))]
mod hours {
    use super::{BlockEnd, FileWriter};
    use replay_columnar::{
        envelope::UNIT_ORDINALS,
        lane::{Clock, ClockHour},
        manifest::Manifest,
        store::{Store, StringSource, journal::Journal},
        unit::{self, Encoder},
    };
    use std::{
        io::{self, Read, Seek, Write},
        sync::{Arc, Mutex, PoisonError},
    };

    /// Every string the hours appended, for the store to read again what it let go of.
    #[derive(Clone, Default)]
    struct Appended(Arc<Mutex<Vec<Vec<u8>>>>);

    impl StringSource for Appended {
        fn text(&mut self, id: u32) -> io::Result<Option<Vec<u8>>> {
            Ok(self.0.lock().unwrap_or_else(PoisonError::into_inner).get(id as usize).cloned())
        }
    }

    /// The block under way: its hours, clock, strings appended and their first id.
    struct OpenBlock {
        hours: u32,
        clock: Clock,
        count: u64,
        strings: Vec<u8>,
        first: u32,
    }

    /// A file's runs written from hours (manifest and sections, as the journal holds them): the
    /// recorder's store and encoder, every block's page with its clock, strings and summaries.
    /// The derived tables are their writers', run each hour as the recorder runs them: what the
    /// hours say of those tables is left out.
    pub struct HourWriter<W: Write + Seek + Read> {
        file: FileWriter<W>,
        store: Store,
        adapter: Journal,
        writers: recorder_derive::Writers,
        columns: Vec<usize>,
        appended: Appended,
        running: bool,
        block: Option<OpenBlock>,
        summaries: crate::campaign::live::Summaries,
        /// The run under way's first hour and its hours so far.
        run: (i32, u32),
    }

    impl<W: Write + Seek + Read> HourWriter<W> {
        pub fn new(file: FileWriter<W>, level: i32) -> io::Result<Self> {
            Self::with_unit(file, level, unit::Settings::at(level))
        }

        /// [`Self::new`], its blocks restored and packed as `settings` say.
        pub fn with_unit(file: FileWriter<W>, level: i32, settings: unit::Settings) -> io::Result<Self> {
            let kinds = crate::columnar_frames::schema()?;
            let columns = kinds.iter().map(Vec::len).collect();
            let mut store = Store::new(kinds.clone(), crate::columnar_frames::keeps_last().to_vec());
            store.declare_logs(&crate::rows::logs())?;
            store.attach(Encoder::new(&kinds, settings)?);
            let appended = Appended::default();
            store.read_strings_from(Box::new(appended.clone()));
            let writers = recorder_derive::writers();
            let adapter = Journal::leaving(&writers.tables(), crate::columnar_frames::names());
            Ok(Self { file, store, adapter, writers, columns, appended, running: false, block: None, summaries: crate::campaign::live::Summaries::new(level)?, run: (0, 0) })
        }

        pub fn file(&mut self) -> &mut FileWriter<W> {
            &mut self.file
        }

        /// The frame of `tables` as the store holds them after the hour written last, named
        /// `(branch, hour)`: what a reader of the file is to read back at that hour.
        pub fn frame(&self, tables: &[crate::rows::TableId], (branch, hour): (u64, i32), mode: crate::scope::CaptureMode) -> io::Result<crate::frame::DecodedFrame> {
            let format = crate::format::FormatVersion::CURRENT.version();
            let mut frame = crate::frame::stored_frame(tables.iter().map(|&table| (table, self.store.view(table.index()))), (branch, hour), mode, format)?;
            for &table in tables {
                let view = self.store.view(table.index());
                if matches!(view.status(), replay_columnar::manifest::Status::Complete) {
                    let held = std::mem::replace(&mut frame.tables[table.index()], crate::frame::not_selected());
                    frame.tables[table.index()] = held.with_sampled_at(view.sampled_at());
                }
            }
            Ok(frame)
        }

        /// [`Self::frame`] of the derived writers' tables, every other table not selected, each
        /// table its writer ran this hour named read without a time, as a reader names it.
        pub fn derived_frame(&self, at: (u64, i32), mode: crate::scope::CaptureMode) -> io::Result<crate::frame::DecodedFrame> {
            let tables: Vec<crate::rows::TableId> = self.writers.tables().into_iter().map(|index| crate::rows::TableId::ALL[index]).collect();
            let mut frame = self.frame(&tables, at, mode)?;
            for table in tables {
                // The writers stamp a table's sample hour with the hour exactly when they ran.
                if self.store.view(table.index()).sampled_at() == Some(at.1) {
                    frame.metrics.table_ns[table.index()] = crate::frame::READ_TIME_NOT_RECORDED;
                }
            }
            Ok(frame)
        }

        /// Starts a run of `branch` at `first_hour`, ending the one under way.
        pub fn start_run(&mut self, branch: u64, epoch: u8, first_hour: i32) -> io::Result<()> {
            self.end_run()?;
            self.file.start_run(branch, epoch, first_hour)?;
            self.store.begin_run(first_hour);
            self.writers.start_run();
            self.summaries.start_run();
            self.run = (first_hour, 0);
            self.appended.0.lock().unwrap_or_else(PoisonError::into_inner).clear();
            self.running = true;
            Ok(())
        }

        /// The run's next hour: its manifest and its sections, in the manifest's order.
        pub fn hour(&mut self, manifest: &[u8], sections: &[u8]) -> io::Result<()> {
            if !self.running {
                return Err(io::Error::other("An hour needs a run under way"));
            }
            let manifest = Manifest::read(manifest, &self.columns)?;
            let first = self.store.strings().next();
            if self.block.is_none() {
                self.summaries.open(&mut self.store, manifest.hour);
            }
            let block = self.block.get_or_insert_with(|| OpenBlock { hours: 0, clock: Clock::default(), count: 0, strings: Vec::new(), first });
            for text in &manifest.appends {
                unit::put_varint(&mut block.strings, text.len() as u64);
                block.strings.extend_from_slice(text);
                self.appended.0.lock().unwrap_or_else(PoisonError::into_inner).push(text.to_vec());
            }
            block.count += manifest.appends.len() as u64;
            block.clock.hour(&ClockHour { flags: manifest.flags, total_ns: manifest.total_ns, pool_bytes: manifest.pool_bytes, dropped_frames: manifest.dropped_frames });
            block.hours += 1;
            let ends = block.hours == UNIT_ORDINALS;
            self.adapter.begin_hour(&mut self.store, &manifest)?;
            let mut at = 0_usize;
            for entry in &manifest.sections {
                let end = at + entry.bytes as usize;
                let bytes = sections.get(at..end).ok_or_else(|| io::Error::other("An hour's sections end before its manifest's"))?;
                self.adapter.section(&mut self.store, entry, bytes)?;
                at = end;
            }
            self.adapter.applied(&mut self.store);
            self.writers.hour(manifest.branch, manifest.hour, self.run.1 == 0, &mut self.store)?;
            self.summaries.hour(&self.store, manifest.hour);
            self.store.end_hour(&mut self.file)?;
            self.adapter.moved(&mut self.store);
            self.run.1 += 1;
            if ends {
                self.end_block()?;
            }
            Ok(())
        }

        fn end_block(&mut self) -> io::Result<()> {
            let Some(block) = self.block.take() else { return Ok(()) };
            let mut strings = Vec::new();
            unit::put_varint(&mut strings, block.count);
            strings.extend_from_slice(&block.strings);
            let clock = block.clock.finish();
            let (first_hour, hours) = self.run;
            let (summaries, span) = self.summaries.finish(first_hour + hours as i32 - 1, (hours - block.hours) / UNIT_ORDINALS, first_hour, &mut self.file)?;
            let (_, span) = self.file.end_block(BlockEnd { clock: &clock, strings: (&strings, block.first), summaries, span })?;
            self.summaries.written(&span);
            self.store.end_block();
            Ok(())
        }

        /// Ends the run under way: its last block, short, when one is under way.
        pub fn end_run(&mut self) -> io::Result<()> {
            if !std::mem::take(&mut self.running) {
                return Ok(());
            }
            self.store.end_run(&mut self.file)?;
            self.end_block()?;
            let tail = self.summaries.tail(self.run.0, self.run.1, &mut self.file)?;
            self.file.tail(&tail)?;
            self.file.finish_run()
        }

        /// Ends the run under way; the file writer.
        pub fn finish(mut self) -> io::Result<FileWriter<W>> {
            self.end_run()?;
            Ok(self.file)
        }
    }
}

/// Kind-only while a capture runs, whatever diagnostic scope encloses it: a writer in a capture
/// must not allocate to fail. Offline writers keep the message.
fn invalid(message: &'static str) -> io::Error {
    if engine_allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block's strings past a chunk's size are cut at string boundaries into chunks of about
    /// that size, each naming its first id; read in order they are the block's strings.
    #[test]
    fn a_block_s_strings_are_cut_into_chunks_that_name_their_first_id() {
        let texts: Vec<Vec<u8>> = (0..3_000_u32).map(|at| vec![(at % 251) as u8; 700 + (at as usize % 900)]).collect();
        let mut raw = Vec::new();
        replay_columnar::unit::put_varint(&mut raw, texts.len() as u64);
        for text in &texts {
            replay_columnar::unit::put_varint(&mut raw, text.len() as u64);
            raw.extend_from_slice(text);
        }
        let chunks = string_chunks(&raw, 40).unwrap();
        assert!(chunks.len() >= 3, "{} chunks", chunks.len());
        let mut id = 40;
        let mut read = Vec::new();
        for (first, chunk) in &chunks {
            assert_eq!(*first, id);
            assert!(chunk.len() <= STRING_CHUNK + 2_000);
            let mut reader = replay_columnar::leaf::Reader::new(chunk);
            for _ in 0..reader.count().unwrap() {
                let length = reader.count().unwrap();
                read.push(reader.raw(length).unwrap().to_vec());
                id += 1;
            }
            reader.finish().unwrap();
        }
        assert_eq!(read, texts);
        assert!(string_chunks(&[0], 9).unwrap().is_empty(), "a block that appended no string has no chunk");
    }
}

/// Where the recorder's store reads again a string it let go of: the strings frames of the
/// blocks its run wrote, read from the file.
pub struct FileStrings {
    file: std::fs::File,
    /// Every string chunk the run's written blocks hold: its first string id and its frame, in
    /// order.
    blocks: std::sync::Arc<std::sync::Mutex<Vec<(u32, replay_columnar::unit::FrameEntry)>>>,
    /// The chunks read during the hour under way, by their place: each decoded, and where each
    /// string lies in it.
    held: std::collections::HashMap<usize, (Vec<u8>, Vec<(u32, u32)>)>,
}

impl FileStrings {
    /// Strings read from `file` through the blocks `blocks` lists as they are written.
    pub fn new(file: std::fs::File, blocks: std::sync::Arc<std::sync::Mutex<Vec<(u32, replay_columnar::unit::FrameEntry)>>>) -> Self {
        Self { file, blocks, held: std::collections::HashMap::new() }
    }
}

impl replay_columnar::store::StringSource for FileStrings {
    fn text(&mut self, id: u32) -> io::Result<Option<Vec<u8>>> {
        let (at, frame, first) = {
            let blocks = self.blocks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let at = blocks.partition_point(|&(first, _)| first <= id);
            let Some(at) = at.checked_sub(1) else { return Ok(None) };
            let (first, frame) = blocks[at].clone();
            (at, frame, first)
        };
        if !self.held.contains_key(&at) {
            let _label = engine_allocation::labels::enter(engine_allocation::labels::owner("strings: chunks read again"), engine_allocation::labels::Kind::Operator);
            let decoded = crate::columnar_frames::strings_frame(&self.file, &frame)?;
            self.held.insert(at, decoded);
        }
        let (raw, places) = &self.held[&at];
        Ok(places.get((id - first) as usize).map(|&(at, length)| raw[at as usize..(at + length) as usize].to_vec()))
    }

    fn release(&mut self) {
        self.held = std::collections::HashMap::new();
    }
}
