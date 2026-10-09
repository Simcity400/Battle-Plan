use super::*;

static EXCLUSIVE: Mutex<()> = Mutex::new(());

#[test]
fn sizing_retries_a_missing_world_and_grows_receipt_pressure_after_a_failed_census() {
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    struct NullState;
    impl engine::Memory for NullState {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == engine::cgame_state::SINGLETON_RVA && output.len() == 8 {
                output.fill(0);
                return true;
            }
            false
        }
    }
    for _ in 0..24 {
        presize(&NullState, 0);
    }
    assert_eq!(capacities(), (0, 0));
    offer(1, 1);
    install_offered();
    {
        let mut storage = STORAGE.lock().unwrap();
        let current = storage.current.as_mut().unwrap();
        current.entries_required = 5_001;
        current.names_required = 30_000;
        let log = Log::new(1);
        let mut entries = [Entry::default()];
        let (_, _, required) = handover(&log, current, &mut entries, &mut [], None).unwrap();
        assert_eq!(required, 30_000);
        assert_eq!(current.names_required, 0);
    }
    let unreadable = engine::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    presize(&unreadable, 0);
    let (entries, names) = capacities();
    assert!(entries >= 5_001 && names >= 30_000);
    assert_eq!(
        *CENSUS_LAST_REFUSAL.lock().unwrap(),
        Some("input demand game state unreadable or address overflow")
    );
    release().unwrap();
}

#[test]
fn independent_fuel_offers_preserve_pending_entry_and_name_growth() {
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    offer(8, 8);
    install_offered();
    offer(9, 0);
    assert_eq!(capacities(), (16, 8));
    offer(0, 9);
    assert_eq!(capacities(), (16, 16));
    let pointer = STORAGE
        .lock()
        .unwrap()
        .offered
        .as_ref()
        .unwrap()
        .entries
        .as_ptr();
    for _ in 0..32 {
        offer(9, 0);
        offer(0, 9);
        collect();
        assert_eq!(capacities(), (16, 16));
        assert_eq!(
            STORAGE
                .lock()
                .unwrap()
                .offered
                .as_ref()
                .unwrap()
                .entries
                .as_ptr(),
            pointer
        );
    }
    crate::allocation_failure_tests::with_allocations_denied(install_offered);
    assert_eq!(capacities(), (16, 16));
    assert!(STORAGE.lock().unwrap().retired.is_some());
    collect();
    assert!(STORAGE.lock().unwrap().retired.is_none());
    release().unwrap();
}

struct Memory(Vec<u8>);
impl engine::Memory for Memory {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let Some(end) = address.checked_add(output.len()) else {
            return false;
        };
        let Some(bytes) = self.0.get(address..end) else {
            return false;
        };
        output.copy_from_slice(bytes);
        true
    }
}
fn name(memory: &mut Memory, bytes: &[u8]) -> native::StoredName {
    memory.0[0x358..0x368].fill(0);
    memory.0[0x358..0x358 + bytes.len()].copy_from_slice(bytes);
    memory.0[0x368..0x370].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
    memory.0[0x370..0x378].copy_from_slice(&15_u64.to_le_bytes());
    native::stored_name(memory, 0x100, 2).unwrap()
}

#[test]
fn stored_names_and_signed_awards_survive_short_handover_and_later_rename() {
    let log = Log::new(2);
    let mut buffer = Buffer::new(2, 16, Default::default()).unwrap();
    let mut memory = Memory(vec![0; 0x400]);
    let first = name(&mut memory, b"before");
    append_buffer(
        &log,
        &mut buffer,
        Entry {
            amount: 0,
            ..Default::default()
        },
        Some(first),
        &memory,
    );
    let second = name(&mut memory, b"after");
    append_buffer(
        &log,
        &mut buffer,
        Entry {
            amount: -9,
            ..Default::default()
        },
        Some(second),
        &memory,
    );
    let mut output = [Entry::default()];
    let mut names = [0; 16];
    let (drained, len, required) =
        handover(&log, &mut buffer, &mut output, &mut names, None).unwrap();
    assert_eq!(
        (drained.copied, drained.pending, len, required),
        (1, 2, 11, 11)
    );
    assert_eq!(output[0].amount, 0);
    assert_eq!(&names[..6], b"before");
    assert_eq!(buffer.names_len, 11);
    let (drained, len, _) = handover(&log, &mut buffer, &mut output, &mut names, None).unwrap();
    assert_eq!((drained.first, drained.cursor, len), (1, 2, 11));
    assert_eq!(output[0].amount, -9);
    assert_eq!(output[0].name, Some((6, 5)));
    assert_eq!(&names[6..11], b"after");
    assert_eq!(buffer.names_len, 0);
    assert_eq!(buffer.names_required, 0);
}

#[test]
fn name_shortage_keeps_money_and_refuses_text_without_truncating_it() {
    let log = Log::new(1);
    let mut buffer = Buffer::new(1, 2, Default::default()).unwrap();
    let mut memory = Memory(vec![0; 0x400]);
    let stored = name(&mut memory, b"long");
    append_buffer(
        &log,
        &mut buffer,
        Entry {
            amount: -1,
            ..Default::default()
        },
        Some(stored),
        &memory,
    );
    let mut output = [Entry::default()];
    let mut names = [0; 2];
    let (_, _, required) = handover(&log, &mut buffer, &mut output, &mut names, None).unwrap();
    assert_eq!(required, 4);
    assert_eq!(output[0].amount, -1);
    assert_eq!(output[0].name, None);
    assert_eq!(
        output[0].name_reason,
        Some("fuel recipient stored name does not fit")
    );
    log.invalidate();
    assert_eq!(log.epoch.load(Ordering::SeqCst), 2);
}

#[test]
fn fuel_buffer_charge_follows_offer_live_retirement_and_actual_release() {
    static LOCAL: crate::raw_memory::Tracker = crate::raw_memory::Tracker::new();
    let buffer = Buffer::new(2, 16, crate::raw_memory::Charge::empty_on(&LOCAL)).unwrap();
    let bytes = LOCAL.snapshot().current.payload_vectors;
    assert!(bytes >= (2 * size_of::<Entry>() + 16) as u64);
    let mut storage = Storage {
        current: None,
        offered: Some(Box::new(buffer)),
        retired: None,
    };
    storage.current = storage.offered.take();
    assert_eq!(LOCAL.snapshot().current.payload_vectors, bytes);
    storage.retired = storage.current.take();
    assert_eq!(LOCAL.snapshot().current.payload_vectors, bytes);
    drop(storage.retired.take());
    assert_eq!(LOCAL.snapshot().current.payload_vectors, 0);
}

#[test]
fn lost_reset_overflow_withholds_interval_and_retains_actual_growth_demand() {
    let log = Log::new(1);
    let mut buffer = Buffer::new(1, 0, Default::default()).unwrap();
    let memory = Memory(Vec::new());
    append_buffer(
        &log,
        &mut buffer,
        Entry {
            amount: 1,
            ..Default::default()
        },
        None,
        &memory,
    );
    append_buffer(
        &log,
        &mut buffer,
        Entry {
            reset: true,
            started: native::Started {
                status: 0x1000,
                ..Default::default()
            },
            ..Default::default()
        },
        None,
        &memory,
    );
    append_buffer(
        &log,
        &mut buffer,
        Entry {
            amount: 2,
            ..Default::default()
        },
        None,
        &memory,
    );
    let mut output = [Entry::default()];
    let (drained, _, _) = handover(&log, &mut buffer, &mut output, &mut [], None).unwrap();
    assert_eq!(drained.copied, 0);
    assert_eq!(drained.pending, 3);
    assert_eq!(buffer.entries_required, 3);
}
