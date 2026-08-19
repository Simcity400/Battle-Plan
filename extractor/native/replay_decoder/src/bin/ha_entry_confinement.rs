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

    rows.push(judge(
        "(c0) clean control",
        "no injury",
        &Expect::Confined(BTreeSet::new()),
        &observe(&pair, &pair.cache_path),
    ));

    let cache = injected_cache(
        &scratch,
        "c1_mid_window",
        &pair,
        &[Injection::RecordPayload(5)],
    );
    rows.push(judge(
        "(c1) damaged delta, mid-window",
        &format!("record for hour {} (position 5)", hour_of(5)),
        &Expect::Confined(confined_window(5)),
        &observe(&pair, &cache),
    ));

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

/// Basis: one process, release build, this machine; the rate is over a 256 MiB
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
