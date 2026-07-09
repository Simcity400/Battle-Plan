use flate2::write::DeflateEncoder;
use flate2::Compression;
use serde_json::json;
use snapshot::pack::ZSTD_COMPRESSION_LEVEL;
use snapshot::zip::ZSTD_WINDOW_LOG;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Instant;

const CHUNK: usize = 4 * 1024 * 1024;

struct CountingSink {
    bytes: u64,
}

impl Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("container_codec_bench: {err}");
            std::process::ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "usage: container_codec_bench <path> [limit_bytes]".to_string())?;
    let meta = std::fs::metadata(&path).map_err(|err| format!("stat {}: {err}", path.display()))?;
    let limit = match args.next() {
        Some(raw) => Some(
            raw.parse::<u64>()
                .map_err(|err| format!("parse limit_bytes {raw:?}: {err}"))?,
        ),
        None => None,
    };
    let input_bytes = limit.unwrap_or(meta.len()).min(meta.len());
    let deflate = bench_deflate(&path, input_bytes)?;
    let zstd = bench_zstd(&path, input_bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "path": path.display().to_string(),
            "input_bytes": input_bytes,
            "deflate6": deflate,
            "zstd_method93": zstd,
            "zstd_settings": {
                "level": ZSTD_COMPRESSION_LEVEL,
                "window_log": ZSTD_WINDOW_LOG,
                "long_distance_matching": true,
                "checksum": true,
            }
        }))
        .unwrap()
    );
    Ok(())
}

fn bench_deflate(path: &PathBuf, input_bytes: u64) -> Result<serde_json::Value, String> {
    let file = File::open(path).map_err(|err| format!("open {}: {err}", path.display()))?;
    let sink = CountingSink { bytes: 0 };
    let mut enc = DeflateEncoder::new(sink, Compression::new(6));
    let started = Instant::now();
    copy_limited(file, input_bytes, &mut enc)?;
    let sink = enc
        .finish()
        .map_err(|err| format!("finish deflate: {err}"))?;
    Ok(metrics(
        input_bytes,
        sink.bytes,
        started.elapsed().as_secs_f64(),
    ))
}

fn bench_zstd(path: &PathBuf, input_bytes: u64) -> Result<serde_json::Value, String> {
    let file = File::open(path).map_err(|err| format!("open {}: {err}", path.display()))?;
    let sink = CountingSink { bytes: 0 };
    let mut enc = zstd::stream::write::Encoder::new(sink, ZSTD_COMPRESSION_LEVEL)
        .map_err(|err| format!("create zstd encoder: {err}"))?;
    enc.long_distance_matching(true)
        .map_err(|err| format!("set zstd ldm: {err}"))?;
    enc.window_log(ZSTD_WINDOW_LOG)
        .map_err(|err| format!("set zstd window_log: {err}"))?;
    enc.include_checksum(true)
        .map_err(|err| format!("set zstd checksum: {err}"))?;
    enc.set_pledged_src_size(Some(input_bytes))
        .map_err(|err| format!("set zstd pledged size: {err}"))?;
    let started = Instant::now();
    copy_limited(file, input_bytes, &mut enc)?;
    let sink = enc.finish().map_err(|err| format!("finish zstd: {err}"))?;
    Ok(metrics(
        input_bytes,
        sink.bytes,
        started.elapsed().as_secs_f64(),
    ))
}

fn copy_limited(mut file: File, mut remaining: u64, out: &mut impl Write) -> Result<(), String> {
    let mut buf = vec![0u8; CHUNK];
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        file.read_exact(&mut buf[..want])
            .map_err(|err| format!("read input: {err}"))?;
        out.write_all(&buf[..want])
            .map_err(|err| format!("write encoder: {err}"))?;
        remaining -= want as u64;
    }
    Ok(())
}

fn metrics(input_bytes: u64, compressed_bytes: u64, seconds: f64) -> serde_json::Value {
    let mib = input_bytes as f64 / (1024.0 * 1024.0);
    let mib_per_second = if seconds > 0.0 {
        Some(mib / seconds)
    } else {
        None
    };
    json!({
        "compressed_bytes": compressed_bytes,
        "ratio": input_bytes as f64 / compressed_bytes.max(1) as f64,
        "seconds": seconds,
        "mib_per_second": mib_per_second,
    })
}
