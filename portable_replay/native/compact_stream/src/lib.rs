//! Neutral file-boundary compression for the canonical H4CR byte stream.
//!
//! This crate deliberately knows nothing about H4CR framing or fold semantics. It
//! only preserves an exact byte stream while storing it either verbatim or as one
//! standard Zstandard frame. Callers auto-detect the storage encoding by magic.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;

use zstd::stream::raw::{DParameter, Decoder as RawDecoder, InBuffer, Operation, OutBuffer};

pub const ZSTD_FRAME_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
pub const DEFAULT_ZSTD_LEVEL: i32 = 3;
pub const ZSTD_WINDOW_LOG: u32 = 27;
/// Maximum decoded input accepted between zstd flushes. A successful flush
/// pushes the compressed prefix through the underlying 1 MiB `BufWriter`, so a
/// process kill can strand at most this much decoded tail in application memory.
pub const ZSTD_FLUSH_INTERVAL_BYTES: usize = 1 << 20;
const BUFFER_BYTES: usize = 1 << 20;

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

/// The uniform hard error a consumer must return when H4CR framing stops
/// before physical EOF on a COMPRESSED source and its own drain-to-EOF found
/// the storage layer intact (footer + content checksum valid).
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

pub fn detect(path: impl AsRef<Path>) -> io::Result<StreamCompression> {
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
    open_with_compression(path, detect(path)?)
}

pub fn open_with_compression(
    path: impl AsRef<Path>,
    compression: StreamCompression,
) -> io::Result<DecodedFile> {
    let path = path.as_ref();
    let file = File::open(path)?;
    let encoded_len = file.metadata()?.len();
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

pub struct StreamWriter {
    inner: WriterInner,
}

enum WriterInner {
    Raw(BufWriter<File>),
    Zstd(PeriodicZstdWriter<BufWriter<File>>),
}

/// Pull decoder that owns the physical-EOF rule instead of delegating it to
/// zstd's high-level `Read` wrapper. The raw operation's `run` return value is
/// zero only after a complete frame (including its content checksum). Physical
/// EOF at any other point is a hard `UnexpectedEof`.
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
            // libzstd may have decoded output buffered after consuming the last
            // physical input bytes. Offer an empty input once per Read call so
            // that output is returned before deciding whether physical EOF is
            // a complete frame or a tear.
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

struct PeriodicZstdWriter<W: Write> {
    encoder: zstd::stream::write::Encoder<'static, W>,
    decoded_since_flush: usize,
    deferred_error: Option<(io::ErrorKind, String)>,
}

impl<W: Write> PeriodicZstdWriter<W> {
    fn new(writer: W) -> io::Result<Self> {
        let mut encoder = zstd::stream::write::Encoder::new(writer, DEFAULT_ZSTD_LEVEL)?;
        encoder.window_log(ZSTD_WINDOW_LOG)?;
        encoder.include_checksum(true)?;
        Ok(Self {
            encoder,
            decoded_since_flush: 0,
            deferred_error: None,
        })
    }

    fn finish(self) -> io::Result<W> {
        if let Some((kind, message)) = self.deferred_error {
            return Err(io::Error::new(kind, message));
        }
        self.encoder.finish()
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
            return self.encoder.write(buf);
        }

        let remaining = ZSTD_FLUSH_INTERVAL_BYTES - self.decoded_since_flush;
        let accepted = remaining.min(buf.len());
        let written = self.encoder.write(&buf[..accepted])?;
        self.decoded_since_flush += written;
        if self.decoded_since_flush == ZSTD_FLUSH_INTERVAL_BYTES {
            if let Err(err) = self.encoder.flush() {
                // The encoder already consumed `written` bytes, so the Write
                // contract requires returning Ok(written). Defer this hard error
                // to the next write/flush/finish instead of inviting a retry that
                // would duplicate decoded input.
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
        self.encoder.flush()?;
        self.decoded_since_flush = 0;
        Ok(())
    }
}

impl StreamWriter {
    pub fn create(path: impl AsRef<Path>, compression: StreamCompression) -> io::Result<Self> {
        let file = File::create(path)?;
        let buffered = BufWriter::with_capacity(BUFFER_BYTES, file);
        let inner = match compression {
            StreamCompression::None => WriterInner::Raw(buffered),
            StreamCompression::Zstd => WriterInner::Zstd(PeriodicZstdWriter::new(buffered)?),
        };
        Ok(Self { inner })
    }

    pub fn finish(self) -> io::Result<()> {
        match self.inner {
            WriterInner::Raw(mut writer) => writer.flush(),
            WriterInner::Zstd(encoder) => {
                let mut writer = encoder.finish()?;
                writer.flush()
            }
        }
    }
}

impl Write for StreamWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.inner {
            WriterInner::Raw(writer) => writer.write(buf),
            WriterInner::Zstd(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.inner {
            WriterInner::Raw(writer) => writer.flush(),
            WriterInner::Zstd(writer) => writer.flush(),
        }
    }
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
        assert_eq!(detect(&zstd_path).unwrap(), StreamCompression::Zstd);
        assert!(encoded_len < decoded_len);

        decode_file(&zstd_path, &decoded_path).unwrap();
        assert_eq!(std::fs::read(decoded_path).unwrap(), bytes);
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

        let mut stored = std::fs::read(&zstd_path).unwrap();
        stored.pop().expect("checksummed zstd frame is non-empty");
        std::fs::write(&zstd_path, stored).unwrap();

        let mut reader = open(&zstd_path).unwrap();
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

        let mut stored = std::fs::read(&zstd_path).unwrap();
        assert_ne!(stored[4] & 0x04, 0, "zstd frame checksum flag must be set");
        let flip_at = stored.len() / 2;
        stored[flip_at] ^= 0x01;
        std::fs::write(&zstd_path, stored).unwrap();

        let mut reader = open(&zstd_path).unwrap();
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
        assert_eq!(writer.encoder.get_ref().flushes, 0);

        writer.write_all(&[0x5a; 2]).unwrap();
        assert_eq!(writer.encoder.get_ref().flushes, 1);
        assert_eq!(writer.decoded_since_flush, 1);

        writer
            .write_all(&vec![0xa5; ZSTD_FLUSH_INTERVAL_BYTES - 1])
            .unwrap();
        assert_eq!(writer.encoder.get_ref().flushes, 2);
        assert_eq!(writer.decoded_since_flush, 0);
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
