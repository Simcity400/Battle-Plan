//! Entry-damage fault-injection harness for the packaged H4IX/H4CP pair.
//!
//! # The question
//!
//! A damaged H4CP ENTRY must cost its CHECKPOINT WINDOW, by name, and never the
//! whole container. The per-hour seal establishes the naming authority and the
//! interior-hole harness makes an injury representable, but both are about hours the recorder never vouched
//! for. This is the other injury: the recorder vouched for every hour, sealed
//! the artifacts, and the packaged bytes rotted afterwards.
//!
//! # What this instrument does
//!
//! It builds ONE clean artifact pair with the real production build
//! (`build_certified_replay_artifacts`, the same function `recorder_host`'s
//! finalize calls), then for each cell copies that pair and flips ONE byte PER
//! INJECTION SITE — post-seal rot, exactly the injury GY-2 names — and asks the
//! real consumer door
//! (`snapshot::CertifiedReplayCache::open_file`, then `reconstruct_json` for
//! every hour) what the recording still serves.
//!
//! Per cell it reports:
//!
//!  * whether the artifact pair was admitted at all;
//!  * which game hours reconstruct (byte-exactly: `reconstruct_json` compares
//!    each hour against the parity digest the fold recorded for it);
//!  * which game hours are refused, and whether the refusal NAMES the injury;
//!  * whether the total proof (`verify_every_hour`, the seal path's own) still
//!    holds, which it must not once any hour is confined.
//!
//! # What is real here and what is not
//!
//! * **Real**: the artifact build, the artifacts, the admission path, the
//!   per-hour reconstruction and every refusal string. The damage is a real
//!   byte flip in a real packaged H4CP on disk.
//! * **Not real**: the base stream BYTES. No captured H4CR spool survives on
//!   this machine. The stream is
//!   synthesized with the record framing the collector writes, over two real
//!   production domains (18 `country_identity`, 13 `tech_live`), and the clean
//!   control cell proves the base pair builds and serves before anything is
//!   injected.
//! * **Not covered here**: the sealed CONTAINER. This harness stops at the
//!   packaged artifact pair; the container-level
//!   join — the cache-only certification chain and the coverage map the replay
//!   authority publishes — is proved by unit tests in
//!   `compact_replay_verification_gates`.
//!
//! Usage: `ha_entry_confinement <scratch-dir>`; the matrix goes to stdout.

// The stream synthesis, production artifact build, injection sites, consumer
// observation, and judging live in `entry_damage_fixture.rs`. The truncation
// harness shares these fixture components so it can
// drive the same fixture through the same door. Nothing in it changed in the
// move, so the matrix this binary prints is the matrix it printed before.
#![allow(dead_code)]

include!("entry_damage_fixture.rs");

fn main() {
    let scratch = match std::env::args().nth(1) {
        Some(dir) => PathBuf::from(dir),
        None => {
            eprintln!("usage: ha_entry_confinement <scratch-dir>");
            std::process::exit(2);
        }
    };
    fs::create_dir_all(&scratch).expect("create scratch dir");
    let scratch = fs::canonicalize(&scratch).expect("canonicalize scratch dir");

    let pair = build_clean_pair(&scratch);
    let mut rows = Vec::new();

    // (c0) the clean control: the same pair, uninjured.
    rows.push(judge(
        "(c0) clean control",
        "no injury",
        &Expect::Confined(BTreeSet::new()),
        &observe(&pair, &pair.cache_path),
    ));

    // (c1) a damaged DELTA in the middle of an interior window. The window's
    // earlier hours must still serve: they never read this record.
    let cache = injected_cache(&scratch, "c1_mid_window", &pair, &[Injection::RecordPayload(5)]);
    rows.push(judge(
        "(c1) damaged delta, mid-window",
        &format!("record for hour {} (position 5)", hour_of(5)),
        &Expect::Confined(confined_window(5)),
        &observe(&pair, &cache),
    ));

    // (c2) the damaged record IS the window's CHECKPOINT, so the whole window
    // goes and nothing outside it does.
    let cache = injected_cache(
        &scratch,
        "c2_checkpoint",
        &pair,
        &[Injection::RecordPayload(4)],
    );
    rows.push(judge(
        "(c2) damaged checkpoint, window boundary",
        &format!("record for hour {} (position 4, a checkpoint)", hour_of(4)),
        &Expect::Confined(confined_window(4)),
        &observe(&pair, &cache),
    ));

    // (c3) the FIRST window, whose checkpoint is the recording's first hour.
    let cache = injected_cache(
        &scratch,
        "c3_first_window",
        &pair,
        &[Injection::RecordPayload(1)],
    );
    rows.push(judge(
        "(c3) damaged delta, first window",
        &format!("record for hour {} (position 1)", hour_of(1)),
        &Expect::Confined(confined_window(1)),
        &observe(&pair, &cache),
    ));

    // (c4) the LAST window, where the confined run reaches the end of the
    // recording and there is no later checkpoint to re-anchor from.
    let cache = injected_cache(
        &scratch,
        "c4_last_window",
        &pair,
        &[Injection::RecordPayload(9)],
    );
    rows.push(judge(
        "(c4) damaged delta, last window",
        &format!("record for hour {} (position 9)", hour_of(9)),
        &Expect::Confined(confined_window(9)),
        &observe(&pair, &cache),
    ));

    // (c5) TWO windows damaged at once: the confinement has to be a union of
    // windows, not one run.
    let cache = injected_cache(
        &scratch,
        "c5_two_windows",
        &pair,
        &[Injection::RecordPayload(2), Injection::RecordPayload(9)],
    );
    let mut both = confined_window(2);
    both.extend(confined_window(9));
    rows.push(judge(
        "(c5) two windows damaged",
        &format!(
            "records for hours {} and {} (positions 2, 9)",
            hour_of(2),
            hour_of(9)
        ),
        &Expect::Confined(both),
        &observe(&pair, &cache),
    ));

    // (c6) the UNCONFINABLE control: damage to the H4CP's own header, which
    // serves no single hour. This must stay a whole-artifact refusal — the
    // confinement must not reach damage it cannot attribute.
    let cache = injected_cache(&scratch, "c6_header", &pair, &[Injection::FileHeader]);
    rows.push(judge(
        "(c6) damaged H4CP header",
        "the artifact's own source-identity field",
        &Expect::WholeArtifactRefusal("header identity is invalid"),
        &observe(&pair, &cache),
    ));

    print_matrix(&rows);
    print_encoder_hash_cost();
}

/// What the per-record digest costs the RECORDER, measured rather than assumed.
///
/// The encoder now hashes every record a second time: once into the running
/// whole-cache digest H4IX still carries, and once into the record's own. The
/// two passes cover the same bytes, so the price is exactly one extra SHA-256
/// sweep of the cache body per finalize — and this repository's SHA-256 is its
/// own implementation (`snapshot::sha256`), not a hardware-accelerated one, so
/// the rate has to be measured here rather than quoted.
///
/// Basis: one process, release build, this machine; the rate is over a 256 MiB
/// resident buffer, and the finalize figure applies it to the 1,532,929,026-byte
/// cache of the retained capture `t48_authoritative` (read back out of its own
/// packaged H4IX by `runs/ha_entry_confinement/h4ix_probe.py`). It is a
/// THROUGHPUT measurement projected onto a real cache size, not a timing of
/// that capture's finalize, which this machine no longer has the source stream
/// to re-run.
fn print_encoder_hash_cost() {
    const BUFFER: usize = 256 * 1024 * 1024;
    const T48_CACHE_BYTES: f64 = 1_532_929_026.0;
    let mut buffer = vec![0u8; BUFFER];
    for (index, byte) in buffer.iter_mut().enumerate() {
        *byte = (index as u64).wrapping_mul(2_654_435_761) as u8;
    }
    let started = std::time::Instant::now();
    let digest = Sha256::hex(&buffer);
    let elapsed = started.elapsed().as_secs_f64();
    let rate = BUFFER as f64 / elapsed / (1024.0 * 1024.0);
    println!("\n## What the per-record digest costs the recorder\n");
    println!(
        "| measurement | value |\n|---|---:|\n\
         | SHA-256 over {} MiB (snapshot::sha256, release, this machine) | {:.2} s |\n\
         | rate | {:.0} MiB/s |\n\
         | ONE extra pass over t48_authoritative's {:.0}-byte cache | {:.2} s per finalize |",
        BUFFER / (1024 * 1024),
        elapsed,
        rate,
        T48_CACHE_BYTES,
        T48_CACHE_BYTES / (rate * 1024.0 * 1024.0),
    );
    println!("\n(digest {digest}, printed so the timed work cannot be optimised away)");
}

fn print_matrix(rows: &[Row]) {
    let passes = rows.iter().filter(|row| row.verdict == "PASS").count();
    println!("# Lane HA — H4CP entry-damage confinement matrix\n");
    println!("**{passes} PASS / {} FAIL**\n", rows.len() - passes);
    println!(
        "checkpoint interval {CHECKPOINT_INTERVAL}, {HOURS} hours, first hour {}\n",
        hour_of(0)
    );
    println!("| cell | injection | verdict | hours served | hours confined |");
    println!("|---|---|---|---|---|");
    for row in rows {
        println!(
            "| {} | {} | {} | {} | {} |",
            row.cell, row.shape, row.verdict, row.served, row.confined
        );
    }
    println!("\n## What each cell's refusal actually says\n");
    for row in rows {
        println!("* **{}** — {}", row.cell, row.detail);
    }
}
