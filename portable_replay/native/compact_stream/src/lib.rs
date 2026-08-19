use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use zstd::stream::raw::{DParameter, Decoder as RawDecoder, InBuffer, Operation, OutBuffer};

pub const ZSTD_FRAME_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
pub const DEFAULT_ZSTD_LEVEL: i32 = 3;
pub const ZSTD_WINDOW_LOG: u32 = 27;
/// before a frame boundary can be released without making the remaining tail
pub const DEFAULT_FRAME_BYTES: u64 = 256 << 20;
pub const ZSTD_FLUSH_INTERVAL_BYTES: usize = 1 << 20;
const BUFFER_BYTES: usize = 1 << 20;

pub fn writer_reserved_bytes(compression: StreamCompression) -> u64 {
    let buffered = BUFFER_BYTES as u64;
    match compression {
        StreamCompression::None => buffered,
        StreamCompression::Zstd => buffered + (1u64 << ZSTD_WINDOW_LOG),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamCompression {
    None,
    Zstd,
}

impl StreamCompression {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" | "off" => Some(Self::None),
            "zstd" | "on" => Some(Self::Zstd),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Zstd => "zstd",
        }
    }
}

pub struct DecodedFile {
    reader: Box<dyn Read>,
    compression: StreamCompression,
    encoded_len: u64,
}

impl DecodedFile {
    pub fn compression(&self) -> StreamCompression {
        self.compression
    }

    pub fn encoded_len(&self) -> u64 {
        self.encoded_len
    }
}

pub fn framing_stop_error(residual_decoded_bytes: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "H4CR framing stopped before physical EOF on a compressed stream \
             ({residual_decoded_bytes} decoded bytes past the stop; storage checksum \
             valid, so the decoded content itself is corrupt)"
        ),
    )
}

impl Read for DecodedFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

/// Sidecar that marks a storage file whose leading bytes were released.
pub const RELEASED_PREFIX_SIDECAR_SUFFIX: &str = ".trim.json";

pub fn released_prefix_sidecar(path: impl AsRef<Path>) -> std::path::PathBuf {
    sidecar_beside(path.as_ref())
}

fn sidecar_beside(path: &Path) -> std::path::PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(RELEASED_PREFIX_SIDECAR_SUFFIX);
    path.with_file_name(name)
}

/// DIFFERENT sidecar and then follow the alias to the released file, so the
/// is never consulted. Reading a released spool through a pre-existing hard
pub fn released_prefix_sidecars(path: impl AsRef<Path>) -> Vec<std::path::PathBuf> {
    sidecar_candidates(path.as_ref())
}

fn sidecar_candidates(path: &Path) -> Vec<std::path::PathBuf> {
    let mut candidates = vec![sidecar_beside(path)];
    if let Ok(canonical) = fs::canonicalize(path) {
        let canonical = sidecar_beside(&canonical);
        if !candidates.contains(&canonical) {
            candidates.push(canonical);
        }
    }
    candidates
}

/// Refuse any read that would start at byte zero of a released storage file.
/// After a release the leading bytes are zeros. Zeros are not the zstd frame
fn refuse_released_prefix(path: &Path) -> io::Result<()> {
    for sidecar in sidecar_candidates(path) {
        match fs::symlink_metadata(&sidecar) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("stat {}: {error}", sidecar.display()),
                ))
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{} had its leading storage released: {} names the live region, and \
                         reading from byte zero would decode the hole as stream content",
                        path.display(),
                        sidecar.display()
                    ),
                ))
            }
        }
    }
    Ok(())
}

/// Refuse to write to a storage file whose leading bytes were released.
fn refuse_trimmed_write_target(path: &Path) -> io::Result<()> {
    for sidecar in sidecar_candidates(path) {
        match fs::symlink_metadata(&sidecar) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("stat {}: {error}", sidecar.display()),
                ))
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "{} still carries the released-prefix sidecar {} of an earlier \
                         trimmed spool; retire that run's artifacts before writing here",
                        path.display(),
                        sidecar.display()
                    ),
                ))
            }
        }
    }
    Ok(())
}

pub fn detect(path: impl AsRef<Path>) -> io::Result<StreamCompression> {
    let path = path.as_ref();
    refuse_released_prefix(path)?;
    detect_storage(path)
}

fn detect_storage(path: &Path) -> io::Result<StreamCompression> {
    #[cfg(test)]
    let _scanner_guard = portable_test_fs_retry::open(path).ok();
    let mut file = File::open(path)?;
    let mut magic = [0u8; 4];
    let read = read_up_to(&mut file, &mut magic)?;
    Ok(if read == magic.len() && magic == ZSTD_FRAME_MAGIC {
        StreamCompression::Zstd
    } else {
        StreamCompression::None
    })
}

pub fn open(path: impl AsRef<Path>) -> io::Result<DecodedFile> {
    let path = path.as_ref();
    refuse_released_prefix(path)?;
    open_with_compression(path, detect_storage(path)?)
}

pub fn open_with_compression(
    path: impl AsRef<Path>,
    compression: StreamCompression,
) -> io::Result<DecodedFile> {
    let path = path.as_ref();
    refuse_released_prefix(path)?;
    open_with_compression_at(path, compression, 0)
}

/// spool trimming: the released prefix is never touched, so a deallocated or
pub fn open_with_compression_at(
    path: impl AsRef<Path>,
    compression: StreamCompression,
    stored_offset: u64,
) -> io::Result<DecodedFile> {
    let path = path.as_ref();
    // read of the released prefix wearing the explicit API's clothes.
    #[cfg(test)]
    let _scanner_guard = portable_test_fs_retry::open(path).ok();
    if stored_offset == 0 {
        refuse_released_prefix(path)?;
    }
    let mut file = File::open(path)?;
    let encoded_len = file.metadata()?.len();
    if stored_offset > encoded_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "stream live offset {stored_offset} is past the {encoded_len}-byte storage file"
            ),
        ));
    }
    if stored_offset != 0 {
        file.seek(SeekFrom::Start(stored_offset))?;
    }
    let reader: Box<dyn Read> = match compression {
        StreamCompression::None => Box::new(BufReader::with_capacity(BUFFER_BYTES, file)),
        StreamCompression::Zstd => {
            let buffered = BufReader::with_capacity(BUFFER_BYTES, file);
            Box::new(StrictZstdReader::new(buffered)?)
        }
    };
    Ok(DecodedFile {
        reader,
        compression,
        encoded_len,
    })
}

/// actually holds. It also owns the released-prefix question — there is no path
pub fn decode_open_file(mut file: File, compression: StreamCompression) -> io::Result<DecodedFile> {
    let encoded_len = file.metadata()?.len();
    file.seek(SeekFrom::Start(0))?;
    let reader: Box<dyn Read> = match compression {
        StreamCompression::None => Box::new(BufReader::with_capacity(BUFFER_BYTES, file)),
        StreamCompression::Zstd => {
            let buffered = BufReader::with_capacity(BUFFER_BYTES, file);
            Box::new(StrictZstdReader::new(buffered)?)
        }
    };
    Ok(DecodedFile {
        reader,
        compression,
        encoded_len,
    })
}

/// what lets a trimmer release storage bytes while still naming, in decoded
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameBoundary {
    pub decoded_offset: u64,
    pub stored_offset: u64,
}

#[derive(Debug, Clone)]
pub struct StreamWriteReceipt {
    pub decoded_len: u64,
    pub stored_len: u64,
    pub compression: StreamCompression,
    pub frame_boundaries: Vec<FrameBoundary>,
}

struct CountingWriter<W> {
    inner: W,
    written: u64,
}

impl<W: Write> CountingWriter<W> {
    fn new(inner: W) -> Self {
        Self { inner, written: 0 }
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.written = self.written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// boundary is dropped, doubling the effective release granularity — which
pub const MAX_RETAINED_FRAME_BOUNDARIES: usize = 4096;

pub struct StreamWriter {
    inner: WriterInner,
    compression: StreamCompression,
    decoded_len: u64,
    frame_bytes: Option<u64>,
    decoded_since_frame: u64,
    frame_boundaries: Vec<FrameBoundary>,
    deferred_error: Option<(io::ErrorKind, String)>,
}

enum WriterInner {
    Raw(CountingWriter<BufWriter<File>>),
    Zstd(PeriodicZstdWriter<CountingWriter<BufWriter<File>>>),
}

struct StrictZstdReader<R: BufRead> {
    reader: R,
    decoder: RawDecoder<'static>,
    finished_frame: bool,
    done: bool,
}

impl<R: BufRead> StrictZstdReader<R> {
    fn new(reader: R) -> io::Result<Self> {
        let mut decoder = RawDecoder::new()?;
        decoder.set_parameter(DParameter::WindowLogMax(ZSTD_WINDOW_LOG))?;
        Ok(Self {
            reader,
            decoder,
            finished_frame: false,
            done: false,
        })
    }
}

impl<R: BufRead> Read for StrictZstdReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.done {
            return Ok(0);
        }

        let mut drain_decoder = true;
        loop {
            if drain_decoder {
                drain_decoder = false;
                let (produced, finished_frame) = {
                    let mut src = InBuffer::around(&[]);
                    let mut dst = OutBuffer::around(buf);
                    let remaining = self.decoder.run(&mut src, &mut dst)?;
                    (dst.pos(), remaining == 0)
                };
                if finished_frame {
                    self.finished_frame = true;
                }
                if produced > 0 {
                    return Ok(produced);
                }
            }

            let input = self.reader.fill_buf()?;
            if input.is_empty() {
                if self.finished_frame {
                    self.done = true;
                    return Ok(0);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "incomplete zstd frame at physical EOF",
                ));
            }

            if self.finished_frame {
                self.decoder.reinit()?;
                self.finished_frame = false;
            }

            let (consumed, produced, finished_frame) = {
                let mut src = InBuffer::around(input);
                let mut dst = OutBuffer::around(buf);
                let remaining = self.decoder.run(&mut src, &mut dst)?;
                (src.pos(), dst.pos(), remaining == 0)
            };
            self.reader.consume(consumed);
            if finished_frame {
                self.finished_frame = true;
            }

            if produced > 0 {
                return Ok(produced);
            }
            if consumed == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "zstd decoder made no progress",
                ));
            }
        }
    }
}

fn new_frame_encoder<W: Write>(
    writer: W,
    level: i32,
) -> io::Result<zstd::stream::write::Encoder<'static, W>> {
    let mut encoder = zstd::stream::write::Encoder::new(writer, level)?;
    encoder.window_log(ZSTD_WINDOW_LOG)?;
    encoder.include_checksum(true)?;
    Ok(encoder)
}

struct PeriodicZstdWriter<W: Write> {
    encoder: Option<zstd::stream::write::Encoder<'static, W>>,
    level: i32,
    decoded_since_flush: usize,
    deferred_error: Option<(io::ErrorKind, String)>,
}

impl<W: Write> PeriodicZstdWriter<W> {
    fn new(writer: W) -> io::Result<Self> {
        Self::with_level(writer, DEFAULT_ZSTD_LEVEL)
    }

    fn with_level(writer: W, level: i32) -> io::Result<Self> {
        Ok(Self {
            encoder: Some(new_frame_encoder(writer, level)?),
            level,
            decoded_since_flush: 0,
            deferred_error: None,
        })
    }

    fn encoder_mut(&mut self) -> io::Result<&mut zstd::stream::write::Encoder<'static, W>> {
        self.encoder.as_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Other,
                "zstd frame writer was left without an encoder by a failed rotation",
            )
        })
    }

    fn rotate_frame(&mut self) -> io::Result<()> {
        if let Some((kind, message)) = self.deferred_error.take() {
            return Err(io::Error::new(kind, message));
        }
        let encoder = self.encoder.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Other,
                "zstd frame rotation found no encoder to close",
            )
        })?;
        let sink = encoder.finish()?;
        self.encoder = Some(new_frame_encoder(sink, self.level)?);
        self.decoded_since_flush = 0;
        Ok(())
    }

    fn finish(self) -> io::Result<W> {
        if let Some((kind, message)) = self.deferred_error {
            return Err(io::Error::new(kind, message));
        }
        self.encoder
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Other,
                    "zstd frame writer was finished without an encoder",
                )
            })?
            .finish()
    }

    fn pending_error(&self) -> Option<io::Error> {
        self.deferred_error
            .as_ref()
            .map(|(kind, message)| io::Error::new(*kind, message.clone()))
    }
}

impl<W: Write> Write for PeriodicZstdWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(err) = self.pending_error() {
            return Err(err);
        }
        if buf.is_empty() {
            return self.encoder_mut()?.write(buf);
        }

        let remaining = ZSTD_FLUSH_INTERVAL_BYTES - self.decoded_since_flush;
        let accepted = remaining.min(buf.len());
        let written = self.encoder_mut()?.write(&buf[..accepted])?;
        self.decoded_since_flush += written;
        if self.decoded_since_flush == ZSTD_FLUSH_INTERVAL_BYTES {
            if let Err(err) = self.encoder_mut()?.flush() {
                self.deferred_error = Some((err.kind(), err.to_string()));
                return Ok(written);
            }
            self.decoded_since_flush = 0;
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(err) = self.pending_error() {
            return Err(err);
        }
        self.encoder_mut()?.flush()?;
        self.decoded_since_flush = 0;
        Ok(())
    }
}

impl StreamWriter {
    pub fn create(path: impl AsRef<Path>, compression: StreamCompression) -> io::Result<Self> {
        Self::create_with_frames(path, compression, None)
    }

    /// the storage prefix in front of a boundary can later be released. `None`
    pub fn create_with_frames(
        path: impl AsRef<Path>,
        compression: StreamCompression,
        frame_bytes: Option<u64>,
    ) -> io::Result<Self> {
        if frame_bytes == Some(0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "stream frame size must be non-zero",
            ));
        }
        let path = path.as_ref();
        refuse_trimmed_write_target(path)?;
        let file = File::create(path)?;
        Self::over_file(
            file,
            compression,
            frame_bytes,
            FrameBoundary {
                decoded_offset: 0,
                stored_offset: 0,
            },
        )
    }

    /// it simply cannot offer a release point inside the pre-existing prefix.
    pub fn append_existing(
        path: impl AsRef<Path>,
        compression: StreamCompression,
        frame_bytes: Option<u64>,
        resume: FrameBoundary,
    ) -> io::Result<Self> {
        if frame_bytes == Some(0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "stream frame size must be non-zero",
            ));
        }
        let path = path.as_ref();
        refuse_trimmed_write_target(path)?;
        #[cfg(test)]
        let _scanner_guard = portable_test_fs_retry::open(path).ok();
        let file = fs::OpenOptions::new().append(true).open(path)?;
        let stored_len = file.metadata()?.len();
        if stored_len != resume.stored_offset {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} is {stored_len} storage bytes, the resume boundary names {}",
                    path.display(),
                    resume.stored_offset
                ),
            ));
        }
        Self::over_file(file, compression, frame_bytes, resume)
    }

    fn over_file(
        file: File,
        compression: StreamCompression,
        frame_bytes: Option<u64>,
        resume: FrameBoundary,
    ) -> io::Result<Self> {
        let mut buffered = CountingWriter::new(BufWriter::with_capacity(BUFFER_BYTES, file));
        buffered.written = resume.stored_offset;
        let inner = match compression {
            StreamCompression::None => WriterInner::Raw(buffered),
            StreamCompression::Zstd => WriterInner::Zstd(PeriodicZstdWriter::new(buffered)?),
        };
        Ok(Self {
            inner,
            compression,
            decoded_len: resume.decoded_offset,
            frame_bytes,
            decoded_since_frame: 0,
            frame_boundaries: vec![resume],
            deferred_error: None,
        })
    }

    pub fn frame_boundaries(&self) -> &[FrameBoundary] {
        &self.frame_boundaries
    }

    fn pending_error(&self) -> Option<io::Error> {
        self.deferred_error
            .as_ref()
            .map(|(kind, message)| io::Error::new(*kind, message.clone()))
    }

    /// longer be chosen as a release point - so the structure stays correct and
    fn push_boundary(&mut self, boundary: FrameBoundary) {
        if self.frame_boundaries.last() == Some(&boundary) {
            return;
        }
        if self.frame_boundaries.len() >= MAX_RETAINED_FRAME_BOUNDARIES {
            let mut kept = Vec::with_capacity(self.frame_boundaries.len() / 2 + 1);
            for (index, retained) in self.frame_boundaries.iter().enumerate() {
                if index % 2 == 0 {
                    kept.push(*retained);
                }
            }
            self.frame_boundaries = kept;
        }
        self.frame_boundaries.push(boundary);
    }

    pub fn decoded_len(&self) -> u64 {
        self.decoded_len
    }

    fn stored_len(&self) -> u64 {
        match &self.inner {
            WriterInner::Raw(writer) => writer.written,
            WriterInner::Zstd(writer) => match writer.encoder.as_ref() {
                Some(encoder) => encoder.get_ref().written,
                None => 0,
            },
        }
    }

    fn rotate_frame(&mut self) -> io::Result<()> {
        match &mut self.inner {
            WriterInner::Raw(writer) => writer.flush()?,
            WriterInner::Zstd(writer) => writer.rotate_frame()?,
        }
        self.decoded_since_frame = 0;
        let boundary = FrameBoundary {
            decoded_offset: self.decoded_len,
            stored_offset: self.stored_len(),
        };
        self.push_boundary(boundary);
        Ok(())
    }

    pub fn close_frame(&mut self) -> io::Result<()> {
        if let Some(error) = self.pending_error() {
            return Err(error);
        }
        self.rotate_frame()
    }

    fn storage_file(&mut self) -> io::Result<&mut BufWriter<File>> {
        Ok(match &mut self.inner {
            WriterInner::Raw(writer) => &mut writer.inner,
            WriterInner::Zstd(writer) => &mut writer.encoder_mut()?.get_mut().inner,
        })
    }

    pub fn sync_storage(&mut self) -> io::Result<()> {
        if let Some(error) = self.pending_error() {
            return Err(error);
        }
        let buffered = self.storage_file()?;
        buffered.flush()?;
        buffered.get_ref().sync_all()
    }

    pub fn finish(self) -> io::Result<StreamWriteReceipt> {
        self.finish_inner(false)
    }

    pub fn finish_durable(self) -> io::Result<StreamWriteReceipt> {
        self.finish_inner(true)
    }

    fn finish_inner(mut self, sync: bool) -> io::Result<StreamWriteReceipt> {
        if let Some(error) = self.pending_error() {
            return Err(error);
        }
        let decoded_len = self.decoded_len;
        let compression = self.compression;
        let frame_boundaries = std::mem::take(&mut self.frame_boundaries);
        let stored_len = match self.inner {
            WriterInner::Raw(mut writer) => {
                writer.flush()?;
                if sync {
                    writer.inner.get_ref().sync_all()?;
                }
                writer.written
            }
            WriterInner::Zstd(encoder) => {
                let mut writer = encoder.finish()?;
                writer.flush()?;
                if sync {
                    writer.inner.get_ref().sync_all()?;
                }
                writer.written
            }
        };
        Ok(StreamWriteReceipt {
            decoded_len,
            stored_len,
            compression,
            frame_boundaries,
        })
    }
}

impl Write for StreamWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(error) = self.pending_error() {
            return Err(error);
        }
        let buf = match self.frame_bytes {
            Some(frame_bytes) => {
                let room = frame_bytes.saturating_sub(self.decoded_since_frame);
                let room = usize::try_from(room).unwrap_or(usize::MAX);
                &buf[..room.min(buf.len())]
            }
            None => buf,
        };
        let written = match &mut self.inner {
            WriterInner::Raw(writer) => writer.write(buf)?,
            WriterInner::Zstd(writer) => writer.write(buf)?,
        };
        self.decoded_len = self.decoded_len.saturating_add(written as u64);
        self.decoded_since_frame = self.decoded_since_frame.saturating_add(written as u64);
        if let Some(frame_bytes) = self.frame_bytes {
            if self.decoded_since_frame >= frame_bytes && written > 0 {
                if let Err(error) = self.rotate_frame() {
                    self.deferred_error = Some((error.kind(), error.to_string()));
                    return Ok(written);
                }
            }
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(error) = self.pending_error() {
            return Err(error);
        }
        match &mut self.inner {
            WriterInner::Raw(writer) => writer.flush(),
            WriterInner::Zstd(writer) => writer.flush(),
        }
    }
}

pub fn encode_to_writer(
    reader: &mut impl Read,
    sink: &mut impl Write,
    compression: StreamCompression,
    level: i32,
    frame_bytes: Option<u64>,
) -> io::Result<(u64, u64)> {
    if frame_bytes == Some(0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "stream frame size must be non-zero",
        ));
    }
    let mut counting = CountingWriter::new(sink);
    let decoded = match compression {
        StreamCompression::None => io::copy(reader, &mut counting)?,
        StreamCompression::Zstd => {
            let mut writer = PeriodicZstdWriter::with_level(counting, level)?;
            let mut decoded = 0u64;
            let mut since_frame = 0u64;
            let mut buf = vec![0u8; 65_536];
            loop {
                let read = reader.read(&mut buf)?;
                if read == 0 {
                    break;
                }
                let mut offset = 0usize;
                while offset < read {
                    let room = match frame_bytes {
                        Some(frame_bytes) => {
                            usize::try_from(frame_bytes.saturating_sub(since_frame))
                                .unwrap_or(usize::MAX)
                                .min(read - offset)
                        }
                        None => read - offset,
                    };
                    debug_assert!(room > 0, "a non-zero frame size always leaves room");
                    writer.write_all(&buf[offset..offset + room])?;
                    offset += room;
                    decoded = decoded.saturating_add(room as u64);
                    since_frame = since_frame.saturating_add(room as u64);
                    if frame_bytes.is_some_and(|frame_bytes| since_frame >= frame_bytes) {
                        writer.rotate_frame()?;
                        since_frame = 0;
                    }
                }
            }
            counting = writer.finish()?;
            decoded
        }
    };
    counting.flush()?;
    Ok((decoded, counting.written))
}

pub fn encode_file(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    compression: StreamCompression,
) -> io::Result<(u64, u64)> {
    let mut reader = open(input)?;
    let mut writer = StreamWriter::create(output.as_ref(), compression)?;
    let decoded_len = io::copy(&mut reader, &mut writer)?;
    writer.finish()?;
    let encoded_len = fs::metadata(output)?.len();
    Ok((decoded_len, encoded_len))
}

pub fn decode_file(input: impl AsRef<Path>, output: impl AsRef<Path>) -> io::Result<(u64, u64)> {
    encode_file(input, output, StreamCompression::None)
}

fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[derive(Default)]
    struct FlushCountingSink {
        bytes: Vec<u8>,
        flushes: usize,
    }

    impl Write for FlushCountingSink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "hoi4_compact_stream_{tag}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn zstd_round_trip_is_byte_identical() {
        let dir = TmpDir::new("round_trip");
        let raw_path = dir.0.join("raw.bin");
        let zstd_path = dir.0.join("compressed.bin");
        let decoded_path = dir.0.join("decoded.bin");
        let mut bytes = Vec::new();
        for i in 0..4096u32 {
            bytes.extend_from_slice(b"H4CR");
            bytes.extend_from_slice(&i.to_le_bytes());
            bytes.extend_from_slice(&[0x5a; 128]);
        }
        std::fs::write(&raw_path, &bytes).unwrap();

        let (decoded_len, encoded_len) =
            encode_file(&raw_path, &zstd_path, StreamCompression::Zstd).unwrap();
        assert_eq!(decoded_len, bytes.len() as u64);
        assert_eq!(
            portable_test_fs_retry::with_read_guard(&zstd_path, || detect(&zstd_path))
                .expect("hold the freshly encoded stream open across the scanner's window")
                .unwrap(),
            StreamCompression::Zstd
        );
        assert!(encoded_len < decoded_len);

        decode_file(&zstd_path, &decoded_path).unwrap();
        assert_eq!(portable_test_fs_retry::read(decoded_path).unwrap(), bytes);
    }

    #[test]
    fn truncated_frame_after_decoded_boundary_is_rejected() {
        let dir = TmpDir::new("truncated_frame");
        let raw_path = dir.0.join("raw.bin");
        let zstd_path = dir.0.join("compressed.bin");
        let decoded_path = dir.0.join("decoded.bin");
        let mut bytes = Vec::new();
        for i in 0..8192u32 {
            bytes.extend_from_slice(b"H4CR");
            bytes.extend_from_slice(&i.to_le_bytes());
            bytes.extend_from_slice(&[0x5a; 128]);
        }
        std::fs::write(&raw_path, &bytes).unwrap();
        encode_file(&raw_path, &zstd_path, StreamCompression::Zstd).unwrap();

        let mut stored = portable_test_fs_retry::read(&zstd_path).unwrap();
        stored.pop().expect("checksummed zstd frame is non-empty");
        std::fs::write(&zstd_path, stored).unwrap();

        let mut reader = portable_test_fs_retry::with_read_guard(&zstd_path, || open(&zstd_path))
            .expect("hold the rewritten stream open across the scanner's window")
            .unwrap();
        let mut decoded = Vec::new();
        let err = reader
            .read_to_end(&mut decoded)
            .expect_err("a torn zstd frame must not decode as clean EOF");
        assert_eq!(decoded, bytes, "EOF lands on a decoded fixture boundary");
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert!(err.to_string().contains("incomplete zstd frame"));

        let err = decode_file(&zstd_path, &decoded_path)
            .expect_err("compact_stream decompress must surface the torn frame");
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn content_checksum_rejects_an_in_frame_byte_flip() {
        let dir = TmpDir::new("checksum");
        let zstd_path = dir.0.join("compressed.bin");
        let mut state = 0x9e37_79b9u32;
        let bytes: Vec<u8> = (0..512 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();

        let mut writer = StreamWriter::create(&zstd_path, StreamCompression::Zstd).unwrap();
        writer.write_all(&bytes).unwrap();
        writer.finish().unwrap();

        let mut stored = portable_test_fs_retry::read(&zstd_path).unwrap();
        assert_ne!(stored[4] & 0x04, 0, "zstd frame checksum flag must be set");
        let flip_at = stored.len() / 2;
        stored[flip_at] ^= 0x01;
        std::fs::write(&zstd_path, stored).unwrap();

        let mut reader = portable_test_fs_retry::with_read_guard(&zstd_path, || open(&zstd_path))
            .expect("hold the rewritten stream open across the scanner's window")
            .unwrap();
        let mut decoded = Vec::new();
        let err = reader
            .read_to_end(&mut decoded)
            .expect_err("content corruption must fail decoding");
        assert!(
            err.to_string().to_ascii_lowercase().contains("checksum"),
            "expected checksum failure, got {err:?}"
        );
    }

    #[test]
    fn zstd_flushes_each_mebibyte_of_decoded_input() {
        let mut writer = PeriodicZstdWriter::new(FlushCountingSink::default()).unwrap();
        writer
            .write_all(&vec![0x5a; ZSTD_FLUSH_INTERVAL_BYTES - 1])
            .unwrap();
        assert_eq!(writer.encoder.as_ref().unwrap().get_ref().flushes, 0);

        writer.write_all(&[0x5a; 2]).unwrap();
        assert_eq!(writer.encoder.as_ref().unwrap().get_ref().flushes, 1);
        assert_eq!(writer.decoded_since_flush, 1);

        writer
            .write_all(&vec![0xa5; ZSTD_FLUSH_INTERVAL_BYTES - 1])
            .unwrap();
        assert_eq!(writer.encoder.as_ref().unwrap().get_ref().flushes, 2);
        assert_eq!(writer.decoded_since_flush, 0);
    }

    fn framed_fixture(records: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        for i in 0..records {
            bytes.extend_from_slice(b"H4CR");
            bytes.extend_from_slice(&i.to_le_bytes());
            bytes.extend_from_slice(&[0x5a; 128]);
        }
        bytes
    }

    fn write_framed(
        path: &Path,
        bytes: &[u8],
        compression: StreamCompression,
        frame: u64,
    ) -> StreamWriteReceipt {
        let mut writer = StreamWriter::create_with_frames(path, compression, Some(frame)).unwrap();
        for chunk in bytes.chunks(65_536) {
            writer.write_all(chunk).unwrap();
        }
        writer.finish().unwrap()
    }

    #[test]
    fn rotated_frames_decode_as_one_stream_and_report_their_boundaries() {
        let dir = TmpDir::new("frames");
        let bytes = framed_fixture(16_384);
        for compression in [StreamCompression::None, StreamCompression::Zstd] {
            let path = dir.0.join(format!("framed_{}.bin", compression.as_str()));
            let frame = 512 * 1024;
            let receipt = write_framed(&path, &bytes, compression, frame);

            assert_eq!(receipt.decoded_len, bytes.len() as u64);
            assert_eq!(receipt.stored_len, fs::metadata(&path).unwrap().len());
            assert_eq!(receipt.compression, compression);
            assert_eq!(receipt.frame_boundaries.first().unwrap().decoded_offset, 0);
            assert_eq!(receipt.frame_boundaries.first().unwrap().stored_offset, 0);
            assert!(
                receipt.frame_boundaries.len() >= 4,
                "2 MiB of input at a 512 KiB frame must rotate: {:?}",
                receipt.frame_boundaries
            );
            for pair in receipt.frame_boundaries.windows(2) {
                assert_eq!(
                    pair[1].decoded_offset - pair[0].decoded_offset,
                    frame,
                    "frames must close on exact decoded multiples"
                );
                assert!(pair[1].stored_offset > pair[0].stored_offset);
            }

            let mut whole = Vec::new();
            open(&path).unwrap().read_to_end(&mut whole).unwrap();
            assert_eq!(
                whole,
                bytes,
                "{} rotation changed decoded bytes",
                compression.as_str()
            );

            for boundary in &receipt.frame_boundaries {
                let mut suffix = Vec::new();
                open_with_compression_at(&path, compression, boundary.stored_offset)
                    .unwrap()
                    .read_to_end(&mut suffix)
                    .unwrap();
                assert_eq!(
                    suffix,
                    bytes[boundary.decoded_offset as usize..],
                    "resume at stored {} / decoded {} lost bytes",
                    boundary.stored_offset,
                    boundary.decoded_offset
                );
            }
        }
    }

    #[test]
    fn a_closed_frame_can_be_resumed_by_a_second_writer() {
        let dir = TmpDir::new("resume");
        let head = framed_fixture(1_024);
        let tail = framed_fixture(512);
        for compression in [StreamCompression::None, StreamCompression::Zstd] {
            let path = dir.0.join(format!("resumed_{}.bin", compression.as_str()));
            let mut writer = StreamWriter::create_with_frames(&path, compression, None).unwrap();
            writer.write_all(&head).unwrap();
            writer.close_frame().unwrap();
            writer.sync_storage().unwrap();
            let boundary = *writer.frame_boundaries().last().unwrap();
            assert_eq!(boundary.decoded_offset, head.len() as u64);
            assert_eq!(
                boundary.stored_offset,
                fs::metadata(&path).unwrap().len(),
                "{}: a closed and synced frame must be entirely on disk",
                compression.as_str()
            );
            let mut so_far = Vec::new();
            open(&path).unwrap().read_to_end(&mut so_far).unwrap();
            assert_eq!(
                so_far,
                head,
                "{} lost the closed prefix",
                compression.as_str()
            );
            drop(writer);

            let mut resumed =
                StreamWriter::append_existing(&path, compression, None, boundary).unwrap();
            resumed.write_all(&tail).unwrap();
            let receipt = resumed.finish_durable().unwrap();
            assert_eq!(receipt.decoded_len, (head.len() + tail.len()) as u64);
            assert_eq!(receipt.stored_len, fs::metadata(&path).unwrap().len());

            let mut whole = Vec::new();
            open(&path).unwrap().read_to_end(&mut whole).unwrap();
            let mut expected = head.clone();
            expected.extend_from_slice(&tail);
            assert_eq!(
                whole,
                expected,
                "{} resume changed the decoded stream",
                compression.as_str()
            );
        }
    }

    #[test]
    fn a_resume_boundary_that_is_not_the_file_end_is_refused() {
        let dir = TmpDir::new("resume_bad");
        let path = dir.0.join("short.bin");
        let bytes = framed_fixture(64);
        let receipt = write_framed(&path, &bytes, StreamCompression::Zstd, u64::MAX);
        let error = match StreamWriter::append_existing(
            &path,
            StreamCompression::Zstd,
            None,
            FrameBoundary {
                decoded_offset: receipt.decoded_len,
                stored_offset: receipt.stored_len - 1,
            },
        ) {
            Ok(_) => panic!("a resume boundary short of the file end was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn rotation_ratio_cost_at_64_mib_frames_is_negligible() {
        let dir = TmpDir::new("frame_ratio");
        let bytes = framed_fixture(262_144);
        let single = dir.0.join("single.bin");
        let framed = dir.0.join("framed.bin");

        let single_receipt = write_framed(&single, &bytes, StreamCompression::Zstd, u64::MAX);
        let framed_receipt = write_framed(&framed, &bytes, StreamCompression::Zstd, 4 << 20);
        assert_eq!(single_receipt.frame_boundaries.len(), 1);
        assert_eq!(framed_receipt.frame_boundaries.len(), 9);

        let overhead = framed_receipt.stored_len as f64 - single_receipt.stored_len as f64;
        let overhead_percent = overhead * 100.0 / single_receipt.stored_len as f64;
        eprintln!(
            "frame_ratio decoded={} single_frame_bytes={} rotated_bytes={} overhead_percent={:.6}",
            bytes.len(),
            single_receipt.stored_len,
            framed_receipt.stored_len,
            overhead_percent
        );
        assert!(
            overhead_percent <= 1.0,
            "frame rotation ratio cost unexpectedly high: {overhead_percent:.6}%"
        );
    }

    #[test]
    fn a_released_storage_prefix_is_never_read_back_as_content() {
        let dir = TmpDir::new("released_prefix");
        let bytes = framed_fixture(16_384);
        let path = dir.0.join("collector_stream.bin");
        let receipt = write_framed(&path, &bytes, StreamCompression::Zstd, 512 * 1024);
        let boundary = receipt.frame_boundaries[2];

        let mut stored = portable_test_fs_retry::read(&path).unwrap();
        for byte in &mut stored[..boundary.stored_offset as usize] {
            *byte = 0;
        }
        fs::write(&path, &stored).unwrap();

        assert_eq!(detect(&path).unwrap(), StreamCompression::None);
        let mut misread = Vec::new();
        open(&path).unwrap().read_to_end(&mut misread).unwrap();
        assert_ne!(misread, bytes, "the hole is not the stream");

        fs::write(released_prefix_sidecar(&path), b"{}").unwrap();
        for error in [
            open(&path).err().expect("open must fail closed"),
            open_with_compression(&path, StreamCompression::Zstd)
                .err()
                .expect("a typed zero-offset open must fail closed"),
            detect(&path).err().expect("detect must fail closed"),
        ] {
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("released"), "{error}");
        }
        let decoded_path = dir.0.join("decoded.bin");
        decode_file(&path, &decoded_path)
            .expect_err("a plain decompress must not copy a released prefix");

        let mut suffix = Vec::new();
        open_with_compression_at(&path, StreamCompression::Zstd, boundary.stored_offset)
            .unwrap()
            .read_to_end(&mut suffix)
            .unwrap();
        assert_eq!(suffix, bytes[boundary.decoded_offset as usize..]);
    }

    #[test]
    fn the_explicit_resume_api_still_refuses_offset_zero_on_a_released_prefix() {
        let dir = TmpDir::new("explicit_zero");
        let bytes = framed_fixture(4_096);
        let path = dir.0.join("collector_stream.bin");
        let receipt = write_framed(&path, &bytes, StreamCompression::Zstd, 128 * 1024);
        let boundary = receipt
            .frame_boundaries
            .iter()
            .copied()
            .find(|boundary| boundary.stored_offset > 0)
            .expect("the fixture must rotate");
        fs::write(released_prefix_sidecar(&path), b"{}").unwrap();

        let error = open_with_compression_at(&path, StreamCompression::Zstd, 0)
            .err()
            .expect("offset zero must fail closed even through the explicit API");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("released"), "{error}");

        let mut suffix = Vec::new();
        open_with_compression_at(&path, StreamCompression::Zstd, boundary.stored_offset)
            .unwrap()
            .read_to_end(&mut suffix)
            .unwrap();
        assert_eq!(suffix, bytes[boundary.decoded_offset as usize..]);
    }

    #[test]
    fn a_leftover_sidecar_refuses_the_next_spool_at_creation() {
        let dir = TmpDir::new("stale_sidecar");
        let path = dir.0.join("collector_stream.bin");
        fs::write(released_prefix_sidecar(&path), b"{}").unwrap();

        let error = StreamWriter::create_with_frames(
            &path,
            StreamCompression::Zstd,
            Some(DEFAULT_FRAME_BYTES),
        )
        .err()
        .expect("a run directory holding a trimmed spool's sidecar must not be written into");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(
            error.to_string().contains("released-prefix sidecar"),
            "{error}"
        );

        portable_test_fs_retry::remove_file(released_prefix_sidecar(&path)).unwrap();
        let mut writer =
            StreamWriter::create_with_frames(&path, StreamCompression::Zstd, Some(1 << 20))
                .unwrap();
        writer.write_all(b"H4CR").unwrap();
        writer.finish().unwrap();
        let mut decoded = Vec::new();
        open(&path).unwrap().read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, b"H4CR");
    }

    #[test]
    fn live_offset_past_the_storage_file_is_rejected() {
        let dir = TmpDir::new("bad_offset");
        let path = dir.0.join("collector_stream.bin");
        fs::write(&path, [0u8; 16]).unwrap();
        let error = match open_with_compression_at(&path, StreamCompression::None, 17) {
            Ok(_) => panic!("an offset past EOF must not open"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn encode_to_writer_matches_the_file_writer_byte_for_byte() {
        let dir = TmpDir::new("encode_to_writer");
        let bytes = framed_fixture(8_192);
        let path = dir.0.join("file.bin");
        let mut writer = StreamWriter::create(&path, StreamCompression::Zstd).unwrap();
        writer.write_all(&bytes).unwrap();
        writer.finish().unwrap();

        let mut sink = Vec::new();
        let (decoded, stored) = encode_to_writer(
            &mut bytes.as_slice(),
            &mut sink,
            StreamCompression::Zstd,
            DEFAULT_ZSTD_LEVEL,
            None,
        )
        .unwrap();
        assert_eq!(decoded, bytes.len() as u64);
        assert_eq!(stored, sink.len() as u64);
        assert_eq!(sink, portable_test_fs_retry::read(&path).unwrap());
    }

    #[test]
    fn periodic_flush_level_3_ratio_cost_is_measured() {
        let dir = TmpDir::new("flush_ratio");
        let raw_path = dir.0.join("raw.bin");
        let periodic_path = dir.0.join("periodic.bin");
        let mut bytes = Vec::new();
        for i in 0..131_072u32 {
            bytes.extend_from_slice(b"H4CR");
            bytes.extend_from_slice(&i.to_le_bytes());
            bytes.extend_from_slice(&[0x5a; 128]);
        }
        std::fs::write(&raw_path, &bytes).unwrap();

        encode_file(&raw_path, &periodic_path, StreamCompression::Zstd).unwrap();
        let periodic_len = std::fs::metadata(&periodic_path).unwrap().len();

        let mut baseline_encoder =
            zstd::stream::write::Encoder::new(Vec::new(), DEFAULT_ZSTD_LEVEL).unwrap();
        baseline_encoder.window_log(ZSTD_WINDOW_LOG).unwrap();
        baseline_encoder.include_checksum(true).unwrap();
        baseline_encoder.write_all(&bytes).unwrap();
        let baseline_len = baseline_encoder.finish().unwrap().len() as u64;

        let overhead_bytes = periodic_len as i64 - baseline_len as i64;
        let overhead_percent = overhead_bytes as f64 * 100.0 / baseline_len as f64;
        eprintln!(
            "flush_ratio decoded_bytes={} baseline_bytes={} periodic_bytes={} overhead_bytes={} overhead_percent={:.6}",
            bytes.len(), baseline_len, periodic_len, overhead_bytes, overhead_percent
        );
        assert!(
            overhead_percent <= 2.0,
            "1 MiB flush ratio cost unexpectedly high: {overhead_percent:.6}%"
        );
    }
}
