//! What two spools cost on disk when the RECORDER'S OWN WRITER stores them.
//!
//! # Why this exists
//!
//! Lane SG's ship/no-ship decision for a `PSD1` domain turns on a stored-size
//! sign: an enveloped d93 spool is 384,181 bytes SMALLER than the raw one, and
//! an enveloped d55 spool is 766,592 bytes LARGER. Those are 0.93% and 1.85% of
//! a 41 MB artefact, so the sign is the whole finding and it has to be produced
//! by the thing that actually writes captures.
//!
//! `runs/sg_dense_delta/sg_spool_compression.py` reproduces
//! `compact_stream::StreamWriter`'s settings by hand — level 3, `window_log` 27,
//! frame checksums, an encoder flush every 1 MiB, a frame rotated every 256 MiB,
//! flush before rotation — and its libzstd is the same 1.5.7 the workspace pins
//! through `zstd-sys 2.0.16+zstd.1.5.7`. That is a strong argument and it is
//! still an argument: zstd's output is not contractually stable across bindings,
//! so a hand-built pipeline is evidence, not proof (Codex review round 3,
//! finding 1). This tool removes the argument by using `StreamWriter` itself.
//!
//! # What it does
//!
//! For each input spool: decode it (`compact_stream::open` handles a raw or an
//! already-zstd capture), stream every byte through a `StreamWriter` configured
//! exactly as `recorder_host::main` configures the capture sink — `Zstd` with
//! `spool_frame_bytes`' `DEFAULT_FRAME_BYTES` — and report the
//! `StreamWriteReceipt`'s own `decoded_len` and `stored_len`. The scratch file is
//! deleted afterwards; only the counts matter.
//!
//! It reads the two spools and writes nothing but its scratch file and its JSON,
//! so it is safe to point at a sealed capture.
//!
//! Usage:
//!   sg_spool_compress <before.bin> <after.bin> --scratch <dir> [--json out.json]

use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

/// One spool, stored as the recorder would store it.
#[derive(Serialize)]
struct Measured {
    path: String,
    decoded_bytes: u64,
    stored_bytes: u64,
    /// Frame boundaries the writer recorded. `StreamWriter` seeds the vector
    /// with the zero boundary at construction and appends one per 256 MiB
    /// rotation, so this EQUALS the frame count -- the same number the Python
    /// harness reports as `zstd_frames` (Codex review round 4, finding 4).
    frame_boundaries: usize,
}

#[derive(Serialize)]
struct Report {
    schema: &'static str,
    writer: &'static str,
    frame_bytes: u64,
    before: Measured,
    after: Measured,
    decoded_ratio: f64,
    stored_ratio: f64,
    stored_delta_bytes: i128,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("sg_spool_compress: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let (before_path, after_path, scratch, json_out) = parse_args()?;
    fs::create_dir_all(&scratch)
        .map_err(|error| format!("create scratch {}: {error}", scratch.display()))?;

    let before = measure(&before_path, &scratch, "sg_before.zst")?;
    let after = measure(&after_path, &scratch, "sg_after.zst")?;

    let report = Report {
        schema: "sg.spool_compress.rust.v1",
        writer: "compact_stream::StreamWriter (StreamCompression::Zstd)",
        frame_bytes: compact_stream::DEFAULT_FRAME_BYTES,
        decoded_ratio: ratio(before.decoded_bytes, after.decoded_bytes),
        stored_ratio: ratio(before.stored_bytes, after.stored_bytes),
        stored_delta_bytes: i128::from(after.stored_bytes) - i128::from(before.stored_bytes),
        before,
        after,
    };

    println!(
        "decoded {} -> {} ({:.3}x) | stored by compact_stream::StreamWriter {} -> {} ({:.3}x, {:+} B)",
        report.before.decoded_bytes,
        report.after.decoded_bytes,
        report.decoded_ratio,
        report.before.stored_bytes,
        report.after.stored_bytes,
        report.stored_ratio,
        report.stored_delta_bytes,
    );

    if let Some(path) = json_out {
        let text = serde_json::to_string_pretty(&report)
            .map_err(|error| format!("encode report: {error}"))?;
        fs::write(&path, format!("{text}\n"))
            .map_err(|error| format!("write {}: {error}", path.display()))?;
    }
    Ok(())
}

fn ratio(before: u64, after: u64) -> f64 {
    if after == 0 {
        0.0
    } else {
        before as f64 / after as f64
    }
}

fn measure(input: &Path, scratch: &Path, name: &str) -> Result<Measured, String> {
    let mut reader = compact_stream::open(input)
        .map_err(|error| format!("open {}: {error}", input.display()))?;
    let out = scratch.join(name);
    // EXACTLY THE CAPTURE SINK'S CONFIGURATION. `recorder_host::main` builds its
    // spool writer as `StreamWriter::create_with_frames(path, compression,
    // spool_frame_bytes(compression))`, and `spool_frame_bytes` is
    // `Some(DEFAULT_FRAME_BYTES)` for every compression. Everything else --
    // level, window log, checksum, the 1 MiB flush cadence -- lives inside
    // `StreamWriter` and cannot be got wrong here.
    let mut writer = compact_stream::StreamWriter::create_with_frames(
        &out,
        compact_stream::StreamCompression::Zstd,
        Some(compact_stream::DEFAULT_FRAME_BYTES),
    )
    .map_err(|error| format!("create {}: {error}", out.display()))?;

    // The capture loop writes whatever the drain hands it and never flushes, so
    // the chunk size here is free; this is the same 4 MiB the offline harnesses
    // use.
    let mut buffer = vec![0u8; 1 << 22];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {error}", input.display()))?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|error| format!("write {}: {error}", out.display()))?;
    }
    let receipt = writer
        .finish()
        .map_err(|error| format!("finish {}: {error}", out.display()))?;
    // The scratch file has served its purpose the moment the receipt exists.
    let _ = fs::remove_file(&out);

    Ok(Measured {
        path: input.display().to_string(),
        decoded_bytes: receipt.decoded_len,
        stored_bytes: receipt.stored_len,
        frame_boundaries: receipt.frame_boundaries.len(),
    })
}

type Args = (PathBuf, PathBuf, PathBuf, Option<PathBuf>);

fn parse_args() -> Result<Args, String> {
    let mut positional: Vec<PathBuf> = Vec::new();
    let mut scratch: Option<PathBuf> = None;
    let mut json_out: Option<PathBuf> = None;
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--scratch" => {
                scratch = Some(PathBuf::from(
                    args.next().ok_or("--scratch needs a directory")?,
                ));
            }
            "--json" => {
                json_out = Some(PathBuf::from(args.next().ok_or("--json needs a path")?));
            }
            other if other.starts_with("--") => {
                return Err(format!("unexpected argument {other:?}"));
            }
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    if positional.len() != 2 {
        return Err(
            "usage: sg_spool_compress <before.bin> <after.bin> --scratch <dir> [--json out.json]"
                .to_string(),
        );
    }
    let scratch = scratch.ok_or("--scratch is required: this tool writes one temporary spool")?;
    let after = positional.pop().expect("two positionals");
    let before = positional.pop().expect("two positionals");
    Ok((before, after, scratch, json_out))
}
