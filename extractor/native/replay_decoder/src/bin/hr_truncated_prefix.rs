#![allow(dead_code)]

include!("entry_damage_fixture.rs");

fn main() {
    let scratch = match std::env::args().nth(1) {
        Some(dir) => PathBuf::from(dir),
        None => {
            eprintln!("usage: hr_truncated_prefix <scratch-dir>");
            std::process::exit(2);
        }
    };
    fs::create_dir_all(&scratch).expect("create scratch dir");
    let scratch = fs::canonicalize(&scratch).expect("canonicalize scratch dir");

    let pair = build_clean_pair(&scratch);
    let published_len = clean_cache_len(&pair);
    let mut rows = Vec::new();

    rows.push(judge(
        "(t0) clean control",
        "no injury",
        &Expect::Confined(BTreeSet::new()),
        &observe(&pair, &pair.cache_path),
    ));

    let cache = truncated_cache(&scratch, "t1_mid_window", &pair, record_start(&pair, 5));
    rows.push(judge(
        "(t1) truncation mid-window",
        &format!(
            "cut at the start of hour {}'s record (position 5)",
            hour_of(5)
        ),
        &Expect::Confined(confined_tail(5)),
        &observe(&pair, &cache),
    ));

    let cache = truncated_cache(&scratch, "t2_checkpoint", &pair, record_start(&pair, 4));
    rows.push(judge(
        "(t2) truncation at a checkpoint boundary",
        &format!(
            "cut at the start of hour {}'s record (position 4, a checkpoint)",
            hour_of(4)
        ),
        &Expect::Confined(confined_tail(4)),
        &observe(&pair, &cache),
    ));

    let cache = truncated_cache(&scratch, "t3_final_window", &pair, record_start(&pair, 11));
    rows.push(judge(
        "(t3) truncation in the final window",
        &format!(
            "cut at the start of hour {}'s record (position 11, the last)",
            hour_of(11)
        ),
        &Expect::Confined(confined_tail(11)),
        &observe(&pair, &cache),
    ));

    let cache = truncated_cache(&scratch, "t4_inside_record", &pair, inside_record(&pair, 5));
    rows.push(judge(
        "(t4) truncation inside a record",
        &format!(
            "cut one byte into hour {}'s record (position 5)",
            hour_of(5)
        ),
        &Expect::Confined(confined_tail(5)),
        &observe(&pair, &cache),
    ));

    let cache = truncated_cache(
        &scratch,
        "t5_header_only",
        &pair,
        snapshot::CHECKPOINT_CACHE_HEADER_LEN,
    );
    rows.push(judge(
        "(t5) only the header survives",
        &format!(
            "cut at {} bytes, the header's length",
            snapshot::CHECKPOINT_CACHE_HEADER_LEN
        ),
        &Expect::Confined(confined_tail(0)),
        &observe(&pair, &cache),
    ));

    let cache = truncated_cache(
        &scratch,
        "t6_headless",
        &pair,
        snapshot::CHECKPOINT_CACHE_HEADER_LEN - 1,
    );
    rows.push(judge(
        "(t6) shorter than its own header",
        &format!(
            "cut at {} bytes, one short of the header",
            snapshot::CHECKPOINT_CACHE_HEADER_LEN - 1
        ),
        &Expect::WholeArtifactRefusal("header"),
        &observe(&pair, &cache),
    ));

    let cache = extended_cache(&scratch, "t7_extended", &pair);
    rows.push(judge(
        "(t7) longer than the index published",
        &format!("one byte appended past the published {published_len}"),
        &Expect::WholeArtifactRefusal("belong to no record"),
        &observe(&pair, &cache),
    ));

    print_matrix(&rows, published_len);
}

fn extended_cache(scratch: &Path, tag: &str, pair: &BuiltPair) -> PathBuf {
    let cell_dir = scratch.join(tag);
    if cell_dir.exists() {
        fs::remove_dir_all(&cell_dir).expect("clear cell dir");
    }
    fs::create_dir_all(&cell_dir).expect("create cell dir");
    let cache_path = cell_dir.join("checkpoints.h4cp");
    let mut cache = fs::read(&pair.cache_path).expect("read clean H4CP");
    cache.push(0);
    fs::write(&cache_path, &cache).expect("write extended H4CP");
    cache_path
}

fn print_matrix(rows: &[Row], published_len: u64) {
    let passes = rows.iter().filter(|row| row.verdict == "PASS").count();
    println!("# Lane HR — H4CP truncation prefix-service matrix\n");
    println!("**{passes} PASS / {} FAIL**\n", rows.len() - passes);
    println!(
        "checkpoint interval {CHECKPOINT_INTERVAL}, {HOURS} hours, first hour {}, \
         published cache length {published_len} bytes\n",
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
