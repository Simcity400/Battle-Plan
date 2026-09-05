//! Lane DTAIL: the four producers that published a failing status word with no
//! outcome behind it, and d67's post-walk fence.
//!
//! Measured on `captures/e2e_03` (120 s at speed 5, 1169 game hours, master
//! ecf9805c recorder): d44 x10, d54 x4 and d32 x1 reached the raw-capture
//! report as *"published a failing PERF status ... and its producer recorded no
//! outcome of its own"*, and d96 x2 under the reserved
//! `producer_declared_no_predicate` code with no sentence at all.

use super::super::*;

/// Law: d32's total IS the sum of its four branch counters.
///
/// The walk's single `invalid_air_wing_rows` total was what set `PARTIAL_DATA`,
/// and it was dropped before anything could publish it. Deriving the total
/// from the parts is what makes "a branch refused and was not counted"
/// unrepresentable: a new branch that forgets to pick a counter cannot move the
/// total either, so the frame stays clean and loud rather than partial and
/// mute.
///
/// What production change makes this fail: `invalid_air_wing_rows` becoming a
/// separately maintained field again.
#[test]
fn d32_total_rejections_are_the_sum_of_the_named_branches() {
    let diag = Domain32Diag {
        invalid_container_rows: 3,
        invalid_pool_rows: 5,
        invalid_object_rows: 7,
        extent_rewitness_failures: 11,
        ..Domain32Diag::default()
    };
    assert_eq!(diag.invalid_air_wing_rows(), 26);
    assert_eq!(Domain32Diag::default().invalid_air_wing_rows(), 0);
}

/// Law: the first NAMED rejection wins and keeps its ordinal; a branch with no
/// name leaves the locator empty rather than claiming ordinal 0.
///
/// Every named call site used to pass ordinal `0`, so the locator named no
/// container and no pool even when it named a reason.
#[test]
fn d32_first_named_rejection_keeps_its_own_ordinal() {
    let mut diag = Domain32Diag::default();
    diag.invalid_object_rows = 4;
    assert_eq!(diag.first_invalid_reason, 0);

    diag.note_named(17, CompactAirStrengthInvalidReason::PoolOwner);
    assert_eq!(
        diag.first_invalid_reason,
        CompactAirStrengthInvalidReason::PoolOwner as u16
    );
    assert_eq!(diag.first_invalid_ordinal, 17);

    diag.note_named(2, CompactAirStrengthInvalidReason::ContainerOwner);
    assert_eq!(
        diag.first_invalid_reason,
        CompactAirStrengthInvalidReason::PoolOwner as u16,
        "the FIRST named refusal owns the locator"
    );
    assert_eq!(diag.first_invalid_ordinal, 17);
}

/// Law: a frame whose only rejections are unnamed publishes the total and NO
/// `LastValue` locator pair.
///
/// Both locator names end in a `RAW_WORD_SUFFIXES` suffix, so the host folds
/// them with `LastValue`; emitting a zero here would erase an earlier failing
/// frame's real reason for the whole capture.
#[test]
fn d32_unnamed_rejections_publish_a_total_and_no_last_value_locator() {
    let units = domain32_declared_diagnostic_units(&Domain32Diag {
        invalid_object_rows: 6,
        ..Domain32Diag::default()
    });
    let named = |name: &str| units.iter().find(|unit| unit.name == name);
    assert_eq!(named("d32_invalid_air_wing_rows").map(|unit| unit.value), Some(6));
    assert!(named("d32_first_invalid_reason").is_none());
    assert!(named("d32_first_invalid_ordinal").is_none());
    assert_eq!(named("d32_invalid_object_rows").map(|unit| unit.value), Some(6));

    let named_units = domain32_declared_diagnostic_units(&Domain32Diag {
        invalid_pool_rows: 1,
        first_invalid_reason: CompactAirStrengthInvalidReason::PoolOwner as u16,
        first_invalid_ordinal: 9,
        ..Domain32Diag::default()
    });
    assert_eq!(
        named_units
            .iter()
            .find(|unit| unit.name == "d32_first_invalid_ordinal")
            .map(|unit| unit.value),
        Some(9)
    );
}

/// Law: every d32 unit name is classified by the shared accumulation rules, and
/// the two locators fold as `LastValue` rather than being summed.
#[test]
fn d32_unit_names_fold_as_the_values_they_are() {
    use compact_contract::diagnostic_channel::{unit_accumulation, Accumulation};
    let units = domain32_declared_diagnostic_units(&Domain32Diag {
        invalid_container_rows: 1,
        first_invalid_reason: CompactAirStrengthInvalidReason::ContainerOwner as u16,
        first_invalid_ordinal: 3,
        ..Domain32Diag::default()
    });
    for unit in &units {
        let class = unit_accumulation(unit.name);
        let expected = if unit.name.ends_with("_reason") || unit.name.ends_with("_ordinal") {
            Accumulation::LastValue
        } else {
            Accumulation::PerFrame
        };
        assert_eq!(class, expected, "{} folds as {class:?}", unit.name);
    }
}

/// Law: d44's roster guard NAMES itself when it poisons, and stays silent when
/// it does not.
///
/// `compact_supply_division_roster_status` was the source of the 0x300 word on
/// all ten degraded d44 hours of `captures/e2e_03`, and it set the bits without
/// leaving anything a reader could attribute them to.
#[test]
fn d44_roster_guard_notes_its_own_cause_exactly_when_it_poisons() {
    let mut clean = Domain44Diag::default();
    assert_eq!(
        compact_supply_division_roster_status(0, 12, 12, 12, true, &mut clean),
        0
    );
    assert_eq!(clean.causes, 0);
    assert_eq!(clean.first_cause, 0);

    for (candidates, roots, sampled, complete) in
        [(12u32, 11usize, 12u32, true), (12, 12, 11, true), (12, 12, 12, false)]
    {
        let mut diag = Domain44Diag::default();
        let status = compact_supply_division_roster_status(
            0, candidates, roots, sampled, complete, &mut diag,
        );
        assert_eq!(
            status,
            COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_SOURCE_UNAVAILABLE
        );
        assert_eq!(diag.first_cause, domain44_cause::DIVISION_ROSTER_SHORT);
        assert_ne!(
            diag.causes & domain44_cause::bit(domain44_cause::DIVISION_ROSTER_SHORT),
            0
        );
    }
}

/// Law: d44's reason names the guard AND the census the guard turns on.
///
/// The published trailer's nine surviving units were the walk's progress
/// markers -- `d44_disrupted_table_failure_substage=502` and
/// `d44_foreign_homebase_failure_substage=603` are both the empty-table SUCCESS
/// branches -- so the capture could not say which of four guards refused.
#[test]
fn d44_reason_names_the_guard_and_its_census() {
    let read = CompactSupplyLogisticsAttritionRead {
        rows: Vec::new(),
        foreign_homebase_rows: Vec::new(),
        foreign_homebase_source_available: true,
        division_attrition_rows: Vec::new(),
        division_attrition_source_available: true,
        attrition_component_trailer: None,
        tooltip_trailer: None,
        read_time_us: 0,
        changed_rows: 0,
        status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        province_rows: 10_272,
        division_rows: 837,
        badge_rows: 2,
        csupply_input_rows: 0,
        observed_province_roots: 10_272,
        observed_division_roots: 840,
        shard_panic_index: None,
        province_walk_passes: Some(1),
        diag: Domain44Diag {
            causes: domain44_cause::bit(domain44_cause::DIVISION_ROSTER_SHORT),
            first_cause: domain44_cause::DIVISION_ROSTER_SHORT,
            division_candidates: 840,
            division_validated_roots: 840,
            division_sampled_rows: 837,
            division_roster_complete: true,
        },
    };
    let reason = domain44_failure_reason(&read);
    assert!(
        reason.contains("division_roster_is_the_live_roster"),
        "{reason}"
    );
    assert!(reason.contains("840 candidate(s)"), "{reason}");
    assert!(reason.contains("837 sampled row(s)"), "{reason}");
    assert!(reason.contains("10272 province row(s)"), "{reason}");
}

/// Law: a d44 frame with no cause says so instead of naming one.
#[test]
fn d44_reason_without_a_cause_does_not_invent_one() {
    let mut read = domain44_reason_fixture();
    read.diag = Domain44Diag::default();
    let reason = domain44_failure_reason(&read);
    assert!(reason.contains("do not explain"), "{reason}");
    assert!(!reason.contains("cause 0"), "{reason}");
}

fn domain44_reason_fixture() -> CompactSupplyLogisticsAttritionRead {
    CompactSupplyLogisticsAttritionRead {
        rows: Vec::new(),
        foreign_homebase_rows: Vec::new(),
        foreign_homebase_source_available: true,
        division_attrition_rows: Vec::new(),
        division_attrition_source_available: true,
        attrition_component_trailer: None,
        tooltip_trailer: None,
        read_time_us: 0,
        changed_rows: 0,
        status_flags: COMPACT_STATUS_PARTIAL_DATA,
        province_rows: 1,
        division_rows: 1,
        badge_rows: 0,
        csupply_input_rows: 0,
        observed_province_roots: 1,
        observed_division_roots: 1,
        shard_panic_index: None,
        province_walk_passes: Some(1),
        diag: Domain44Diag::default(),
    }
}

/// Law: d54's orphan-owner refusal names how MANY orders were orphaned.
///
/// The exit published `domain54_orders_group_order_army_disagreement=10536` --
/// an army id, not a count -- and the report rendered it as one. "One army was
/// created under the read" and "the census is systematically short" are
/// different findings and only the count separates them.
#[test]
fn d54_orphan_refusal_reason_carries_the_count_and_the_census() {
    let read = CompactArmyBattleplansDepthRead {
        payload: Vec::new(),
        read_time_us: 0,
        status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        emitted_rows: 0,
        observed_army_rows: 41,
        observed_order_rows: 118,
        target_state_rows: 0,
        path_geometry_rows: 0,
        diagnostic_units: Vec::new(),
        refusal: Some(Domain54Refusal::OrphanOrderOwners {
            orphan_rows: 3,
            first_owner_kind: battleplans54::ORDER_OWNER_KIND_ARMY,
            first_army_id: 10_536,
            census_armies: 41,
            census_army_groups: 7,
        }),
    };
    let reason = domain54_failure_reason(&read);
    assert!(reason.contains("3 live order row(s)"), "{reason}");
    assert!(reason.contains("id 10536"), "{reason}");
    assert!(reason.contains("census held 41 army"), "{reason}");
    assert!(reason.contains("and 7 army-group owner(s)"), "{reason}");
}

/// Law: every d54 refusal variant renders a sentence that names its branch.
///
/// The variants exist so the reason cannot drift from the exit that produced
/// it; a variant added without a sentence would fall through to the encoder
/// wording and describe a frame that never reached the encoder.
#[test]
fn every_d54_refusal_names_its_branch() {
    let with = |refusal: Domain54Refusal| CompactArmyBattleplansDepthRead {
        payload: Vec::new(),
        read_time_us: 0,
        status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
        emitted_rows: 0,
        observed_army_rows: 0,
        observed_order_rows: 0,
        target_state_rows: 0,
        path_geometry_rows: 0,
        diagnostic_units: Vec::new(),
        refusal: Some(refusal),
    };
    for (refusal, needle) in [
        (
            Domain54Refusal::InputStatusFailure { inherited: 0x300 },
            "0x00000300",
        ),
        (Domain54Refusal::MissingArmyOrdersDependency, "d27"),
        (Domain54Refusal::MissingOobCommandDependency, "d28"),
        (
            Domain54Refusal::DependencyHardFailure {
                army: 0x100,
                oob: 0x200,
            },
            "d27=0x00000100",
        ),
        (
            Domain54Refusal::OrdersGroupManagerRejected { code: 3 },
            "error code 3",
        ),
        (
            Domain54Refusal::OrphanOrderOwners {
                orphan_rows: 1,
                first_owner_kind: battleplans54::ORDER_OWNER_KIND_ARMY_GROUP,
                first_army_id: 4,
                census_armies: 0,
                census_army_groups: 0,
            },
            "orders-group census does not hold",
        ),
        (
            Domain54Refusal::CarrierEncodeRejected { code: 4 },
            "encoder rejected",
        ),
        (Domain54Refusal::WorkerPanic, "worker panicked"),
    ] {
        let read = with(refusal);
        let reason = domain54_failure_reason(&read);
        assert!(reason.contains(needle), "{refusal:?} -> {reason}");
        assert!(reason.starts_with("d54 battleplan depth"), "{reason}");
    }
}

/// Law: d96 is a DECLARED outcome site, not a passenger on d92's row.
///
/// It publishes its own status word from its own refusal, so a silent d96
/// failure has to be the loud `MigratedButSilent` finding rather than the
/// reserved `producer_declared_no_predicate` code the report showed twice.
#[test]
fn d96_is_declared_beside_d92() {
    assert!(MIGRATED_OUTCOME_DOMAINS.contains(&COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_LIVE));
    assert!(
        MIGRATED_OUTCOME_DOMAINS.contains(&COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_WEIGHT_LIVE),
        "d96 publishes its own PERF word and must be held to its own outcome"
    );
}

/// Law: d96's reason names the guard the walk actually refused on, and the
/// country it refused inside when there was one.
///
/// `captures/e2e_03` published `d96_accumulator_unavailable=1` beside
/// `d96_failure_tag_db_id=17` and a reason that was the literal string
/// `producer_declared_no_predicate`.
#[test]
fn d96_reason_names_the_guard_and_the_country() {
    let read = CompactCountryPreferredTacticRead {
        payload: Vec::new(),
        tactic_weight_payload: Some(Vec::new()),
        observed_countries: 439,
        selected_countries: 22,
        tactic_weight_override_count: 0,
        read_time_us: 0,
        status_flags: 0,
        tactic_weight_status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
        diagnostic_units: Vec::new(),
        tactic_weight_diagnostic_units: Vec::new(),
        refusal: None,
        tactic_weight_refusal: Some(Domain96Refusal::Weight(
            PreferredTacticWeightError::Country {
                tag_db_id: 17,
                source: PreferredTacticWeightRowError::AccumulatorUnavailable,
            },
        )),
    };
    let reason = domain96_failure_reason(&read);
    assert!(reason.contains("d96_accumulator_unavailable"), "{reason}");
    assert!(reason.contains("tag_db_id 17"), "{reason}");
    assert!(reason.contains("439 country row(s)"), "{reason}");
}

/// Law: when d92 is what refused, d96's sentence carries d92's own sentence
/// rather than restating d96's bit.
#[test]
fn d96_dependency_refusal_carries_d92s_reason() {
    let read = CompactCountryPreferredTacticRead {
        payload: Vec::new(),
        tactic_weight_payload: Some(Vec::new()),
        observed_countries: 0,
        selected_countries: 0,
        tactic_weight_override_count: 0,
        read_time_us: 0,
        status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
        tactic_weight_status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
        diagnostic_units: Vec::new(),
        tactic_weight_diagnostic_units: Vec::new(),
        refusal: Some(Domain92Refusal::Collection(
            PreferredTacticCollectionError::RosterChanged,
        )),
        tactic_weight_refusal: Some(Domain96Refusal::D92DependencyUnavailable),
    };
    let reason = domain96_failure_reason(&read);
    assert!(reason.contains("d92 roster dependency"), "{reason}");
    assert!(reason.contains("d92_country_roster_changed"), "{reason}");
}

/// Law: d67's post-walk fence has a NAMED cause for "the callbacks never went
/// quiet", and it fails the frame exactly as the queue-changed cause does.
///
/// Before this, a callback merely EXECUTING as the 439-country walk finished
/// refused the fence identically to a queue that had mutated, and both
/// published cause 12. `captures/e2e_03` lost a complete instance walk -- 162
/// exact instances and 47 id rows -- to a frame whose only cause was that
/// fence, with nothing on the wire able to say which of the two it was.
#[test]
fn d67_fence_quiescence_is_a_named_frame_torn_cause() {
    assert_eq!(domain67_cause::COUNT, 18);
    assert_eq!(domain67_cause::ZERO_FENCE_CALLBACKS_NOT_QUIESCED, 17);
    assert_ne!(
        domain67_cause::FRAME_TORN_CAUSES
            & domain67_cause::bit(domain67_cause::ZERO_FENCE_CALLBACKS_NOT_QUIESCED),
        0,
        "an unprovable drain fails the frame closed, like every other fence cause"
    );
    assert_eq!(
        domain67_cause::name(domain67_cause::ZERO_FENCE_CALLBACKS_NOT_QUIESCED),
        "zero_removal_callbacks_quiesced_at_the_fence"
    );
    assert_ne!(
        domain67_cause::name(domain67_cause::ZERO_FENCE_CALLBACKS_NOT_QUIESCED),
        domain67_cause::name(domain67_cause::ZERO_RESERVATION_CHANGED_UNDER_THE_READ)
    );
}

/// Law: an idle hook quiesces immediately and spends nothing.
///
/// The wait is on the collector thread and is what keeps the fence off the
/// clean path: a frame that finds the hooked routine idle must pay no yields
/// at all, or the budget would be a per-hour cost rather than a race-breaker.
#[test]
fn d67_quiescence_wait_costs_nothing_when_no_callback_is_running() {
    let mut yields = u32::MAX;
    assert!(
        country_dynamic_modifiers_live_lane::a729_hook::await_production_callback_quiescence(
            D67_ZERO_FENCE_QUIESCENCE_YIELD_BUDGET,
            &mut yields,
        )
    );
    assert_eq!(yields, 0);
}

/// Law: the quiescence unit is published on every frame and folds per frame.
///
/// It is a cost, not a raw observation: a clean frame carrying a non-zero value
/// is a race this frame WON, and summing it over a capture is the right
/// reading.
#[test]
fn d67_quiescence_unit_is_a_per_frame_cost() {
    use compact_contract::diagnostic_channel::{unit_accumulation, Accumulation};
    assert_eq!(
        unit_accumulation("d67_zero_fence_quiescence_yields"),
        Accumulation::PerFrame
    );
}

/// Law: a zero-removal stamped with an EARLIER hour never reaches the wire.
///
/// A refused d67 frame rolls its reservation back -- custody may only be
/// committed once the sink has accepted the frame -- so the next hour's drain
/// re-delivers those events still carrying the refused hour. DMI2 admission
/// requires an event's observed hour to be the frame's own, so publishing them
/// turned one refused hour into two: the second failed the fold outright with
/// `synchronous zero-removal event failed owner/hour/current-set admission`.
///
/// What production change makes this fail: publishing the drain unfiltered
/// again, or comparing raw hour values instead of the fold's own hour index.


#[test]
fn d67_rare_event_counters_are_emitted_only_when_they_fired() {
    let quiet = CompactCountryDynamicModifiersRead {
        partiality: Vec::new(),
        rows: Vec::new(),
        exact_instances: Vec::new(),
        zero_removals: Vec::new(),
        schema_v2_admitted: true,
        observed_rows: 0,
        read_time_us: 0,
        status_flags: 0,
        hook_reservations: PendingHookReservations::default(),
        diag: Domain67Diag::default(),
    };
    let names = |read: &CompactCountryDynamicModifiersRead| {
        compact_country_dynamic_modifiers_diagnostic_units(read)
            .into_iter()
            .map(|unit| unit.name)
            .collect::<Vec<_>>()
    };
    let quiet_names = names(&quiet);
    assert!(!quiet_names.contains(&"d67_zero_fence_quiescence_yields"));
    assert!(!quiet_names.contains(&"d67_stale_zero_removals_dropped"));

    let raced = CompactCountryDynamicModifiersRead {
        diag: Domain67Diag {
            zero_fence_quiescence_yields: 3,
            stale_zero_removals_dropped: 2,
            ..Domain67Diag::default()
        },
        ..quiet
    };
    let raced_units = compact_country_dynamic_modifiers_diagnostic_units(&raced);
    assert_eq!(
        raced_units
            .iter()
            .find(|unit| unit.name == "d67_zero_fence_quiescence_yields")
            .map(|unit| unit.value),
        Some(3)
    );
    assert_eq!(
        raced_units
            .iter()
            .find(|unit| unit.name == "d67_stale_zero_removals_dropped")
            .map(|unit| unit.value),
        Some(2)
    );
    // LAST, so they can never displace the cause units above them in the
    // degraded record's shared reason budget.
    assert_eq!(
        raced_units[raced_units.len() - 2].name,
        "d67_zero_fence_quiescence_yields"
    );
    assert_eq!(
        raced_units[raced_units.len() - 1].name,
        "d67_stale_zero_removals_dropped"
    );
}
