//! Lane CC measurement instrument: build the certified binary replay artifacts
//! for an already-captured H4CR stream at a chosen checkpoint interval and
//! report the byte accounting the size levers are argued from.
//!
//! Read-only with respect to its input; every output lands in the caller's own
//! empty spill directory.
//!
//! Usage:
//!   cc_cache_probe <stream.bin> <empty-out-dir> <game-id> <legacy|identity_v1>
//!                  <source-len> <source-sha256> <checkpoint-interval-hours>

use std::path::PathBuf;

/// mimalloc, because the process this instrument stands in for uses mimalloc.
///
/// This instrument TIMES `build_certified_replay_artifacts` end to end, and
/// that work is allocation-bound -- whole-world `serde_json::Value` and
/// `CanonValue` trees, millions of small `String`s per game hour. The Windows
/// system heap serializes badly under that load, so an instrument on the
/// default allocator reports a wall time the production recorder (which
/// installs mimalloc in its own `main`) never pays. Allocator choice cannot
/// change a produced byte, so the artifact digests this instrument reports are
/// unaffected.
#[global_allocator]
static GLOBAL_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 7 {
        eprintln!(
            "usage: cc_cache_probe <stream.bin> <out-dir> <game-id> <legacy|identity_v1> \
             <source-len> <source-sha256> <checkpoint-interval-hours>"
        );
        std::process::exit(2);
    }
    let stream = PathBuf::from(&args[0]);
    let out_dir = PathBuf::from(&args[1]);
    let game_id = args[2].clone();
    let identity_mode = match args[3].as_str() {
        "legacy" => replay_decoder::StreamIdentityMode::Legacy,
        "identity_v1" => replay_decoder::StreamIdentityMode::IdentityV1,
        other => {
            eprintln!("unknown identity mode {other}");
            std::process::exit(2);
        }
    };
    let source_len: u64 = args[4].parse().expect("source-len");
    let source_sha256 = args[5].clone();
    let interval: u32 = args[6].parse().expect("checkpoint-interval-hours");

    let started = std::time::Instant::now();
    match replay_decoder::build_certified_replay_artifacts(
        &stream,
        &out_dir,
        game_id,
        identity_mode,
        source_len,
        &source_sha256,
        interval,
    ) {
        Ok(artifacts) => {
            println!(
                "{{\"checkpoint_interval_hours\":{},\"hour_count\":{},\"checkpoint_count\":{},\
                 \"max_seek_deltas\":{},\"checkpoint_cache_len\":{},\"hour_index_len\":{},\
                 \"legacy_json_bytes_avoided\":{},\"production_render_blob_count\":{},\
                 \"production_render_blob_bytes\":{},\"elapsed_s\":{:.1},\
                 \"checkpoint_cache_path\":{:?},\"hour_index_path\":{:?}}}",
                artifacts.checkpoint_interval_hours,
                artifacts.hour_count,
                artifacts.checkpoint_count,
                artifacts.max_seek_deltas,
                artifacts.checkpoint_cache_len,
                artifacts.hour_index_len,
                artifacts.legacy_json_bytes_avoided,
                artifacts.production_render_blob_count,
                artifacts.production_render_blob_bytes,
                started.elapsed().as_secs_f64(),
                artifacts.checkpoint_cache_path.display().to_string(),
                artifacts.hour_index_path.display().to_string(),
            );
        }
        Err(error) => {
            eprintln!("cc_cache_probe: {error}");
            std::process::exit(1);
        }
    }
}
