//! Held recording files and selected, block-positioned preparation.

use super::*;

#[cfg(test)]
thread_local! {
    static HELD_VISIT_PASSES: std::cell::RefCell<Option<Vec<bool>>> = const { std::cell::RefCell::new(None) };
    static HELD_READ_BYTES: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(super) fn measure_held_reads<T>(read: impl FnOnce() -> T) -> (T, u64) {
    HELD_READ_BYTES.with(|bytes| bytes.set(Some(0)));
    let result = read();
    let bytes = HELD_READ_BYTES.with(|bytes| bytes.replace(None).unwrap());
    (result, bytes)
}

#[cfg(test)]
pub(super) fn measure_held_passes<T>(read: impl FnOnce() -> T) -> (T, Vec<bool>) {
    HELD_VISIT_PASSES.with(|passes| *passes.borrow_mut() = Some(Vec::new()));
    let result = read();
    let passes = HELD_VISIT_PASSES.with(|passes| passes.borrow_mut().take().unwrap());
    (result, passes)
}

struct PositionalReader<'a> {
    file: &'a File,
    offset: u64,
    remaining: u64,
}

impl Read for PositionalReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let length = self.remaining.min(output.len() as u64) as usize;
        if length == 0 {
            return Ok(0);
        }
        #[cfg(windows)]
        let read = {
            use std::os::windows::fs::FileExt;
            self.file.seek_read(&mut output[..length], self.offset)?
        };
        #[cfg(unix)]
        let read = {
            use std::os::unix::fs::FileExt;
            self.file.read_at(&mut output[..length], self.offset)?
        };
        self.offset += read as u64;
        self.remaining -= read as u64;
        Ok(read)
    }
}

impl Replay {
    /// Open a held, read-only recording revision without buffering its blocks.
    /// Header/trailer admission and recovery are shared with `open`.
    pub fn open_path(path: &Path) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(
                windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ
                    | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE
                    | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_DELETE,
            );
        }
        let mut file = options.open(path)?;
        let mut replay = Self::open_file_metadata(&mut file)?;
        replay.file = Some(file);
        Ok(replay)
    }

    pub(super) fn open_file_metadata(file: &mut (impl Read + Seek)) -> io::Result<Self> {
        let mut replay = Self::open_metadata(commit::locate(&mut commit::Seeking(file))?)?;
        replay.held_length = replay.revision.end;
        Ok(replay)
    }

    pub fn visit_held_bytes(
        &self,
        mut visit: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        let length = if self.file.is_some() {
            self.held_length
        } else {
            self.revision.end
        };
        let mut buffer = [0; 64 * 1024];
        let mut offset = 0;
        while offset < length {
            let count = (length - offset).min(buffer.len() as u64) as usize;
            self.read_at(offset, &mut buffer[..count])?;
            // The live slot moves; a held revision's byte stream must not.
            if let Some(slot) = self.slot {
                let start = (slot as u64).max(offset);
                let end = ((slot + commit::SLOT_SIZE) as u64).min(offset + count as u64);
                if start < end {
                    let saved = self.revision.slot();
                    buffer[(start - offset) as usize..(end - offset) as usize]
                        .copy_from_slice(&saved[(start - slot as u64) as usize..(end - slot as u64) as usize]);
                }
            }
            visit(&buffer[..count])?;
            offset += count as u64;
        }
        Ok(())
    }

    pub fn is_file_backed(&self) -> bool {
        self.file.is_some()
    }

    /// Whether this recording holds the one at `earlier` unchanged, as an append leaves it: every
    /// byte of its saves but its commit slot, and each trailer list as the start of this one's.
    /// Replacing `earlier` with this then loses nothing. A damaged recording is never held.
    pub fn holds(&self, earlier: &Path) -> io::Result<bool> {
        let earlier = Self::open_path(earlier)?;
        let end = earlier.revision.end;
        if self.needed_recovery()
            || earlier.needed_recovery()
            || earlier.slot != self.slot
            || end > self.revision.end
            || !self.sessions.starts_with(&earlier.sessions)
            || !self.branches.starts_with(&earlier.branches)
            || !self.gaps.starts_with(&earlier.gaps)
            || !self.controls.starts_with(&earlier.controls)
            || !self.recovery_notes.starts_with(&earlier.recovery_notes)
            || !self.index.starts_with(&earlier.index)
        {
            return Ok(false);
        }
        // The slot names each file's own newest save.
        let slot = earlier
            .slot
            .map_or(0..0, |slot| slot as u64..(slot + commit::SLOT_SIZE) as u64);
        let mut theirs = engine_allocation::filled(end.min(1 << 20) as usize, 0)?;
        let mut ours = engine_allocation::filled(theirs.len(), 0)?;
        let mut at = 0;
        while at < end {
            let length = (theirs.len() as u64).min(end - at) as usize;
            let (theirs, ours) = (&mut theirs[..length], &mut ours[..length]);
            earlier.read_at(at, theirs)?;
            self.read_at(at, ours)?;
            let blank = slot.start.max(at)..slot.end.min(at + length as u64);
            if !blank.is_empty() {
                let blank = (blank.start - at) as usize..(blank.end - at) as usize;
                theirs[blank.clone()].fill(0);
                ours[blank].fill(0);
            }
            if theirs != ours {
                return Ok(false);
            }
            at += length as u64;
        }
        Ok(true)
    }

    pub(super) fn held_block_bytes(&self, index: &BlockIndex) -> io::Result<Vec<u8>> {
        let _cpu = crate::cpu::scope("source_read", None);
        let mut bytes = engine_allocation::filled(to_usize(index.length)?, 0)?;
        self.read_at(index.offset, &mut bytes)?;
        Ok(bytes)
    }

    pub(super) fn held_grouped(&self, index: &BlockIndex) -> io::Result<bool> {
        if index.length < 4 {
            return Ok(false);
        }
        let mut magic = [0; 4];
        self.read_at(index.offset, &mut magic)?;
        Ok(grouped::is_grouped(&magic))
    }

    pub(super) fn block_range(
        &self,
        index: &BlockIndex,
        offset: usize,
        length: usize,
    ) -> io::Result<Cow<'_, [u8]>> {
        let _cpu = crate::cpu::scope("source_read", None);
        let end = offset
            .checked_add(length)
            .filter(|end| *end as u64 <= index.length)
            .ok_or_else(|| invalid("grouped section outside its block"))?;
        let start = index
            .offset
            .checked_add(offset as u64)
            .ok_or_else(|| invalid("block offset overflow"))?;
        if self.file.is_some() {
            let mut bytes = engine_allocation::filled(length, 0)?;
            self.read_at(start, &mut bytes)?;
            return Ok(Cow::Owned(bytes));
        }
        let end = index
            .offset
            .checked_add(end as u64)
            .ok_or_else(|| invalid("block offset overflow"))?;
        self.bytes
            .get(to_usize(start)?..to_usize(end)?)
            .map(Cow::Borrowed)
            .ok_or_else(|| invalid("block outside replay"))
    }

    pub(super) fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<()> {
        match &self.file {
            Some(file) => {
                #[cfg(test)]
                HELD_READ_BYTES.with(|bytes| {
                    if let Some(total) = bytes.get() {
                        bytes.set(Some(total + output.len() as u64));
                    }
                });
                PositionalReader {
                    file,
                    offset,
                    remaining: output.len() as u64,
                }
                .read_exact(output)
            }
            None => {
                let start = to_usize(offset)?;
                let end = start
                    .checked_add(output.len())
                    .ok_or_else(|| invalid("block offset overflow"))?;
                output.copy_from_slice(
                    self.bytes
                        .get(start..end)
                        .ok_or_else(|| invalid("block outside replay"))?,
                );
                Ok(())
            }
        }
    }

    pub(super) fn visit_file_block(
        &self,
        index: &BlockIndex,
        selected: Option<&[TableId]>,
        visit: &mut dyn FnMut(Frame) -> io::Result<bool>,
    ) -> io::Result<()> {
        self.visit_file_block_with(index, selected, visit, &mut || Ok(()))
    }

    fn visit_file_block_with(
        &self,
        index: &BlockIndex,
        selected: Option<&[TableId]>,
        visit: &mut dyn FnMut(Frame) -> io::Result<bool>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        check()?;
        let file = self.file.as_ref().unwrap();
        let position = self
            .block_position(index)
            .ok_or_else(|| invalid("block outside replay"))?;
        // Admission checks all wire grammar and the checksum before any visitor receives a frame.
        if !self.verified[position].load(Ordering::Acquire) {
            self.visit_file_pass(
                file,
                index,
                selected,
                &mut |_| {
                    check()?;
                    Ok(true)
                },
                true,
            )?;
            self.verified[position].store(true, Ordering::Release);
        }
        self.visit_file_pass(
            file,
            index,
            selected,
            &mut |frame| {
                check()?;
                visit(frame)
            },
            false,
        )
    }

    fn visit_file_pass(
        &self,
        file: &File,
        index: &BlockIndex,
        selected: Option<&[TableId]>,
        visit: &mut dyn FnMut(Frame) -> io::Result<bool>,
        validate: bool,
    ) -> io::Result<()> {
        #[cfg(test)]
        HELD_VISIT_PASSES.with(|passes| {
            if let Some(passes) = &mut *passes.borrow_mut() {
                passes.push(validate);
            }
        });
        if index.raw_length > MAX_BLOCK_RAW_BYTES as u64 {
            return Err(invalid("block exceeds raw size limit"));
        }
        let mut magic = [0; 4];
        PositionalReader {
            file,
            offset: index.offset,
            remaining: magic.len() as u64,
        }
        .read_exact(&mut magic)?;
        if magic != [0x28, 0xb5, 0x2f, 0xfd] {
            return Err(invalid("block is not a zstd frame"));
        }
        let mut decoder = zstd::stream::read::Decoder::with_buffer(io::BufReader::with_capacity(
            64 * 1024,
            PositionalReader {
                file,
                offset: index.offset,
                remaining: index.length,
            },
        ))
        .map_err(zstd_error)?
        .single_frame();
        decoder.window_log_max(26).map_err(zstd_error)?;
        let mut raw = Inflated {
            stream: decoder.take(index.raw_length + 1),
            read: 0,
        };
        // Only the completed grammar and trailing-frame checks may publish admission to other readers.
        self.visit_inflated_frames(
            &mut raw,
            index,
            selected,
            visit,
            !validate && self.is_verified(index),
        )
        .map_err(zstd_error)?;
        if validate {
            let buffered = raw.stream.into_inner().finish();
            let unread = buffered.buffer().len() as u64 + buffered.into_inner().remaining;
            if unread != 0 {
                self.verified[self.block_position(index).unwrap()].store(false, Ordering::Release);
                return Err(invalid(
                    "block contains trailing or concatenated zstd frames",
                ));
            }
        }
        Ok(())
    }
}
