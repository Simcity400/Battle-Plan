//! Where a recording's saves end, and saving to a recording in place.
//!
//! A recording grows by appending: each save writes its blocks after the last
//! one, then a trailer naming only what it adds and the trailer before it,
//! then its footer. The footer at the end of the file names the newest save.
//! The header's commit slot holds a copy of the last complete save's footer, so
//! an interrupted save, which leaves bytes after that footer, reopens as the
//! save before it. Nothing a complete save wrote is ever written again.

use super::*;
use crate::rows::compatibility::CHECKPOINT_FORMAT;

/// The newest complete save's footer and its checksum, then four zero bytes.
pub(super) const SLOT_SIZE: usize = 32;

/// Whether recordings of `format` have a commit slot and linked trailers.
pub(super) fn appendable(format: FormatVersion) -> bool {
    format.version() >= CHECKPOINT_FORMAT
}

/// One complete save of a recording: its newest trailer and where the save ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision {
    /// Where the save's footer ends.
    pub end: u64,
    pub trailer: u64,
    pub length: u64,
    pub crc: u32,
}

impl Revision {
    /// A number naming this save of its recording, for records kept beside it.
    pub fn digest(self) -> u64 {
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        [self.end, self.trailer, self.length, u64::from(self.crc)]
            .into_iter()
            .fold(0xcbf2_9ce4_8422_2325, |hash, word| {
                (hash ^ word).wrapping_mul(PRIME)
            })
    }

    fn footer(self) -> [u8; FOOTER_SIZE] {
        let mut footer = [0; FOOTER_SIZE];
        footer[..4].copy_from_slice(FOOTER_MAGIC);
        footer[4..12].copy_from_slice(&self.trailer.to_le_bytes());
        footer[12..20].copy_from_slice(&self.length.to_le_bytes());
        footer[20..].copy_from_slice(&self.crc.to_le_bytes());
        footer
    }

    pub(super) fn slot(self) -> [u8; SLOT_SIZE] {
        let footer = self.footer();
        let mut slot = [0; SLOT_SIZE];
        slot[..FOOTER_SIZE].copy_from_slice(&footer);
        slot[FOOTER_SIZE..FOOTER_SIZE + 4].copy_from_slice(&crc32fast::hash(&footer).to_le_bytes());
        slot
    }
}

/// A footer ending at `end`: its trailer's start, length and checksum.
fn footer_at(bytes: &[u8], end: u64) -> Option<Revision> {
    let mut footer = Decoder::new(bytes);
    if footer.take(4).ok()? != FOOTER_MAGIC {
        return None;
    }
    let (trailer, length, crc) = (footer.u64().ok()?, footer.u64().ok()?, footer.u32().ok()?);
    footer.finish().ok()?;
    let footer_end = trailer
        .checked_add(length)?
        .checked_add(FOOTER_SIZE as u64)?;
    (footer_end == end).then_some(Revision {
        end,
        trailer,
        length,
        crc,
    })
}

/// A recording's bytes, resident or in a file.
pub(super) trait Source {
    fn length(&mut self) -> io::Result<u64>;
    fn read_at(&mut self, offset: u64, output: &mut [u8]) -> io::Result<()>;
}

pub(super) struct Resident<'a>(pub(super) &'a [u8]);

impl Source for Resident<'_> {
    fn length(&mut self) -> io::Result<u64> {
        Ok(self.0.len() as u64)
    }
    fn read_at(&mut self, offset: u64, output: &mut [u8]) -> io::Result<()> {
        let start = to_usize(offset)?;
        let bytes = start
            .checked_add(output.len())
            .and_then(|end| self.0.get(start..end))
            .ok_or_else(|| invalid_static("truncated field"))?;
        output.copy_from_slice(bytes);
        Ok(())
    }
}

pub(super) struct Seeking<'a, R>(pub(super) &'a mut R);

impl<R: Read + Seek> Source for Seeking<'_, R> {
    fn length(&mut self) -> io::Result<u64> {
        self.0.seek(SeekFrom::End(0))
    }
    fn read_at(&mut self, offset: u64, output: &mut [u8]) -> io::Result<()> {
        self.0.seek(SeekFrom::Start(offset))?;
        self.0.read_exact(output)
    }
}

fn read_vec(source: &mut dyn Source, offset: u64, length: u64) -> io::Result<Vec<u8>> {
    let mut bytes = engine_allocation::filled(to_usize(length)?, 0)?;
    source.read_at(offset, &mut bytes)?;
    Ok(bytes)
}

/// A recording's header, its trailers oldest first, and its newest complete save.
pub(super) struct Located {
    /// Through the commit slot, when the format has one.
    pub(super) header: Vec<u8>,
    pub(super) slot: Option<usize>,
    pub(super) trailers: Vec<(usize, Vec<u8>)>,
    pub(super) revision: Revision,
    pub(super) warnings: Vec<String>,
}

/// The header's bytes, through its commit slot, and where that slot is.
fn header(source: &mut dyn Source, length: u64) -> io::Result<(Vec<u8>, Option<usize>)> {
    if length < 14 {
        return Err(invalid_static("truncated field"));
    }
    let mut prefix = [0; 14];
    source.read_at(0, &mut prefix)?;
    expect_magic(&mut Decoder::new(&prefix[..4]), HEADER_MAGIC)?;
    let format = FormatVersion::read(u16::from_le_bytes([prefix[4], prefix[5]]))?;
    let identity_at = 14_u64
        .checked_add(u64::from(u32::from_le_bytes(
            prefix[10..].try_into().unwrap(),
        )))
        .filter(|at| at.checked_add(4).is_some_and(|end| end <= length))
        .ok_or_else(|| invalid_static("truncated field"))?;
    let mut size = [0; 4];
    source.read_at(identity_at, &mut size)?;
    let slot = identity_at + 4 + u64::from(u32::from_le_bytes(size));
    let end = slot
        + if appendable(format) {
            SLOT_SIZE as u64
        } else {
            0
        };
    if end > length {
        return Err(invalid_static("truncated field"));
    }
    let slot = appendable(format).then(|| to_usize(slot)).transpose()?;
    Ok((read_vec(source, 0, end)?, slot))
}

/// The footer the file ends with, as opening it checks it.
fn last_footer(source: &mut dyn Source, length: u64) -> io::Result<Revision> {
    if length < FOOTER_SIZE as u64 {
        return Err(invalid("missing replay footer"));
    }
    let mut bytes = [0; FOOTER_SIZE];
    source.read_at(length - FOOTER_SIZE as u64, &mut bytes)?;
    expect_magic(&mut Decoder::new(&bytes[..4]), FOOTER_MAGIC)?;
    footer_at(&bytes, length).ok_or_else(|| invalid("invalid trailer extent"))
}

/// The save the commit slot names, when its footer is still where it says.
fn slot_save(source: &mut dyn Source, slot: &[u8], length: u64) -> Option<Revision> {
    let (footer, crc) = slot.split_at(FOOTER_SIZE);
    if crc32fast::hash(footer).to_le_bytes() != crc[..4] {
        return None;
    }
    let mut bytes = Decoder::new(&footer[4..20]);
    let end = bytes
        .u64()
        .ok()?
        .checked_add(bytes.u64().ok()?)?
        .checked_add(FOOTER_SIZE as u64)?;
    let revision = footer_at(footer, end).filter(|revision| revision.end <= length)?;
    let mut found = [0; FOOTER_SIZE];
    source.read_at(end - FOOTER_SIZE as u64, &mut found).ok()?;
    (found[..] == *footer).then_some(revision)
}

/// The trailer an appendable recording's trailer adds to, from the link before its index.
fn trailer_link(trailer: &[u8]) -> io::Result<Option<(u64, u64)>> {
    let mut input = Decoder::new(trailer);
    expect_magic(&mut input, TRAILER_MAGIC)?;
    for _ in 0..5 {
        let count = input.count(4)?;
        for _ in 0..count {
            let length = input.u32()? as usize;
            input.take(length)?;
        }
    }
    let (start, length) = (input.u64()?, input.u64()?);
    Ok((start, length).ne(&(0, 0)).then_some((start, length)))
}

fn read_trailers(
    source: &mut dyn Source,
    header_length: usize,
    linked: bool,
    revision: Revision,
) -> io::Result<(Vec<(usize, Vec<u8>)>, Vec<String>)> {
    let mut warnings = Vec::new();
    let mut trailers = Vec::new();
    let (mut start, mut trailer_length, mut crc) =
        (revision.trailer, revision.length, revision.crc);
    loop {
        if start < header_length as u64 {
            return Err(invalid("invalid trailer extent"));
        }
        let bytes = read_vec(source, start, trailer_length)?;
        if crc32fast::hash(&bytes) != crc {
            engine_allocation::reserve(&mut warnings, 1)?;
            warnings.push("trailer CRC mismatch".into());
        }
        let link = if linked { trailer_link(&bytes)? } else { None };
        engine_allocation::reserve(&mut trailers, 1)?;
        trailers.push((to_usize(start)?, bytes));
        let Some((previous, previous_length)) = link else {
            break;
        };
        // Each trailer ends, footer and all, before the next one starts.
        let end = previous
            .checked_add(previous_length)
            .and_then(|end| end.checked_add(FOOTER_SIZE as u64))
            .filter(|end| *end <= start)
            .ok_or_else(|| invalid("invalid trailer chain"))?;
        let mut footer = [0; FOOTER_SIZE];
        source.read_at(end - FOOTER_SIZE as u64, &mut footer)?;
        let found = footer_at(&footer, end)
            .filter(|found| (found.trailer, found.length) == (previous, previous_length))
            .ok_or_else(|| invalid("invalid trailer chain"))?;
        (start, trailer_length, crc) = (previous, previous_length, found.crc);
    }
    trailers.reverse();
    Ok((trailers, warnings))
}

fn locate_snapshot(
    source: &mut dyn Source,
    length: u64,
    header: Vec<u8>,
    slot: Option<usize>,
) -> io::Result<Located> {
    let candidate = |source: &mut dyn Source, revision| {
        read_trailers(source, header.len(), slot.is_some(), revision)
            .map(|(trailers, warnings)| (revision, trailers, warnings))
    };
    let last = last_footer(source, length).and_then(|revision| candidate(source, revision));
    let selected = match last {
        Ok(found) if found.2.is_empty() => found,
        last => {
            let earlier = slot
                .and_then(|at| slot_save(source, &header[at..at + SLOT_SIZE], length))
                .and_then(|revision| candidate(source, revision).ok())
                .filter(|found| found.2.is_empty());
            match earlier {
                Some(found) => found,
                None => last?,
            }
        }
    };
    Ok(Located {
        header,
        slot,
        trailers: selected.1,
        revision: selected.0,
        warnings: selected.2,
    })
}

/// Retry a failed snapshot while its length or slot changes, or a slot write may be torn.
pub(super) fn locate(source: &mut dyn Source) -> io::Result<Located> {
    for attempt in 0..8 {
        let length = source.length()?;
        let (bytes, slot) = header(source, length)?;
        let sampled = slot.map(|at| bytes[at..at + SLOT_SIZE].to_vec());
        let result = locate_snapshot(source, length, bytes, slot);
        if result.as_ref().is_ok_and(|found| found.warnings.is_empty()) {
            return result;
        }
        let current_length = source.length()?;
        let current_slot = slot
            .map(|at| read_vec(source, at as u64, SLOT_SIZE as u64))
            .transpose()?;
        let torn_slot = sampled.as_ref().is_some_and(|slot| {
            crc32fast::hash(&slot[..FOOTER_SIZE]).to_le_bytes()
                != slot[FOOTER_SIZE..FOOTER_SIZE + 4]
        });
        if length == current_length && sampled == current_slot && (!torn_slot || attempt == 7) {
            return result;
        }
        std::thread::yield_now();
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "the recording changed throughout the commit snapshot; retry opening it",
    ))
}

/// Write a trailer adding these entries, linked to `previous`, and its footer, at the end of `out`,
/// whose first byte is at `at` in the file.
#[allow(clippy::too_many_arguments)]
pub(super) fn encode_trailer(
    out: &mut Vec<u8>,
    at: u64,
    sessions: &[SessionStart],
    branches: &[BranchStart],
    gaps: &[Gap],
    controls: &[ControlRecord],
    recovery_notes: &[RecoveryNote],
    previous: Option<(u64, u64)>,
    index: &[BlockIndex],
) -> io::Result<Revision> {
    let trailer_start = out.len();
    out.extend_from_slice(TRAILER_MAGIC);
    encode_list(out, sessions, |session, out| session.encode(out))?;
    encode_list(out, branches, BranchStart::encode)?;
    encode_list(out, gaps, Gap::encode)?;
    encode_list(out, controls, ControlRecord::encode)?;
    encode_list(out, recovery_notes, RecoveryNote::encode)?;
    let (start, length) = previous.unwrap_or((0, 0));
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&length.to_le_bytes());
    put_count(out, index.len())?;
    for block in index {
        out.extend_from_slice(&block.branch.to_le_bytes());
        out.extend_from_slice(&block.first_hour.to_le_bytes());
        out.extend_from_slice(&block.last_hour.to_le_bytes());
        out.extend_from_slice(&block.offset.to_le_bytes());
        out.extend_from_slice(&block.length.to_le_bytes());
        out.extend_from_slice(&block.raw_length.to_le_bytes());
    }
    let length = (out.len() - trailer_start) as u64;
    let trailer = at + trailer_start as u64;
    let revision = Revision {
        end: trailer + length + FOOTER_SIZE as u64,
        trailer,
        length,
        crc: crc32fast::hash(&out[trailer_start..]),
    };
    out.extend_from_slice(&revision.footer());
    Ok(revision)
}

/// Point a resident recording's commit slot at the footer it ends with.
pub(super) fn stamp_slot(bytes: &mut [u8], slot: Option<usize>) -> io::Result<()> {
    let Some(at) = slot else {
        return Ok(());
    };
    let revision = last_footer(&mut Resident(bytes), bytes.len() as u64)?;
    bytes[at..at + SLOT_SIZE].copy_from_slice(&revision.slot());
    Ok(())
}

/// What the trailer names of the bytes it ends: each earlier trailer and its footer.
pub(super) fn trailer_regions(trailers: &[(usize, Vec<u8>)]) -> Vec<(u64, u64)> {
    trailers[..trailers.len().saturating_sub(1)]
        .iter()
        .map(|(start, bytes)| (*start as u64, (*start + bytes.len() + FOOTER_SIZE) as u64))
        .collect()
}

pub(super) fn rollback_temporary(path: &Path) -> PathBuf {
    crate::publication::temporary_beside(path, "rollback")
}

fn remove_rollback_temporary(path: &Path) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        result => result?,
    };
    let mut ordinary = metadata.is_file() && !metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ordinary &= metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            == 0;
    }
    if !ordinary {
        return Err(invalid("the stale rollback copy is not an ordinary file"));
    }
    std::fs::remove_file(path)
}

impl Replay {
    /// This save of the recording.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Whether this undamaged recording's linked trailer chain includes this exact save.
    pub fn contains_revision(&self, revision: Revision) -> io::Result<bool> {
        self.require_undamaged()?;
        if self.revision == revision {
            return Ok(true);
        }
        if self.slot.is_none() || !self.regions.contains(&(revision.trailer, revision.end)) {
            return Ok(false);
        }
        let mut footer = [0; FOOTER_SIZE];
        self.read_at(revision.end - FOOTER_SIZE as u64, &mut footer)?;
        Ok(footer == revision.footer())
    }

    /// Whether `other` is the same save of the same recording, as far as its trailers tell.
    pub fn same_revision(&self, other: &Replay) -> bool {
        self.revision == other.revision
            && self.campaign_id == other.campaign_id
            && self.index == other.index
    }

    /// The bytes of the recording's complete saves; an interrupted save's follow them.
    pub fn saved_length(&self) -> u64 {
        self.revision.end
    }

    /// Open a recording to add to it in place. Others may read it meanwhile, never delete,
    /// rename or replace it.
    pub fn open_appendable(path: &Path) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(
                windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ
                    | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE,
            );
        }
        let mut file = options.open(path)?;
        let mut replay = Self::open_file_metadata(&mut file)?;
        replay.file = Some(file);
        replay.appendable = true;
        replay.append_path = Some(path.to_owned());
        Ok(replay)
    }

    /// Add the recovered segments to this recording in place, as
    /// [`Replay::append_recovered_with_reader`] adds them to a copy: crash-safe, holding
    /// only the new blocks. Returns once the new save is flushed, reopened and every new block
    /// read back whole; a failure leaves the recording as it was.
    /// `prepare` receives the intended revision before any bytes are changed.
    pub fn append_in_place(
        &mut self,
        segments: &[(u32, Recovery<Recovered>)],
        read: impl Fn(u32, &FrameMetadata) -> io::Result<Frame> + Sync,
        mut progress: impl FnMut() -> io::Result<()>,
        mut observe: impl FnMut(&Observed) -> io::Result<()>,
        ahead: Option<Ahead>,
        prepare: impl FnOnce(Revision) -> io::Result<()>,
    ) -> io::Result<()> {
        if !self.appendable {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "this recording was not opened to be added to",
            ));
        }
        self.require_undamaged()?;
        if self.format != FormatVersion::CURRENT || self.slot.is_none() {
            return Err(invalid(format!(
                "Replay append requires format {FORMAT_VERSION}; this recording is format {}; open it read-only and record into a new file",
                self.format.version()
            )));
        }
        require_current_spools(segments)?;
        let records = ordered_records(segments)?;
        let numbers = segment_numbers(segments);
        let read_frame = |position: usize, frame: &FrameMetadata| read(numbers[position], frame);
        let pending = collect(
            &records,
            &self.identity,
            &self.campaign_id,
            Some(self),
            self.keyframe_interval,
            Observer::Hours(&mut observe),
            Frames::Shared(&read_frame, &mut progress),
            MAX_BLOCK_RAW_BYTES,
            ahead,
        )?;
        let notes = recovery_notes(segments)?;
        let before = self.revision;
        let mut mutated = false;
        let written = self.write_save(pending, &notes, prepare, &mut mutated);
        let checked = written.and_then(|()| self.reread(before));
        if let Err(error) = checked {
            // Replace the failed save without invalidating readers that already opened it.
            if mutated && let Err(rollback) = self.cut_to(before) {
                return Err(io::Error::new(
                    error.kind(),
                    format!("{error}; the recording could not be restored: {rollback}"),
                ));
            }
            return Err(error);
        }
        Ok(())
    }

    /// Write the save after the last complete one and commit it, footer then slot.
    fn write_save(
        &mut self,
        pending: Pending,
        notes: &[RecoveryNote],
        prepare: impl FnOnce(Revision) -> io::Result<()>,
        mutated: &mut bool,
    ) -> io::Result<()> {
        let _phase = Packing::phase(Phase::Write);
        let slot = self.slot.unwrap() as u64;
        let before = self.revision;
        let previous = Some((before.trailer, before.length));
        let mut index = Vec::new();
        let mut at = before.end;
        engine_allocation::reserve(&mut index, pending.blocks.len())?;
        for block in &pending.blocks {
            index.push(BlockIndex {
                branch: block.branch,
                first_hour: block.first_hour,
                last_hour: block.last_hour,
                offset: at,
                length: block.bytes.len() as u64,
                raw_length: block.raw_length,
            });
            at += block.bytes.len() as u64;
        }
        let mut trailer = Vec::new();
        let revision = encode_trailer(
            &mut trailer,
            at,
            &pending.sessions,
            &pending.branches,
            &pending.gaps,
            &pending.controls,
            notes,
            previous,
            &index,
        )?;
        prepare(revision)?;
        if self.file.as_ref().unwrap().metadata()?.len() > before.end {
            *mutated = true;
            self.cut_to(before)?;
        }
        let mut file = Budgeted::new(self.file.as_mut().unwrap());
        // A crash from here on reopens the recording as the save the slot names.
        let mut named = [0; SLOT_SIZE];
        file.seek(SeekFrom::Start(slot))?;
        file.read_exact(&mut named)?;
        if named != before.slot() {
            *mutated = true;
            file.seek(SeekFrom::Start(slot))?;
            file.write_all(&before.slot())?;
            file.get_ref().sync_data()?;
        }
        file.seek(SeekFrom::Start(before.end))?;
        *mutated = true;
        for block in pending.blocks {
            file.write_all(&block.bytes)?;
        }
        let (body, footer) = trailer.split_at(trailer.len() - FOOTER_SIZE);
        file.write_all(body)?;
        file.get_ref().sync_data()?;
        // The footer at the end commits the save.
        file.write_all(footer)?;
        file.get_ref().sync_data()?;
        file.seek(SeekFrom::Start(slot))?;
        file.write_all(&revision.slot())?;
        file.get_ref().sync_data()
    }

    /// Reopen the save just written and read every block it added back whole.
    fn reread(&mut self, before: Revision) -> io::Result<()> {
        let _phase = Packing::phase(Phase::Readback);
        let mut file = self.file.take().unwrap();
        let reopened = Self::open_file_metadata(&mut file);
        self.file = Some(file);
        let mut reopened = reopened?;
        reopened.require_undamaged()?;
        if reopened.revision.end <= before.end
            || reopened.campaign_id != self.campaign_id
            || !reopened.sessions.starts_with(&self.sessions)
            || !reopened.branches.starts_with(&self.branches)
            || !reopened.gaps.starts_with(&self.gaps)
            || !reopened.controls.starts_with(&self.controls)
            || !reopened.recovery_notes.starts_with(&self.recovery_notes)
            || !reopened.index.starts_with(&self.index)
        {
            return Err(invalid(
                "the save written does not reopen as the recording with its additions",
            ));
        }
        reopened.file = self.file.take();
        reopened.appendable = true;
        reopened.append_path = self.append_path.take();
        let added = self.index.len();
        *self = reopened;
        for block in self.index[added..].to_vec() {
            self.read_block_parallel(
                &block,
                &Parallel {
                    workers: pack::workers(),
                    on_worker: &pack::refresh,
                },
            )?;
        }
        Ok(())
    }

    /// Replace, never truncate: readers holding any published revision retain its bytes.
    fn cut_to(&mut self, revision: Revision) -> io::Result<()> {
        let path = self.append_path.as_ref().unwrap().clone();
        self.replace_revision(&path, revision)?;
        *self = Self::open_appendable(&path)?;
        Ok(())
    }

    fn replace_revision(&mut self, path: &Path, revision: Revision) -> io::Result<()> {
        let mut footer = [0; FOOTER_SIZE];
        self.read_at(
            revision
                .end
                .checked_sub(FOOTER_SIZE as u64)
                .ok_or_else(|| invalid("invalid rollback revision"))?,
            &mut footer,
        )?;
        if footer != revision.footer() {
            return Err(invalid(
                "the save to go back to is no longer in the recording",
            ));
        }
        let temporary = rollback_temporary(path);
        remove_rollback_temporary(&temporary)?;
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.share_mode(0);
            }
            let mut output = Budgeted::new(options.open(&temporary)?);
            let mut chunk = engine_allocation::filled(1 << 20, 0)?;
            let mut at = 0;
            while at < revision.end {
                let length = (chunk.len() as u64).min(revision.end - at) as usize;
                self.read_at(at, &mut chunk[..length])?;
                output.write_all(&chunk[..length])?;
                at += length as u64;
            }
            if let Some(slot) = self.slot {
                output.seek(SeekFrom::Start(slot as u64))?;
                output.write_all(&revision.slot())?;
            }
            output.get_ref().sync_all()?;
            drop(output);
            let checked = Self::open_path(&temporary)?;
            checked.require_undamaged()?;
            if checked.revision != revision {
                return Err(invalid(
                    "the rollback copy did not reopen at the requested save",
                ));
            }
            for block in &checked.index {
                checked.read_block(block)?;
            }
            // The writer blocks replacement until it closes; read-only handles share deletion.
            self.file.take();
            replace_file(&temporary, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    /// Replace the recording with an earlier save; open readers keep the replaced file intact.
    pub fn roll_back(path: &Path, revision: Revision) -> io::Result<()> {
        let mut replay = Self::open_appendable(path)?;
        if replay.revision == revision
            && replay.file.as_ref().unwrap().metadata()?.len() == revision.end
        {
            return crate::publication::sync_file(path);
        }
        if replay.revision != revision
            && !replay.regions.iter().any(|&(_, end)| end == revision.end)
        {
            return Err(invalid(
                "the recording no longer holds the save to go back to",
            ));
        }
        replay.cut_to(revision)
    }

    /// Copy the recording's complete saves to a new file at `path`, flushed.
    pub fn copy_saved(&self, path: &Path) -> io::Result<()> {
        let mut output = Budgeted::new(OpenOptions::new().write(true).create_new(true).open(path)?);
        let copied = (|| {
            let mut chunk = engine_allocation::filled(1 << 20, 0)?;
            let mut at = 0;
            while at < self.revision.end {
                let length = (chunk.len() as u64).min(self.revision.end - at) as usize;
                self.read_at(at, &mut chunk[..length])?;
                output.write_all(&chunk[..length])?;
                at += length as u64;
            }
            output.get_ref().sync_all()
        })();
        if copied.is_err() {
            drop(output);
            let _ = std::fs::remove_file(path);
        }
        copied
    }
}

/// The recovery notes a finalization of these segments adds, in segment order.
pub(super) fn recovery_notes<R: SpoolRecord>(
    segments: &[(u32, Recovery<R>)],
) -> io::Result<Vec<RecoveryNote>> {
    let mut sorted: Vec<_> = segments.iter().collect();
    sorted.sort_unstable_by_key(|(number, _)| *number);
    let mut notes = Vec::new();
    for (number, recovery) in sorted {
        let session = recovery
            .records
            .first()
            .and_then(|record| match record.view() {
                RecordRef::SessionStart(session) => Some(session.id),
                RecordRef::BranchStart(branch) => Some(branch.session),
                _ => None,
            });
        if let Some(reason) = &recovery.damage {
            engine_allocation::reserve(&mut notes, 1)?;
            notes.push(RecoveryNote {
                session,
                segment: *number,
                valid_bytes: recovery.valid_bytes as u64,
                reason: reason.clone(),
            });
        }
    }
    Ok(notes)
}
