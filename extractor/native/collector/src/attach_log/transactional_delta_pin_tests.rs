use super::*;

const D29: u16 = COMPACT_DOMAIN_SHIPS;
const D32: u16 = COMPACT_DOMAIN_AIR_WING_LOADOUTS;
const D42: u16 = COMPACT_DOMAIN_AIR_COMBAT_LOSSES_LIVE;
const UNRELATED: u16 = COMPACT_DOMAIN_WEATHER;

fn ship(instance_id: u32, strength_raw: u32) -> CompactShipRow {
    CompactShipRow {
        instance_id,
        deleted: false,
        task_force_id: Some(201),
        owner_tag_db_id: Some(1),
        definition: Some("destroyer_1".into()),
        category_enum: 1,
        strength_raw,
        organisation_raw: 1_000_000,
        experience_raw: 0,
        sunk_convoys: 0,
    }
}

fn loadout(wing_id: u32, body_addr: usize, amount_raw: i64) -> CompactAirWingLoadoutRow {
    CompactAirWingLoadoutRow {
        owner_tag_db_id: 1,
        wing_id,
        generation: 0,
        body_addr,
        deleted: false,
        air_base_id: 9001,
        equipment: vec![CompactAirWingLoadoutEntry {
            variant_id: 900,
            amount_raw,
        }],
    }
}

fn rejected(domains: &[u16]) -> BTreeSet<u16> {
    domains.iter().copied().collect()
}

fn settle_frame(trackers: &mut CompactPerfGateTrackers, frame_rejected_domains: &BTreeSet<u16>) {
    if !frame_rejected_domains.is_empty() {
        reset_trackers_after_rejected_bracket(trackers, frame_rejected_domains);
    }
    commit_delta_pin_transactions(trackers);
}

fn deleted_ids(rows: &[CompactShipRow]) -> Vec<u32> {
    rows.iter()
        .filter(|row| row.deleted)
        .map(|row| row.instance_id)
        .collect()
}

fn live_ids(rows: &[CompactShipRow]) -> Vec<u32> {
    rows.iter()
        .filter(|row| !row.deleted)
        .map(|row| row.instance_id)
        .collect()
}

#[test]
fn a_ship_destroyed_during_its_own_domains_gap_is_tombstoned_after_it() {
    let mut trackers = CompactPerfGateTrackers::default();

    let (rows, _changed, deleted) =
        compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert_eq!(
        live_ids(&rows),
        vec![1, 2],
        "the keyframe carries both ships"
    );
    assert_eq!(deleted, 0, "nothing is deleted on the first read");
    settle_frame(&mut trackers, &rejected(&[]));

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10)], 0);
    settle_frame(&mut trackers, &rejected(&[D29]));

    let (rows, _changed, deleted) =
        compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10)], 0);
    assert_eq!(
        deleted_ids(&rows),
        vec![2],
        "the first d29 emission after its own gap must tombstone the ship that vanished \
         during the gap; the pre-fix code emitted a whole snapshot with no deletions at all: \
         live {:?}, deleted {:?}",
        live_ids(&rows),
        deleted_ids(&rows),
    );
    assert_eq!(deleted, 1, "the deleted_rows header count must agree");
}

#[test]
fn the_air_wing_unit_is_one_transaction_in_the_source() {
    let source = super::tests::collector_source_lf();
    for call in [
        "trackers.air_wings.commit_pin()",
        "trackers.air_wing_loadouts.commit_pin()",
        "trackers.air_combat_losses_live.commit_pin()",
    ] {
        assert!(
            source.contains(call),
            "`{call}` is missing from `commit_delta_pin_transactions`. All three members of \
             the air-wing unit commit together or none of them may: a member left uncommitted \
             is rolled back by a LATER frame's rejection, past state the fold already applied."
        );
    }
    assert!(
        source.contains(
            "\nconst AIR_WING_TRANSACTIONAL_UNIT: &[u16] = &[\n    \
             COMPACT_DOMAIN_AIR_WINGS,\n    COMPACT_DOMAIN_AIR_WING_LOADOUTS,\n    \
             COMPACT_DOMAIN_AIR_COMBAT_LOSSES_LIVE,\n];\n"
        ),
        "the air-wing unit's membership list changed. It is the set of domains the fold keys \
         on one `AirWingKey`; changing it is a fold-side claim, not a producer-side one."
    );
    for member in [
        "COMPACT_DOMAIN_AIR_WINGS",
        "COMPACT_DOMAIN_AIR_WING_LOADOUTS",
        "COMPACT_DOMAIN_AIR_COMBAT_LOSSES_LIVE",
    ] {
        let one_line =
            format!("if !domain_scoped_tracker_tag_survived({member}, rejected_domains) {{");
        let wrapped = format!(
            "if !domain_scoped_tracker_tag_survived(\n            {member},\n            \
             rejected_domains,\n        ) {{"
        );
        assert!(
            source.contains(&one_line) || source.contains(&wrapped),
            "{member} no longer rolls back on ITS OWN rejection. Every member's pin must be \
             restored exactly when its own records became a `DOMAIN_GAP`, and never because a \
             sibling's did."
        );
    }
    let private_counter_sites = [
        "next_compact_air_wing_generation",
        "next_compact_air_wing_loadout_generation",
        "next_compact_air_strength_generation",
    ];
    for site in private_counter_sites {
        assert!(
            !source.contains(site),
            "`{site}` is back. A per-domain generation counter is what made d30/d32 agree by \
             coincidence; the agreement must come from `CompactAirWingGenerationAuthority`."
        );
    }
}

#[test]
fn a_row_first_seen_on_the_rejected_frame_is_re_emitted_not_suppressed() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10)], 0);
    settle_frame(&mut trackers, &rejected(&[]));

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(3, 30)], 0);
    settle_frame(&mut trackers, &rejected(&[D29]));

    let (rows, ..) =
        compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(3, 30)], 0);
    assert_eq!(
        live_ids(&rows),
        vec![3],
        "ship 3 was only ever staged on a frame the fold discarded, so the next accepted \
         frame must deliver it: {rows:?}"
    );
    assert!(
        deleted_ids(&rows).is_empty(),
        "nothing left the world, so nothing may be tombstoned: {rows:?}"
    );
}

#[test]
fn an_unrelated_domains_rejection_does_not_roll_the_d29_pin_back() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10)], 0);
    settle_frame(&mut trackers, &rejected(&[]));

    let (rows, ..) =
        compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert_eq!(live_ids(&rows), vec![2]);
    settle_frame(&mut trackers, &rejected(&[UNRELATED]));

    let (rows, ..) =
        compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert!(
        rows.is_empty(),
        "d29 survived its own bracket, so its pin still holds both ships and nothing changed: \
         {rows:?}"
    );
}

#[test]
fn a_committed_pin_is_not_rolled_back_by_a_later_frames_rejection() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10)], 0);
    settle_frame(&mut trackers, &rejected(&[]));

    let (rows, ..) =
        compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert_eq!(live_ids(&rows), vec![2]);
    settle_frame(&mut trackers, &rejected(&[]));

    settle_frame(&mut trackers, &rejected(&[D29]));

    let (rows, ..) =
        compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert!(
        rows.is_empty(),
        "frame 2 was committed, so the frame-3 rejection must not roll the pin back to \
         frame 1's state and re-send ship 2: {rows:?}"
    );
}

#[test]
fn a_rejected_first_frame_rolls_the_pin_back_to_empty() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    settle_frame(&mut trackers, &rejected(&[D29]));
    assert!(
        trackers.ships.previous.is_empty(),
        "nothing was ever committed, so the rollback target is the empty pin"
    );

    let (rows, _changed, deleted) =
        compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert_eq!(live_ids(&rows), vec![1, 2], "the next frame is a keyframe");
    assert_eq!(deleted, 0, "a keyframe carries no tombstones");
}

#[test]
fn two_advances_in_one_frame_roll_back_past_both() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10)], 0);
    settle_frame(&mut trackers, &rejected(&[]));

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    let _ = compact_ship_delta_rows(
        &mut trackers.ships,
        vec![ship(1, 10), ship(2, 20), ship(3, 30)],
        0,
    );
    settle_frame(&mut trackers, &rejected(&[D29]));

    let (rows, ..) = compact_ship_delta_rows(
        &mut trackers.ships,
        vec![ship(1, 10), ship(2, 20), ship(3, 30)],
        0,
    );
    assert_eq!(
        live_ids(&rows),
        vec![2, 3],
        "the rollback target is the last COMMITTED pin, so both ships staged inside the \
         rejected frame must be delivered: {rows:?}"
    );
}

#[test]
fn the_frame_builder_commits_the_delta_pin_transactions_after_the_reset() {
    let source = super::tests::collector_source_lf();
    let reset_call =
        "\n        reset_trackers_after_rejected_bracket(trackers, &rejected_domains);\n    }\n";
    let commit_call = "\n    commit_delta_pin_transactions(trackers);\n";

    let reset_at = source
        .find(reset_call)
        .expect("the frame builder still closes the `if rejected_any` block with the reset");
    let commit_at = source
        .find(commit_call)
        .expect("the frame builder must call `commit_delta_pin_transactions` unconditionally");
    assert_eq!(
        source.matches(commit_call).count(),
        1,
        "exactly one unconditional commit call, or a frame commits twice"
    );
    assert!(
        commit_at > reset_at,
        "the commit must run AFTER the rejected-bracket reset, or a rolled-back pin is \
         re-committed from the value the reset just discarded"
    );
    let between = &source[reset_at + reset_call.len()..commit_at];
    assert!(
        !between.contains("if ") && !between.contains("return"),
        "the commit is no longer unconditionally reached from the reset: {between:?}"
    );
}

#[test]
fn a_withheld_frame_that_is_also_rejected_keeps_the_committed_pin() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    settle_frame(&mut trackers, &rejected(&[]));

    let (rows, ..) = compact_ship_delta_rows(
        &mut trackers.ships,
        vec![ship(1, 10)],
        COMPACT_STATUS_PARTIAL_DATA,
    );
    assert!(rows.is_empty(), "a withheld frame emits nothing");
    settle_frame(&mut trackers, &rejected(&[D29]));

    let (rows, ..) =
        compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert!(
        rows.is_empty(),
        "the withheld frame never advanced the pin, so the committed pin still holds both \
         ships and nothing changed: {rows:?}"
    );
}

const WING_A: usize = HEAP_LO + 0x100;
const WING_B: usize = HEAP_LO + 0x200;

fn wing(wing_id: u32, body_addr: usize, count: u32) -> CompactAirWingRow {
    CompactAirWingRow {
        owner_tag_db_id: 1,
        wing_id,
        generation: 0,
        body_addr,
        deleted: false,
        count,
        mission_mask: 1,
        mission_region: Some(88),
        effective_planes: count,
    }
}

fn strength(wing_id: u32, body_addr: usize, count: u32) -> CompactAirStrengthSample {
    CompactAirStrengthSample {
        owner_tag_db_id: 1,
        wing_id,
        generation: 0,
        body_addr,
        count,
        mission_mask: 1,
        mission_region: Some(88),
        equipment_amount_sum: (count as i64) * COMPACT_AIR_STRENGTH_EQUIPMENT_AMOUNT_PER_PLANE,
        air_accidents: 0,
        experience_raw: 0,
    }
}

fn d30_keys(rows: &[CompactAirWingRow]) -> Vec<(u32, u32, u64)> {
    rows.iter()
        .map(|row| (row.owner_tag_db_id, row.wing_id, row.generation))
        .collect()
}

fn d32_keys(rows: &[CompactAirWingLoadoutRow]) -> Vec<(u32, u32, u64)> {
    rows.iter()
        .map(|row| (row.owner_tag_db_id, row.wing_id, row.generation))
        .collect()
}

fn air_unit_frame(
    trackers: &mut CompactPerfGateTrackers,
    wings: Vec<CompactAirWingRow>,
    loadouts: Vec<CompactAirWingLoadoutRow>,
    strengths: Vec<CompactAirStrengthSample>,
) -> (
    Vec<CompactAirWingRow>,
    Vec<CompactAirWingLoadoutRow>,
    Vec<CompactAirStrengthDeltaRow>,
) {
    let generations = &trackers.air_wing_generations;
    let (d30_rows, ..) =
        compact_air_wing_delta_rows(&mut trackers.air_wings, generations, wings, 0);
    let (d32_rows, ..) = compact_air_wing_loadout_delta_rows(
        &mut trackers.air_wing_loadouts,
        generations,
        loadouts,
        0,
    );
    let (d42_rows, ..) = compact_air_strength_delta_rows(
        &mut trackers.air_combat_losses_live,
        generations,
        strengths,
        0,
    );
    (d30_rows, d32_rows, d42_rows)
}

#[test]
fn d32_tombstones_a_loadout_that_vanished_across_its_own_gap() {
    let mut trackers = CompactPerfGateTrackers::default();

    let (_, d32_rows, _) = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    assert_eq!(d32_keys(&d32_rows).len(), 2);
    settle_frame(&mut trackers, &rejected(&[]));

    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    settle_frame(
        &mut trackers,
        &rejected(&[COMPACT_DOMAIN_AIR_WINGS, D32, D42]),
    );

    let (d30_rows, d32_rows, _) = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10)],
        vec![loadout(700, WING_A, 100_000)],
        vec![strength(700, WING_A, 10)],
    );

    let d32_deleted: Vec<u32> = d32_rows
        .iter()
        .filter(|row| row.deleted)
        .map(|row| row.wing_id)
        .collect();
    assert_eq!(
        d32_deleted,
        vec![701],
        "the first d32 emission after its own gap must tombstone the loadout that vanished \
         during the gap; the pre-fix code emitted a whole snapshot with no deletions at all: \
         {d32_rows:?}"
    );
    let d30_deleted: Vec<u32> = d30_rows
        .iter()
        .filter(|row| row.deleted)
        .map(|row| row.wing_id)
        .collect();
    assert_eq!(
        d30_deleted,
        vec![701],
        "and d30 must tombstone the same wing, or the fold's orphan prune is the only thing \
         removing the loadout row: {d30_rows:?}"
    );
}

#[test]
fn an_air_wing_member_that_published_keeps_its_pin() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(701, WING_B, 20)],
        vec![loadout(701, WING_B, 50_000)],
        vec![strength(701, WING_B, 20)],
    );
    settle_frame(&mut trackers, &rejected(&[]));

    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(701, WING_B, 30)],
        vec![loadout(701, WING_B, 60_000)],
        vec![strength(701, WING_B, 30)],
    );
    settle_frame(&mut trackers, &rejected(&[D42]));

    let (d30_rows, d32_rows, _) = air_unit_frame(
        &mut trackers,
        vec![wing(701, WING_B, 20)],
        vec![loadout(701, WING_B, 50_000)],
        vec![strength(701, WING_B, 20)],
    );

    assert_eq!(
        d30_rows.iter().map(|row| row.wing_id).collect::<Vec<_>>(),
        vec![701],
        "d30 published 30 and the fold applied it, so its pin must hold 30 and the reversion \
         to 20 must be emitted; a pin rewound by d42's rejection would call it unchanged and \
         the fold would keep 30 forever: {d30_rows:?}"
    );
    assert_eq!(
        d32_rows.iter().map(|row| row.wing_id).collect::<Vec<_>>(),
        vec![701],
        "d32 published this value and the fold applied it, so d32's pin must not be rewound by \
         d42's rejection; a rewound pin calls the reversion unchanged and the fold keeps 20 \
         forever: {d32_rows:?}"
    );
}

#[test]
fn the_rejected_air_wing_member_rolls_its_own_pin_back() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10)],
        vec![loadout(700, WING_A, 100_000)],
        vec![strength(700, WING_A, 10)],
    );
    settle_frame(&mut trackers, &rejected(&[]));

    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    settle_frame(&mut trackers, &rejected(&[D42]));

    let (_, _, d42_rows) = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    assert_eq!(
        d42_rows.iter().map(|row| row.wing_id).collect::<Vec<_>>(),
        vec![701],
        "wing B was only ever staged on a frame d42 never delivered, so d42's next accepted \
         frame owes the fold its baseline row: {d42_rows:?}"
    );
}

#[test]
fn d30_and_d32_publish_identical_keys_across_a_shared_rejection() {
    let mut trackers = CompactPerfGateTrackers::default();

    let (d30_rows, d32_rows, _) = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10)],
        vec![loadout(700, WING_A, 100_000)],
        vec![strength(700, WING_A, 10)],
    );
    assert_eq!(
        d30_keys(&d30_rows),
        d32_keys(&d32_rows),
        "the two domains must agree from the start, or nothing below means anything"
    );
    settle_frame(&mut trackers, &rejected(&[]));

    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    settle_frame(
        &mut trackers,
        &rejected(&[COMPACT_DOMAIN_AIR_WINGS, D32, D42]),
    );

    let (d30_rows, d32_rows, _) = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );

    let after_d30 = d30_keys(&d30_rows);
    let after_d32 = d32_keys(&d32_rows);
    assert_eq!(
        after_d30.len(),
        1,
        "only wing B moved across the gap, so only wing B is re-published: {d30_rows:?}"
    );
    assert_eq!(
        after_d30, after_d32,
        "d30 and d32 disagree on the post-gap generations, so every d32 loadout would be \
         deleted by `prune_air_wing_loadout_orphans_for_current_hour`: d30 {after_d30:?} vs \
         d32 {after_d32:?}"
    );
}

#[test]
fn a_wing_keeps_its_generation_across_a_rejected_frame() {
    let mut trackers = CompactPerfGateTrackers::default();

    let (d30_rows, ..) = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    let mut before = d30_keys(&d30_rows);
    before.sort_unstable();
    settle_frame(&mut trackers, &rejected(&[]));

    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(701, WING_B, 21), wing(700, WING_A, 11)],
        vec![loadout(701, WING_B, 50_001), loadout(700, WING_A, 100_001)],
        vec![strength(701, WING_B, 21), strength(700, WING_A, 11)],
    );
    settle_frame(
        &mut trackers,
        &rejected(&[COMPACT_DOMAIN_AIR_WINGS, D32, D42]),
    );

    let (d30_rows, ..) = air_unit_frame(
        &mut trackers,
        vec![wing(701, WING_B, 21), wing(700, WING_A, 11)],
        vec![loadout(701, WING_B, 50_001), loadout(700, WING_A, 100_001)],
        vec![strength(701, WING_B, 21), strength(700, WING_A, 11)],
    );
    let mut after = d30_keys(&d30_rows);
    after.sort_unstable();
    assert_eq!(
        after, before,
        "a wing that survived the gap was handed a NEW generation, so the fold keeps its old \
         row forever: before {before:?} after {after:?}"
    );
}

#[test]
fn the_generation_authority_is_not_rolled_back_with_the_pins() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10)],
        vec![loadout(700, WING_A, 100_000)],
        vec![strength(700, WING_A, 10)],
    );
    settle_frame(&mut trackers, &rejected(&[]));
    assert_eq!(trackers.air_wing_generations.assigned_count(), 1);

    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    settle_frame(
        &mut trackers,
        &rejected(&[COMPACT_DOMAIN_AIR_WINGS, D32, D42]),
    );

    assert_eq!(
        trackers.air_wing_generations.assigned_count(),
        2,
        "wing B's assignment survives the rejection: the number is bound to the BODY, not to \
         the frame that first saw it"
    );
    assert!(
        trackers.air_wing_generations.next_generation() >= 2,
        "and the counter does not rewind, so no later body can be handed a number an earlier \
         one already used"
    );
}

const D31: u16 = COMPACT_DOMAIN_AIR_REGION_ACTIVITY;

fn d31_bucket(region_id: u32, plane_count: u32) -> CompactAirRegionActivityRow {
    CompactAirRegionActivityRow {
        region_id,
        owner_tag_db_id: 1,
        mission_mask: 7,
        deleted: false,
        wing_count: 1,
        plane_count,
        effective_planes: plane_count,
    }
}

#[test]
fn d31_asks_for_a_resync_after_its_own_bracket_was_rejected() {
    let mut trackers = CompactPerfGateTrackers::default();

    let (rows, ..) = compact_air_region_activity_delta_rows(
        &mut trackers.air_region_activity,
        vec![d31_bucket(88, 100), d31_bucket(99, 50)],
        0,
    );
    assert_eq!(rows.len(), 2);
    commit_delta_pin_transactions(&mut trackers);

    let rejected: BTreeSet<u16> = [D31].into_iter().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    commit_delta_pin_transactions(&mut trackers);
    assert!(
        trackers.air_region_activity.baseline_published,
        "frame 1 published, so the fold holds a d31 baseline"
    );
    assert!(
        trackers.air_region_activity.resync_owed,
        "the wipe destroyed a pin the fold has a baseline for; that is what owes a resync"
    );

    let status = d31_status_after_read(&mut trackers, vec![d31_bucket(88, 140)]);
    assert_ne!(
        status & compact_contract::COMPACT_STATUS_BASELINE_CLEAR,
        0,
        "the first record after d31's own gap must ask the fold to REPLACE, or the bucket \
         that emptied during the gap is merged back in and never removed: status {status:#x}"
    );
    assert_eq!(
        status & COMPACT_STATUS_DELTA_DATA,
        0,
        "a resync is a whole payload; `BASELINE_CLEAR | DELTA_DATA` is refused by the contract"
    );
    assert!(
        !trackers.air_region_activity.resync_owed,
        "the debt is discharged when the bit reaches the wire"
    );

    let status = d31_status_after_read(&mut trackers, vec![d31_bucket(88, 141)]);
    assert_eq!(
        status & compact_contract::COMPACT_STATUS_BASELINE_CLEAR,
        0,
        "the resync is one record, not a mode: {status:#x}"
    );
}

#[test]
fn a_first_d31_record_is_a_keyframe_and_not_a_resync() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = compact_air_region_activity_delta_rows(
        &mut trackers.air_region_activity,
        vec![d31_bucket(88, 100)],
        0,
    );
    assert!(
        !trackers.air_region_activity.previous.is_empty(),
        "the read advanced the pin, or this fixture proves nothing"
    );
    let rejected: BTreeSet<u16> = [D31].into_iter().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    commit_delta_pin_transactions(&mut trackers);
    assert!(
        !trackers.air_region_activity.baseline_published,
        "a rejected record reached nothing, so it published nothing"
    );
    assert!(!trackers.air_region_activity.resync_owed);

    let status = d31_status_after_read(&mut trackers, vec![d31_bucket(88, 100)]);
    assert_eq!(
        status & compact_contract::COMPACT_STATUS_BASELINE_CLEAR,
        0,
        "a stream whose only d31 read was REJECTED has published nothing, so there is no fold \
         baseline to clear: status {status:#x}"
    );
}

#[test]
fn an_empty_post_gap_d31_payload_still_asks_for_the_resync() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = d31_status_after_read(&mut trackers, vec![d31_bucket(88, 100)]);
    assert!(trackers.air_region_activity.baseline_published);

    let rejected: BTreeSet<u16> = [D31].into_iter().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    commit_delta_pin_transactions(&mut trackers);
    assert!(trackers.air_region_activity.resync_owed);

    let status = d31_status_after_read(&mut trackers, Vec::new());
    assert_ne!(
        status & compact_contract::COMPACT_STATUS_BASELINE_CLEAR,
        0,
        "an empty post-gap walk is a complete claim that every bucket is gone, and it is \
         exactly the payload the fold would otherwise merge into nothing: status {status:#x}"
    );
}

#[test]
fn an_unrelated_rejection_does_not_make_d31_owe_a_resync() {
    let mut trackers = CompactPerfGateTrackers::default();
    let _ = compact_air_region_activity_delta_rows(
        &mut trackers.air_region_activity,
        vec![d31_bucket(88, 100)],
        0,
    );
    commit_delta_pin_transactions(&mut trackers);

    let rejected: BTreeSet<u16> = [COMPACT_DOMAIN_WEATHER].into_iter().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    commit_delta_pin_transactions(&mut trackers);

    assert!(
        !trackers.air_region_activity.resync_owed,
        "d31 survived its own bracket, so its pin still agrees with the fold's"
    );
    assert!(
        !trackers.air_region_activity.previous.is_empty(),
        "and the allowlist retained it, or this fixture proves nothing"
    );
}

fn d31_status_after_read(
    trackers: &mut CompactPerfGateTrackers,
    rows: Vec<CompactAirRegionActivityRow>,
) -> u32 {
    let mut status_flags = 0u32;
    let tracker = &mut trackers.air_region_activity;
    let delta_frame = !tracker.previous.is_empty();
    let _ = compact_air_region_activity_delta_rows(tracker, rows, status_flags);
    if delta_frame {
        status_flags |= COMPACT_STATUS_DELTA_DATA;
    } else if tracker.resync_owed && !compact_contract::compact_status_is_failure(status_flags) {
        status_flags |= compact_contract::COMPACT_STATUS_BASELINE_CLEAR;
        tracker.resync_owed = false;
    }
    commit_delta_pin_transactions(trackers);
    status_flags
}

#[test]
fn the_d31_resync_decision_has_exactly_one_production_spelling() {
    let source = super::tests::collector_source_lf();
    let condition = "    } else if tracker.resync_owed && \
                     !compact_contract::compact_status_is_failure(status_flags) {\n";
    assert_eq!(
        source.matches(condition).count(),
        1,
        "the d31 resync condition is not where this module's helper copied it from. Update \
         `d31_status_after_read` in the same edit, or the gates above stop describing \
         production."
    );
    assert_eq!(
        source
            .matches("status_flags |= compact_contract::COMPACT_STATUS_BASELINE_CLEAR;")
            .count(),
        1,
        "`BASELINE_CLEAR` is set at exactly one producer site. A second site is a second \
         policy, and the bit's rule 2 -- never on a routine keyframe -- is only checkable \
         where there is one place to check."
    );
}
