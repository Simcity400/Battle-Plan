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
        modifiers: engine_native_parts::modifier_accumulator::SnapshotCapacity {
            nodes: 2,
            entries: 1,
        },
        ..carmy::StatusCapacities::new(2)
    })
    .unwrap()
}

#[test]
fn manpower_appends_wrapping_and_cached_thresholds_stay_lazy() {
    let source = manpower_fixture(&[1; 65]);
    let mut admitted = packet(130);
    let memory = ScalarBatchMemory::new(&source);
    admitted.capture_level(&memory, 0, A1, None).unwrap();
    memory.reads.borrow_mut().clear();
    let second = admitted.capture_level(&memory, 0, A1, None).unwrap();
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
        let input = packet.capture_level(&memory, 0, A1, None).unwrap();
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
        packet(2).capture_level(&memory, 0, A1, None),
        Err("division presentation array invalid")
    );
    assert_eq!(*memory.reads.borrow(), [(A1 + carmy::MANPOWER_DATA, 16)]);
    assert_eq!(memory.batches.get(), 0);
}

#[test]
fn worker_manpower_and_lazy_thresholds_grow_before_first_and_later_payloads() {
    let mut growing = packet(0);
    for count in [50, 120] {
        growing.clear();
        let mut source = manpower_fixture(&vec![1; count]);
        source.bytes.resize(0x40000 + (count + 1) * 8, 0);
        let mut header = vec![0; 16];
        array(&mut header, 0, 0x40000, count as u32 + 1);
        source
            .globals
            .insert(carmy::EXPERIENCE_THRESHOLDS_RVA.0, header);
        p64(&mut source.bytes, 0x40000 + (count - 1) * 8, 151);
        p64(&mut source.bytes, 0x40000 + count * 8, u64::MAX);
        let actual = ScalarBatchMemory::new(&source);
        let result = growing.capture_level(&actual, 0, A1, None).unwrap();
        assert_eq!((result.manpower_len, result.thresholds_len), (count, count));
        assert_eq!(growing.manpower, vec![1; count]);
        assert!(growing.thresholds.capacity() > count);
        assert!(!actual.reads.borrow().contains(&(0x40000 + count * 8, 8)));
        let expected = ScalarBatchMemory::new(&source);
        let mut presized = carmy::StatusPacket::with_capacities(carmy::StatusCapacities {
            manpower: count,
            thresholds: count + 1,
            ..carmy::StatusCapacities::new(2)
        })
        .unwrap();
        let reference = presized.capture_level(&expected, 0, A1, None).unwrap();
        assert_eq!(reference.thresholds_len, result.thresholds_len);
        assert_eq!(*actual.reads.borrow(), *expected.reads.borrow());
        growing.clear();
        let fault = 0x16004 + (count - 1) * 8;
        let actual = ScalarBatchMemory::new(&source);
        actual.fault.set(Some(fault));
        assert_eq!(
            growing.capture_level(&actual, 0, A1, None),
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
fn worker_acclimatization_registry_grows_from_original_and_later_headers() {
    use engine_native_parts::{
        acclimatization_category as category,
        acclimatization_data as data,
    };
    let mut growing = packet(0);
    for count in [50, 120] {
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
            .insert(category::REGISTRY_RVA.0, 0x20000_u64.to_le_bytes().to_vec());
        source
            .globals
            .insert(data::CAMO_SHOW_AT_RVA.0, 50_000_i64.to_le_bytes().to_vec());
        array(&mut source.bytes, 0x20000, 0x60000, count as u32);
        for index in 0..count {
            let pointer = 0x80000 + index * 0x40;
            p64(&mut source.bytes, 0x30000 + index * 16, pointer as u64);
            p64(
                &mut source.bytes,
                0x30008 + index * 16,
                index as u64 * 10_000,
            );
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
        let mut presized = carmy::StatusPacket::with_capacities(carmy::StatusCapacities {
            acclimatization_categories: count,
            ..carmy::StatusCapacities::new(2)
        })
        .unwrap();
        let (object, winner) =
            carmy::acclimatization_winner(&expected, 0, &seeds[0], None).unwrap();
        assert_eq!(
            presized.read_camouflage(&expected, 0, &object, winner),
            Ok(last)
        );
        assert_eq!(*actual.reads.borrow(), *expected.reads.borrow());
    }
}

#[test]
fn unavailable_air_supply_is_local_to_the_noncore_movement_operand() {
    let source = manpower_fixture(&[1; 2]);
    for (exiled, bypass) in [(0, 0), (1, 0), (0, 1)] {
        let mut seed = carmy::DivisionSeed {
            pointer: A1,
            division: presentation_source()[0].division,
            ..Default::default()
        };
        carmy::capture_division_seeds(&source, std::slice::from_mut(&mut seed));
        seed.air_supply = None;
        seed.exiled = Some(exiled);
        seed.noncore_supply_bypass = Some(bypass);
        let mut packet = packet(65);
        let outcome = packet.capture_holding(
            &source,
            0,
            GS,
            &[seed],
            Outcome::Rows { len: 1 },
            &[],
            &[],
        );
        assert_eq!(outcome, Outcome::Rows { len: 1 });
        assert_eq!(packet.units[0].air_supply, None);
        let mut rows = [carmy::StatusRow::default()];
        assert_eq!(
            carmy::project_status(&packet, &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_ne!(
            rows[0].move_flags & carmy::StatusRow::MOVE_NONCORE_AIR_REFUSED,
            0
        );
    }
}
