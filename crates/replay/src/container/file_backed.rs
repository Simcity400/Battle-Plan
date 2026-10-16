//! Held recording files and selected, block-positioned preparation.

use super::*;
use std::sync::Mutex;

#[cfg(test)]
thread_local! {
    static HELD_READ_BYTES: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(super) fn measure_held_reads<T>(read: impl FnOnce() -> T) -> (T, u64) {
    HELD_READ_BYTES.with(|bytes| bytes.set(Some(0)));
    let result = read();
    let bytes = HELD_READ_BYTES.with(|bytes| bytes.replace(None).unwrap());
    (result, bytes)
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
        replay.file = Some(Mutex::new(file));
        Ok(replay)
    }

    pub(super) fn open_file_metadata(file: &mut (impl Read + Seek)) -> io::Result<Self> {
        let length = file.seek(SeekFrom::End(0))?;
        if length < FOOTER_SIZE as u64 {
            return Err(invalid("missing replay footer"));
        }
        file.seek(SeekFrom::Start(length - FOOTER_SIZE as u64))?;
        let mut bytes = [0; FOOTER_SIZE];
        file.read_exact(&mut bytes)?;
        let mut footer = Decoder::new(&bytes);
        expect_magic(&mut footer, FOOTER_MAGIC)?;
        let start = footer.u64()?;
        let trailer_length = footer.u64()?;
        let crc = footer.u32()?;
        footer.finish()?;
        if start.checked_add(trailer_length) != Some(length - FOOTER_SIZE as u64) {
            return Err(invalid("invalid trailer extent"));
        }
        let read = |file: &mut dyn Read, length: u64| -> io::Result<Vec<u8>> {
            let mut bytes = engine::allocation::filled(to_usize(length)?, 0)?;
            file.read_exact(&mut bytes)?;
            Ok(bytes)
        };
        file.seek(SeekFrom::Start(start))?;
        let trailer = read(file, trailer_length)?;
        let mut warnings = Vec::new();
        if crc32fast::hash(&trailer) != crc {
            engine::allocation::reserve(&mut warnings, 1)?;
            warnings.push("trailer CRC mismatch".into());
        }
        // The existing header has two length-prefixed values after its fixed prefix.
        if start < 14 {
            return Err(invalid("truncated field"));
        }
        file.seek(SeekFrom::Start(0))?;
        let mut prefix = [0; 14];
        file.read_exact(&mut prefix)?;
        let identity_at = 14_u64
            .checked_add(u64::from(u32::from_le_bytes(
                prefix[10..14].try_into().unwrap(),
            )))
            .filter(|end| end.checked_add(4).is_some_and(|end| end <= start))
            .ok_or_else(|| invalid("truncated field"))?;
        file.seek(SeekFrom::Start(identity_at))?;
        let mut size = [0; 4];
        file.read_exact(&mut size)?;
        let end = identity_at
            .checked_add(4)
            .and_then(|end| end.checked_add(u64::from(u32::from_le_bytes(size))))
            .filter(|end| *end <= start)
            .ok_or_else(|| invalid("truncated field"))?;
        file.seek(SeekFrom::Start(0))?;
        let header = read(file, end)?;
        Self::open_metadata(&header, &trailer, to_usize(start)?, warnings)
    }

    pub fn is_file_backed(&self) -> bool {
        self.file.is_some()
    }

    /// Compare original compressed bytes with bounded scratch space. Metadata
    /// equality alone cannot prove that a publication preserved existing blocks.
    pub fn block_bytes_equal(&self, other: &Replay, block: &BlockIndex) -> io::Result<bool> {
        if self.block_position(block).is_none() || other.block_position(block).is_none() {
            return Ok(false);
        }
        let mut left = [0; 64 * 1024];
        let mut right = [0; 64 * 1024];
        let mut offset = block.offset;
        let end = offset
            .checked_add(block.length)
            .ok_or_else(|| invalid("block offset overflow"))?;
        while offset < end {
            let count = (end - offset).min(left.len() as u64) as usize;
            self.read_at(offset, &mut left[..count])?;
            other.read_at(offset, &mut right[..count])?;
            if left[..count] != right[..count] {
                return Ok(false);
            }
            offset += count as u64;
        }
        Ok(true)
    }

    pub(super) fn held_block_bytes(&self, index: &BlockIndex) -> io::Result<Vec<u8>> {
        let mut bytes = engine::allocation::filled(to_usize(index.length)?, 0)?;
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
        let end = offset
            .checked_add(length)
            .filter(|end| *end as u64 <= index.length)
            .ok_or_else(|| invalid("grouped section outside its block"))?;
        let start = index
            .offset
            .checked_add(offset as u64)
            .ok_or_else(|| invalid("block offset overflow"))?;
        if self.file.is_some() {
            let mut bytes = engine::allocation::filled(length, 0)?;
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
                let mut file = file
                    .lock()
                    .map_err(|_| io::Error::other("recording file lock poisoned"))?;
                file.seek(SeekFrom::Start(offset))?;
                file.read_exact(output)
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
        visit: impl FnMut(Frame) -> io::Result<bool>,
    ) -> io::Result<()> {
        self.visit_file_block_with(index, selected, visit, &mut || Ok(()))
    }

    fn visit_file_block_with(
        &self,
        index: &BlockIndex,
        selected: Option<&[TableId]>,
        mut visit: impl FnMut(Frame) -> io::Result<bool>,
        check: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        check()?;
        let mut file = self
            .file
            .as_ref()
            .unwrap()
            .lock()
            .map_err(|_| io::Error::other("recording file lock poisoned"))?;
        let position = self
            .block_position(index)
            .ok_or_else(|| invalid("block outside replay"))?;
        // No visitor receives a frame from a malformed block. Admission streams
        // all wire grammar and the checksum first, retaining only one payload.
        if !self.verified[position].load(Ordering::Acquire) {
            self.visit_file_pass(
                &mut file,
                index,
                selected,
                |_| {
                    check()?;
                    Ok(true)
                },
                true,
            )?;
            self.verified[position].store(true, Ordering::Release);
        }
        self.visit_file_pass(
            &mut file,
            index,
            selected,
            |frame| {
                check()?;
                visit(frame)
            },
            false,
        )
    }

    fn visit_file_pass(
        &self,
        file: &mut File,
        index: &BlockIndex,
        selected: Option<&[TableId]>,
        visit: impl FnMut(Frame) -> io::Result<bool>,
        validate: bool,
    ) -> io::Result<()> {
        if index.raw_length > MAX_BLOCK_RAW_BYTES as u64 {
            return Err(invalid("block exceeds raw size limit"));
        }
        file.seek(SeekFrom::Start(index.offset))?;
        let mut magic = [0; 4];
        file.read_exact(&mut magic)?;
        if magic != [0x28, 0xb5, 0x2f, 0xfd] {
            return Err(invalid("block is not a zstd frame"));
        }
        file.seek(SeekFrom::Start(index.offset))?;
        let mut decoder = zstd::stream::read::Decoder::with_buffer(io::BufReader::with_capacity(
            64 * 1024,
            file.take(index.length),
        ))
        .map_err(zstd_error)?
        .single_frame();
        decoder.window_log_max(26).map_err(zstd_error)?;
        let mut raw = Inflated {
            stream: decoder.take(index.raw_length + 1),
            read: 0,
        };
        // During admission verified remains false, so visit_inflated reads to end.
        self.visit_inflated(&mut raw, index, selected, visit)
            .map_err(zstd_error)?;
        if validate {
            let buffered = raw.stream.into_inner().finish();
            let unread = buffered.buffer().len() as u64 + buffered.into_inner().limit();
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
