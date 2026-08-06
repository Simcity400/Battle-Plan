//! Lane CZ measurement instrument driver. See
//! `replay_decoder::cz_finalize_profile` for what is measured and why.
//!
//! Usage:
//!   cz_finalize_profile <stream.bin> <empty-out-dir> <game-id>
//!                       <legacy|identity_v1> <source-len> <source-sha256>
//!                       <checkpoint-interval-hours> [--sub-terms]

use std::path::PathBuf;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let sub_terms = args.iter().any(|a| a == "--sub-terms");
    args.retain(|a| a != "--sub-terms");
    if args.len() != 7 {
        eprintln!(
            "usage: cz_finalize_profile <stream.bin> <out-dir> <game-id> \
             <legacy|identity_v1> <source-len> <source-sha256> \
             <checkpoint-interval-hours> [--sub-terms]"
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
    ) {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("cz_finalize_profile: {error}");
            std::process::exit(1);
        }
    }
}
