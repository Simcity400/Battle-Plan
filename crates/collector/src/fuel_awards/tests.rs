use super::*;

static EXCLUSIVE: Mutex<()> = Mutex::new(());

#[test]
fn worker_handover_grows_original_five_thousand_country_receipts_and_mod_name_bytes() {
    let mut entries = Vec::new();
    let mut names = Vec::new();
    for count in [5000, 12_000] {
        let log = Log::new(count);
        let buffer = Buffer::with_stride(count, count * 48, 48, Default::default()).unwrap();
        let epoch = log.epoch.load(Ordering::SeqCst);
        let mut memory = Memory(vec![0; 0x431]);
        memory.0[0x358..0x360].copy_from_slice(&0x400_u64.to_le_bytes());
        memory.0[0x368..0x370].copy_from_slice(&48_u64.to_le_bytes());
        memory.0[0x370..0x378].copy_from_slice(&48_u64.to_le_bytes());
        for index in 0..count {
            memory.0[0x400..0x430].fill((index % 251) as u8);
            assert!(append_buffer(
                &log,
                &buffer,
                Entry {
                    country: (index % 5000) as u32,
                    amount: -(index as i64),
                    ..Default::default()
                },
                Some(native::stored_name(&memory, 0x100, 2).unwrap()),
                &memory,
            ));
        }
        let (drained, copied_names, required) = handover_buffers(
            &log,
            &buffer,
            None,
            engine::walk::Buffer::Growing(&mut entries),
            engine::walk::Buffer::Growing(&mut names),
            None,
        )
        .unwrap();
        assert_eq!(
            (drained.copied, drained.pending, copied_names, required),
            (count, count, count * 48, count * 48)
        );
        assert_eq!(entries[count - 1].country, ((count - 1) % 5000) as u32);
        assert_eq!(entries[count - 1].amount, -((count - 1) as i64));
        assert_eq!(entries[count - 1].name, Some(((count - 1) * 48, 48)));
        assert_eq!(
            &names[(count - 1) * 48..count * 48],
            &[((count - 1) % 251) as u8; 48]
        );
        assert_eq!(log.consumed.load(Ordering::Acquire), count as u64);
        assert_eq!(log.epoch.load(Ordering::SeqCst), epoch);
        assert!(drained.complete());
    }
}

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
    assert!(capacities().0 >= 1);
    assert_eq!(capacities().1, 0);
    offer(1, 1);
    install_offered();
    {
        let mut current = CURRENT.lock().unwrap();
        let current = current.as_mut().unwrap();
        current.entries_required.store(5_001, Ordering::Relaxed);
        current.names_demand.store(30_000, Ordering::Relaxed);
        ENTRY_DEMAND.store(
            current.entries_required.load(Ordering::Relaxed),
            Ordering::Release,
        );
        NAME_DEMAND.store(
            current.names_demand.load(Ordering::Relaxed),
            Ordering::Release,
        );
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
    assert_eq!(capacities(), (16, 16));
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
    let buffer = Buffer::new(2, 16, Default::default()).unwrap();
    let mut memory = Memory(vec![0; 0x400]);
    let first = name(&mut memory, b"before");
    append_buffer(
        &log,
        &buffer,
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
        &buffer,
        Entry {
            amount: -9,
            ..Default::default()
        },
        Some(second),
        &memory,
    );
    let mut output = [Entry::default()];
    let mut names = [0; 16];
    let (drained, len, required) = handover(&log, &buffer, &mut output, &mut names, None).unwrap();
    assert_eq!(
        (drained.copied, drained.pending, len, required),
        (1, 2, 6, 11)
    );
    assert_eq!(output[0].amount, 0);
    assert_eq!(&names[..6], b"before");
    let (drained, len, _) = handover(&log, &buffer, &mut output, &mut names, None).unwrap();
    assert_eq!((drained.first, drained.cursor, len), (1, 2, 5));
    assert_eq!(output[0].amount, -9);
    assert_eq!(output[0].name, Some((0, 5)));
    assert_eq!(&names[..5], b"after");
}

#[test]
fn name_shortage_keeps_money_and_refuses_text_without_truncating_it() {
    let log = Log::new(1);
    let buffer = Buffer::new(1, 2, Default::default()).unwrap();
    let mut memory = Memory(vec![0; 0x400]);
    let stored = name(&mut memory, b"long");
    append_buffer(
        &log,
        &buffer,
        Entry {
            amount: -1,
            ..Default::default()
        },
        Some(stored),
        &memory,
    );
    let mut output = [Entry::default()];
    let mut names = [0; 2];
    let (_, _, required) = handover(&log, &buffer, &mut output, &mut names, None).unwrap();
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
        offered: Some(Box::new(buffer)),
        retired: None,
    };
    let mut current = storage.offered.take();
    assert_eq!(LOCAL.snapshot().current.payload_vectors, bytes);
    storage.retired = current.take();
    assert_eq!(LOCAL.snapshot().current.payload_vectors, bytes);
    drop(storage.retired.take());
    assert_eq!(LOCAL.snapshot().current.payload_vectors, 0);
}

#[test]
fn lost_reset_overflow_withholds_interval_and_retains_actual_growth_demand() {
    let _guard = EXCLUSIVE.lock().unwrap();
    let settled_before = COUNTERS[2].load(Ordering::Relaxed);
    let overflow_before = COUNTERS[10].load(Ordering::Relaxed);
    let log = Log::new(1);
    let buffer = Buffer::new(1, 0, Default::default()).unwrap();
    let memory = Memory(Vec::new());
    append_buffer(
        &log,
        &buffer,
        Entry {
            amount: 1,
            ..Default::default()
        },
        None,
        &memory,
    );
    append_buffer(
        &log,
        &buffer,
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
        &buffer,
        Entry {
            amount: 2,
            ..Default::default()
        },
        None,
        &memory,
    );
    let mut output = [Entry::default()];
    let (drained, _, _) = handover(&log, &buffer, &mut output, &mut [], None).unwrap();
    assert_eq!(drained.copied, 0);
    assert_eq!(drained.pending, 3);
    assert_eq!(buffer.entries_required.load(Ordering::Relaxed), 3);
    assert_eq!(COUNTERS[2].load(Ordering::Relaxed), settled_before);
    assert_eq!(COUNTERS[10].load(Ordering::Relaxed), overflow_before + 1);
}

fn started(root: u64) -> Pending {
    Pending {
        root,
        epoch: LOG.epoch.load(Ordering::SeqCst),
        started: native::Started {
            country: root as u32,
            ..Default::default()
        },
        received: [0; 3],
        refused: false,
        cause: None,
    }
}

#[test]
fn worker_storage_lease_cannot_lose_native_recipient_or_terminal_root() {
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    let mut memory = Memory(vec![0; 0x400]);
    let stored = name(&mut memory, b"retained");
    NAME_STRIDE.store(stored.byte_len(), Ordering::Release);
    offer(2, 8);
    install_offered();
    let busy_before = COUNTERS[6].load(Ordering::Relaxed);
    let epoch = LOG.epoch.load(Ordering::SeqCst);
    let worker = STORAGE.lock().unwrap();
    crate::allocation_failure_tests::with_allocations_denied(|| {
        assert!(append(
            Entry {
                root: 1,
                recipient: Some(native::Recipient::default()),
                amount: -7,
                ..Default::default()
            },
            Some(stored),
            &memory,
        ));
        append_root(started(1), native::Settled::default(), &memory);
    });
    drop(worker);
    assert_eq!(COUNTERS[6].load(Ordering::Relaxed), busy_before);
    assert_eq!(LOG.epoch.load(Ordering::SeqCst), epoch);
    let mut entries = [Entry::default(); 2];
    let mut names = [0; 8];
    LOG.enabled.store(true, Ordering::SeqCst);
    let (drained, len, required) = drain(&mut entries, &mut names).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!(
        (drained.copied, drained.pending, len, required),
        (2, 2, 8, 8)
    );
    assert_eq!(entries[0].amount, -7);
    assert_eq!(entries[1].root, 1);
    assert!(entries[1].recipient.is_none() && !entries[1].refused);
    assert_eq!(&names, b"retained");
    release().unwrap();
}

#[test]
fn hourly_storage_lease_cannot_lose_recipient_terminal_root_or_reset() {
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    offer(4, 0);
    install_offered();
    let before = APPEND_REFUSALS
        .each_ref()
        .map(|value| value.load(Ordering::Relaxed));
    let epoch = LOG.epoch.load(Ordering::SeqCst);
    let current = CURRENT.lock().unwrap();
    let (recipient, reset) = crate::allocation_failure_tests::with_allocations_denied(|| {
        let recipient = append(
            Entry {
                recipient: Some(native::Recipient::default()),
                amount: -7,
                ..Default::default()
            },
            None,
            &Memory(Vec::new()),
        );
        append_root(started(1), native::Settled::default(), &Memory(Vec::new()));
        let reset = append(
            Entry {
                reset: true,
                started: native::Started {
                    status: 0x1000,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            &Memory(Vec::new()),
        );
        (recipient, reset)
    });
    drop(current);
    assert!(
        recipient,
        "hourly drain ownership lost the native recipient"
    );
    assert!(reset, "hourly drain ownership lost the native reset");
    assert_eq!(
        APPEND_REFUSALS
            .each_ref()
            .map(|value| value.load(Ordering::Relaxed)),
        before
    );
    assert_eq!(LOG.epoch.load(Ordering::SeqCst), epoch);
    LOG.enabled.store(true, Ordering::SeqCst);
    let mut entries = [Entry::default(); 4];
    let (drained, _, _) = drain(&mut entries, &mut []).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!((drained.copied, drained.pending), (3, 3));
    assert_eq!(entries[0].amount, -7);
    assert_eq!(entries[1].root, 1);
    assert!(entries[2].reset);
    release().unwrap();
}

#[test]
fn rejected_5000_country_receipt_burst_preserves_growth_after_failed_census() {
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    offer(1, 1);
    install_offered();
    let mut memory = Memory(vec![0; 0x400]);
    let _ = name(&mut memory, b"recorded");
    let current = CURRENT.lock().unwrap();
    crate::allocation_failure_tests::with_allocations_denied(|| {
        for country in 1..=5000 {
            let stored = native::stored_name(&memory, 0x100, 2).unwrap();
            assert_eq!(
                append(
                    Entry {
                        country,
                        recipient: Some(native::Recipient::default()),
                        ..Default::default()
                    },
                    Some(stored),
                    &memory,
                ),
                country == 1
            );
            append_root(started(country as u64), native::Settled::default(), &memory);
        }
    });
    drop(current);
    assert_eq!(ENTRY_DEMAND.load(Ordering::Acquire), 10_000);
    assert_eq!(NAME_DEMAND.load(Ordering::Acquire), 40_000);
    let unreadable = engine::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    presize(&unreadable, 0);
    let (entries, names) = capacities();
    assert!(entries >= 10_000 && names >= 40_000);
    let mut output = [Entry::default()];
    LOG.enabled.store(true, Ordering::SeqCst);
    let (drained, _, required) = drain(&mut output, &mut []).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!(
        (drained.copied, drained.pending, required),
        (0, 10_000, 40_000)
    );
    crate::allocation_failure_tests::with_allocations_denied(install_offered);
    collect();
    for _ in 0..2 {
        assert!(append(Entry::default(), None, &memory));
    }
    LOG.enabled.store(true, Ordering::SeqCst);
    let (drained, _, required) = drain(&mut output, &mut []).unwrap();
    assert_eq!((drained.copied, drained.pending, required), (1, 2, 0));
    let (drained, _, _) = drain(&mut output, &mut []).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!((drained.copied, drained.pending), (1, 1));
    assert_eq!(ENTRY_DEMAND.load(Ordering::Acquire), 10_000);
    assert_eq!(NAME_DEMAND.load(Ordering::Acquire), 40_000);
    release().unwrap();
}

#[test]
fn missing_live_storage_names_every_lost_append_lane_without_allocation() {
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    let before = APPEND_REFUSALS
        .each_ref()
        .map(|counter| counter.load(Ordering::Relaxed));
    let absent_before = COUNTERS[11].load(Ordering::Relaxed);
    crate::allocation_failure_tests::with_allocations_denied(|| {
        let memory = Memory(Vec::new());
        assert!(!append(
            Entry {
                recipient: Some(native::Recipient::default()),
                ..Default::default()
            },
            None,
            &memory
        ));
        append_root(started(1), native::Settled::default(), &memory);
        assert!(!append(
            Entry {
                reset: true,
                ..Default::default()
            },
            None,
            &memory
        ));
    });
    for (lane, previous) in before.into_iter().enumerate() {
        assert_eq!(APPEND_REFUSALS[lane].load(Ordering::Relaxed), previous + 1);
        assert!(
            append_reason(lane + 4)
                .unwrap()
                .ends_with("live storage absent")
        );
    }
    assert_eq!(COUNTERS[11].load(Ordering::Relaxed), absent_before + 3);
    release().unwrap();
}

#[test]
fn empty_drain_preserves_attempts_of_an_active_native_callback() {
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    offer(2, 0);
    install_offered();
    let active = LOG.enter();
    ENTRY_ATTEMPTS.store(1, Ordering::SeqCst);
    ENTRY_DEMAND.store(1, Ordering::Release);
    LOG.cursor.fetch_add(1, Ordering::SeqCst);
    let mut output = [Entry::default(); 2];
    LOG.enabled.store(true, Ordering::SeqCst);
    let (drained, _, _) = drain(&mut output, &mut []).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!((drained.copied, drained.pending), (0, 1));
    assert_eq!(ENTRY_CONSUMED.load(Ordering::Relaxed), 0);
    drop(active);
    release().unwrap();
}

#[test]
fn five_thousand_country_reset_burst_survives_the_hourly_owner_lease() {
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    offer(5000, 0);
    install_offered();
    let epoch = LOG.epoch.load(Ordering::SeqCst);
    let current = CURRENT.lock().unwrap();
    crate::allocation_failure_tests::with_allocations_denied(|| {
        for country in 1..=5000 {
            assert!(append(
                Entry {
                    country,
                    reset: true,
                    started: native::Started {
                        status: country as usize * 0x1000,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                None,
                &Memory(Vec::new())
            ));
        }
    });
    drop(current);
    let mut entries = vec![Entry::default(); 5000];
    LOG.enabled.store(true, Ordering::SeqCst);
    let (drained, _, _) = drain(&mut entries, &mut []).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!((drained.copied, drained.pending), (5000, 5000));
    assert_eq!(LOG.epoch.load(Ordering::SeqCst), epoch);
    assert!(
        entries
            .iter()
            .enumerate()
            .all(|(index, entry)| entry.reset && entry.country as usize == index + 1)
    );
    release().unwrap();
}
#[test]
fn stale_callback_pointer_follows_prepared_growth_without_epoch_loss() {
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    offer(1, 0);
    install_offered();
    let stale = LIVE.load(Ordering::SeqCst);
    offer(2, 0);
    install_offered();
    assert_ne!(LIVE.load(Ordering::SeqCst), stale);
    let epoch = LOG.epoch.load(Ordering::SeqCst);
    let call = LOG.enter();
    let lease = publishing(unsafe { &*stale }).unwrap();
    assert_eq!(
        lease.0 as *const Buffer as *mut Buffer,
        LIVE.load(Ordering::SeqCst)
    );
    crate::allocation_failure_tests::with_allocations_denied(|| {
        assert!(append_buffer(
            &LOG,
            lease.0,
            Entry {
                amount: -19,
                ..Default::default()
            },
            None,
            &Memory(Vec::new())
        ));
    });
    drop(lease);
    drop(call);
    LOG.enabled.store(true, Ordering::SeqCst);
    let mut entries = [Entry::default(); 2];
    let (drained, _, _) = drain(&mut entries, &mut []).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!(drained.copied, 1);
    assert_eq!(entries[0].amount, -19);
    assert_eq!(LOG.epoch.load(Ordering::SeqCst), epoch);
    collect();
    release().unwrap();
}

#[test]
fn publication_between_empty_check_and_seal_survives_growth_and_collection() {
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    offer(1, 0);
    install_offered();
    offer(2, 0);
    let epoch = LOG.epoch.load(Ordering::SeqCst);
    {
        let mut current = CURRENT.lock().unwrap();
        let mut storage = STORAGE.lock().unwrap();
        // install_offered observed quiet/empty before this native publication.
        assert!(append(
            Entry {
                amount: 17,
                ..Default::default()
            },
            None,
            &Memory(Vec::new())
        ));
        crate::allocation_failure_tests::with_allocations_denied(|| {
            install_prepared(&mut current, &mut storage);
        });
    }
    collect();
    assert!(STORAGE.lock().unwrap().retired.is_some());
    assert!(append(
        Entry {
            amount: -23,
            ..Default::default()
        },
        None,
        &Memory(Vec::new())
    ));
    LOG.enabled.store(true, Ordering::SeqCst);
    let mut entries = [Entry::default(); 2];
    let (drained, _, _) = drain(&mut entries, &mut []).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!((drained.copied, drained.pending), (2, 2));
    assert_eq!((entries[0].amount, entries[1].amount), (17, -23));
    assert_eq!(LOG.epoch.load(Ordering::SeqCst), epoch);
    collect();
    assert!(STORAGE.lock().unwrap().retired.is_none());
    release().unwrap();
}

#[test]
fn stale_publisher_can_fill_successor_before_the_boundary_publishes_it() {
    let log = Log::new(1);
    let old = Buffer::new(1, 0, Default::default()).unwrap();
    let mut fresh = Buffer::new(2, 0, Default::default()).unwrap();
    let memory = Memory(Vec::new());
    assert!(append_buffer(
        &log,
        &old,
        Entry {
            amount: 5,
            ..Default::default()
        },
        None,
        &memory
    ));
    old.next.store(&mut fresh, Ordering::Release);
    assert!(
        old.publishers
            .compare_exchange(0, usize::MAX, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    );
    let lease = publishing(&old).unwrap();
    assert!(append_buffer(
        &log,
        lease.0,
        Entry {
            amount: 9,
            ..Default::default()
        },
        None,
        &memory
    ));
    drop(lease);
    let epoch = log.epoch.load(Ordering::SeqCst);
    // The boundary initializes the same frontier after the stale publisher.
    assert!(successor(&old).is_some());
    let mut entries = Vec::new();
    let (drained, _, _) = handover_buffers(
        &log,
        &fresh,
        Some(&old),
        engine::walk::Buffer::Growing(&mut entries),
        engine::walk::Buffer::Growing(&mut Vec::new()),
        None,
    )
    .unwrap();
    assert_eq!((drained.first, drained.cursor, drained.copied), (0, 2, 2));
    assert_eq!((entries[0].amount, entries[1].amount), (5, 9));
    assert_eq!(log.epoch.load(Ordering::SeqCst), epoch);
}

#[test]
fn concurrent_callbacks_and_short_drains_keep_all_receipts_and_name_bytes() {
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    NAME_STRIDE.store(6, Ordering::Release);
    offer(5000, 30_000);
    install_offered();
    LOG.enabled.store(true, Ordering::SeqCst);
    let epoch = LOG.epoch.load(Ordering::SeqCst);
    let done = AtomicUsize::new(0);
    let mut seen = vec![false; 5000];
    let mut output = [Entry::default(); 137];
    let mut names = [0; 137 * 6];
    let mut count = 0;
    struct Done<'a>(&'a AtomicUsize);
    impl Drop for Done<'_> {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Release);
        }
    }
    std::thread::scope(|scope| {
        for lane in 0..8 {
            let done = &done;
            scope.spawn(move || {
                let _done = Done(done);
                let mut memory = Memory(vec![0; 0x400]);
                for country in (lane..5000).step_by(8) {
                    let stored = name(&mut memory, b"before");
                    assert!(append(
                        Entry {
                            country: country as u32,
                            ..Default::default()
                        },
                        Some(stored),
                        &memory
                    ));
                    std::thread::yield_now();
                }
            });
        }
        loop {
            let (drained, _, _) = drain(&mut output, &mut names).unwrap();
            if done.load(Ordering::Acquire) == 8 && drained.copied == 0 {
                break;
            }
            assert!(drained.copied <= drained.pending);
            for entry in &output[..drained.copied] {
                let index = entry.country as usize;
                assert!(!seen[index]);
                seen[index] = true;
                let (start, length) = entry.name.unwrap();
                assert_eq!(&names[start..start + length], b"before");
                count += 1;
            }
            std::thread::yield_now();
        }
    });
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!(count, 5000);
    assert!(seen.into_iter().all(|observed| observed));
    assert_eq!(LOG.epoch.load(Ordering::SeqCst), epoch);
    assert_eq!(ENTRY_CONSUMED.load(Ordering::Relaxed), 5000);
    assert_eq!(NAME_CONSUMED.load(Ordering::Relaxed), 30_000);
    release().unwrap();
}
