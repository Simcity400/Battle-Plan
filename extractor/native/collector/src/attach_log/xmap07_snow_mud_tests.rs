use super::*;

#[test]
fn unavailable_passive_chain_returns_a_named_refusal_without_grid_bytes() {
    let memory = map_passive_carriers_live_lane::memory::SliceMemory::new(0x1_4000_0000);
    let mut tracker = CompactXmap07SnowMudTracker::default();

    let read = read_compact_xmap07_snow_mud_from_memory(&memory, 0, 44_000_000, &mut tracker);

    // kills: converting an unavailable anchor into an empty or zero-filled success payload.
    assert!(read.snapshot.is_refusal());
    assert!(read.snapshot.grid.is_empty());
    assert_eq!(read.snapshot.width, 0);
    assert_eq!(read.snapshot.height, 0);
    assert_eq!(
        read.refusal.as_ref().map(|refusal| refusal.reason),
        Some("map_anchor.game_root_unavailable")
    );
    assert_eq!(tracker.epoch_key, None);
    assert_eq!(tracker.generation, 0);
    assert_eq!(tracker.capture_ordinal, 0);
}

#[test]
fn tracker_ordinals_are_one_based_and_anchor_changes_open_a_fresh_generation() {
    let mut tracker = CompactXmap07SnowMudTracker::default();
    let hash_a = [0x11; 32];
    let hash_b = [0x22; 32];

    let first = advance_compact_xmap07_snow_mud_tracker(
        &mut tracker,
        0x2_0003_0000,
        44_000_000,
        hash_a,
    );
    let second = advance_compact_xmap07_snow_mud_tracker(
        &mut tracker,
        0x2_0003_0000,
        44_000_001,
        hash_a,
    );
    let replacement = advance_compact_xmap07_snow_mud_tracker(
        &mut tracker,
        0x2_0004_0000,
        44_000_002,
        hash_b,
    );

    // kills: making ordinals zero-based or carrying an ordinal across an anchor replacement.
    assert_eq!(first, (1, 1, 44_000_000, true));
    assert_eq!(second, (1, 2, 44_000_000, false));
    assert_eq!(replacement, (2, 1, 44_000_002, true));
}
