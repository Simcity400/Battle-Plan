use portable_replay_format::replay_cache::{
    CacheBuildHour, CacheIdentityMode, CertifiedReplayCache, ReplayCacheEncoder,
};
use serde_json::json;

const TILE_EDGE: u32 = 16;
const VANILLA_WIDTH: u32 = 1_408;
const VANILLA_HEIGHT: u32 = 512;
const TILE_RAW_LEN: usize = 16 * 16 * 4;

fn zero_tile_base64() -> String {
    let mut tile = "A".repeat(TILE_RAW_LEN.div_ceil(3) * 4);
    tile.replace_range(tile.len() - 2.., "==");
    tile
}

fn world(hour: i64, tiles: &[String]) -> CacheBuildHour {
    let date = format!("1936.1.1.{}", hour + 1);
    CacheBuildHour {
        hour_index: hour,
        date: date.clone(),
        h4cr_start_offset: u64::try_from(hour).unwrap() * 100,
        h4cr_end_offset: (u64::try_from(hour).unwrap() + 1) * 100,
        world_json: serde_json::to_vec(&json!({
            "date": date,
            "game_id": "g",
            "hour_index": hour,
            "schema_version": "1",
            "world": {
                "_xmap07_snow_mud_runtime": {
                    "tile_edge": TILE_EDGE,
                    "tiles": tiles,
                }
            }
        }))
        .unwrap(),
    }
}

#[test]
fn one_dirty_snow_mud_tile_emits_only_one_tile_sized_delta() {
    // The portable cache is a lower-level crate and cannot construct the render-stream carrier
    // without reversing the shipping dependency direction. This uses that carrier's exact JSON
    // tile-array shape and drives the cache's public production encoder instead.
    //
    // kills: replacing a fixed-length array structurally with the entire 3.84 MB tile array when
    // one element changes, instead of emitting Patch::Array for the single dirty index.
    let tiles_x = VANILLA_WIDTH.div_ceil(TILE_EDGE);
    let tiles_y = VANILLA_HEIGHT.div_ceil(TILE_EDGE);
    let tile_count = usize::try_from(tiles_x.checked_mul(tiles_y).unwrap()).unwrap();
    let unchanged_tile = zero_tile_base64();
    let before = vec![unchanged_tile.clone(); tile_count];
    let mut after = before.clone();
    after[tile_count / 2].replace_range(0..1, "B");
    assert_eq!(
        before
            .iter()
            .zip(&after)
            .filter(|(left, right)| left != right)
            .count(),
        1
    );

    let source_sha256 = "00".repeat(32);
    let mut encoder = ReplayCacheEncoder::new(Vec::new(), &source_sha256, 744).unwrap();
    encoder.push(world(0, &before)).unwrap();
    encoder.push(world(1, &after)).unwrap();
    let (cache, index) = encoder
        .finish("g", "1", CacheIdentityMode::IdentityV1, 200)
        .unwrap();
    let delta = &index.hours()[1];

    assert_eq!(delta.delta_steps(), 1);
    let certified =
        CertifiedReplayCache::open(&index.to_bytes().unwrap(), cache, 200, &source_sha256).unwrap();
    let (reconstructed, _) = certified.reconstruct_json(1).unwrap();
    let reconstructed: serde_json::Value = serde_json::from_slice(&reconstructed).unwrap();
    assert_eq!(
        reconstructed["world"]["_xmap07_snow_mud_runtime"]["tiles"],
        serde_json::to_value(&after).unwrap(),
        "the delta must reconstruct the exact dirty tile array before its size is credited"
    );
    assert!(
        delta.cache_record_len() < u64::try_from(unchanged_tile.len() + 256).unwrap(),
        "one dirty tile wrote a {}-byte delta for a {}-byte base64 tile",
        delta.cache_record_len(),
        unchanged_tile.len()
    );
}
