use super::*;
use std::sync::atomic::AtomicU64;

#[derive(Default)]
pub(super) struct Prefix {
    blocks: usize,
    digest: blake3::Hasher,
}

impl Prefix {
    pub(super) fn blocks(&self) -> usize {
        self.blocks
    }
    pub(super) fn accept(
        &mut self,
        replay: &Replay,
        position: usize,
        block: blake3::Hash,
    ) -> io::Result<()> {
        if position != self.blocks {
            return Err(invalid("reader source prefix is not contiguous"));
        }
        let index = &replay.index[position];
        self.digest
            .update(&(replay.ordinals[position] as u64).to_le_bytes());
        self.digest.update(&index.branch.to_le_bytes());
        self.digest.update(&index.first_hour.to_le_bytes());
        self.digest.update(&index.last_hour.to_le_bytes());
        self.digest.update(&index.offset.to_le_bytes());
        self.digest.update(&index.length.to_le_bytes());
        self.digest.update(&index.raw_length.to_le_bytes());
        self.digest.update(block.as_bytes());
        self.blocks += 1;
        Ok(())
    }
    pub(super) fn verify(
        &self,
        replay: &Replay,
        meter: &Arc<AtomicU64>,
        check: &mut Check<'_>,
    ) -> io::Result<()> {
        if self.blocks > replay.index.len() {
            return Err(changed());
        }
        let mut actual = Self::default();
        for position in 0..self.blocks {
            check.run()?;
            let index = &replay.index[position];
            let block = block_digest(replay, index, meter, check)?;
            actual.accept(replay, position, block)?;
        }
        check.run()?;
        if actual.digest.finalize() != self.digest.finalize() {
            return Err(changed());
        }
        Ok(())
    }
}

pub(super) fn block_digest(
    replay: &Replay,
    index: &BlockIndex,
    meter: &Arc<AtomicU64>,
    check: &mut Check<'_>,
) -> io::Result<blake3::Hash> {
    let result = super::super::grouped::source_digest(replay, index, meter, check);
    result.map_err(|error| {
        if check.failed() || !recording_damage(&error) {
            error
        } else {
            changed()
        }
    })
}

fn changed() -> io::Error {
    invalid("recording differs from reader source prefix")
}

struct Range {
    offset: usize,
    length: usize,
    digest: blake3::Hasher,
}

pub(crate) struct Evidence {
    ranges: Vec<Option<Range>>,
    meter: Arc<AtomicU64>,
}

impl Evidence {
    pub(crate) fn new(count: usize, meter: Arc<AtomicU64>) -> io::Result<Self> {
        Ok(Self {
            ranges: allocation::collect_vec((0..count).map(|_| None))?,
            meter,
        })
    }
    pub(crate) fn record(
        &mut self,
        id: usize,
        offset: usize,
        bytes: &[u8],
        check: &mut Check<'_>,
    ) -> io::Result<()> {
        let range = self.ranges[id].get_or_insert_with(|| Range {
            offset,
            length: 0,
            digest: blake3::Hasher::new(),
        });
        if range.offset.checked_add(range.length) != Some(offset) {
            return Err(changed());
        }
        for part in bytes.chunks(64 * 1024) {
            check.run()?;
            range.digest.update(part);
            range.length = allocation::add(range.length, part.len())?;
            self.meter.fetch_add(part.len() as u64, Ordering::Relaxed);
        }
        Ok(())
    }
    pub(crate) fn read(
        &mut self,
        id: usize,
        replay: &Replay,
        index: &BlockIndex,
        offset: usize,
        length: usize,
        check: &mut Check<'_>,
    ) -> io::Result<()> {
        for at in (0..length).step_by(64 * 1024) {
            check.run()?;
            let count = (length - at).min(64 * 1024);
            self.record(
                id,
                offset + at,
                &replay.block_range(index, offset + at, count)?,
                check,
            )?;
        }
        if length == 0 {
            self.record(id, offset, &[], check)?;
        }
        Ok(())
    }
    pub(crate) fn verify(
        &self,
        replay: &Replay,
        index: &BlockIndex,
        check: &mut Check<'_>,
    ) -> io::Result<blake3::Hash> {
        for range in self.ranges.iter().flatten() {
            let mut actual = blake3::Hasher::new();
            for at in (0..range.length).step_by(64 * 1024) {
                check.run()?;
                let count = (range.length - at).min(64 * 1024);
                actual.update(&replay.block_range(index, range.offset + at, count)?);
                self.meter.fetch_add(count as u64, Ordering::Relaxed);
            }
            if actual.finalize() != range.digest.finalize() {
                return Err(changed());
            }
        }
        check.run()?;
        Ok(self.digest())
    }
    pub(crate) fn digest(&self) -> blake3::Hash {
        let mut digest = blake3::Hasher::new();
        for (id, range) in self.ranges.iter().enumerate() {
            if let Some(range) = range {
                digest.update(&(id as u64).to_le_bytes());
                digest.update(&(range.offset as u64).to_le_bytes());
                digest.update(&(range.length as u64).to_le_bytes());
                digest.update(range.digest.finalize().as_bytes());
            }
        }
        digest.finalize()
    }
}

pub(super) struct Lease {
    #[cfg(windows)]
    file: File,
    #[cfg(windows)]
    start: u64,
    #[cfg(windows)]
    length: u64,
}

impl Lease {
    pub(super) fn same_file(&self, replay: &Replay) -> io::Result<bool> {
        #[cfg(windows)]
        {
            let Some(file) = &replay.file else {
                return Ok(false);
            };
            Ok(Self::identity(&self.file)? == Self::identity(file)?)
        }
        #[cfg(not(windows))]
        {
            let _ = replay;
            Ok(false)
        }
    }
    #[cfg(windows)]
    fn identity(file: &File) -> io::Result<(u64, [u8; 16])> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
        };
        let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                std::ptr::from_mut(&mut info).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok((info.VolumeSerialNumber, info.FileId.Identifier))
    }
    pub(super) fn acquire(replay: &Replay, end: u64) -> io::Result<Option<Self>> {
        let Some(file) = &replay.file else {
            return Ok(None);
        };
        let start = replay.index.first().map_or(end, |index| index.offset);
        if end <= start {
            return Ok(None);
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::{
                Storage::FileSystem::{LOCKFILE_FAIL_IMMEDIATELY, LockFileEx},
                System::IO::OVERLAPPED,
            };
            // Recorder sources use ordinary writes, never writable mappings that bypass this lease.
            let file = file.try_clone()?;
            let length = end - start;
            let mut at: OVERLAPPED = unsafe { std::mem::zeroed() };
            at.Anonymous.Anonymous.Offset = start as u32;
            at.Anonymous.Anonymous.OffsetHigh = (start >> 32) as u32;
            if unsafe {
                LockFileEx(
                    file.as_raw_handle(),
                    LOCKFILE_FAIL_IMMEDIATELY,
                    0,
                    length as u32,
                    (length >> 32) as u32,
                    &mut at,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Some(Self {
                file,
                start,
                length,
            }))
        }
        #[cfg(not(windows))]
        {
            let _ = file;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "immutable recording byte-range leases require Windows",
            ))
        }
    }
}

#[cfg(windows)]
impl Drop for Lease {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{Storage::FileSystem::UnlockFileEx, System::IO::OVERLAPPED};
        let mut at: OVERLAPPED = unsafe { std::mem::zeroed() };
        at.Anonymous.Anonymous.Offset = self.start as u32;
        at.Anonymous.Anonymous.OffsetHigh = (self.start >> 32) as u32;
        unsafe {
            UnlockFileEx(
                self.file.as_raw_handle(),
                0,
                self.length as u32,
                (self.length >> 32) as u32,
                &mut at,
            );
        }
    }
}
