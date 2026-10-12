//! Optimistic, verified recompression without changing frame contracts.

use super::*;
use std::{fs, sync::atomic::AtomicU64, time::SystemTime};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// The same length/modified-time revision used by recording consumers.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct ReplayRevision {
    length: u64,
    modified: SystemTime,
}
impl ReplayRevision {
    fn from_metadata(metadata: &fs::Metadata) -> io::Result<Self> {
        if !metadata.is_file() {
            return Err(invalid("Recording is not a file"));
        }
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified()?,
        })
    }
}
pub fn replay_revision(path: &Path) -> io::Result<ReplayRevision> {
    ReplayRevision::from_metadata(&fs::metadata(path)?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionOutcome {
    Compacted,
    AlreadySmall,
    Changed,
}
#[derive(Clone, Debug)]
pub struct CompactionResult {
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub blocks_encoded: usize,
    pub outcome: CompactionOutcome,
    /// The revision actually verified/left behind, never a later append's revision.
    pub revision: ReplayRevision,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactionStage {
    Reading,
    Started {
        blocks: usize,
    },
    TempCreated,
    BlockBytes {
        block: usize,
        raw_bytes: u64,
    },
    BlockWritten(usize),
    BeforeVerification,
    FrameEncoded {
        block: usize,
        frame: u32,
    },
    FrameVerified {
        block: usize,
        frame: u32,
    },
    /// Owned wire buffers (metadata plus frame payloads), excluding codec workspace.
    Buffered {
        bytes: usize,
    },
    Verified,
    BeforeReplace,
    RevisionChecked,
    BeforeSwap,
}

type BlockStream<'a> =
    io::Take<zstd::stream::read::Decoder<'static, io::BufReader<io::Take<&'a mut File>>>>;

// The compressed bytes stay on disk. The single-frame decoder's remaining input
// proves that no trailing bytes or concatenated frames were silently ignored.
fn inflate_file<'a>(
    file: &'a mut File,
    block: &BlockIndex,
) -> io::Result<Inflated<BlockStream<'a>>> {
    if block.raw_length > MAX_BLOCK_RAW_BYTES as u64 {
        return Err(invalid("block exceeds raw size limit"));
    }
    file.seek(SeekFrom::Start(block.offset))?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic)?;
    if magic != [0x28, 0xb5, 0x2f, 0xfd] {
        return Err(invalid("block is not a zstd frame"));
    }
    file.seek(SeekFrom::Start(block.offset))?;
    let mut decoder =
        zstd::stream::read::Decoder::with_buffer(io::BufReader::new(file.take(block.length)))?
            .single_frame();
    decoder.window_log_max(26)?;
    Ok(Inflated {
        stream: decoder.take(block.raw_length + 1),
        read: 0,
    })
}

fn finish_block(mut input: Inflated<BlockStream<'_>>, block: &BlockIndex) -> io::Result<()> {
    if input.read != block.raw_length || input.stream.read(&mut [0])? != 0 {
        return Err(invalid("block raw length mismatch"));
    }
    let buffered = input.stream.into_inner().finish();
    let unread = buffered.buffer().len() as u64 + buffered.into_inner().limit();
    if unread != 0 {
        return Err(invalid(
            "block contains trailing or concatenated zstd frames",
        ));
    }
    Ok(())
}

fn read_bytes(
    file: &mut File,
    length: usize,
    path: &Path,
    checkpoint: &mut impl FnMut(CompactionStage, &Path) -> io::Result<()>,
) -> io::Result<Vec<u8>> {
    let mut bytes = engine::allocation::filled(length, 0)?;
    for chunk in bytes.chunks_mut(64 * 1024) {
        checkpoint(CompactionStage::Reading, path)?;
        file.read_exact(chunk)?;
    }
    Ok(bytes)
}

struct FileReplay {
    metadata: Replay,
    header: Vec<u8>,
    trailer: Vec<u8>,
    length: u64,
}
impl FileReplay {
    fn read(
        file: &mut File,
        path: &Path,
        checkpoint: &mut impl FnMut(CompactionStage, &Path) -> io::Result<()>,
    ) -> io::Result<Self> {
        let length = file.metadata()?.len();
        if length < (FOOTER_SIZE + 18) as u64 {
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
        if start < 18 || start.checked_add(trailer_length) != Some(length - FOOTER_SIZE as u64) {
            return Err(invalid("invalid trailer extent"));
        }
        file.seek(SeekFrom::Start(start))?;
        let trailer = read_bytes(file, to_usize(trailer_length)?, path, checkpoint)?;
        if crc32fast::hash(&trailer) != crc {
            return Err(invalid("trailer CRC mismatch"));
        }
        // Fixed prefix, length-prefixed campaign identity, length-prefixed content identity.
        file.seek(SeekFrom::Start(0))?;
        let mut prefix = [0; 14];
        file.read_exact(&mut prefix)?;
        let campaign_length = u32::from_le_bytes(prefix[10..14].try_into().unwrap()) as u64;
        let identity_length_at = 14_u64
            .checked_add(campaign_length)
            .filter(|end| end.checked_add(4).is_some_and(|v| v <= start))
            .ok_or_else(|| invalid("header outside replay"))?;
        file.seek(SeekFrom::Start(identity_length_at))?;
        let mut size = [0; 4];
        file.read_exact(&mut size)?;
        let header_end = identity_length_at
            .checked_add(4)
            .and_then(|end| end.checked_add(u32::from_le_bytes(size) as u64))
            .filter(|end| *end <= start)
            .ok_or_else(|| invalid("header outside replay"))?;
        file.seek(SeekFrom::Start(0))?;
        let header = read_bytes(file, to_usize(header_end)?, path, checkpoint)?;
        checkpoint(CompactionStage::Reading, path)?;
        // The trailer checksum was verified above, so no warning is carried in.
        let metadata = Replay::open_metadata(&header, &trailer, to_usize(start)?, Vec::new())?;
        // A recording that needed recovery to open (skipped index entries or records,
        // unindexed bytes) is never rewritten; its stored bytes stay exactly as they are.
        if metadata.needed_recovery() {
            return Err(invalid(
                "This recording is damaged, so it can't be compacted.",
            ));
        }
        Ok(Self {
            metadata,
            header,
            trailer,
            length,
        })
    }
    fn trailer_metadata(&self) -> &[u8] {
        &self.trailer[..self.trailer.len() - self.metadata.index.len() * 40]
    }
    fn buffered(&self) -> usize {
        self.header.capacity() + self.trailer.capacity()
    }
}

/// Only the owned candidate is reserved against writes, and only while it is verified.
#[cfg(windows)]
fn candidate_file(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .share_mode(
            windows_sys::Win32::Storage::FileSystem::FILE_SHARE_DELETE
                | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ,
        )
        .open(path)
}
#[cfg(not(windows))]
fn candidate_file(path: &Path) -> io::Result<File> {
    File::open(path)
}

struct Temporary(PathBuf);
impl Temporary {
    fn create(destination: &Path) -> io::Result<(Self, File)> {
        let mut name = destination.as_os_str().to_owned();
        name.push(format!(
            ".compact.{}.{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let path = PathBuf::from(name);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok((Self(path), file))
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn compact(path: &Path) -> io::Result<CompactionResult> {
    compact_with(path, |_, _| Ok(()))
}

/// Read with ordinary sharing: readers and appends remain free to proceed.
/// Check the revision immediately before atomic publication; discard stale or
/// non-smaller candidates. Checkpoints cancel work and exercise failure cleanup.
pub fn compact_with(
    path: &Path,
    mut checkpoint: impl FnMut(CompactionStage, &Path) -> io::Result<()>,
) -> io::Result<CompactionResult> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("battleplan") || ext.eq_ignore_ascii_case("hoi4replay"))
    {
        return Err(invalid(
            "Choose a finished .battleplan recording, not a spool or temporary file",
        ));
    }
    let path = path.canonicalize()?;
    let before = replay_revision(&path)?;
    checkpoint(CompactionStage::Reading, &path)?;
    let mut input_file = File::open(&path)?;
    let original = FileReplay::read(&mut input_file, &path, &mut checkpoint).map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidData
            || error.kind() == io::ErrorKind::UnexpectedEof
        {
            invalid(format!(
                "Still being recorded or invalid recording: {error}"
            ))
        } else {
            error
        }
    })?;
    let mut result = CompactionResult {
        bytes_before: original.length,
        bytes_after: original.length,
        blocks_encoded: 0,
        outcome: CompactionOutcome::AlreadySmall,
        revision: before.clone(),
    };
    let (temporary, mut output) = Temporary::create(&path)?;
    checkpoint(
        CompactionStage::Started {
            blocks: original.metadata.index.len(),
        },
        &temporary.0,
    )?;
    checkpoint(CompactionStage::TempCreated, &temporary.0)?;
    checkpoint(
        CompactionStage::Buffered {
            bytes: original.buffered(),
        },
        &temporary.0,
    )?;
    output.write_all(&original.header)?;
    let mut index = Vec::new();
    engine::allocation::reserve(&mut index, original.metadata.index.len())?;
    let mut buffer = [0; 64 * 1024];
    for (position, block) in original.metadata.index.iter().enumerate() {
        checkpoint(CompactionStage::Reading, &temporary.0)?;
        let offset = output.stream_position()?;
        let mut input = inflate_file(&mut input_file, block)?;
        // Encode directly to disk; no full original or compressed candidate buffer.
        let mut encoder = zstd::stream::write::Encoder::new(&mut output, 19)?;
        encoder.long_distance_matching(true)?;
        encoder.window_log(26)?;
        encoder.include_checksum(true)?;
        let mut size = [0; 4];
        input.fill(&mut size)?;
        let count = u32::from_le_bytes(size);
        if count == 0 || u64::from(count) > block.raw_length.saturating_sub(4) / 4 {
            return Err(invalid("invalid block frame count"));
        }
        encoder.write_all(&size)?;
        for frame in 0..count {
            checkpoint(CompactionStage::Reading, &temporary.0)?;
            input.fill(&mut size)?;
            let mut remaining = u32::from_le_bytes(size) as usize;
            if remaining as u64 > block.raw_length.saturating_sub(input.read) {
                return Err(invalid("invalid block frame length"));
            }
            encoder.write_all(&size)?;
            while remaining > 0 {
                let count = remaining.min(buffer.len());
                input.fill(&mut buffer[..count])?;
                checkpoint(
                    CompactionStage::BlockBytes {
                        block: position,
                        raw_bytes: input.read,
                    },
                    &temporary.0,
                )?;
                encoder.write_all(&buffer[..count])?;
                remaining -= count;
            }
            checkpoint(
                CompactionStage::FrameEncoded {
                    block: position,
                    frame,
                },
                &temporary.0,
            )?;
        }
        finish_block(input, block)?;
        encoder.finish()?;
        let mut length = output.stream_position()? - offset;
        result.blocks_encoded += 1;
        // If stronger compression grows a block, overwrite it with exact original bytes.
        if length >= block.length {
            output.set_len(offset)?;
            output.seek(SeekFrom::Start(offset))?;
            input_file.seek(SeekFrom::Start(block.offset))?;
            let mut remaining = block.length;
            while remaining > 0 {
                checkpoint(CompactionStage::Reading, &temporary.0)?;
                let count = remaining.min(buffer.len() as u64) as usize;
                input_file.read_exact(&mut buffer[..count])?;
                output.write_all(&buffer[..count])?;
                remaining -= count as u64;
            }
            length = block.length;
        }
        index.push(BlockIndex {
            offset,
            length,
            ..block.clone()
        });
        checkpoint(CompactionStage::BlockWritten(position), &temporary.0)?;
    }
    // Preserve historical metadata; stream only the rebuilt fixed-width index/footer.
    let trailer_start = output.stream_position()?;
    let mut checksum = crc32fast::Hasher::new();
    let trailer_metadata = original.trailer_metadata();
    output.write_all(trailer_metadata)?;
    checksum.update(trailer_metadata);
    for block in &index {
        checkpoint(CompactionStage::Reading, &temporary.0)?;
        let mut bytes = [0; 40];
        bytes[..8].copy_from_slice(&block.branch.to_le_bytes());
        bytes[8..12].copy_from_slice(&block.first_hour.to_le_bytes());
        bytes[12..16].copy_from_slice(&block.last_hour.to_le_bytes());
        bytes[16..24].copy_from_slice(&block.offset.to_le_bytes());
        bytes[24..32].copy_from_slice(&block.length.to_le_bytes());
        bytes[32..].copy_from_slice(&block.raw_length.to_le_bytes());
        output.write_all(&bytes)?;
        checksum.update(&bytes);
    }
    let trailer_length = output.stream_position()? - trailer_start;
    output.write_all(FOOTER_MAGIC)?;
    output.write_all(&trailer_start.to_le_bytes())?;
    output.write_all(&trailer_length.to_le_bytes())?;
    output.write_all(&checksum.finalize().to_le_bytes())?;
    output.sync_all()?;
    drop(output);
    checkpoint(CompactionStage::BeforeVerification, &temporary.0)?;
    let mut candidate_file = candidate_file(&temporary.0)?;
    let candidate_revision = ReplayRevision::from_metadata(&candidate_file.metadata()?)?;
    let candidate = FileReplay::read(&mut candidate_file, &temporary.0, &mut checkpoint)?;
    verify(
        &original,
        &candidate,
        &mut input_file,
        &mut candidate_file,
        |stage| checkpoint(stage, &temporary.0),
    )?;
    checkpoint(CompactionStage::Verified, &temporary.0)?;
    // Windows replacement opens the candidate for writing, so the reservation
    // ends with verification; a candidate changed since is never published.
    drop(candidate_file);
    checkpoint(CompactionStage::BeforeReplace, &temporary.0)?;
    if replay_revision(&path)? != before {
        result.outcome = CompactionOutcome::Changed;
        return Ok(result);
    }
    if candidate.length >= original.length {
        return Ok(result);
    }
    checkpoint(CompactionStage::RevisionChecked, &temporary.0)?;
    checkpoint(CompactionStage::BeforeSwap, &temporary.0)?;
    if replay_revision(&temporary.0)? != candidate_revision {
        return Err(invalid(
            "Compaction verification failed: the compacted copy changed after verification",
        ));
    }
    if !publish_candidate(&temporary.0, &path, &before, &candidate_revision)? {
        result.outcome = CompactionOutcome::Changed;
        return Ok(result);
    }
    result.bytes_after = candidate.length;
    result.outcome = CompactionOutcome::Compacted;
    result.revision = candidate_revision;
    Ok(result)
}

fn backup_path(path: &Path) -> io::Result<PathBuf> {
    loop {
        let mut name = path.as_os_str().to_owned();
        name.push(format!(
            ".compact.{}.{}.backup",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let backup = PathBuf::from(name);
        if !backup.try_exists()? {
            return Ok(backup);
        }
    }
}

/// Atomically capture the file actually displaced, rather than trusting a prior
/// path check. A backup has no cleanup guard: uncertain data is always retained.
fn publish_candidate(
    candidate: &Path,
    path: &Path,
    before: &ReplayRevision,
    candidate_revision: &ReplayRevision,
) -> io::Result<bool> {
    publish_candidate_with(
        candidate,
        path,
        before,
        candidate_revision,
        swap_with_backup,
    )
}

fn publish_candidate_with(
    candidate: &Path,
    path: &Path,
    before: &ReplayRevision,
    candidate_revision: &ReplayRevision,
    mut swap: impl FnMut(&Path, &Path, &Path) -> io::Result<()>,
) -> io::Result<bool> {
    let backup = backup_path(path)?;
    swap(candidate, path, &backup).map_err(|error| {
        let error = replacement_error(error);
        if backup.try_exists().unwrap_or(true) {
            io::Error::other(format!(
                "{error}; recording backup retained at {}",
                backup.display()
            ))
        } else {
            error
        }
    })?;
    let displaced = replay_revision(&backup).map_err(|error| {
        io::Error::other(format!(
            "{error}; recording backup retained at {}",
            backup.display()
        ))
    })?;
    if displaced == *before {
        fs::remove_file(&backup).map_err(|error| {
            io::Error::other(format!(
                "{error}; verified recording backup retained at {}",
                backup.display()
            ))
        })?;
        return Ok(true);
    }
    // A concurrent publisher won the revision-check/swap race. Restore its file
    // and capture whatever is now at the destination as well, so a second publish
    // during rollback cannot be lost either.
    let displaced_candidate = backup_path(path).map_err(|error| {
        io::Error::other(format!(
            "{error}; recording backup retained at {}",
            backup.display()
        ))
    })?;
    swap(&backup, path, &displaced_candidate).map_err(|error| {
        io::Error::other(format!("Could not restore the concurrently published recording: {error}; retain backups {} and {}", backup.display(), displaced_candidate.display()))
    })?;
    let restored_over = replay_revision(&displaced_candidate).map_err(|error| {
        io::Error::other(format!(
            "{error}; recording backup retained at {}",
            displaced_candidate.display()
        ))
    })?;
    if restored_over != *candidate_revision {
        return Err(io::Error::other(format!(
            "Another recording was published during restoration; recording backup retained at {}",
            displaced_candidate.display()
        )));
    }
    fs::remove_file(&displaced_candidate).map_err(|error| {
        io::Error::other(format!(
            "{error}; candidate backup retained at {}",
            displaced_candidate.display()
        ))
    })?;
    Ok(false)
}

#[cfg(windows)]
fn swap_with_backup(source: &Path, destination: &Path, backup: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let backup: Vec<u16> = backup.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
        ReplaceFileW(
            destination.as_ptr(),
            source.as_ptr(),
            backup.as_ptr(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn swap_with_backup(source: &Path, destination: &Path, backup: &Path) -> io::Result<()> {
    // The unique backup name is owned by this operation. Publication uses a
    // no-replace link: a publisher filling the gap is never overwritten.
    fs::rename(destination, backup)?;
    fs::hard_link(source, destination)?;
    fs::remove_file(source)?;
    if let Some(parent) = destination.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn replacement_error(error: io::Error) -> io::Error {
    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(code) if code == windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32 || code == windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED as i32 || code == windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION as i32)
    {
        return io::Error::new(io::ErrorKind::WouldBlock, "Busy; try again later");
    }
    error
}

fn verify(
    old: &FileReplay,
    new: &FileReplay,
    old_file: &mut File,
    new_file: &mut File,
    mut checkpoint: impl FnMut(CompactionStage) -> io::Result<()>,
) -> io::Result<()> {
    if old.header != new.header
        || old.metadata.index.len() != new.metadata.index.len()
        || old.trailer_metadata() != new.trailer_metadata()
    {
        return Err(invalid("Compaction verification failed: metadata changed"));
    }
    for (position, (a, b)) in old
        .metadata
        .index
        .iter()
        .zip(&new.metadata.index)
        .enumerate()
    {
        if (a.branch, a.first_hour, a.last_hour, a.raw_length)
            != (b.branch, b.first_hour, b.last_hour, b.raw_length)
        {
            return Err(invalid(
                "Compaction verification failed: block semantics changed",
            ));
        }
        checkpoint(CompactionStage::Reading)?;
        let mut left = inflate_file(old_file, a)?;
        let mut right = inflate_file(new_file, b)?;
        let mut x = [0; 4];
        let mut y = [0; 4];
        left.fill(&mut x)?;
        right.fill(&mut y)?;
        let count = u32::from_le_bytes(x);
        if x != y || count == 0 || u64::from(count) > a.raw_length.saturating_sub(4) / 4 {
            return Err(invalid("Compaction verification failed: frame count"));
        }
        let mut left_order = BlockOrder::new(a);
        let mut right_order = BlockOrder::new(b);
        let mut p = Vec::new();
        let mut q = Vec::new();
        for frame in 0..count {
            checkpoint(CompactionStage::Reading)?;
            left.fill(&mut x)?;
            right.fill(&mut y)?;
            let length = u32::from_le_bytes(x) as usize;
            if x != y || length as u64 > a.raw_length.saturating_sub(left.read) {
                return Err(invalid("Compaction verification failed: frame length"));
            }
            engine::allocation::resize(&mut p, length, 0)?;
            engine::allocation::resize(&mut q, length, 0)?;
            checkpoint(CompactionStage::Buffered {
                bytes: old.buffered() + new.buffered() + p.capacity() + q.capacity(),
            })?;
            for chunk in p.chunks_mut(64 * 1024) {
                checkpoint(CompactionStage::Reading)?;
                left.fill(chunk)?;
            }
            for chunk in q.chunks_mut(64 * 1024) {
                checkpoint(CompactionStage::Reading)?;
                right.fill(chunk)?;
            }
            let first = decode_payload(&p, None, old.metadata.format)?;
            let second = decode_payload(&q, None, new.metadata.format)?;
            left_order.next(&first)?;
            right_order.next(&second)?;
            // Raw equality also protects historical bytes intentionally omitted
            // by today's named compatibility refusals.
            if p != q || first != second {
                return Err(invalid("Compaction verification failed: frame changed"));
            }
            checkpoint(CompactionStage::FrameVerified {
                block: position,
                frame,
            })?;
        }
        checkpoint(CompactionStage::Reading)?;
        finish_block(left, a)?;
        finish_block(right, b)?;
        left_order.finish()?;
        right_order.finish()?;
    }
    Ok(())
}

#[cfg(test)]
mod publication_tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "hoi4-compaction-backup-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }
        fn setup(&self) -> (PathBuf, PathBuf, ReplayRevision, ReplayRevision) {
            let recording = self.0.join("synthetic.battleplan");
            let candidate = self.0.join("synthetic.tmp");
            fs::write(&recording, b"original synthetic recording").unwrap();
            fs::write(&candidate, b"candidate").unwrap();
            let before = replay_revision(&recording).unwrap();
            let after = replay_revision(&candidate).unwrap();
            (recording, candidate, before, after)
        }
        fn backup_with(&self, bytes: &[u8]) -> PathBuf {
            fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.extension().is_some_and(|ext| ext == "backup")
                        && fs::read(path).unwrap() == bytes
                })
                .expect("uncertain recording survives in a backup")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_swap_error_retains_and_reports_the_displaced_recording() {
        let fixture = Fixture::new();
        let (recording, candidate, before, after) = fixture.setup();
        let error = publish_candidate_with(
            &candidate,
            &recording,
            &before,
            &after,
            |source, destination, backup| {
                swap_with_backup(source, destination, backup)?;
                Err(io::Error::other("synthetic post-swap failure"))
            },
        )
        .unwrap_err();
        let backup = fixture.backup_with(b"original synthetic recording");
        assert!(error.to_string().contains(&backup.display().to_string()));
    }

    #[test]
    fn restoration_failure_retains_and_reports_the_newer_recording() {
        let fixture = Fixture::new();
        let (recording, candidate, before, after) = fixture.setup();
        fs::write(
            &recording,
            b"concurrently published synthetic recording with more hours",
        )
        .unwrap();
        let mut swaps = 0;
        let error = publish_candidate_with(
            &candidate,
            &recording,
            &before,
            &after,
            |source, destination, backup| {
                swaps += 1;
                if swaps == 2 {
                    return Err(io::Error::other("synthetic restoration failure"));
                }
                swap_with_backup(source, destination, backup)
            },
        )
        .unwrap_err();
        let backup =
            fixture.backup_with(b"concurrently published synthetic recording with more hours");
        assert!(error.to_string().contains(&backup.display().to_string()));
    }

    #[test]
    fn another_publish_during_restoration_is_retained_and_reported() {
        let fixture = Fixture::new();
        let (recording, candidate, before, after) = fixture.setup();
        fs::write(
            &recording,
            b"first concurrently published synthetic recording",
        )
        .unwrap();
        let mut swaps = 0;
        let error = publish_candidate_with(
            &candidate,
            &recording,
            &before,
            &after,
            |source, destination, backup| {
                swaps += 1;
                if swaps == 2 {
                    let newer = fixture.0.join("newer.tmp");
                    fs::write(
                        &newer,
                        b"second concurrently published recording with every appended hour",
                    )
                    .unwrap();
                    replace_file(&newer, destination)?;
                }
                swap_with_backup(source, destination, backup)
            },
        )
        .unwrap_err();
        assert_eq!(
            fs::read(&recording).unwrap(),
            b"first concurrently published synthetic recording"
        );
        let backup = fixture
            .backup_with(b"second concurrently published recording with every appended hour");
        assert!(error.to_string().contains(&backup.display().to_string()));
    }
}
