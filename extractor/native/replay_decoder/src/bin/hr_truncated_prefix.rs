//! Lane HR — truncation fault-injection harness for the packaged H4IX/H4CP
//! pair (task #192, residual HA-2).
//!
//! # The question
//!
//! Lane HA closed GY-2: a damaged H4CP RECORD costs its checkpoint window and
//! nothing beyond it. It left one length shape refused whole — a cache whose
//! LENGTH moved — with this argument, at `CacheAudit::finish`:
//!
//! > an insertion moves every later byte out from under the offsets the index
//! > named, and nothing here can tell an insertion from a truncation without
//! > assuming which end moved.
//!
//! That argument is answered rather than overruled. Nothing has to tell an
//! insertion from a truncation, because nothing has to attribute the DAMAGE —
//! only the PROOF. H4IX v4 publishes a digest per cache record, taken over the
//! bytes at the offset the index names. A position whose record closed and
//! hashed to that digest IS that record: a statement about bytes, which bytes
//! that never arrived cannot revoke. So the verified PREFIX serves and
//! everything from the first position that did not close-and-match is
//! unvouched and named.
//!
//! At this lane's base commit the answer was still "the whole artifact", from
//! one comparison — `admit_cache`'s `index.cache_len != cache.cache_len()` —
//! made before a single per-record digest was read.
//!
//! # What this instrument does
//!
//! It shares `entry_damage_fixture.rs` with `ha_entry_confinement`: the same
//! twelve hours at checkpoint interval 4, built by the same real production
//! path (`build_certified_replay_artifacts`), asked of the same real consumer
//! door (`snapshot::CertifiedReplayCache::open_file`, then `reconstruct_json`
//! per hour, then the producer's own `verify_every_hour`). Only the injection
//! differs: instead of flipping a byte, a cell CUTS the packaged H4CP short.
//!
//! The three positions the task names are cells (t1), (t2) and (t3); (t4) is
//! the mid-record control, (t5) the everything-lost boundary, and (t6) and (t7)
//! the two length shapes that must stay whole-artifact refusals.
//!
//! # What is real here and what is not
//!
//! Identical to lane HA's harness, and stated there: real build, real
//! artifacts, real admission, real per-hour reconstruction, real refusal
//! strings; synthetic base stream bytes, because no captured H4CR spool
//! survives on this machine. The clean control cell proves the pair builds and
//! serves all twelve hours before anything is cut.
//!
//! Usage: `hr_truncated_prefix <scratch-dir>`; the matrix goes to stdout.

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

    // (t0) the clean control, uncut: the same pair the injected cells are cut
    // from must serve all twelve hours, or nothing below measures anything.
    rows.push(judge(
        "(t0) clean control",
        "no injury",
        &Expect::Confined(BTreeSet::new()),
        &observe(&pair, &pair.cache_path),
    ));

    // (t1) TRUNCATION MID-WINDOW. The cut lands exactly at the start of
    // position 5's record, in the interior window 4..=7. Positions 0..=4 closed
    // and matched before the bytes ran out, so hours 0..=4 serve; 5..=11 do
    // not, because their records are not in the artifact.
    //
    // Note the shape difference from a damaged RECORD at the same position
    // (lane HA's cell c1, which confines 5,6,7): lost bytes are not confined to
    // a window, because every LATER window's records are missing too. The
    // confinement law is unchanged — each unvouched position costs its own
    // window from itself onward — and the union over a contiguous tail is that
    // tail.
    let cache = truncated_cache(&scratch, "t1_mid_window", &pair, record_start(&pair, 5));
    rows.push(judge(
        "(t1) truncation mid-window",
        &format!("cut at the start of hour {}'s record (position 5)", hour_of(5)),
        &Expect::Confined(confined_tail(5)),
        &observe(&pair, &cache),
    ));

    // (t2) TRUNCATION AT A CHECKPOINT BOUNDARY. Position 4 is a checkpoint, so
    // the cut takes the anchor its whole window re-anchors from as well as the
    // window itself. Hour 3 is the last hour of the previous window and must
    // still serve — its own checkpoint is position 0 and every record it
    // replays through arrived.
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

    // (t3) TRUNCATION IN THE FINAL WINDOW. Eleven of twelve hours survive a cut
    // that takes only the last record — which is the cell the old whole-cache
    // refusal cost the most, and the cheapest possible loss for a recording
    // that was interrupted while being copied.
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

    // (t4) THE CUT LANDS INSIDE A RECORD rather than on its boundary. A record
    // that arrived incomplete is not a record: the confinement must be the
    // same as (t1)'s. This is the cell that fails if `finish` vouches for the
    // record in flight.
    let cache = truncated_cache(&scratch, "t4_inside_record", &pair, inside_record(&pair, 5));
    rows.push(judge(
        "(t4) truncation inside a record",
        &format!("cut one byte into hour {}'s record (position 5)", hour_of(5)),
        &Expect::Confined(confined_tail(5)),
        &observe(&pair, &cache),
    ));

    // (t5) EVERYTHING PAST THE HEADER IS GONE. The header is complete, so the
    // artifact's identity is established and every hour can be refused BY NAME
    // — which is a better answer than a whole-artifact refusal even though no
    // hour serves. Coverage is judged down, never up; naming twelve excluded
    // hours is not the same claim as refusing to say anything.
    let cache = truncated_cache(
        &scratch,
        "t5_header_only",
        &pair,
        snapshot::CHECKPOINT_CACHE_HEADER_LEN,
    );
    rows.push(judge(
        "(t5) only the header survives",
        &format!("cut at {} bytes, the header's length", snapshot::CHECKPOINT_CACHE_HEADER_LEN),
        &Expect::Confined(confined_tail(0)),
        &observe(&pair, &cache),
    ));

    // (t6) SHORTER THAN THE HEADER. Nothing here can say which recording these
    // bytes belong to, so there is no hour to name and no coverage to judge.
    // Whole-artifact refusal, by name.
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

    // (t7) LONGER THAN THE INDEX PUBLISHED. The one length shape that stays a
    // whole refusal: the index accounts for `cache_len` bytes and no more, so
    // extra bytes are described by no row — and an INSERTION anywhere inside
    // the cache produces exactly this shape, with every later record displaced
    // from the offset its digest was taken at. This is the cell that would fail
    // if the narrowing had been "any length disagreement confines".
    let cache = extended_cache(&scratch, "t7_extended", &pair);
    rows.push(judge(
        "(t7) longer than the index published",
        &format!("one byte appended past the published {published_len}"),
        &Expect::WholeArtifactRefusal("belong to no record"),
        &observe(&pair, &cache),
    ));

    print_matrix(&rows, published_len);
}

/// Copy the clean H4CP with one byte appended. Not in the shared fixture
/// because only this harness has a length shape to extend.
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
