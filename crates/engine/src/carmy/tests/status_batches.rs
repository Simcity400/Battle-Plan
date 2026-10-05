use super::*;
fn manpower_fixture(values: &[i32]) -> PresentationMemory {
    let mut source = presentation_fixture();
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
        manpower,
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
