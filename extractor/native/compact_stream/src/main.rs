use compact_stream::{decode_file, encode_file, StreamCompression};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

/// Max-speed decoded input rate measured on the t31 live capture
/// (2 373 041 632 bytes over the 60 s capture window).
const MAX_SPEED_INPUT_MB_PER_S: f64 = 39.55;

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("compact_stream: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    if matches!(
        args.first().map(String::as_str),
        None | Some("-h") | Some("--help")
    ) {
        print_usage();
        return Ok(());
    }
    if args[0] == "bench" {
        return bench(&args[1..]);
    }
    if args.len() != 3 {
        return Err(usage());
    }
    let input = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);
    reject_same_path(&input, &output)?;
    let (decoded, encoded) = match args[0].as_str() {
        "compress" => encode_file(&input, &output, StreamCompression::Zstd),
        "decompress" => decode_file(&input, &output),
        other => return Err(format!("unknown mode {other:?}\n{}", usage())),
    }
    .map_err(|error| format!("{} -> {}: {error}", input.display(), output.display()))?;
    println!(
        "input={} output={} decoded_bytes={} stored_bytes={}",
        input.display(),
        output.display(),
        decoded,
        encoded
    );
    Ok(())
}

/// Measure the spool storage codec on real capture bytes.
///
/// The input is decoded into RAM once so the timed section is the encoder and
/// nothing else — no source disk read, no sink disk write. One thread, so the
/// reported MB/s is MB/s per core by construction; that is the number the
/// capture loop pays, because the loop compresses inline on the pipe thread.
fn bench(args: &[String]) -> Result<(), String> {
    if args.is_empty() || args.len() > 4 {
        return Err(usage());
    }
    let input = PathBuf::from(&args[0]);
    let cap_bytes: usize = match args.get(1) {
        Some(value) => value
            .parse()
            .map_err(|error| format!("bad decoded byte cap {value:?}: {error}"))?,
        None => 512 << 20,
    };
    let levels: Vec<i32> = match args.get(2) {
        Some(value) => value
            .split(',')
            .map(|level| {
                level
                    .trim()
                    .parse()
                    .map_err(|error| format!("bad level {level:?}: {error}"))
            })
            .collect::<Result<_, _>>()?,
        None => vec![1, 2, 3, 4, 5],
    };

    // Production rotates the storage frame on this cadence; measuring without
    // it would leave the encoder-restart cost out of the number.
    let frame_bytes: Option<u64> = match args.get(3) {
        Some(value) if value == "none" => None,
        Some(value) => Some(
            value
                .parse()
                .map_err(|error| format!("bad frame byte cadence {value:?}: {error}"))?,
        ),
        None => Some(compact_stream::DEFAULT_FRAME_BYTES),
    };

    let mut decoded = Vec::with_capacity(cap_bytes.min(1 << 30));
    let mut reader = compact_stream::open(&input)
        .map_err(|error| format!("open {}: {error}", input.display()))?;
    reader
        .by_ref()
        .take(cap_bytes as u64)
        .read_to_end(&mut decoded)
        .map_err(|error| format!("read {}: {error}", input.display()))?;
    if decoded.is_empty() {
        return Err(format!("{} decoded to zero bytes", input.display()));
    }
    println!(
        "input={} decoded_bytes={} frame_bytes={frame_bytes:?} max_speed_input_mb_per_s={MAX_SPEED_INPUT_MB_PER_S}",
        input.display(),
        decoded.len()
    );

    for level in levels {
        // Feed in the exact 64 KiB bursts the recorder host reads from the
        // collector pipe, so the per-call overhead is the production one.
        let started = Instant::now();
        let mut sink = std::io::sink();
        let (decoded_len, stored_len) = compact_stream::encode_to_writer(
            &mut ChunkedReader::new(&decoded, 65_536),
            &mut sink,
            StreamCompression::Zstd,
            level,
            frame_bytes,
        )
        .map_err(|error| format!("encode at level {level}: {error}"))?;
        let elapsed = started.elapsed().as_secs_f64();
        let mb_per_s = decoded_len as f64 / 1e6 / elapsed;
        let core_fraction = MAX_SPEED_INPUT_MB_PER_S / mb_per_s;
        let ratio = decoded_len as f64 / stored_len as f64;
        let stored_mb_per_s = MAX_SPEED_INPUT_MB_PER_S / ratio;
        println!(
            "level={level} decoded_bytes={decoded_len} stored_bytes={stored_len} ratio={ratio:.3} \
             elapsed_s={elapsed:.3} encode_mb_per_s_per_core={mb_per_s:.1} \
             core_fraction_at_max_speed={core_fraction:.4} \
             disk_write_mb_per_s={stored_mb_per_s:.3} \
             eight_hour_spool_gb={:.2}",
            stored_mb_per_s * 28_800.0 / 1000.0
        );
    }
    println!(
        "baseline_uncompressed disk_write_mb_per_s={MAX_SPEED_INPUT_MB_PER_S:.3} eight_hour_spool_gb={:.2}",
        MAX_SPEED_INPUT_MB_PER_S * 28_800.0 / 1000.0
    );
    Ok(())
}

/// Hands out a fixed-size slice per `read`, reproducing the recorder host's
/// pipe-burst write pattern instead of one giant buffer.
struct ChunkedReader<'a> {
    bytes: &'a [u8],
    chunk: usize,
}

impl<'a> ChunkedReader<'a> {
    fn new(bytes: &'a [u8], chunk: usize) -> Self {
        Self { bytes, chunk }
    }
}

impl std::io::Read for ChunkedReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let take = self.chunk.min(buf.len()).min(self.bytes.len());
        buf[..take].copy_from_slice(&self.bytes[..take]);
        self.bytes = &self.bytes[take..];
        Ok(take)
    }
}

fn reject_same_path(input: &Path, output: &Path) -> Result<(), String> {
    let input = std::fs::canonicalize(input)
        .map_err(|error| format!("input {}: {error}", input.display()))?;
    let output = if output.exists() {
        std::fs::canonicalize(output)
            .map_err(|error| format!("output {}: {error}", output.display()))?
    } else {
        let parent = output.parent().unwrap_or_else(|| Path::new("."));
        let parent = std::fs::canonicalize(parent)
            .map_err(|error| format!("output parent {}: {error}", parent.display()))?;
        parent.join(output.file_name().ok_or("output path has no file name")?)
    };
    if input == output {
        return Err("input and output must be different paths".to_string());
    }
    Ok(())
}

fn print_usage() {
    eprintln!("{}", usage());
}

fn usage() -> String {
    "usage:\n  compact_stream compress <input> <output>\n  compact_stream decompress <input> <output>\n  compact_stream bench <input> [max_decoded_bytes] [levels,csv] [frame_bytes|none]".to_string()
}
