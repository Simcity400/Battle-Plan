//! Lane OD: the domain-100 size lever, MEASURED on the production cache encoder.
//!
//! `xmap07_tile_delta_size.rs` pins the ONE-dirty-tile property. This file prices the whole curve
//! and, critically, pins the property at MORE THAN ONE dirty tile as well: an encoder that kept
//! array patching for a single changed element but fell back to replacing the whole array for two
//! or more would satisfy the one-tile test while destroying every result below it.
//!
//! Run it for the numbers:
//!   cargo test --test xmap07_tile_delta_measure -- --nocapture
//!
//! WHAT THIS DOES AND DOES NOT ESTABLISH. The encoder, the grid dimensions and the record sizes
//! are real. The DIRTY-TILE RATE is a free parameter here, because it is a property of the
//! engine's snow/mud simulation and no recording on this machine schedules domain 100 (all 13
//! sealed captures were scanned for `_xmap07_snow_mud_runtime`; none carries it). So this prices
//! the whole curve and leaves the operating point to the first capture that schedules the domain.
//! The curve is what makes that safe: break-even is at ~99% of tiles dirty, so the change cannot
//! be a meaningful regression at any operating point.

use portable_replay_format::replay_cache::{CacheBuildHour, CacheIdentityMode, ReplayCacheEncoder};
use serde_json::json;

const TILE_EDGE: u32 = 16;
const VANILLA_WIDTH: u32 = 1_408;
const VANILLA_HEIGHT: u32 = 512;
const GRID_RAW_LEN: usize = (VANILLA_WIDTH as usize) * (VANILLA_HEIGHT as usize) * 4;
const TILE_RAW_LEN: usize = (TILE_EDGE as usize) * (TILE_EDGE as usize) * 4;

/// A base64 string of exactly the length real base64 of `raw_len` bytes has. Only the LENGTH, and
/// the fact that one character differs, matter to the cache, which sees an opaque JSON string.
fn base64_shaped(raw_len: usize, seed: u8) -> String {
    let mut text = String::from_utf8(vec![b'A' + seed % 26; raw_len.div_ceil(3) * 4]).unwrap();
    let pad = match raw_len % 3 {
        0 => 0,
        1 => 2,
        _ => 1,
    };
    if pad > 0 {
        text.replace_range(text.len() - pad.., &"=".repeat(pad));
    }
    text
}

fn hour(index: i64, runtime: serde_json::Value) -> CacheBuildHour {
    let date = format!("1936.1.1.{}", index + 1);
    CacheBuildHour {
        hour_index: index,
        date: date.clone(),
        h4cr_start_offset: u64::try_from(index).unwrap() * 100,
        h4cr_end_offset: (u64::try_from(index).unwrap() + 1) * 100,
        world_json: serde_json::to_vec(&json!({
            "date": date,
            "game_id": "g",
            "hour_index": index,
            "schema_version": "1",
            "world": { "_xmap07_snow_mud_runtime": runtime }
        }))
        .unwrap(),
    }
}

/// Encode exactly two hours and return the SECOND hour's delta record length -- the per-game-hour
/// cost of whatever changed between them.
fn delta_bytes(before: serde_json::Value, after: serde_json::Value) -> u64 {
    let source_sha256 = "00".repeat(32);
    let mut encoder = ReplayCacheEncoder::new(Vec::new(), &source_sha256, 744).unwrap();
    encoder.push(hour(0, before)).unwrap();
    encoder.push(hour(1, after)).unwrap();
    let (_cache, index) = encoder
        .finish("g", "1", CacheIdentityMode::IdentityV1, 200)
        .unwrap();
    index.hours()[1].cache_record_len()
}

fn tiles(count: usize, dirty: usize) -> Vec<String> {
    let clean = base64_shaped(TILE_RAW_LEN, 0);
    let mut out = vec![clean; count];
    // Spread the dirty tiles evenly so the index gaps are realistic rather than one dense run.
    for slot in 0..dirty {
        let index = if dirty == 1 {
            count / 2
        } else {
            slot * count / dirty
        };
        out[index] = base64_shaped(TILE_RAW_LEN, 1);
    }
    out
}

fn tiled_delta(tile_count: usize, dirty: usize) -> u64 {
    delta_bytes(
        json!({ "tile_edge": TILE_EDGE, "tiles": tiles(tile_count, 0) }),
        json!({ "tile_edge": TILE_EDGE, "tiles": tiles(tile_count, dirty) }),
    )
}

#[test]
fn domain_100_delta_cost_curve_on_the_production_encoder() {
    let tiles_x = VANILLA_WIDTH.div_ceil(TILE_EDGE);
    let tiles_y = VANILLA_HEIGHT.div_ceil(TILE_EDGE);
    let tile_count = usize::try_from(tiles_x * tiles_y).unwrap();
    let tile_b64 = base64_shaped(TILE_RAW_LEN, 0).len() as u64;

    // BEFORE: master's shape. One base64 string for the whole grid; one changed cell rewrites it.
    let before_clean = base64_shaped(GRID_RAW_LEN, 0);
    let mut before_dirty = before_clean.clone();
    before_dirty.replace_range(0..1, "B");
    let whole_string_delta = delta_bytes(
        json!({ "bytes": before_clean }),
        json!({ "bytes": before_dirty }),
    );
    let unchanged_delta = delta_bytes(
        json!({ "bytes": base64_shaped(GRID_RAW_LEN, 0) }),
        json!({ "bytes": base64_shaped(GRID_RAW_LEN, 0) }),
    );

    println!("\n=== domain 100 (xmap07_snow_mud_live) delta cost, production H4CP encoder ===");
    println!("grid {VANILLA_WIDTH}x{VANILLA_HEIGHT}x4 = {GRID_RAW_LEN} raw bytes");
    println!("tile edge {TILE_EDGE} -> {tiles_x}x{tiles_y} = {tile_count} tiles\n");
    println!("BEFORE  whole-grid string, grid unchanged  : {unchanged_delta:>10} bytes/hour");
    println!("BEFORE  whole-grid string, ANY cell changed: {whole_string_delta:>10} bytes/hour\n");
    println!("AFTER   tiled array");
    println!("{:>14}  {:>12}  {:>10}", "dirty", "bytes/hour", "vs BEFORE");

    let mut measured = Vec::new();
    for dirty in [0usize, 1, 2, 8, 28, 141, 704, 1408, 2816] {
        let after = tiled_delta(tile_count, dirty);
        measured.push((dirty, after));
        let share = after as f64 / whole_string_delta as f64 * 100.0;
        let pct = dirty as f64 / tile_count as f64 * 100.0;
        println!("{dirty:>7} ({pct:>5.1}%)  {after:>12}  {share:>9.2}%");
    }
    println!();

    // kills: reverting the carrier to a single whole-grid base64 string.
    let one = measured.iter().find(|(d, _)| *d == 1).unwrap().1;
    assert!(
        one * 100 < whole_string_delta,
        "one dirty tile cost {one} bytes against a whole-grid replace of {whole_string_delta}"
    );

    // kills: an encoder that keeps Patch::Array for a single changed element but falls back to a
    // whole-array Replace as soon as two or more elements change -- which would pass the one-tile
    // gate while making every row below it wrong. Each of these must stay PROPORTIONAL to its
    // dirty count, so they are bounded against the per-tile cost rather than against the grid.
    for &(dirty, cost) in &measured {
        if dirty == 0 {
            continue;
        }
        // Per dirty tile the grammar adds its replace tag, the string tag, the string's length
        // varint and the index-gap varint; eight bytes is a comfortable bound on that framing.
        // The slack is irrelevant to what this kills: a whole-array replace at two dirty tiles
        // would cost ~3.87 MB against a ~6.8 KB ceiling.
        let ceiling = (tile_b64 + 8) * dirty as u64 + 4_096;
        assert!(
            cost <= ceiling,
            "{dirty} dirty tiles cost {cost} bytes, above the {ceiling}-byte proportional \
             ceiling; the array patch is no longer emitting per-element edits"
        );
    }
}
