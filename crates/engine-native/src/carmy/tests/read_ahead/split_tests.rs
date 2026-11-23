use super::*;
use carmy::{StatusPacket, StatusPart};

struct Fault<'a> {
    source: &'a PresentationMemory,
    address: Option<usize>,
}
impl Memory for Fault<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        !self
            .address
            .is_some_and(|at| address <= at && at - address < output.len())
            && self.source.copy(address, output)
    }
}

fn serial(
    memory: &impl Memory,
    seeds: &[carmy::DivisionSeed],
    outcome: Outcome,
    held: &[Option<crate::csub_unit_definition::DivisionMaxima>],
) -> (StatusPacket, Vec<carmy::StatusRow>, u64) {
    let mut packet = StatusPacket::new(seeds.len()).unwrap();
    packet.capture_holding(memory, 0, GS, seeds, outcome, held, &[]);
    let refused = packet.capture_movement_organisation(memory, 0, GS, seeds);
    let mut rows = vec![carmy::StatusRow::default(); seeds.len()];
    let outcome = packet.project(&mut rows);
    rows.truncate(outcome.len());
    (packet, rows, refused)
}

fn split_hour(
    memory: &(impl Memory + Sync),
    seeds: &[carmy::DivisionSeed],
    source: Outcome,
    held: &[Option<crate::csub_unit_definition::DivisionMaxima>],
    packet: &mut StatusPacket,
    parts: &mut [StatusPart],
) -> (Vec<carmy::StatusRow>, u64, u64) {
    let growths_before = crate::allocation::growths();
    packet.prepare_holding(memory, 0, GS, seeds, source, held, &[]);
    let admitted = packet.prepared_units();
    let count = parts.len();
    for (index, part) in parts.iter_mut().enumerate() {
        part.range = admitted * index / count..admitted * (index + 1) / count;
    }
    let part_growths = std::thread::scope(|scope| {
        let jobs: Vec<_> = parts
            .iter_mut()
            .rev()
            .map(|part| {
                let packet = &*packet;
                scope.spawn(move || {
                    let before = crate::allocation::growths();
                    part.capture(memory, 0, GS, seeds, packet);
                    crate::allocation::growths() - before
                })
            })
            .collect();
        jobs.into_iter().map(|job| job.join().unwrap()).sum::<u64>()
    });
    let refused = parts.iter().map(|part| part.organisation_refused).sum();
    packet.stitch_status_parts(parts);
    let mut rows = vec![carmy::StatusRow::default(); seeds.len()];
    let outcome = packet.project(&mut rows);
    rows.truncate(outcome.len());
    (
        rows,
        refused,
        crate::allocation::growths() - growths_before + part_growths,
    )
}

fn assert_same(
    memory: &(impl Memory + Sync),
    seeds: &[carmy::DivisionSeed],
    source: Outcome,
    held: &[Option<crate::csub_unit_definition::DivisionMaxima>],
) {
    let (expected, rows, refused) = serial(memory, seeds, source, held);
    for count in [1, 2, 3, 7, 15, 23] {
        let mut packet = StatusPacket::new(seeds.len()).unwrap();
        let mut parts: Vec<_> = (0..count).map(|_| StatusPart::vacant()).collect();
        let mut warmed = None;
        for hour in 0..4 {
            let actual = split_hour(memory, seeds, source, held, &mut packet, &mut parts);
            assert_eq!(
                packet.outcome(),
                expected.outcome(),
                "parts={count}, hour={hour}"
            );
            assert_eq!(actual.0, rows, "parts={count}, hour={hour}");
            assert_eq!(actual.1, refused, "parts={count}, hour={hour}");
            for (index, seed) in seeds.iter().enumerate() {
                assert_eq!(
                    packet.unit_maxima(index),
                    expected.unit_maxima(index),
                    "{}",
                    seed.division
                );
            }
            for row in &actual.0 {
                assert_eq!(packet.supply_origins(row), expected.supply_origins(row));
            }
            let bytes = packet.allocated_bytes()
                + parts.iter().map(StatusPart::allocated_bytes).sum::<usize>();
            if hour >= 2 {
                assert_eq!(actual.2, 0, "steady parts must not reallocate");
                assert_eq!(
                    *warmed.get_or_insert(bytes),
                    bytes,
                    "steady parts retain their storage"
                );
            }
        }
        let demanded: Vec<_> = seeds
            .iter()
            .step_by((seeds.len() / 3).max(1))
            .map(|seed| seed.division)
            .collect();
        let (mut border_expected, _, _) = serial(memory, seeds, source, held);
        assert_eq!(
            packet.capture_border_stats(memory, 0, demanded.iter().copied()),
            border_expected.capture_border_stats(memory, 0, demanded.iter().copied()),
        );
        let mut actual = vec![carmy::StatusRow::default(); seeds.len()];
        let mut baseline = actual.clone();
        assert_eq!(
            packet.project(&mut actual),
            border_expected.project(&mut baseline)
        );
        assert_eq!(actual, baseline, "border stats join across parts");
    }
}

#[test]
fn status_split_distinct_divisions_and_countries_match_serial() {
    let (source, mut seeds, mut held) = distinct_status_world(129);
    for index in [1, 63, 64, 65, 100, 128] {
        seeds[index].stat_block = seeds[0].stat_block;
        held[index] = held[0].map(|mut value| {
            value.organisation = -17;
            value
        });
    }
    assert_same(&source, &seeds, Outcome::Rows { len: seeds.len() }, &held);
}

#[test]
fn status_split_faults_preserve_first_reason_and_prepared_maxima() {
    let (source, original, _) = distinct_status_world(129);
    for index in [0, 1, 63, 64, 65, 128] {
        for preparation in [false, true] {
            let mut seeds = original.clone();
            if preparation {
                seeds[index].handle = None;
            } else {
                seeds[index].entrenchment = None;
            }
            seeds[128].primary_vtable = None;
            assert_same(&source, &seeds, Outcome::Rows { len: seeds.len() }, &[]);
        }
    }
    for address in [
        original[64].stat_block.unwrap(),
        A1 + crate::cunit_supply_inputs::ASKED,
        carmy::EXERCISE_MAXIMUM_LEVEL_RVA.0,
        GS + cgame_state::COUNTRIES,
    ] {
        let memory = Fault {
            source: &source,
            address: Some(address),
        };
        assert_same(
            &memory,
            &original,
            Outcome::Rows {
                len: original.len(),
            },
            &[],
        );
    }
}

#[test]
fn status_split_empty_truncated_and_failed_sources_match_serial() {
    let (source, seeds, _) = distinct_status_world(65);
    assert_same(&source, &[], Outcome::Rows { len: 0 }, &[]);
    assert_same(
        &source,
        &seeds,
        Outcome::Truncated {
            len: seeds.len(),
            unread_rows: 17,
        },
        &[],
    );
    assert_same(
        &source,
        &[],
        Outcome::Unavailable {
            reason: "synthetic census failure",
        },
        &[],
    );
}

#[test]
fn status_split_optional_organisation_refusals_keep_the_serial_prefix() {
    let (source, mut seeds, _) = distinct_status_world(129);
    for index in [0, 17, 63, 64, 128] {
        seeds[index].ordinary_route = None;
    }
    assert!(serial(&source, &seeds, Outcome::Rows { len: seeds.len() }, &[]).2 >= 5);
    assert_same(&source, &seeds, Outcome::Rows { len: seeds.len() }, &[]);
    seeds[65].handle = None;
    assert_same(&source, &seeds, Outcome::Rows { len: seeds.len() }, &[]);
}

#[test]
fn status_split_failed_part_keeps_the_first_range_reason() {
    let (memory, seeds, _) = distinct_status_world(65);
    let mut packet = StatusPacket::new(seeds.len()).unwrap();
    let mut parts = [
        StatusPart::vacant(),
        StatusPart::vacant(),
        StatusPart::vacant(),
    ];
    split_hour(
        &memory,
        &seeds,
        Outcome::Rows { len: seeds.len() },
        &[],
        &mut packet,
        &mut parts,
    );
    packet.prepare_holding(
        &memory,
        0,
        GS,
        &seeds,
        Outcome::Rows { len: seeds.len() },
        &[],
        &[],
    );
    parts[2].fail("later worker failed");
    parts[1].fail("first worker failed");
    assert_eq!(
        packet.stitch_status_parts(&parts),
        Outcome::Unavailable {
            reason: "first worker failed"
        }
    );
}

#[test]
fn status_split_nonempty_supply_origins_and_footer_faults_rebase_exactly() {
    use crate::cunit_supply_inputs as supply;
    let (mut source, mut seeds, _) = distinct_status_world(65);
    let origins = 0x4200_0000;
    let mut rows = vec![0; 2 * supply::ORIGIN_STRIDE];
    for index in 0..2 {
        p64(
            &mut rows,
            index * supply::ORIGIN_STRIDE + supply::ORIGIN_REQUESTED,
            100_003 + index as u64,
        );
        p64(
            &mut rows,
            index * supply::ORIGIN_STRIDE + supply::ORIGIN_RECEIVED,
            77_007 + index as u64,
        );
        p32(
            &mut rows,
            index * supply::ORIGIN_STRIDE + supply::ORIGIN_PROVINCE,
            17 + index as u32,
        );
    }
    source.globals.insert(origins, rows);
    array(&mut source.bytes, A1 + supply::ORIGINS, origins, 2);
    let primary = source.bytes.len();
    let prototype = source.bytes[A1..A1 + 0x6000].to_vec();
    source.bytes.extend_from_slice(&prototype);
    seeds[33].pointer = primary;
    for address in [
        None,
        Some(primary + supply::ASKED),
        Some(primary + supply::ORIGINS),
        Some(origins + supply::ORIGIN_RECEIVED),
    ] {
        let memory = Fault {
            source: &source,
            address,
        };
        let (packet, rows, _) = serial(&memory, &seeds, Outcome::Rows { len: seeds.len() }, &[]);
        assert_eq!(packet.outcome(), Outcome::Rows { len: seeds.len() });
        assert_eq!(rows.len(), seeds.len());
        assert_same(&memory, &seeds, Outcome::Rows { len: seeds.len() }, &[]);
    }
}

#[test]
fn status_split_exercise_shared_memos_and_faults_match_serial() {
    let (mut source, seeds) = status_world(129);
    source.bytes[G1 + corders_group::TRAINING] = 1;
    let (_, rows, _) = serial(&source, &seeds, Outcome::Rows { len: seeds.len() }, &[]);
    assert_eq!(rows.len(), seeds.len());
    assert!(rows.iter().all(|row| row.exercising == 1));
    for address in [
        None,
        Some(G1 + corders_group::TRAINING),
        Some(GS + cgame_state::HUMAN_STATUS),
        Some(GS + cgame_state::AUTONOMOUS_STATUS),
        Some(0x15001),
        Some(0x15011),
        Some(carmy::EXERCISE_MAXIMUM_LEVEL_RVA.0),
        Some(carmy::EXERCISE_MINIMUM_STRENGTH_RVA.0),
        Some(A1 + carmy::DEPLOYMENT_TYPE),
    ] {
        assert_same(
            &Fault {
                source: &source,
                address,
            },
            &seeds,
            Outcome::Rows { len: seeds.len() },
            &[],
        );
    }
}

#[test]
fn status_split_exercise_manpower_thresholds_and_earlier_scalar_fault_match_serial() {
    let (mut source, seeds) = status_world(129);
    source.bytes[G1 + corders_group::TRAINING] = 1;
    source.bytes[G1 + corders_group::STOP_AT_MAXIMUM] = 1;
    array(&mut source.bytes, A1 + carmy::MANPOWER_DATA, 0x16000, 65);
    for index in 0..65 {
        p32(&mut source.bytes, 0x16004 + index * 8, 1);
    }
    p64(&mut source.bytes, A1 + carmy::EXPERIENCE, 13_000);
    for autonomous in [false, true] {
        source.bytes[0x15011] = u8::from(autonomous);
        for address in [
            None,
            Some(G1 + corders_group::STOP_AT_MAXIMUM),
            Some(A1 + carmy::MANPOWER_DATA),
            Some(0x16004 + 64 * 8),
            Some(carmy::EXPERIENCE_THRESHOLDS_RVA.0),
            Some(0x15100 + 8),
        ] {
            assert_same(
                &Fault {
                    source: &source,
                    address,
                },
                &seeds,
                Outcome::Rows { len: seeds.len() },
                &[],
            );
        }
    }
    let mut failed = seeds;
    failed[1].entrenchment = None;
    failed[64].orders_group = None;
    assert_same(&source, &failed, Outcome::Rows { len: failed.len() }, &[]);
}

#[test]
fn status_split_camouflage_weather_and_shared_category_faults_match_serial() {
    use crate::{acclimatization_category as category, acclimatization_data as data};
    let (mut source, mut seeds) = status_world(129);
    let registry = source.bytes.len();
    let pointers = registry + 0x100;
    let categories = pointers + 0x100;
    source.bytes.resize(categories + 10 * 0x40, 0);
    source.globals.insert(
        category::REGISTRY_RVA.0,
        (registry as u64).to_le_bytes().to_vec(),
    );
    source
        .globals
        .insert(data::CAMO_SHOW_AT_RVA.0, 50_000_i64.to_le_bytes().to_vec());
    array(&mut source.bytes, registry, pointers, 10);
    for index in 0..10 {
        let pointer = categories + index * 0x40;
        p64(&mut source.bytes, pointers + index * 8, pointer as u64);
        p32(
            &mut source.bytes,
            pointer + category::TOKEN,
            index as u32 + 17,
        );
        p64(
            &mut source.bytes,
            pointer + data::CATEGORY_FACTOR,
            50_000 + index as u64,
        );
    }
    for (index, seed) in seeds.iter_mut().enumerate() {
        let category = categories + (index % 10) * 0x40;
        let entries = 0x20000 + index * 0x300;
        p64(&mut source.bytes, entries, category as u64);
        p64(&mut source.bytes, entries + 8, 75_000);
        let object = seed.acclimatization.as_mut().unwrap();
        p64(object, data::ACTIVE, category as u64);
        p64(object, data::MAXIMUM, 100_000);
    }
    let (_, rows, _) = serial(&source, &seeds, Outcome::Rows { len: seeds.len() }, &[]);
    assert_eq!(rows.len(), seeds.len());
    assert!(
        rows.iter()
            .all(|row| row.camouflage.is_some_and(|value| value.category.is_some()))
    );
    assert!(
        rows.iter()
            .all(|row| row.weather_factor.is_some_and(|value| value < 100_000))
    );
    for address in [
        None,
        Some(data::CAMO_SHOW_AT_RVA.0),
        Some(category::REGISTRY_RVA.0),
        Some(pointers + 3 * 8),
        Some(categories + category::TOKEN),
        Some(categories + data::CATEGORY_FACTOR),
        Some(0x20000 + 64 * 0x300 + 8),
    ] {
        assert_same(
            &Fault {
                source: &source,
                address,
            },
            &seeds,
            Outcome::Rows { len: seeds.len() },
            &[],
        );
    }
}

#[test]
fn status_split_organisation_geography_memos_and_route_faults_match_serial() {
    let source = normal_route_fixture();
    let mut first = [carmy::DivisionSeed {
        pointer: A1,
        division: presentation_source()[0].division,
        ..Default::default()
    }];
    carmy::capture_division_seeds(&source, &mut first);
    let seeds: Vec<_> = (0..129)
        .map(|index| {
            let division = handle(9, index + 1);
            carmy::DivisionSeed {
                division,
                handle: Some(division),
                location: Some(P1),
                organisation: Some(i64::MAX),
                naval_transfer_membership: Some(0),
                strategic_redeployment: Some(0),
                ..first[0]
            }
        })
        .collect();
    let (_, rows, refused) = serial(&source, &seeds, Outcome::Rows { len: seeds.len() }, &[]);
    assert_eq!(refused, 0);
    assert_eq!(rows.len(), seeds.len());
    assert!(
        rows.iter()
            .all(|row| row.movement_organisation == Some(None))
    );
    for address in [
        None,
        Some(P1 + cprovince::ID),
        Some(0x12000 + crate::cprovince::definition::TERRAIN_FLAGS),
        Some(GS + cgame_state::PROVINCES),
        Some(PROVINCES + 2 * 8),
        Some(0x18600 + crate::cprovince::definition::ID),
        Some(carmy::organisation_movement::LOW_ORGANISATION_FOR_ATTACK_RVA.0),
    ] {
        assert_same(
            &Fault {
                source: &source,
                address,
            },
            &seeds,
            Outcome::Rows { len: seeds.len() },
            &[],
        );
    }
}

#[test]
fn native_border_join_before_owned_stitch_keeps_exact_rows() {
    let (memory, seeds, held) = distinct_status_world(65);
    let outcome = Outcome::Rows { len: seeds.len() };
    let (mut expected, _, _) = serial(&memory, &seeds, outcome, &held);
    let mut packet = StatusPacket::new(seeds.len()).unwrap();
    let mut parts: Vec<_> = (0..7).map(|_| StatusPart::vacant()).collect();
    split_hour(&memory, &seeds, outcome, &held, &mut packet, &mut parts);
    packet.prepare_holding(&memory, 0, GS, &seeds, outcome, &held, &[]);
    assert_eq!(packet.prepare_status_stitch(&parts), expected.outcome());
    let demanded = [seeds[1].division, seeds[33].division, seeds[64].division];
    assert_eq!(
        packet.capture_border_stats(&memory, 0, demanded),
        expected.capture_border_stats(&memory, 0, demanded)
    );
    packet.finish_status_stitch(&parts);
    let mut actual = vec![carmy::StatusRow::default(); seeds.len()];
    let mut baseline = actual.clone();
    assert_eq!(packet.project(&mut actual), expected.project(&mut baseline));
    assert_eq!(actual, baseline);
}

#[test]
fn staged_maxima_settle_reads_no_staged_block_and_keeps_exact_rows() {
    struct Blocks<'a> {
        source: &'a PresentationMemory,
        blocks: Vec<usize>,
        reads: Cell<usize>,
    }
    impl Memory for Blocks<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self.blocks.contains(&address) {
                self.reads.set(self.reads.get() + 1);
            }
            self.source.copy(address, output)
        }
    }
    let (memory, mut seeds, held) = distinct_status_world(65);
    seeds[6].stat_block = seeds[5].stat_block;
    let outcome = Outcome::Rows { len: seeds.len() };
    for carried in [false, true] {
        let held: Vec<_> = if carried {
            held.clone()
        } else {
            vec![None; seeds.len()]
        };
        let (serial_packet, expected, _) = serial(&memory, &seeds, outcome, &held);
        let staged_seeds: Vec<usize> = (0..seeds.len())
            .filter(|&index| index != 9 && held[index].is_none())
            .collect();
        let pointers: Vec<usize> = staged_seeds
            .iter()
            .map(|&index| seeds[index].stat_block.unwrap())
            .collect();
        let mut reads = vec![Err("unstaged"); pointers.len()];
        crate::csub_unit_definition::stage_division_maxima(&memory, 0, &pointers, &mut reads);
        let mut staged = vec![None; seeds.len()];
        for (&index, read) in staged_seeds.iter().zip(reads) {
            staged[index] = Some(read);
        }
        let blocks = Blocks {
            source: &memory,
            blocks: pointers,
            reads: Cell::new(0),
        };
        let mut packet = StatusPacket::new(seeds.len()).unwrap();
        packet.prepare_holding_staged(&blocks, 0, GS, &seeds, outcome, &held, &[], &|index| {
            staged[index]
        });
        assert_eq!(blocks.reads.get(), 0, "carried={carried}");
        let admitted = packet.prepared_units();
        let mut parts: Vec<_> = (0..3).map(|_| StatusPart::vacant()).collect();
        let count = parts.len();
        for (index, part) in parts.iter_mut().enumerate() {
            part.range = admitted * index / count..admitted * (index + 1) / count;
            part.capture(&memory, 0, GS, &seeds, &packet);
        }
        packet.stitch_status_parts(&parts);
        let mut rows = vec![carmy::StatusRow::default(); seeds.len()];
        let projected = packet.project(&mut rows);
        rows.truncate(projected.len());
        assert_eq!(rows, expected, "carried={carried}");
        for index in 0..seeds.len() {
            assert_eq!(packet.unit_maxima(index), serial_packet.unit_maxima(index));
        }
    }
}
