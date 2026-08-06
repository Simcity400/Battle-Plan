//! Lane CZ measurement instrument driver. See
//! `replay_decoder::cz_finalize_profile` for what is measured and why.
//!
//! Usage:
//!   cz_finalize_profile <stream.bin> <empty-out-dir> <game-id>
//!                       <legacy|identity_v1> <source-len> <source-sha256>
//!                       <checkpoint-interval-hours> [--sub-terms]
//!                       [--entity-census]

use std::path::PathBuf;

/// mimalloc, because the process this instrument stands in for uses mimalloc.
///
/// `recorder_host`'s own `main` installs it (`recorder_host/src/main.rs`) for
/// exactly the work measured here: the finalize fold is allocation-bound --
/// whole-world `serde_json::Value` and `CanonValue` trees, millions of small
/// `String`s per game hour, built and dropped every hour -- and the Windows
/// system heap serializes badly under that load. An instrument on the default
/// allocator therefore does not measure the production process: it inflates the
/// total, and it inflates the allocation-heavy phases MORE than the rest, which
/// is precisely the comparison this instrument exists to make. Allocator choice
/// cannot change any produced byte, so the artifact digests are unaffected.
#[global_allocator]
static GLOBAL_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let sub_terms = args.iter().any(|a| a == "--sub-terms");
    let entity_census = args.iter().any(|a| a == "--entity-census");
    args.retain(|a| a != "--sub-terms" && a != "--entity-census");
    if args.len() != 7 {
        eprintln!(
            "usage: cz_finalize_profile <stream.bin> <out-dir> <game-id> \
             <legacy|identity_v1> <source-len> <source-sha256> \
             <checkpoint-interval-hours> [--sub-terms] [--entity-census]"
        );
        std::process::exit(2);
    }
    let stream = PathBuf::from(&args[0]);
    let out_dir = PathBuf::from(&args[1]);
    let identity_mode = match args[3].as_str() {
        "legacy" => replay_decoder::StreamIdentityMode::Legacy,
        "identity_v1" => replay_decoder::StreamIdentityMode::IdentityV1,
        other => {
            eprintln!("unknown identity mode {other}");
            std::process::exit(2);
        }
    };
    let source_len: u64 = args[4].parse().expect("source-len");
    let interval: u32 = args[6].parse().expect("checkpoint-interval-hours");

    match replay_decoder::cz_finalize_profile::profile_certified_replay_artifacts(
        &stream,
        &out_dir,
        &args[2],
        identity_mode,
        source_len,
        &args[5],
        interval,
        sub_terms,
        entity_census,
    ) {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("cz_finalize_profile: {error}");
            std::process::exit(1);
        }
    }
}
