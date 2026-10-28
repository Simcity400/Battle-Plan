use super::super::reader::source::Evidence;
use super::*;
use engine_allocation as allocation;
use std::sync::atomic::AtomicU64;
use std::{ops::Range, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Continuity {
    Continue,
}

pub(crate) struct ColumnChunk {
    pub bytes: Vec<u8>,
    pub section: usize,
    pub slot: usize,
    pub first: usize,
    pub count: usize,
    pub continuity: Continuity,
}

impl ColumnChunk {
    pub(crate) fn validate(&self, section: usize, first: usize, count: usize) -> io::Result<()> {
        if self.section != section
            || self.slot != 0
            || self.first != first
            || self.count != count
            || self.continuity != Continuity::Continue
        {
            return Err(invalid_static(
                "column chunk differs from its section descriptor",
            ));
        }
        Ok(())
    }
}

pub(crate) fn inflate(
    compressed: &[u8],
    raw: usize,
    keep: bool,
    check: &mut Check<'_>,
) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    Directory::inflate_checked(compressed, raw, &mut bytes, keep, check)?;
    Ok(bytes)
}

fn section(id: usize) -> Section {
    match id {
        HEADER => Section::Header,
        DIAGNOSTICS => Section::Diagnostics,
        id if id % 2 == 0 => Section::Keyframe(TableId::ALL[(id - 2) / 2]),
        id => Section::Later(TableId::ALL[(id - 3) / 2]),
    }
}

#[derive(Default)]
pub(crate) struct Credits {
    live: AtomicUsize,
    peak: AtomicUsize,
    inflation: std::sync::Mutex<()>,
    pub(crate) source_bytes: Arc<AtomicU64>,
    pub(crate) validation_stored: AtomicU64,
    pub(crate) validation_raw: AtomicU64,
}

#[derive(Clone)]
pub(crate) struct Credit(Arc<Lease>);

struct Lease {
    owner: Arc<Credits>,
}

impl Credits {
    fn inflation(&self, check: &mut Check<'_>) -> io::Result<std::sync::MutexGuard<'_, ()>> {
        loop {
            check.run()?;
            match self.inflation.try_lock() {
                Ok(lock) => return Ok(lock),
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Err(io::Error::other("replay inflation worker panicked"));
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::park_timeout(std::time::Duration::from_millis(10))
                }
            }
        }
    }
    pub(crate) fn acquire(self: &Arc<Self>, limit: usize) -> io::Result<Credit> {
        let before = self
            .live
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |live| {
                (live < limit).then_some(live + 1)
            })
            .map_err(|_| io::Error::other("raw block credits exhausted"))?;
        self.peak.fetch_max(before + 1, Ordering::Relaxed);
        Ok(Credit(Arc::new(Lease {
            owner: Arc::clone(self),
        })))
    }
    pub(crate) fn counts(&self) -> (usize, usize) {
        (
            self.live.load(Ordering::Acquire),
            self.peak.load(Ordering::Relaxed),
        )
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.owner.live.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct BlockChunks {
    pub index: BlockIndex,
    pub position: usize,
    pub hours: Vec<Hour>,
    pub entries: Vec<u8>,
    pub(super) present: Present,
    pub(super) directory: Directory,
    pub sections: Vec<Option<ColumnChunk>>,
    pub failures: Vec<Option<io::Error>>,
    pub diagnostics: cursor::DiagnosticsCursor,
    verified: Vec<usize>,
    integrity_errors: Vec<Option<io::Error>>,
    evidence: Evidence,
    _credit: Credit,
}

impl BlockChunks {
    pub(crate) fn load(
        replay: &Replay,
        position: usize,
        selected: &[bool; TABLE_COUNT],
        later: bool,
        credit: Credit,
        check: &mut Check<'_>,
    ) -> io::Result<Self> {
        #[cfg(test)]
        let _memory =
            super::super::reader::ownership::category(super::super::reader::ownership::Kind::Raw);
        let owner = Arc::clone(&credit.0.owner);
        let _inflation = owner.inflation(check)?;
        let index = &replay.index[position];
        check.run()?;
        if index.raw_length > MAX_BLOCK_RAW_BYTES as u64 {
            return Err(invalid("block exceeds raw size limit"));
        }
        if replay.format.version() < GROUPED_FORMAT {
            return Err(invalid(format!(
                "grouped block in a format {} recording",
                replay.format.version()
            )));
        }
        let mut evidence = Evidence::new(SECTIONS + 1, Arc::clone(&owner.source_bytes))?;
        let directory = Directory::read_admitted(
            replay,
            index,
            Directory::prefix(replay, index)?,
            &mut |offset, bytes| evidence.record(0, offset, bytes, check),
        )?;
        let present = Present::new(replay.format.version(), replay.capture_mode);
        check_header_size(directory.frames, &present)?;
        let load = |id: usize,
                    evidence: &mut Evidence,
                    check: &mut Check<'_>|
         -> io::Result<Option<ColumnChunk>> {
            #[cfg(test)]
            let _step = super::super::check::Scope::enter(super::super::check::Step::Source(id));
            let Some(chunk) = replay
                .section_chunks_from(index, section(id), &directory)?
                .next()
            else {
                return Ok(None);
            };
            let mut bytes = Vec::new();
            chunk.inflate_admitted(
                &mut bytes,
                true,
                &mut |offset, compressed, check| evidence.record(id + 1, offset, compressed, check),
                check,
            )?;
            Ok(Some(ColumnChunk {
                bytes,
                section: id,
                slot: 0,
                first: usize::from(id >= 3 && id % 2 == 1),
                count: if id < 2 { directory.frames } else { 1 },
                continuity: Continuity::Continue,
            }))
        };
        let header = load(HEADER, &mut evidence, check)?
            .map(|chunk| chunk.bytes)
            .unwrap_or_default();
        let (hours, entries) = read_header_with(&header, index, directory.frames, &present, check)?;
        let diagnostics = load(DIAGNOSTICS, &mut evidence, check)?
            .ok_or_else(|| invalid_static("grouped block has no diagnostics"))?;
        let diagnostics =
            cursor::DiagnosticsCursor::new(diagnostics.bytes, &present, directory.frames, check)?;
        let mut sections = allocation::collect_vec((0..SECTIONS).map(|_| None))?;
        let mut failures = allocation::collect_vec((0..TABLE_COUNT).map(|_| None))?;
        let mut verified = allocation::filled(SECTIONS, 0)?;
        let mut integrity_errors = allocation::collect_vec((0..SECTIONS).map(|_| None))?;
        verified[HEADER] = usize::from(directory.sections[HEADER].is_some());
        verified[DIAGNOSTICS] = 1;
        for table in TableId::ALL {
            check.run()?;
            // Illegal stored mode/lifetime sections are rejected even when unselected.
            keyframe_refusal(&directory, replay.format, replay.capture_mode, None, table)?;
            if !selected[table.index()] {
                continue;
            }
            for section in [keyframe_section(table), later_section(table)] {
                if !later && section == later_section(table) {
                    continue;
                }
                match load(section, &mut evidence, check) {
                    Ok(chunk) => {
                        sections[section] = chunk;
                        verified[section] = usize::from(sections[section].is_some());
                    }
                    Err(error) if check.failed() || !super::super::recording_damage(&error) => {
                        return Err(error);
                    }
                    Err(error) => {
                        integrity_errors[section] =
                            Some(io::Error::new(error.kind(), error.to_string()));
                        failures[table.index()] = Some(error);
                        break;
                    }
                }
            }
        }
        Ok(Self {
            index: index.clone(),
            position,
            hours,
            entries,
            present,
            directory,
            sections,
            failures,
            diagnostics,
            verified,
            integrity_errors,
            evidence,
            _credit: credit,
        })
    }

    pub(crate) fn finish(
        &mut self,
        replay: &Replay,
        check: &mut Check<'_>,
    ) -> io::Result<blake3::Hash> {
        let owner = Arc::clone(&self._credit.0.owner);
        let _inflation = owner.inflation(check)?;
        for id in 0..SECTIONS {
            check.run()?;
            if let Some(error) = self.integrity_errors[id].take() {
                return Err(error);
            }
            let chunks = replay.section_chunks_from(&self.index, section(id), &self.directory)?;
            for chunk in chunks.skip(self.verified[id]) {
                let mut scratch = Vec::new();
                chunk.inflate_admitted(
                    &mut scratch,
                    false,
                    &mut |offset, bytes, check| self.evidence.record(id + 1, offset, bytes, check),
                    check,
                )?;
                self.verified[id] += 1;
            }
        }
        self.evidence.verify(replay, &self.index, check)
    }

    pub(crate) fn later_count(&self, table: TableId) -> usize {
        self.directory.chunk_extents(Section::Later(table)).len()
    }
    pub(crate) fn load_later(
        &mut self,
        replay: &Replay,
        table: TableId,
        slot: usize,
        check: &mut Check<'_>,
    ) -> io::Result<Option<ColumnChunk>> {
        #[cfg(test)]
        let _memory =
            super::super::reader::ownership::category(super::super::reader::ownership::Kind::Raw);
        let owner = Arc::clone(&self._credit.0.owner);
        let _inflation = owner.inflation(check)?;
        let id = later_section(table);
        if slot != self.verified[id] {
            return Err(invalid_static("later chunks are not sequential"));
        }
        let Some(chunk) = replay
            .section_chunks_from(&self.index, Section::Later(table), &self.directory)?
            .nth(slot)
        else {
            return Ok(None);
        };
        let first = self.hours.partition_point(|&hour| hour < chunk.first_hour);
        let end = self.hours.partition_point(|&hour| hour <= chunk.last_hour);
        let mut bytes = Vec::new();
        #[cfg(test)]
        let _step = super::super::check::Scope::enter(super::super::check::Step::Source(id));
        if let Err(error) = chunk.inflate_admitted(
            &mut bytes,
            true,
            &mut |offset, compressed, check| {
                self.evidence.record(id + 1, offset, compressed, check)
            },
            check,
        ) {
            if !check.failed() && super::super::recording_damage(&error) {
                self.integrity_errors[id] =
                    Some(io::Error::new(error.kind(), error.to_string()));
            }
            return Err(error);
        }
        self.verified[id] += 1;
        Ok(Some(ColumnChunk {
            bytes,
            section: id,
            slot,
            first,
            count: end - first,
            continuity: Continuity::Continue,
        }))
    }
    pub(crate) fn validate_point_envelopes(
        &self,
        replay: &Replay,
        selected: &[bool; TABLE_COUNT],
        check: &mut Check<'_>,
    ) -> io::Result<()> {
        let owner = Arc::clone(&self._credit.0.owner);
        let _inflation = owner.inflation(check)?;
        for table in TableId::ALL {
            if !selected[table.index()] {
                continue;
            }
            let chunks =
                replay.section_chunks_from(&self.index, Section::Later(table), &self.directory)?;
            let stored = chunks.len() != 0;
            let mut nonempty = false;
            for (slot, chunk) in chunks.enumerate() {
                if slot < self.verified[later_section(table)] {
                    nonempty |= chunk.raw_length != 0;
                    continue;
                }
                let mut raw = Vec::new();
                chunk.inflate_admitted(
                    &mut raw,
                    true,
                    &mut |_, bytes, _| {
                        owner
                            .validation_stored
                            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                        Ok(())
                    },
                    check,
                )?;
                owner
                    .validation_raw
                    .fetch_add(raw.len() as u64, Ordering::Relaxed);
                if raw.is_empty() {
                    continue;
                }
                if chunked::columns(&raw)?
                    .iter()
                    .all(|column| column.is_empty())
                {
                    return Err(invalid_static("non-empty chunk holds no column bytes"));
                }
                let fields = HISTORY[table.index()].fields(table, replay.format.version());
                let expected = FIELDS
                    + fields
                    + usize::from(super::super::aliases::private::stored(
                        table,
                        replay.format.version(),
                    ));
                if slot != 0 && chunked::columns(&raw)?.len() != expected {
                    return Err(invalid_static("chunk column counts disagree"));
                }
                let mut columns = ColumnReader::stored(
                    &raw,
                    HISTORY[table.index()].fields(table, replay.format.version()),
                    table,
                    replay.format.version(),
                )?;
                if !nonempty {
                    columns.modelled_rows(table, self.directory.layout)?;
                    nonempty = true;
                }
            }
            if stored && !nonempty {
                return Err(invalid_static("later section contains only empty chunks"));
            }
        }
        Ok(())
    }

    pub(crate) fn take_key(&mut self, table: TableId) -> Option<ColumnChunk> {
        self.sections[keyframe_section(table)].take()
    }
    pub(crate) fn credit(&self) -> Credit {
        self._credit.clone()
    }
    pub(crate) fn take_later(&mut self, table: TableId) -> Option<ColumnChunk> {
        let mut chunk = self.sections[later_section(table)].take()?;
        let last = (i64::from(self.index.first_hour) + LATER_CHUNK_HOURS as i64)
            .min(i64::from(self.index.last_hour));
        chunk.first = 1;
        chunk.count = self.hours.partition_point(|&hour| i64::from(hour) <= last) - 1;
        Some(chunk)
    }
    pub(crate) fn entered(&self, table: TableId, at: usize) -> bool {
        get(
            &self.entries[at * self.present.width()..(at + 1) * self.present.width()],
            self.present.ordinal[table.index()],
        )
    }
    pub(crate) fn refusal(
        &self,
        replay: &Replay,
        table: TableId,
    ) -> io::Result<Option<TableUpdate>> {
        keyframe_refusal(
            &self.directory,
            replay.format,
            replay.capture_mode,
            None,
            table,
        )
    }
}

pub(super) struct Columns {
    raw: Arc<Vec<u8>>,
    ranges: Vec<Range<usize>>,
    marks: Option<Range<usize>>,
}

impl Columns {
    pub(super) fn new(
        chunk: Option<ColumnChunk>,
        table: TableId,
        fields: usize,
        version: u16,
    ) -> io::Result<Self> {
        let Some(chunk) = chunk.filter(|chunk| !chunk.bytes.is_empty()) else {
            return Ok(Self {
                raw: Arc::new(Vec::new()),
                ranges: allocation::collect_vec((0..FIELDS + fields).map(|_| 0..0))?,
                marks: None,
            });
        };
        if chunk.section >= 3 && chunk.section % 2 == 1 {
            chunked::columns(&chunk.bytes)?;
        }
        let raw = chunk.bytes;
        let input = ColumnReader::stored(&raw, fields, table, version)?;
        if input.0.iter().all(|column| column.remaining() == 0)
            && input.1.is_none_or(|marks| marks.is_empty())
        {
            return Err(invalid_static("non-empty chunk holds no column bytes"));
        }
        let range = |bytes: &[u8]| {
            let start = bytes.as_ptr() as usize - raw.as_ptr() as usize;
            start..start + bytes.len()
        };
        let ranges = allocation::collect_vec(input.0.iter().map(|column| range(column.rest())))?;
        let marks = input.1.map(range);
        drop(input);
        Ok(Self {
            raw: Arc::new(raw),
            ranges,
            marks,
        })
    }
    pub(super) fn with<T>(
        &mut self,
        read: impl FnOnce(&mut ColumnReader<'_>) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut input = ColumnReader(
            allocation::collect_vec(
                self.ranges
                    .iter()
                    .map(|range| Decoder::new(&self.raw[range.clone()])),
            )?,
            None,
            0,
        );
        let value = read(&mut input)?;
        for (range, column) in self.ranges.iter_mut().zip(input.0.iter()) {
            range.start = range.end - column.remaining();
        }
        Ok(value)
    }
    #[cfg(test)]
    pub(super) fn owner(&self) -> std::sync::Weak<Vec<u8>> {
        Arc::downgrade(&self.raw)
    }
    pub(super) fn marks(&self) -> Option<&[u8]> {
        self.marks.as_ref().map(|range| &self.raw[range.clone()])
    }
    pub(super) fn finish(&self) -> io::Result<()> {
        if self.ranges.iter().any(|range| !range.is_empty()) {
            Err(invalid_static("grouped section has unread column bytes"))
        } else {
            Ok(())
        }
    }
}
