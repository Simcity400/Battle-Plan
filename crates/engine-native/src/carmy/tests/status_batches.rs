use super::*;
fn manpower_fixture(values: &[i32]) -> PresentationMemory {
    let mut source = presentation_fixture();
    source
        .bytes
        .resize(source.bytes.len().max(0x16000 + values.len() * 8), 0);
    array(
        &mut source.bytes,
        A1 + carmy::MANPOWER_DATA,
        0x16000,
        values.len() as u32,
    );
    for (index, &value) in values.iter().enumerate() {
        p32(&mut source.bytes, 0x16000 + index * 8, 0xfeed_beef);
        p32(&mut source.bytes, 0x16004 + index * 8, value as u32);
    }
    p64(
        &mut source.bytes,
        A1 + carmy::EXPERIENCE,
        values.len() as u64 * 150,
    );
    source
}

fn packet(manpower: usize) -> carmy::StatusPacket {
    carmy::StatusPacket::with_capacities(carmy::StatusCapacities {
        membership: crate::corders_group_member::MembershipCapacity {
            queries: 2,
            groups: 1,
            ..Default::default()
        },
        manpower,
        thresholds: 3,
        modifiers: crate::modifier_accumulator::SnapshotCapacity {
            nodes: 1,
            entries: 1,
        },
        ..carmy::StatusCapacities::new(2)
    })
    .unwrap()
}

#[test]
fn manpower_batches_keep_every_word_and_failure_prefix() {
    for count in [0_usize, 1, 63, 64, 65, 129] {
        let source = manpower_fixture(&vec![1; count]);
        for fault in std::iter::once(None).chain((0..count).map(Some)) {
            let memory = ScalarBatchMemory::new(&source);
            memory.fault.set(fault.map(|index| 0x16004 + index * 8));
            let mut packet = packet(count);
            let allocated = packet.allocated_bytes();
            let result = packet.capture_level(&memory, 0, A1);
            let retained = fault.unwrap_or(count);
            assert_eq!(packet.manpower.len(), retained);
            assert_eq!(packet.manpower, vec![1; retained]);
            assert_eq!(packet.required_capacities().manpower, count);
            assert_eq!(packet.allocated_bytes(), allocated);
            let mut reads = vec![(A1 + carmy::MANPOWER_DATA, 16)];
            reads.extend(
                (0..fault.map_or(count, |index| index + 1)).map(|index| (0x16004 + index * 8, 4)),
            );
            if let Some(index) = fault {
                assert_eq!(result, Err("division presentation field unreadable"));
                assert!(packet.thresholds_header.is_none());
                assert_eq!(memory.batches.get(), index / 64 + 1);
            } else {
                let input = result.unwrap();
                assert_eq!((input.manpower_start, input.manpower_len), (0, count));
                assert_eq!(input.experience, (count != 0).then_some(count as i64 * 150));
                assert_eq!(input.thresholds_len, if count == 0 { 1 } else { 2 });
                assert_eq!(packet.level(input), Ok(i32::from(count != 0)));
                if count != 0 {
                    reads.push((A1 + carmy::EXPERIENCE, 8));
                }
                reads.push((carmy::EXPERIENCE_THRESHOLDS_RVA, 16));
                reads.push((0x15100, 8));
                if count != 0 {
                    reads.push((0x15108, 8));
                }
                assert_eq!(memory.batches.get(), count.div_ceil(64));
            }
            assert_eq!(*memory.reads.borrow(), reads);
        }
    }
}

#[test]
fn manpower_admission_wrapping_and_cached_thresholds_stay_lazy() {
    let source = manpower_fixture(&[1; 65]);
    let memory = ScalarBatchMemory::new(&source);
    let mut refused = packet(64);
    assert_eq!(
        refused.capture_level(&memory, 0, A1),
        Err("division status manpower packet capacity exceeded")
    );
    assert_eq!(*memory.reads.borrow(), [(A1 + carmy::MANPOWER_DATA, 16)]);
    assert_eq!(memory.batches.get(), 0);
    assert_eq!(refused.required_capacities().manpower, 65);
    assert!(refused.manpower.is_empty());

    let mut admitted = packet(130);
    let memory = ScalarBatchMemory::new(&source);
    admitted.capture_level(&memory, 0, A1).unwrap();
    memory.reads.borrow_mut().clear();
    let second = admitted.capture_level(&memory, 0, A1).unwrap();
    assert_eq!((second.manpower_start, second.manpower_len), (65, 65));
    assert_eq!(admitted.required_capacities().manpower, 130);
    let mut expected = vec![(A1 + carmy::MANPOWER_DATA, 16)];
    expected.extend((0..65).map(|index| (0x16004 + index * 8, 4)));
    expected.push((A1 + carmy::EXPERIENCE, 8));
    assert_eq!(*memory.reads.borrow(), expected);

    for values in [vec![-1], vec![i32::MAX, 1], vec![0], vec![40, 60]] {
        let source = manpower_fixture(&values);
        let memory = ScalarBatchMemory::new(&source);
        let mut packet = packet(values.len());
        let input = packet.capture_level(&memory, 0, A1).unwrap();
        assert_eq!(packet.manpower, values);
        let positive = values
            .iter()
            .fold(0_i32, |sum, value| sum.wrapping_add(*value))
            > 0;
        assert_eq!(input.experience.is_some(), positive);
        assert_eq!(
            memory
                .reads
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == A1 + carmy::EXPERIENCE)
                .count(),
            usize::from(positive)
        );
        assert!(!memory.reads.borrow().contains(&(0x15110, 8)));
    }

    let mut source = manpower_fixture(&[1; 2]);
    p64(
        &mut source.bytes,
        A1 + carmy::MANPOWER_DATA,
        usize::MAX as u64 - 8,
    );
    let memory = ScalarBatchMemory::new(&source);
    assert_eq!(
        packet(2).capture_level(&memory, 0, A1),
        Err("division presentation array invalid")
    );
    assert_eq!(*memory.reads.borrow(), [(A1 + carmy::MANPOWER_DATA, 16)]);
    assert_eq!(memory.batches.get(), 0);
}

#[test]
fn status_capture_preserves_primary_word_faults_and_source_admission() {
    let mut source = manpower_fixture(&[1; 65]);
    source.bytes[G1 + corders_group::TRAINING] = 1;
    source.bytes[G1 + corders_group::STOP_AT_MAXIMUM] = 1;
    let mut seeds = [carmy::DivisionSeed {
        pointer: A1,
        division: presentation_source()[0].division,
        ..Default::default()
    }; 2];
    carmy::capture_division_seeds(&source, &mut seeds);
    let memory = ScalarBatchMemory::new(&source);
    let mut captured = packet(65);
    let allocated = captured.allocated_bytes();
    let expected = Outcome::Truncated {
        len: 1,
        unread_rows: 1,
    };
    assert_eq!(
        captured.capture(&memory, 0, GS, &seeds, Outcome::Rows { len: 2 }, 1),
        expected
    );
    assert_eq!(captured.allocated_bytes(), allocated);
    assert_eq!(captured.manpower, vec![1; 65]);
    let trace = memory.reads.borrow().clone();
    for index in 0..65 {
        let fault = 0x16004 + index * 8;
        let ordinal = trace.iter().position(|&span| span == (fault, 4)).unwrap();
        let memory = ScalarBatchMemory::new(&source);
        memory.fault.set(Some(fault));
        let mut failed = packet(65);
        assert_eq!(
            failed.capture(&memory, 0, GS, &seeds, Outcome::Rows { len: 2 }, 1),
            Outcome::Unavailable {
                reason: "division presentation field unreadable"
            }
        );
        assert_eq!(failed.manpower, vec![1; index]);
        assert_eq!(failed.required_capacities().manpower, 65);
        assert_eq!(*memory.reads.borrow(), trace[..=ordinal]);
        assert_eq!(failed.allocated_bytes(), allocated);
    }
    let memory = ScalarBatchMemory::new(&source);
    let mut omitted = packet(65);
    assert_eq!(
        omitted.capture(&memory, 0, GS, &seeds, Outcome::Rows { len: 2 }, 0),
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
    assert!(omitted.manpower.is_empty());
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == A1 + carmy::MANPOWER_DATA)
    );
    let mut rows = [carmy::StatusRow::default()];
    assert_eq!(captured.project(&mut rows), expected);
    assert_eq!(rows[0].division, seeds[0].division);
}

#[test]
fn worker_manpower_and_lazy_thresholds_grow_before_first_and_later_payloads() {
    let mut growing = packet(0);
    growing.enable_growth();
    for count in [5_000, 12_000] {
        growing.clear();
        let mut source = manpower_fixture(&vec![1; count]);
        source.bytes.resize(0x40000 + (count + 1) * 8, 0);
        let mut header = vec![0; 16];
        array(&mut header, 0, 0x40000, count as u32 + 1);
        source
            .globals
            .insert(carmy::EXPERIENCE_THRESHOLDS_RVA, header);
        // Stop at the first strict upper experience threshold. Keep an unread
        // suffix to prove storage growth adds no payload reads.
        p64(&mut source.bytes, 0x40000 + (count - 1) * 8, 151);
        p64(&mut source.bytes, 0x40000 + count * 8, u64::MAX);
        let actual = ScalarBatchMemory::new(&source);
        let result = growing.capture_level(&actual, 0, A1).unwrap();
        assert_eq!((result.manpower_len, result.thresholds_len), (count, count));
        assert_eq!(growing.manpower, vec![1; count]);
        assert!(growing.thresholds.capacity() > count);
        assert!(!actual.reads.borrow().contains(&(0x40000 + count * 8, 8)));
        let expected = ScalarBatchMemory::new(&source);
        let mut fixed = carmy::StatusPacket::with_capacities(carmy::StatusCapacities {
            manpower: count,
            thresholds: count + 1,
            ..carmy::StatusCapacities::new(2)
        })
        .unwrap();
        let reference = fixed.capture_level(&expected, 0, A1).unwrap();
        assert_eq!(reference.thresholds_len, result.thresholds_len);
        assert_eq!(*actual.reads.borrow(), *expected.reads.borrow());
        growing.clear();
        let fault = 0x16004 + (count - 1) * 8;
        let actual = ScalarBatchMemory::new(&source);
        actual.fault.set(Some(fault));
        assert_eq!(
            growing.capture_level(&actual, 0, A1),
            Err("division presentation field unreadable")
        );
        assert_eq!(growing.manpower.len(), count - 1);
        assert_eq!(
            actual
                .reads
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == fault)
                .count(),
            1
        );
        assert!(growing.thresholds_header.is_none());
    }
}

#[test]
fn worker_acclimatization_registry_grows_from_original_5000_and_later_headers() {
    use crate::{acclimatization_category as category, acclimatization_data as data};
    let mut growing = packet(0);
    growing.enable_growth();
    for count in [5_000, 12_000] {
        growing.clear();
        let mut source = presentation_fixture();
        source.bytes.resize(0x80000 + count * 0x40, 0);
        let object = A1 + carmy::ACCLIMATIZATION;
        array(
            &mut source.bytes,
            object + data::DATA,
            0x30000,
            count as u32,
        );
        p64(&mut source.bytes, object + data::MAXIMUM, 250_000);
        source
            .globals
            .insert(category::REGISTRY_RVA, 0x20000_u64.to_le_bytes().to_vec());
        source
            .globals
            .insert(data::CAMO_SHOW_AT_RVA, 50_000_i64.to_le_bytes().to_vec());
        array(&mut source.bytes, 0x20000, 0x60000, count as u32);
        for index in 0..count {
            let pointer = 0x80000 + index * 0x40;
            p64(&mut source.bytes, 0x30000 + index * 16, pointer as u64);
            p64(&mut source.bytes, 0x30008 + index * 16, index as u64 * 100);
            p64(&mut source.bytes, 0x60000 + index * 8, pointer as u64);
            p32(
                &mut source.bytes,
                pointer + category::TOKEN,
                index as u32 + 1,
            );
        }
        let mut seeds = [carmy::DivisionSeed {
            pointer: A1,
            division: presentation_source()[0].division,
            ..Default::default()
        }];
        carmy::capture_division_seeds(&source, &mut seeds);
        let actual = ScalarBatchMemory::new(&source);
        // The last entry weighs most and every entry is read.
        let last = carmy::Camouflage {
            category: Some(count as u32),
        };
        let (object, winner) = carmy::acclimatization_winner(&actual, 0, &seeds[0], None).unwrap();
        assert_eq!(
            growing.read_camouflage(&actual, 0, &object, winner),
            Ok(last)
        );
        assert_eq!(
            growing.required_capacities().acclimatization_categories,
            count
        );
        let expected = ScalarBatchMemory::new(&source);
        let mut fixed = carmy::StatusPacket::with_capacities(carmy::StatusCapacities {
            acclimatization_categories: count,
            ..carmy::StatusCapacities::new(2)
        })
        .unwrap();
        let (object, winner) =
            carmy::acclimatization_winner(&expected, 0, &seeds[0], None).unwrap();
        assert_eq!(
            fixed.read_camouflage(&expected, 0, &object, winner),
            Ok(last)
        );
        assert_eq!(*actual.reads.borrow(), *expected.reads.borrow());
    }
}

#[test]
fn acquired_status_demand_excludes_allocator_spare_capacity() {
    const COUNT: usize = 5_001;
    let source = manpower_fixture(&vec![1; COUNT]);
    let mut packet = carmy::StatusPacket::with_capacities(carmy::StatusCapacities {
        units: 1,
        ..Default::default()
    })
    .unwrap();
    packet.enable_growth();
    packet.manpower.reserve(30_000);
    packet.thresholds.reserve(30_000);
    for _ in 0..20 {
        packet.clear();
        let input = packet.capture_level(&source, 0, A1).unwrap();
        assert_eq!(input.manpower_len, COUNT);
        assert_eq!(packet.capacities.manpower, COUNT);
        assert_eq!(packet.capacities.thresholds, 3);
        assert_eq!(packet.required_capacities().manpower, COUNT);
    }
}

/// A seed's carried maxima stand in for a read of its stat block, as the
/// read would; maxima of another block are not the seed's and are ignored.
#[test]
fn carried_maxima_replace_the_stat_block_read() {
    use crate::csub_unit_definition::DivisionMaxima;
    let source = manpower_fixture(&[1; 2]);
    let mut seeds = [carmy::DivisionSeed {
        pointer: A1,
        division: presentation_source()[0].division,
        ..Default::default()
    }; 2];
    carmy::capture_division_seeds(&source, &mut seeds);
    let stats = seeds[0].stat_block.unwrap();
    let capture = |held: &[Option<DivisionMaxima>]| {
        let memory = ScalarBatchMemory::new(&source);
        let mut packet = packet(65);
        let outcome =
            packet.capture_holding(&memory, 0, GS, &seeds, Outcome::Rows { len: 2 }, 1, held);
        let reads = memory.reads.borrow().clone();
        (outcome, packet.unit_maxima(0), reads)
    };
    let (outcome, fresh, reads) = capture(&[]);
    let fresh = fresh.unwrap();
    assert_eq!(fresh.pointer, stats);
    assert!(reads.iter().any(|&(at, _)| at == stats));
    let held = DivisionMaxima {
        // Any other value: the fixture's maximum sits at the top of the range.
        organisation: fresh.organisation.wrapping_sub(7),
        ..fresh
    };
    let (carried_outcome, carried, reads) = capture(&[Some(held)]);
    assert_eq!(carried_outcome, outcome);
    assert_eq!(carried, Some(held));
    use crate::csub_unit_definition::{CAN_EXFILTRATE_FROM_COAST, MAXIMUM_ORGANISATION};
    for at in [
        stats,
        stats + MAXIMUM_ORGANISATION,
        stats + CAN_EXFILTRATE_FROM_COAST,
    ] {
        assert!(!reads.iter().any(|read| read.0 == at), "{at:x}");
    }
    let other = DivisionMaxima {
        pointer: stats + 0x10,
        ..held
    };
    assert_eq!(capture(&[Some(other)]).1, Some(fresh));
}
