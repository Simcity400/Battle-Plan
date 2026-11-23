use super::invalid;
use replay_settings::compression::Settings;
use engine_allocation::zstd::{Compressor, Holder};
use replay_columnar::{
    container::{ContainerWriter, Framing, Root, SectionKey, StateEntry, StateFile, Tags, schema},
    envelope::{self, Extent, RowPlace, SPAN_BLOCKS, SpanPages, UNIT_ORDINALS},
    pages::{Block, BlockHead, Entry, RunDirectory, STRING_CHUNK, SpanDirectory, SpanRow, StringChunk},
    unit::{BlockPage, FrameEntry, PartSink},
};
use std::io::{self, Read, Seek, SeekFrom, Write};

static WRITER: Holder = Holder::new("zstd: columnar writer");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Written {
    pub frames: u64,
    pub clock: u64,
    pub strings: u64,
    pub summaries: u64,
    pub pages: u64,
    pub roots: u64,
}

struct Run {
    run: u32,
    branch: u64,
    epoch: u8,
    first_hour: i32,
    hours: u32,
    chain: [u8; 32],
    spans: Vec<SpanPages>,
    open: Vec<Extent>,
    blocks: Vec<Block>,
    tail: Vec<(u32, Extent)>,
}

#[derive(Clone, Copy, Debug)]
pub struct RunCursor {
    pub run: u32,
    pub hours: u32,
    pub first_hour: i32,
    pub branch: u64,
}

pub struct Resumed {
    pub root: Root,
    pub at: replay_columnar::container::Revision,
    pub last: Option<(u64, i32)>,
    pub open: Option<OpenRun>,
}

pub struct OpenRun {
    pub run: u32,
    pub directory: RunDirectory,
    pub blocks: Vec<Block>,
}

pub struct BlockEnd<'a> {
    pub clock: &'a [u8],
    pub strings: (&'a [u8], u32),
    pub summaries: Vec<(u32, Vec<u8>)>,
    pub span: Vec<(u32, u8, Vec<u8>)>,
}

pub struct FileWriter<W: Write + Seek + Read> {
    container: ContainerWriter<W>,
    compressor: Compressor,
    frames: Option<replay_columnar::unit::FrameCompressor>,
    tables: usize,
    run: Option<Run>,
    runs: Vec<Extent>,
    page: Option<BlockPage>,
    poisoned: bool,
    written: Written,
    last: Option<(u64, i32)>,
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_tamper(mut self, tamper: impl Fn(&mut Block) + Send + 'static) -> Self {
        self.tamper = Some(Box::new(tamper));
        self
    }

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

    fn poison<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        self.poisoned |= result.is_err();
        result
    }

    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    pub fn next_write(&self) -> (u32, u32) {
        match &self.run {
            Some(run) => (run.run, run.hours),
            None => (self.runs.len() as u32, 0),
        }
    }

    pub fn record_derived_versions(&mut self, tables: usize, versions: &[(usize, u32)]) -> io::Result<()> {
        let next = self.next_write();
        for &(table, version) in versions {
            if table < tables {
                let key = SectionKey::derived(table as u32);
                if self.container.section_of(key).is_none() {
                    self.container.section(key, version)?;
                    if next != (0, 0) {
                        let bytes = replay_recording::columnar_frames::encode_derived_from(next.0, next.1)?;
                        let extent = self.container.put(envelope::Kind::Stored, Tags::default(), bytes.len() as u64, &bytes)?;
                        self.container.append(key, extent)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn open_run(&self) -> Option<RunCursor> {
        self.run.as_ref().filter(|run| run.hours > 0).map(|run| RunCursor {
            run: run.run,
            hours: run.hours,
            first_hour: run.first_hour,
            branch: run.branch,
        })
    }

    pub fn held_bytes(&self) -> usize {
        self.run.as_ref().map_or(0, |run| run.blocks.iter().map(block_bytes).sum())
    }

    pub fn written(&self) -> Written {
        self.written
    }

    pub fn framing(&self) -> Framing {
        self.container.framing()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn file_mut(&mut self) -> &mut W {
        self.container.file_mut()
    }

    pub fn last(&self) -> Option<(u64, i32)> {
        self.last
    }

    pub fn saved(&self) -> Option<replay_columnar::container::Revision> {
        self.container.saved()
    }

    pub fn container(&mut self) -> &mut ContainerWriter<W> {
        &mut self.container
    }

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

    fn extent(&mut self, kind: envelope::Kind, tags: Tags, raw: &[u8]) -> io::Result<Extent> {
        let stored = self.compressed(raw)?;
        let extent = self.container.put(kind, tags, raw.len() as u64, &stored)?;
        self.written.pages += extent.stored;
        Ok(extent)
    }

    #[allow(clippy::type_complexity, reason = "the block's head and its span's pieces, returned together")]
    pub fn end_block(&mut self, end: BlockEnd<'_>) -> io::Result<(BlockHead, Vec<(u32, u8, Extent)>)> {
        self.usable()?;
        let result = self.write_block(end);
        self.poison(result)
    }

    pub fn span_summaries(&mut self, table: u32) -> io::Result<Vec<Vec<u8>>> {
        let extents: Vec<Extent> = match &self.run {
            Some(run) => run.blocks.iter().filter_map(|block| block.tables.get(table as usize).and_then(|entry| entry.summary)).collect(),
            None => Vec::new(),
        };
        extents.iter().map(|extent| self.summary(extent.offset, extent.stored, extent.digest)).collect()
    }

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

    pub fn finish_run(&mut self) -> io::Result<()> {
        self.usable()?;
        if self.page.is_some() {
            return Err(invalid("A run ends once its last block's page was written"));
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
