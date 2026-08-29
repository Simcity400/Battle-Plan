//! Every registered counter, driven from the lane's own diagnostics struct
//! through the collector's copy, the emission array and the record encoder,
//! then read back out of the bytes.
//!
//! The census in `compact_contract` proves a counter is DECLARED and proves the
//! lane's lift into that struct; this proves the declared name arrives carrying
//! the value the struct held.

use super::*;

use compact_contract::counter_wire_registry::{rows_for, CounterWireRow, PRODUCERS};
use compact_contract::diagnostic_channel::{decode_units, DiagUnit, UnitPresence, UnitRole};

/// `elapsed_wall_ms | read_time_us | row_count | changed_rows | unit_count |
/// changed_unit_count | status_flags | reserved`.
const PERF_PAYLOAD_PREFIX_LEN: usize = 8 + 8 + 4 + 4 + 4 + 4 + 4 + 4;
/// `PDU1` plus the trailer version.
const TRAILER_HEADER_LEN: usize = 4 + 2;

/// One producer's arrival proof.
///
/// `probe(base, truths)` fills the LANE's diagnostics struct: the i-th
/// registered field takes [`value(base, i)`] if it is an integer, and bit i of
/// `truths` if it is a boolean. Two integers copied into each other's slots
/// therefore read as two wrong numbers, and the boolean walk below gives every
/// boolean a value no other boolean has on at least one pass.
struct ArrivalCase {
    producer_struct: &'static str,
    domain_id: u16,
    probe: Probe,
}

/// Which diagnostic funnel this producer's units really go through.
///
/// The funnel is part of the arrival claim: an untyped producer's units reach
/// the wire as `Undeclared`/`Undeclared`, which is what `recorder_host`'s
/// consumer test builds its probes with, and a declared producer's carry the
/// role its declaration gives each field. Driving both through one funnel here
/// would prove a path production does not take.
#[derive(Clone, Copy)]
enum Probe {
    Untyped(fn(u64, u64) -> Vec<CompactDomainPerfDiagnosticUnit>),
    Declared(fn(u64, u64) -> Vec<DeclaredDiagnosticUnit>),
}

/// What this row must arrive declared as, taken from the REGISTRY.
///
/// Not from the producer's own generated `COUNTER_ROLES`: the emission builds
/// its units off that table, so comparing the wire to it would be one statement
/// made twice and no change to a declaration could fail it. The registry states
/// the role, the census pins the registry against the `=> Role` in the source,
/// and this pins the bytes against the registry.
fn declaration(row: &CounterWireRow) -> (UnitRole, UnitPresence) {
    match row.role {
        UnitRole::Undeclared => (UnitRole::Undeclared, UnitPresence::Undeclared),
        role => (role, UnitPresence::Observed),
    }
}

/// One producer's units, on the wire, through the funnel it really uses.
fn probe_units(case: &ArrivalCase, base: u64, truths: u64) -> Vec<DiagUnit> {
    match case.probe {
        Probe::Untyped(probe) => untyped_units_on_the_wire(case.domain_id, &probe(base, truths)),
        Probe::Declared(probe) => declared_units_on_the_wire(case.domain_id, &probe(base, truths)),
    }
}

fn truth(truths: u64, index: usize) -> bool {
    truths >> index & 1 == 1
}

/// The i-th integer counter's value. `base` 0 means the quiet hour: every
/// counter reads zero, which is a reading the wire must still carry.
fn value(base: u64, index: u64) -> u64 {
    if base == 0 {
        0
    } else {
        base + index
    }
}

/// d102 drives the EMISSION function itself, with no measured units beside the
/// counters: the trailer's other pairs are this hour's derived measurements,
/// not registered counters, and the arrival law is one unit per registered row.
fn d102_probe(base: u64, _truths: u64) -> Vec<CompactDomainPerfDiagnosticUnit> {
    d102_fog_derivation_inputs_units(
        Some(d102_fog_input_counters(
            &fog_derivation_inputs_live_lane::FogInputWalk {
                rows: Vec::new(),
                unit_observations: Vec::new(),
                address_reads: Vec::new(),
                countries_walked: value(base, 0) as u32,
                units_without_handle: value(base, 1),
                viewer_levels_non_positive: value(base, 2),
                units_undispatched_class: value(base, 3),
            },
        )),
        &[],
    )
}

/// d95's units carry a declared ROLE, so its probe goes through the typed
/// funnel the emission really uses.
fn d95_probe(base: u64, truths: u64) -> Vec<DeclaredDiagnosticUnit> {
    // THE PRODUCTION FUNCTION, not its body copied out: a wrapper that edited
    // the units after building them would otherwise pass this untouched.
    d95_declared_diagnostic_units(
        &spotted_units_live_lane::CensusDiagnostics {
            hooks_installed: truth(truths, 0),
            site_entries_intel_getter: value(base, 1),
            site_entries_land_frame: value(base, 2),
            site_entries_land_eligibility: value(base, 3),
            site_entries_taskforce_frame: value(base, 4),
            land_candidates_seen: value(base, 5),
            land_rejected_unreadable_handle: value(base, 6),
            land_rejected_below_intel_threshold: value(base, 7),
            land_rejected_by_eligibility: value(base, 8),
            land_rejected_taskforce_state: value(base, 9),
            land_abandoned_pending: value(base, 10),
            land_admitted: value(base, 11),
            land_frames_without_enumeration: value(base, 12),
            land_frames_producer_gate_closed: value(base, 13),
            land_admitted_rows_lost_to_replacement: value(base, 14),
            taskforce_candidates_seen: value(base, 15),
            taskforce_rejected_unreadable_handle: value(base, 16),
            taskforce_rejected_below_intel_threshold: value(base, 17),
            taskforce_rejected_by_eligibility: value(base, 18),
            taskforce_rejected_taskforce_state: value(base, 19),
            taskforce_abandoned_pending: value(base, 20),
            taskforce_admitted: value(base, 21),
            taskforce_frames_without_enumeration: value(base, 22),
            taskforce_frames_producer_gate_closed: value(base, 23),
            taskforce_admitted_rows_lost_to_replacement: value(base, 24),
            taskforce_frames_source_collection_empty: value(base, 25),
            next_sequence: value(base, 26),
        },
    )
}

fn d79_probe(base: u64, _truths: u64) -> Vec<CompactDomainPerfDiagnosticUnit> {
    d79_hook_counter_units(&hud_role_observer_live_lane::a729_hook::HookDiagnostics {
        callback_count: value(base, 0),
        rejected_observation_count: value(base, 1),
        authenticated_surface_count: value(base, 2) as usize,
        in_flight_callbacks: value(base, 3) as usize,
        next_sequence: value(base, 4),
        current_error: None,
    })
}

fn d87_probe(base: u64, truths: u64) -> Vec<CompactDomainPerfDiagnosticUnit> {
    d87_hook_counter_units(&decision_panel_live_lane::a729_hook::HookDiagnostics {
        installed: truth(truths, 0),
        epoch_open: truth(truths, 1),
        pending_receipt: truth(truths, 2),
        authenticated_snapshot: truth(truths, 3),
        materializer_calls: value(base, 4),
        insertion_calls: value(base, 5),
        text_events: value(base, 6),
        can_take_calls: value(base, 7),
        timeout_calls: value(base, 8),
        in_flight_callbacks: value(base, 9) as usize,
        next_generation: value(base, 10),
        current_error: truth(truths, 11),
    })
}

fn d72_probe(base: u64, truths: u64) -> Vec<CompactDomainPerfDiagnosticUnit> {
    d72_hook_counter_units(
        &resolved_tooltip_instances_live_lane::a729_hook::HookDiagnostics {
            installed: truth(truths, 0),
            install_epoch: value(base, 1),
            callbacks_open: truth(truths, 2),
            in_flight_callbacks: value(base, 3) as usize,
            controller_calls: value(base, 4),
            observation_sequence: value(base, 5),
            owner_generation: value(base, 6),
            has_observed_baseline: truth(truths, 7),
            poisoned: truth(truths, 8),
        },
    )
}

fn d78_probe(base: u64, truths: u64) -> Vec<CompactDomainPerfDiagnosticUnit> {
    d78_hook_counter_units(
        &design_catalog_context_live_lane::a729_hook::HookDiagnostics {
            callback_count: value(base, 0),
            latest_transition_sequence: value(base, 1),
            next_transition_sequence: value(base, 2),
            in_flight_callbacks: value(base, 3) as usize,
            has_complete_snapshot: truth(truths, 4),
            current_error: None,
        },
    )
}

const ARRIVAL_CASES: &[ArrivalCase] = &[
    ArrivalCase {
        producer_struct: "D95Counters",
        domain_id: COMPACT_DOMAIN_POV_SPOTTED_UNITS_LIVE,
        probe: Probe::Declared(d95_probe),
    },
    ArrivalCase {
        producer_struct: "D102FogInputCounters",
        domain_id: COMPACT_DOMAIN_FOG_DERIVATION_INPUTS_LIVE,
        probe: Probe::Untyped(d102_probe),
    },
    ArrivalCase {
        producer_struct: "D79HookCounters",
        domain_id: COMPACT_DOMAIN_HUD_SURFACE_ACTIVATION_LIVE,
        probe: Probe::Untyped(d79_probe),
    },
    ArrivalCase {
        producer_struct: "D87HookCounters",
        domain_id: decision_panel_live_lane::DOMAIN_ID,
        probe: Probe::Untyped(d87_probe),
    },
    ArrivalCase {
        producer_struct: "D72HookCounters",
        domain_id: COMPACT_DOMAIN_RESOLVED_TOOLTIP_INSTANCES_LIVE,
        probe: Probe::Untyped(d72_probe),
    },
    ArrivalCase {
        producer_struct: "D78HookCounters",
        domain_id: COMPACT_DOMAIN_DESIGN_CATALOG_CONTEXT_LIVE,
        probe: Probe::Untyped(d78_probe),
    },
];

/// What the i-th registered field is set to under `truths`.
fn expected(row: &CounterWireRow, index: usize, base: u64, truths: u64) -> u64 {
    if row.source_type == "bool" {
        u64::from(truth(truths, index))
    } else {
        value(base, index as u64)
    }
}

fn perf_record(domain_id: u16) -> CompactDomainPerfRecord {
    CompactDomainPerfRecord {
        domain_id,
        read_time_us: 0,
        row_count: 0,
        changed_rows: 0,
        unit_count: 0,
        changed_unit_count: 0,
        status_flags: 0,
    }
}

/// The units a DOMAIN_PERF record carries, read back out of the encoded bytes.
fn untyped_units_on_the_wire(
    domain_id: u16,
    units: &[CompactDomainPerfDiagnosticUnit],
) -> Vec<DiagUnit> {
    let mut out = Vec::new();
    append_compact_domain_perf_record_with_diagnostics(
        &mut out,
        1,
        0,
        perf_record(domain_id),
        0,
        units,
    );
    units_in(&out, domain_id)
}

/// The same, through the funnel that carries a declared role.
fn declared_units_on_the_wire(domain_id: u16, units: &[DeclaredDiagnosticUnit]) -> Vec<DiagUnit> {
    let mut out = Vec::new();
    append_compact_domain_perf_record_with_declared_diagnostics(
        &mut out,
        1,
        0,
        perf_record(domain_id),
        0,
        units,
    );
    units_in(&out, domain_id)
}

fn units_in(out: &[u8], domain_id: u16) -> Vec<DiagUnit> {
    let header =
        compact_contract::parse_compact_record_header(out).expect("the record header parses");
    assert_eq!(header.record_type, COMPACT_RECORD_TYPE_DOMAIN_PERF);
    assert_eq!(header.domain_id, domain_id);
    let payload = &out[compact_contract::COMPACT_RECORD_HEADER_LEN..header.record_len()];
    assert!(
        payload.len() > PERF_PAYLOAD_PREFIX_LEN + TRAILER_HEADER_LEN,
        "the record carries no diagnostic trailer at all"
    );
    let trailer = &payload[PERF_PAYLOAD_PREFIX_LEN..];
    assert_eq!(
        &trailer[..4],
        COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC,
        "the trailer does not start with PDU1"
    );
    let (decoded, _) = decode_units(&trailer[TRAILER_HEADER_LEN..]).expect("the trailer decodes");
    decoded
}

/// Every boolean pattern the walk runs: all false, all true, and one-hot for
/// each registered field. Two boolean fields always differ on at least one
/// pass, so copying one into the other's slot cannot pass.
fn truth_patterns(row_count: usize) -> Vec<u64> {
    let mut out = vec![0, (1u64 << row_count) - 1];
    for index in 0..row_count {
        out.push(1u64 << index);
    }
    out
}

/// d102's WHOLE trailer, measured units included, read back out of the bytes.
///
/// Every other arrival case drives the producer alone, with an empty measured
/// slice, and asserts one unit per registered row. That leaves the rest of the
/// trailer unproven: d102's emission copies the hour's measured units in front
/// of its counters, and a unit pushed inside the units builder itself would
/// join them under any name at all, in a function no carrier census reads
/// (Codex VB2 round 7, finding 1).
///
/// This drives the real builder with a NON-EMPTY measured slice and holds every
/// arriving unit to the registry or to the producer's `measured_units`
/// inventory -- by what ARRIVES, not by where it was written.
///
/// MUTATION: push a unit inside `d102_fog_derivation_inputs_units`, under any
/// name, and this fails.
#[test]
fn d102_publishes_its_measured_units_and_nothing_else() {
    let producer = PRODUCERS
        .iter()
        .find(|producer| producer.producer_struct == "D102FogInputCounters")
        .expect("d102 is a registered producer");
    let registered: std::collections::BTreeSet<&str> = rows_for(producer.producer_struct)
        .iter()
        .map(|row| row.unit)
        .collect();
    let named: std::collections::BTreeSet<&str> = producer
        .measured_units
        .iter()
        .map(|(name, _)| *name)
        .collect();
    assert!(!named.is_empty(), "d102 declares a measured-unit carrier");
    let measured: Vec<CompactDomainPerfDiagnosticUnit> = named
        .iter()
        .enumerate()
        .map(|(index, name)| CompactDomainPerfDiagnosticUnit {
            name,
            value: 4_000 + index as u64,
        })
        .collect();
    let units = d102_fog_derivation_inputs_units(
        Some(d102_fog_input_counters(
            &fog_derivation_inputs_live_lane::FogInputWalk {
                rows: Vec::new(),
                unit_observations: Vec::new(),
                address_reads: Vec::new(),
                countries_walked: 1,
                units_without_handle: 2,
                viewer_levels_non_positive: 3,
                units_undispatched_class: 4,
            },
        )),
        &measured,
    );
    let arrived = untyped_units_on_the_wire(producer.domain_id, &units);
    assert_eq!(
        arrived.len(),
        measured.len() + registered.len(),
        "d102 published {} units for {} measured and {} registered",
        arrived.len(),
        measured.len(),
        registered.len()
    );
    for unit in &arrived {
        assert!(
            registered.contains(unit.name.as_str()) || named.contains(unit.name.as_str()),
            "{} arrived in d102's trailer and is neither a registry row nor a named              measured unit",
            unit.name
        );
    }
    // THE MEASURED UNITS COME FIRST. A counter that moved in front of them
    // would change every consumer's positional reading of the trailer.
    for (index, unit) in arrived.iter().enumerate() {
        let is_measured = index < measured.len();
        assert_eq!(
            is_measured,
            named.contains(unit.name.as_str()),
            "{} arrived at position {index}, which is not where its kind belongs",
            unit.name
        );
    }
}

/// d102's MEASURED VALUES are the hour's, not just its names.
///
/// The test above proves no unit joins the trailer unaccounted. It says nothing
/// about the numbers: replace `fog_derivation_inputs_radar_rows` with a literal
/// zero and the census the walk read never reaches the capture, which is the
/// defect class itself (Codex VB2 round 8, finding 1).
///
/// This drives the REAL compute -- the function that builds the read out of a
/// walk -- with a census of known shape, then the real units builder and the
/// real encoder, and reads the row counts back out of the bytes.
///
/// MUTATION: hand any `*_rows` unit a constant, swap two of them, or take
/// `units_walked` off the observations, and this fails.
#[test]
fn d102_measured_row_counts_are_the_census_the_walk_read() {
    use compact_contract::fog_derivation_inputs as fgi;

    const UNITS: [(u8, &str); 7] = [
        (fgi::TABLE_RADAR_COVERAGE, "fog_derivation_inputs_radar_rows"),
        (
            fgi::TABLE_STATIONED_PROVINCE,
            "fog_derivation_inputs_stationed_province_rows",
        ),
        (
            fgi::TABLE_STATIONED_UNIT,
            "fog_derivation_inputs_stationed_unit_rows",
        ),
        (
            fgi::TABLE_UNIT_VIEWER_LEVEL,
            "fog_derivation_inputs_viewer_level_rows",
        ),
        (fgi::TABLE_UNIT_CLASS, "fog_derivation_inputs_unit_class_rows"),
        (
            fgi::TABLE_COUNTRY_ORDINAL,
            "fog_derivation_inputs_country_ordinal_rows",
        ),
        (
            fgi::TABLE_VOLUNTEER_HOST,
            "fog_derivation_inputs_volunteer_host_rows",
        ),
    ];

    // The real compute, the real units builder and the real encoder, driven
    // with a census of the caller's shape and read back out of the bytes.
    let emit = |counts: [u32; 7], walked: u32| -> Vec<DiagUnit> {
        let mut walk_rows = Vec::new();
        for (index, (table, _)) in UNITS.iter().enumerate() {
            for row in 0..counts[index] {
                walk_rows.push(fgi::FogInputRow {
                    table: *table,
                    k0: row,
                    k1: 0,
                    k2: 0,
                    k3: 0,
                    k4: 0,
                    value: row as i32,
                });
            }
        }
        // `walked` distinct units, one of them visited twice and one a declined
        // `(0, 0)` visit, so the count is the UNITS the walk resolved and cannot
        // be read off any row count.
        let mut walk_unit_observations: Vec<fgi::UnitObservation> = (1..=walked)
            .map(|unit| fgi::UnitObservation {
                type_id: 40 + unit,
                stable_id: unit,
                start: 0,
                len: 0,
            })
            .collect();
        walk_unit_observations.push(fgi::UnitObservation {
            type_id: 41,
            stable_id: 1,
            start: 0,
            len: 0,
        });
        walk_unit_observations.push(fgi::UnitObservation {
            type_id: 0,
            stable_id: 0,
            start: 0,
            len: 0,
        });
        let observation = CompactFogDerivationInputsObservation {
            bracket_read_time_us: 0,
            walk_rows,
            walk_unit_observations,
            walk_address_reads: Vec::new(),
            observed_country_count: 7,
            walk_counters: Some(D102FogInputCounters {
                countries_walked: 7,
                units_without_handle: 0,
                viewer_levels_non_positive: 0,
                units_undispatched_class: 0,
            }),
            previous: &[],
            baselined: false,
            cadence_reached: false,
            refusal: None,
            frame_status_failure: false,
        };
        let read = compute_compact_fog_derivation_inputs(observation, 0, 100);
        let units = d102_fog_derivation_inputs_units(read.walk_counters, &read.diagnostic_units);
        untyped_units_on_the_wire(COMPACT_DOMAIN_FOG_DERIVATION_INPUTS_LIVE, &units)
    };
    let value_of = |arrived: &[DiagUnit], name: &str| -> u64 {
        arrived
            .iter()
            .find(|unit| unit.name == name)
            .unwrap_or_else(|| panic!("{name} is not in the trailer"))
            .value
    };

    // TWO CENSUSES, and every unit follows ITS OWN table across both. One
    // census can be satisfied by a literal -- a production change replacing a
    // value with the number the fixture expects passes a single-shape proof and
    // loses the measurement on every other hour. Two shapes with no count in
    // common cannot be followed by any constant (Codex VB2 round 11, finding 1).
    const FIRST: [u32; 7] = [11, 13, 17, 19, 23, 29, 31];
    const SECOND: [u32; 7] = [3, 5, 2, 37, 41, 1, 43];
    let first = emit(FIRST, 9);
    let second = emit(SECOND, 5);
    for (index, (_, unit)) in UNITS.iter().enumerate() {
        assert_eq!(
            value_of(&first, unit),
            u64::from(FIRST[index]),
            "{unit} did not carry the rows the first census holds"
        );
        assert_eq!(
            value_of(&second, unit),
            u64::from(SECOND[index]),
            "{unit} did not carry the rows the second census holds"
        );
        assert_ne!(
            FIRST[index], SECOND[index],
            "the two censuses must differ in every table, or a constant could \
             satisfy one of them"
        );
    }
    assert_eq!(
        value_of(&first, "fog_derivation_inputs_units_walked"),
        9,
        "units_walked did not carry the units the first walk resolved"
    );
    assert_eq!(
        value_of(&second, "fog_derivation_inputs_units_walked"),
        5,
        "units_walked did not carry the units the second walk resolved"
    );
    // The rest ARRIVE, which `value_of` proves by panicking when one does not,
    // and the two flags are pinned by the census the fixture chose.
    let _ = value_of(&first, "fog_derivation_inputs_compute_us");
    assert_eq!(
        value_of(&first, "fog_derivation_inputs_full_census"),
        1,
        "an unbaselined tracker takes the whole census"
    );
    assert_eq!(
        value_of(&first, "fog_derivation_inputs_keyframe_by_cadence"),
        0,
        "the fixture's cadence is not due"
    );
}

/// MUTATION: drop one pair from any producer's emission, rename a unit, copy
/// the wrong lane field into a producer field (integer OR boolean), or append a
/// second unit under a name already published, and this fails naming the unit.
#[test]
fn every_registered_counter_arrives_on_the_wire_with_the_value_the_lane_computed() {
    const BASE: u64 = 9_000;
    for case in ARRIVAL_CASES {
        let rows = rows_for(case.producer_struct);
        assert!(
            !rows.is_empty(),
            "{} has no registry rows",
            case.producer_struct
        );
        for truths in truth_patterns(rows.len()) {
            let units = probe_units(case, BASE, truths);
            // A duplicate name would let `find` below read one value while the
            // host's fold takes the other; the count and the uniqueness check
            // together leave no room for a second unit under any name.
            assert_eq!(
                units.len(),
                rows.len(),
                "{} published {} units for {} registered counters",
                case.producer_struct,
                units.len(),
                rows.len()
            );
            let mut seen = std::collections::BTreeSet::new();
            for unit in &units {
                assert!(
                    seen.insert(unit.name.clone()),
                    "{} is published twice in one record, so a consumer folds whichever \
                     comes last",
                    unit.name
                );
            }
            for (index, row) in rows.iter().enumerate() {
                let unit = units
                    .iter()
                    .find(|unit| unit.name == row.unit)
                    .unwrap_or_else(|| {
                        panic!(
                            "{} computes {} and the wire carries no unit named {}",
                            row.source_struct, row.field, row.unit
                        )
                    });
                // The declaration each producer reaches the wire under.
                // Pinned because a producer that stopped declaring its roles
                // would still fold correctly and would still lose what the
                // roles say, which no value check can see.
                assert_eq!(
                    (unit.role, unit.presence),
                    declaration(row),
                    "{} did not arrive under the declaration its producer publishes",
                    row.unit
                );
                let want = expected(row, index, BASE, truths);
                assert_eq!(
                    unit.value, want,
                    "{} arrived as {} on the wire; the lane computed {want} \
                     (boolean pattern {truths:#b})",
                    row.unit, unit.value
                );
            }
        }
    }
}

/// A zero is a reading, not an absence.
///
/// Every registered counter starts at zero and most of them stay there in a
/// quiet hour. A trailer that dropped zero-valued units would publish nothing
/// for exactly the counters whose zero is the interesting answer -- no
/// rejections, no drops -- and the sentinel walk above, which gives every field
/// a nonzero value, could not tell.
///
/// MUTATION: skip zero-valued pairs in `counter_units` and this fails.
#[test]
fn every_registered_counter_arrives_when_its_value_is_zero() {
    for case in ARRIVAL_CASES {
        let rows = rows_for(case.producer_struct);
        let units = probe_units(case, 0, 0);
        assert_eq!(
            units.len(),
            rows.len(),
            "{} published {} units for {} registered counters when every counter read zero",
            case.producer_struct,
            units.len(),
            rows.len()
        );
        for row in &rows {
            let unit = units
                .iter()
                .find(|unit| unit.name == row.unit)
                .unwrap_or_else(|| panic!("{} is absent from a record of zeros", row.unit));
            assert_eq!(
                unit.value, 0,
                "{} read zero and the wire carries {}",
                row.unit, unit.value
            );
        }
    }
}

/// A registered counter with no arrival case would be a claim nothing checks.
///
/// This also pins `CounterWireProducer::domain_id` against the collector's own
/// domain constant, which is what the host-side consumer test folds against.
#[test]
fn the_arrival_cases_cover_every_registered_producer() {
    let covered: Vec<&str> = ARRIVAL_CASES
        .iter()
        .map(|case| case.producer_struct)
        .collect();
    for producer in PRODUCERS {
        let case = ARRIVAL_CASES
            .iter()
            .find(|case| case.producer_struct == producer.producer_struct)
            .unwrap_or_else(|| {
                panic!(
                    "{} is registered but has no arrival case",
                    producer.producer_struct
                )
            });
        assert_eq!(
            case.domain_id, producer.domain_id,
            "{}'s registered domain id is not the constant the collector emits under",
            producer.producer_struct
        );
    }
    assert_eq!(
        covered.len(),
        PRODUCERS.len(),
        "an arrival case names a producer that is not registered"
    );
}

/// The walk must be able to fail: two registered fields that carry the same
/// value on every pass could be swapped without the arrival test noticing.
#[test]
fn no_two_registered_fields_agree_on_every_pass() {
    const BASE: u64 = 9_000;
    for case in ARRIVAL_CASES {
        let rows = rows_for(case.producer_struct);
        let patterns = truth_patterns(rows.len());
        for left in 0..rows.len() {
            for right in (left + 1)..rows.len() {
                let separated = patterns.iter().any(|truths| {
                    expected(rows[left], left, BASE, *truths)
                        != expected(rows[right], right, BASE, *truths)
                });
                assert!(
                    separated,
                    "{}'s {} and {} carry the same value on every pass, so the harness \
                     could not tell them apart",
                    case.producer_struct, rows[left].unit, rows[right].unit
                );
            }
        }
    }
}

/// A zero is a reading only when something read it.
///
/// d102's four counters ride every d102 record. On an hour whose walk never ran
/// -- a frame that already carried a failure status, unreadable guards, a
/// panicked worker -- a zero would say "the walk dropped nothing", which is the
/// original defect class with the sign flipped: a number in the capture that
/// nobody measured. On an hour whose walk DID run and was refused afterwards,
/// the counts are real and dropping them would lose exactly what this registry
/// exists to carry.
///
/// MUTATION: give `CompactFogDerivationInputsRead::refused` a default counter
/// set instead of `None`, or stop carrying `walk_counters` through either
/// refusal arm, and this fails.
#[test]
fn d102_publishes_its_walk_counters_only_when_the_walk_ran() {
    use compact_contract::fog_derivation_inputs as fgi;

    let registered: Vec<&str> = rows_for("D102FogInputCounters")
        .iter()
        .map(|row| row.unit)
        .collect();
    assert_eq!(registered.len(), 4, "d102 has four registered counters");

    let counted = fog_derivation_inputs_live_lane::FogInputWalk {
        rows: Vec::new(),
        unit_observations: Vec::new(),
        address_reads: Vec::new(),
        countries_walked: 11,
        units_without_handle: 22,
        viewer_levels_non_positive: 33,
        units_undispatched_class: 44,
    };

    let observation = |walk_counters, frame_status_failure, address_reads| {
        CompactFogDerivationInputsObservation {
            bracket_read_time_us: 0,
            walk_rows: Vec::new(),
            walk_unit_observations: Vec::new(),
            walk_address_reads: address_reads,
            observed_country_count: 11,
            walk_counters,
            previous: &[],
            baselined: false,
            cadence_reached: false,
            refusal: None,
            frame_status_failure,
        }
    };
    let published = |read: &CompactFogDerivationInputsRead| -> Vec<(&'static str, u64)> {
        d102_fog_derivation_inputs_units(read.walk_counters, &read.diagnostic_units)
            .into_iter()
            .filter(|unit| registered.contains(&unit.name))
            .map(|unit| (unit.name, unit.value))
            .collect()
    };

    // THE WALK DID NOT RUN: the frame already carried a failure status.
    let read = compute_compact_fog_derivation_inputs(observation(None, true, Vec::new()), 0, 0);
    assert!(
        published(&read).is_empty(),
        "an hour that walked nothing published {:?}, which is a measurement \
         nobody took",
        published(&read)
    );

    // THE WALK RAN and the post-bracket sort refused it: one address read as two
    // identities, which is a torn read of the hour rather than a reason to lose
    // what the walk counted.
    let torn = vec![
        fgi::AddressIdentity {
            address: 0x1000,
            identity: 1,
        },
        fgi::AddressIdentity {
            address: 0x1000,
            identity: 2,
        },
    ];
    let read = compute_compact_fog_derivation_inputs(
        observation(Some(d102_fog_input_counters(&counted)), false, torn),
        0,
        0,
    );
    assert!(
        read.refusal.is_some() || read.snapshot.is_refusal(),
        "the torn walk was not refused, so this proves nothing"
    );
    assert_eq!(
        published(&read),
        vec![
            ("fog_derivation_inputs_countries_walked", 11),
            ("fog_derivation_inputs_units_without_handle", 22),
            ("fog_derivation_inputs_viewer_levels_non_positive", 33),
            ("fog_derivation_inputs_units_undispatched_class", 44),
        ],
        "a walk that ran and was then refused must publish what it counted"
    );
}
