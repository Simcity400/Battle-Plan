//! Lane FR measurement instrument driver. See `replay_decoder::fr_mem_profile`
//! for what is measured and why.
//!
//! Usage:
//!   fr_mem_profile <stream.bin> <empty-spill-dir> <report-dir> <game-id>
//!                  <legacy|identity_v1> <source-len> <source-sha256>
//!                  <checkpoint-interval-hours>

use std::path::PathBuf;

/// mimalloc, wrapped in the allocation-tracking shim.
///
/// mimalloc because the production recorder installs mimalloc, and the fold
/// this instrument stands in for is allocation-bound: measuring the Windows
/// system heap would measure a process that does not exist. The tracking
/// wrapper because the whole point of this instrument is to reconcile the named
/// component table against the live Rust-allocator heap, and only the allocator knows the
/// latter. Neither choice can change a produced byte, so the artifact digests
/// are unaffected -- but the WALL TIME is affected, and this instrument says so
/// in its own report rather than inviting the comparison.
#[global_allocator]
static GLOBAL_ALLOC: recorder_host::mem_account::TrackingAlloc<mimalloc::MiMalloc> =
    recorder_host::mem_account::TrackingAlloc(mimalloc::MiMalloc);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 8 {
        eprintln!(
            "usage: fr_mem_profile <stream.bin> <empty-spill-dir> <report-dir> <game-id> \
             <legacy|identity_v1> <source-len> <source-sha256> <checkpoint-interval-hours>"
        );
        std::process::exit(2);
    }
    let stream = PathBuf::from(&args[0]);
    let spill_dir = PathBuf::from(&args[1]);
    let report_dir = PathBuf::from(&args[2]);
    let identity_mode = match args[4].as_str() {
        "legacy" => replay_decoder::StreamIdentityMode::Legacy,
        "identity_v1" => replay_decoder::StreamIdentityMode::IdentityV1,
        other => {
            eprintln!("unknown identity mode {other}");
            std::process::exit(2);
        }
    };
    let source_len: u64 = args[5].parse().expect("source-len");
    let interval: u32 = args[7].parse().expect("checkpoint-interval-hours");

    match replay_decoder::fr_mem_profile::profile_finalize_memory(
        &stream,
        &spill_dir,
        &report_dir,
        &args[3],
        identity_mode,
        source_len,
        &args[6],
        interval,
    ) {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("fr_mem_profile: {error}");
            std::process::exit(1);
        }
    }
}
