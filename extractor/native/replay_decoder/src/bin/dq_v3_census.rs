//! Lane EE stage-1 density census. See `replay_decoder::dq_v3_census` for what
//! it measures and why the conversion order has to come out of it.
//!
//! Usage:
//!   dq_v3_census <stream.bin> <game-id> <legacy|identity_v1> <source-len>
//!                <depth-list> <max-hours>
//!
//! `<depth-list>` is a comma-separated list of grain depths, e.g. `1,2,3`. Every
//! depth is censused for every top-level member, because the grain depth that
//! minimizes a section's bytes is an OPTIMUM per section and cannot be read off
//! one depth's numbers.
//!
//! Nothing here is timed, so this binary does not need -- and deliberately does
//! not set -- `FOLD_PROFILE=1`.

use std::path::PathBuf;

/// mimalloc, matching the prototype driver. The census is not a timing
/// instrument, but it allocates in the same pattern the prototype does and the
/// Windows system heap serializes badly under it.
#[global_allocator]
static GLOBAL_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 6 {
        eprintln!(
            "usage: dq_v3_census <stream.bin> <game-id> <legacy|identity_v1> \
             <source-len> <depth-list> <max-hours>"
        );
        std::process::exit(2);
    }
    let stream = PathBuf::from(&args[0]);
    let identity_mode = match args[2].as_str() {
        "legacy" => replay_decoder::StreamIdentityMode::Legacy,
        "identity_v1" => replay_decoder::StreamIdentityMode::IdentityV1,
        other => {
            eprintln!("unknown identity mode {other}");
            std::process::exit(2);
        }
    };
    let source_len: u64 = args[3].parse().expect("source-len");
    let max_hours: u64 = args[5].parse().expect("max-hours");

    match replay_decoder::dq_v3_census::run(
        &stream,
        &args[1],
        identity_mode,
        source_len,
        &args[4],
        max_hours,
    ) {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("dq_v3_census: {error}");
            std::process::exit(1);
        }
    }
}
