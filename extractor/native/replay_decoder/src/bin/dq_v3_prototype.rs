//! Lane DQ H4CP v3 stage-2 prototype driver. See
//! `replay_decoder::dq_v3_prototype` for what is measured and why.
//!
//! Usage:
//!   dq_v3_prototype <stream.bin> <game-id> <legacy|identity_v1> <source-len>
//!                   <grain-depth-spec> <max-hours> [<sections>]
//!
//! The optional seventh argument converts only the named subset of the covered
//! sections. It exists so a tranche's band can be read against the band it grew
//! from IN ONE MACHINE WINDOW: this machine has moved 35-55% between sessions,
//! which is several times the difference a tranche makes, so a comparison
//! against a previous session's JSON measures the machine. Omitting it converts
//! everything, which is what the six-argument invocation has always meant.
//!
//! `<grain-depth-spec>` is a default depth, optionally followed by
//! `,<section>=<depth>` overrides -- for example `3,divisions=1,map=5`. One
//! depth for every section measures a compromise rather than the design: the
//! same depth lands on a 44-byte child in `divisions` and a 594-byte one in
//! `map`.
//!
//! The spec to pass is `dq_v3_prototype::MEASURED_GRAIN_DEPTHS`, which carries
//! the depth every section was censused to be cheapest at. It is a constant
//! rather than a default because a band has to say which depths produced it,
//! and a depth that the driver supplied silently would not appear in the
//! command line the band is filed against. It is also inert for any section a
//! run does not activate, so the same string drives every tranche
//! configuration and no two of them can differ in a depth by accident.
//!
//! `FOLD_PROFILE=1` is required: the adapter-inclusive bound charges v3 the
//! fold's own per-section `build_snapshot` spans, and without the profiler those
//! spans are zero, which would silently turn the upper bound into the lower one.

use std::path::PathBuf;

/// mimalloc, because the process this prototype stands in for uses mimalloc.
///
/// Both lanes here are allocation-bound -- v2 builds and drops whole-section
/// `CanonValue` trees, v3 builds and drops per-grain byte vectors -- and the
/// Windows system heap serializes badly under that load. Measuring on a
/// different allocator would not measure the production process, and it would
/// penalize the two lanes by different amounts, which is precisely the
/// comparison this instrument exists to make.
#[global_allocator]
static GLOBAL_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 6 && args.len() != 7 {
        eprintln!(
            "usage: dq_v3_prototype <stream.bin> <game-id> <legacy|identity_v1> \
             <source-len> <grain-depth> <max-hours> [<section,section,...>]"
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

    match replay_decoder::dq_v3_prototype::run(
        &stream,
        &args[1],
        identity_mode,
        source_len,
        &args[4],
        max_hours,
        args.get(6).map(String::as_str),
    ) {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("dq_v3_prototype: {error}");
            std::process::exit(1);
        }
    }
}
