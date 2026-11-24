use super::*;

static EXCLUSIVE: Mutex<()> = Mutex::new(());

#[test]
fn detach_waits_for_admitted_callbacks_after_stopping_receipt_admission() {
    let _exclusive = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    LOG.enabled.store(true, Ordering::SeqCst);
    let call = LOG.enter();
    assert!(LOG.enabled.load(Ordering::SeqCst));
    let mut hooks = Hooks {
        taps: Vec::new(),
        publication_attempted: false,
    };
    assert_eq!(hooks.set_enabled(false), Ok(false));
    assert!(!LOG.enabled.load(Ordering::SeqCst));
    assert_eq!(active(), 1);
    drop(call);
    assert_eq!(active(), 0);
    assert!(stop_admission_and_drained());
}

#[test]
fn sizing_retries_a_missing_world_and_grows_receipt_pressure_after_a_failed_census() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    struct NullState;
    impl engine_native_memory::Memory for NullState {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == engine_native::cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
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
        let mut domains = CURRENT.lock().unwrap();
        let current = domains.current.as_mut().unwrap();
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
    let unreadable = engine_native_memory::memory::SliceMemory {
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
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    offer(8, 8);
    install_offered();
    offer(9, 0);
    assert_eq!(capacities(), (16, 16));
    offer(0, 9);
    assert_eq!(capacities(), (16, 16));
    let offered = || OFFERED.load(Ordering::Acquire) as usize;
    let pointer = offered();
    assert_ne!(pointer, 0);
    for _ in 0..32 {
        offer(9, 0);
        offer(0, 9);
        collect();
        assert_eq!(capacities(), (16, 16));
        assert_eq!(offered(), pointer);
    }
    crate::test_support::with_allocations_denied(install_offered);
    assert_eq!(capacities(), (16, 16));
    assert_eq!(offered(), 0);
    assert!(CURRENT.lock().unwrap().retired.is_some());
    crate::test_support::with_allocations_denied(install_offered);
    assert!(CURRENT.lock().unwrap().retired.is_none());
    assert!(!DISPOSED.load(Ordering::Acquire).is_null());
    collect();
    assert!(DISPOSED.load(Ordering::Acquire).is_null());
    release().unwrap();
}

struct Memory(Vec<u8>);
impl engine_native_memory::Memory for Memory {
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
fn stored_names_and_signed_awards_survive_a_growing_handover_and_later_rename() {
    let log = Log::new(2);
    let spill = Spill::new();
    let buffer = Buffer::new(2, 16, Default::default()).unwrap();
    let mut memory = Memory(vec![0; 0x400]);
    let first = name(&mut memory, b"before");
    append_buffer(
        &log,
        &spill,
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
        &spill,
        &buffer,
        Entry {
            amount: -9,
            ..Default::default()
        },
        Some(second),
        &memory,
    );
    let mut output = vec![Entry::default()];
    let mut names = vec![0; 16];
    let (drained, len, required) =
        handover_buffers(&log, &spill, &buffer, None, engine_native_memory::walk::Buffer::Growing(&mut output), engine_native_memory::walk::Buffer::Growing(&mut names), None).unwrap();
    assert_eq!(
        (drained.copied, drained.pending, len, required),
        (2, 2, 11, 11)
    );
    assert_eq!(output[0].amount, 0);
    assert_eq!(output[1].amount, -9);
    assert_eq!(output[1].name, Some((6, 5)));
    assert_eq!(&names[..11], b"beforeafter");
}

#[test]
fn a_name_past_the_stride_is_kept_whole_with_its_money() {
    let log = Log::new(1);
    let spill = Spill::new();
    let buffer = Buffer::new(1, 2, Default::default()).unwrap();
    let mut memory = Memory(vec![0; 0x400]);
    let stored = name(&mut memory, b"long");
    assert!(append_buffer(
        &log,
        &spill,
        &buffer,
        Entry {
            amount: -1,
            ..Default::default()
        },
        Some(stored),
        &memory,
    ));
    let mut output = vec![Entry::default()];
    let mut names = vec![0; 4];
    let (drained, len, required) =
        handover_buffers(&log, &spill, &buffer, None, engine_native_memory::walk::Buffer::Growing(&mut output), engine_native_memory::walk::Buffer::Growing(&mut names), None).unwrap();
    assert_eq!((drained.copied, len, required), (1, 4, 4));
    assert_eq!(output[0].amount, -1);
    assert_eq!(output[0].name, Some((0, 4)));
    assert_eq!(output[0].name_reason, None);
    assert_eq!(&names, b"long");
    assert!(spill.held().entries.is_empty());
    assert_eq!(log.epoch.load(Ordering::SeqCst), 1);
}

#[test]
fn a_spill_never_waits_for_the_drain_and_a_burst_keeps_every_receipt_and_name_in_order() {
    let log = Log::new(1);
    let spill = Spill::new();
    let buffer = Buffer::new(1, 2, Default::default()).unwrap();
    let mut memory = Memory(vec![0; 0x400]);
    name(&mut memory, b"long");
    let consumer = spill.held.lock().unwrap();
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let (log, spill, buffer, memory) = (&log, &spill, &buffer, &memory);
        scope.spawn(move || {
            for amount in 1..=50 {
                let stored = (amount > 1).then(|| native::stored_name(memory, 0x100, 2).unwrap());
                let entry = Entry {
                    amount,
                    ..Default::default()
                };
                assert!(append_buffer(log, spill, buffer, entry, stored, memory));
            }
            done.send(()).unwrap();
        });
        finished
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("a fuel receipt spill waited for the drain's lock");
        drop(consumer);
    });
    let mut output = Vec::new();
    let mut names = Vec::new();
    let (drained, len, required) = handover_buffers(
        &log,
        &spill,
        &buffer,
        None,
        engine_native_memory::walk::Buffer::Growing(&mut output),
        engine_native_memory::walk::Buffer::Growing(&mut names),
        None,
    )
    .unwrap();
    assert_eq!((drained.copied, drained.pending, len, required), (50, 50, 196, 196));
    assert_eq!((output[0].amount, output[0].name), (1, None));
    for (n, entry) in output[1..50].iter().enumerate() {
        assert_eq!(entry.amount, n as i64 + 2);
        assert_eq!(entry.name, Some((n * 4, 4)));
    }
    assert_eq!(names[..196], b"long".repeat(49)[..]);
    assert!(spill.held().entries.is_empty());
    assert_eq!(log.epoch.load(Ordering::SeqCst), 1);
}

#[test]
fn fuel_buffer_charge_follows_offer_live_retirement_and_actual_release() {
    static LOCAL: crate::raw_memory::Tracker = crate::raw_memory::Tracker::new();
    let buffer = Buffer::new(2, 16, crate::raw_memory::Charge::empty_on(&LOCAL)).unwrap();
    let bytes = LOCAL.snapshot().current.payload_vectors;
    assert!(bytes >= (2 * size_of::<Entry>() + 16) as u64);
    let mut domains = Domains {
        current: None,
        retired: None,
    };
    let offered = Box::into_raw(Box::new(buffer));
    domains.current = Some(unsafe { Box::from_raw(offered) });
    assert_eq!(LOCAL.snapshot().current.payload_vectors, bytes);
    domains.retired = domains.current.take();
    assert_eq!(LOCAL.snapshot().current.payload_vectors, bytes);
    drop(domains.retired.take());
    assert_eq!(LOCAL.snapshot().current.payload_vectors, 0);
}

#[test]
fn a_burst_past_the_buffer_is_handed_over_whole_and_only_failed_storage_withholds_it() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let settled_before = COUNTERS[2].load(Ordering::Relaxed);
    let overflow_before = COUNTERS[10].load(Ordering::Relaxed);
    let log = Log::new(1);
    let spill = Spill::new();
    let buffer = Buffer::new(1, 0, Default::default()).unwrap();
    let memory = Memory(Vec::new());
    let reset = Entry {
        reset: true,
        started: native::Started {
            status: 0x1000,
            ..Default::default()
        },
        ..Default::default()
    };
    let burst = [
        Entry {
            amount: 1,
            ..Default::default()
        },
        reset,
        Entry {
            amount: 2,
            ..Default::default()
        },
    ];
    for entry in burst {
        assert!(append_buffer(&log, &spill, &buffer, entry, None, &memory));
    }
    let mut output = vec![Entry::default(); 3];
    let (drained, _, _) = handover_buffers(&log, &spill, &buffer, None, engine_native_memory::walk::Buffer::Growing(&mut output), engine_native_memory::walk::Buffer::Count, None).unwrap();
    assert_eq!((drained.copied, drained.pending), (3, 3));
    assert_eq!(output[0].amount, 1);
    assert!(output[1].reset && output[1].started.status == 0x1000);
    assert_eq!(output[2].amount, 2);
    assert_eq!(buffer.entries_required.load(Ordering::Relaxed), 3);
    assert_eq!(COUNTERS[10].load(Ordering::Relaxed), overflow_before);
    spill.release();
    assert!(append_buffer(&log, &spill, &buffer, burst[0], None, &memory));
    crate::test_support::with_allocations_denied(|| {
        assert!(!append_buffer(&log, &spill, &buffer, reset, None, &memory));
    });
    let (drained, _, _) = handover_buffers(&log, &spill, &buffer, None, engine_native_memory::walk::Buffer::Growing(&mut output), engine_native_memory::walk::Buffer::Count, None).unwrap();
    assert_eq!((drained.first, drained.cursor, drained.copied), (5, 5, 0));
    assert_eq!(COUNTERS[2].load(Ordering::Relaxed), settled_before);
    assert_eq!(COUNTERS[10].load(Ordering::Relaxed), overflow_before + 1);
    assert_eq!(spill.lost.load(Ordering::Acquire), u64::MAX);
}

#[test]
fn receipts_before_the_first_buffer_are_handed_over_after_it_installs() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    for amount in [3, -4] {
        assert!(append(
            &LOG.enter(),
            Entry {
                amount,
                ..Default::default()
            },
            None,
            &Memory(Vec::new())
        ));
    }
    offer(1, 0);
    install_offered();
    assert!(!LIVE.load(Ordering::SeqCst).is_null());
    assert!(append(
        &LOG.enter(),
        Entry {
            amount: 5,
            ..Default::default()
        },
        None,
        &Memory(Vec::new())
    ));
    LOG.enabled.store(true, Ordering::SeqCst);
    let mut entries = Vec::new();
    let (drained, _, _) = drain(&mut entries, &mut Vec::new()).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!((drained.copied, drained.pending), (3, 3));
    assert_eq!(
        entries[..3].iter().map(|entry| entry.amount).collect::<Vec<_>>(),
        [3, -4, 5]
    );
    release().unwrap();
}

fn started(root: u64) -> Pending {
    Pending {
        root,
        epoch: LOG.epoch.load(Ordering::SeqCst),
        started: native::Started {
            country: root as u32,
            ..Default::default()
        },
        located: native::Located {
            owner: 0,
            country: engine_native_memory::country_identity::Country {
                pointer: 0,
                canonical_slot: 0,
                id: root as u32,
            },
        },
        stamp: (0, 0),
        received: [0; 3],
        refused: false,
        cause: None,
    }
}

#[test]
fn only_a_standing_root_lends_its_location() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let standing = Pending {
        stamp: stamp(),
        ..started(7)
    };
    assert_eq!(held(&standing), Some(standing.located));
    let refused = Pending {
        refused: true,
        cause: Some(1),
        ..standing
    };
    assert_eq!(held(&refused), None);
    let stale_epoch = Pending {
        epoch: standing.epoch.wrapping_add(1),
        ..standing
    };
    assert_eq!(held(&stale_epoch), None);
    let earlier_hour = Pending {
        stamp: (standing.stamp.0, standing.stamp.1.wrapping_sub(1)),
        ..standing
    };
    assert_eq!(held(&earlier_hour), None);
}

#[test]
fn worker_storage_lease_cannot_lose_native_recipient_or_terminal_root() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    let mut memory = Memory(vec![0; 0x400]);
    let stored = name(&mut memory, b"retained");
    NAME_STRIDE.store(stored.byte_len(), Ordering::Release);
    offer(2, 8);
    install_offered();
    let busy_before = COUNTERS[6].load(Ordering::Relaxed);
    let epoch = LOG.epoch.load(Ordering::SeqCst);
    let worker = SIZING.lock().unwrap();
    crate::test_support::with_allocations_denied(|| {
        assert!(append(
            &LOG.enter(),
            Entry {
                root: 1,
                recipient: Some(native::Recipient::default()),
                amount: -7,
                ..Default::default()
            },
            Some(stored),
            &memory,
        ));
        append_root(
            &LOG.enter(),
            started(1),
            native::Settled::default(),
            &memory,
        );
    });
    drop(worker);
    assert_eq!(COUNTERS[6].load(Ordering::Relaxed), busy_before);
    assert_eq!(LOG.epoch.load(Ordering::SeqCst), epoch);
    let mut entries = vec![Entry::default(); 2];
    let mut names = vec![0; 8];
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
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    offer(4, 0);
    install_offered();
    let before = APPEND_REFUSALS
        .each_ref()
        .map(|value| value.load(Ordering::Relaxed));
    let epoch = LOG.epoch.load(Ordering::SeqCst);
    let current = CURRENT.lock().unwrap();
    let (recipient, reset) = crate::test_support::with_allocations_denied(|| {
        let recipient = append(
            &LOG.enter(),
            Entry {
                recipient: Some(native::Recipient::default()),
                amount: -7,
                ..Default::default()
            },
            None,
            &Memory(Vec::new()),
        );
        append_root(
            &LOG.enter(),
            started(1),
            native::Settled::default(),
            &Memory(Vec::new()),
        );
        let reset = append(
            &LOG.enter(),
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
    let mut entries = vec![Entry::default(); 4];
    let (drained, _, _) = drain(&mut entries, &mut Vec::new()).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!((drained.copied, drained.pending), (3, 3));
    assert_eq!(entries[0].amount, -7);
    assert_eq!(entries[1].root, 1);
    assert!(entries[2].reset);
    release().unwrap();
}

#[test]
fn rejected_country_receipt_burst_preserves_growth_after_failed_census() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    NAME_STRIDE.store(8, Ordering::Release);
    offer(1, 1);
    install_offered();
    let mut memory = Memory(vec![0; 0x400]);
    let _ = name(&mut memory, b"recorded");
    let current = CURRENT.lock().unwrap();
    crate::test_support::with_allocations_denied(|| {
        for country in 1..=5 {
            let stored = native::stored_name(&memory, 0x100, 2).unwrap();
            assert_eq!(
                append(
                    &LOG.enter(),
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
            append_root(
                &LOG.enter(),
                started(country as u64),
                native::Settled::default(),
                &memory,
            );
        }
    });
    drop(current);
    assert_eq!(ENTRY_DEMAND.load(Ordering::Acquire), 10);
    assert_eq!(NAME_DEMAND.load(Ordering::Acquire), 40);
    let unreadable = engine_native_memory::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    presize(&unreadable, 0);
    let (entries, names) = capacities();
    assert!(entries >= 10 && names >= 40);
    let mut output = vec![Entry::default()];
    LOG.enabled.store(true, Ordering::SeqCst);
    let (drained, _, required) = drain(&mut output, &mut Vec::new()).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!(
        (drained.copied, drained.pending, required),
        (0, 10, 40)
    );
    crate::test_support::with_allocations_denied(install_offered);
    collect();
    for _ in 0..2 {
        assert!(append(&LOG.enter(), Entry::default(), None, &memory));
    }
    LOG.enabled.store(true, Ordering::SeqCst);
    let (drained, _, required) = drain(&mut output, &mut Vec::new()).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!((drained.copied, drained.pending, required), (2, 2, 0));
    assert_eq!(ENTRY_DEMAND.load(Ordering::Acquire), 10);
    assert_eq!(NAME_DEMAND.load(Ordering::Acquire), 40);
    release().unwrap();
}

#[test]
fn missing_live_storage_names_every_lost_append_lane_without_allocation() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    let before = APPEND_REFUSALS
        .each_ref()
        .map(|counter| counter.load(Ordering::Relaxed));
    let absent_before = COUNTERS[11].load(Ordering::Relaxed);
    crate::test_support::with_allocations_denied(|| {
        let memory = Memory(Vec::new());
        assert!(!append(
            &LOG.enter(),
            Entry {
                recipient: Some(native::Recipient::default()),
                ..Default::default()
            },
            None,
            &memory
        ));
        append_root(
            &LOG.enter(),
            started(1),
            native::Settled::default(),
            &memory,
        );
        assert!(!append(
            &LOG.enter(),
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
    }
    assert_eq!(COUNTERS[11].load(Ordering::Relaxed), absent_before + 3);
    release().unwrap();
}

#[test]
fn empty_drain_preserves_attempts_of_an_active_native_callback() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    offer(2, 0);
    install_offered();
    let active = LOG.enter();
    ENTRY_ATTEMPTS.store(1, Ordering::SeqCst);
    ENTRY_DEMAND.store(1, Ordering::Release);
    LOG.cursor.fetch_add(1, Ordering::SeqCst);
    let mut output = vec![Entry::default(); 2];
    LOG.enabled.store(true, Ordering::SeqCst);
    let (drained, _, _) = drain(&mut output, &mut Vec::new()).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!((drained.copied, drained.pending), (0, 1));
    assert_eq!(ENTRY_CONSUMED.load(Ordering::Relaxed), 0);
    drop(active);
    release().unwrap();
}

#[test]
fn country_reset_burst_survives_the_hourly_owner_lease() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    offer(5, 0);
    install_offered();
    let epoch = LOG.epoch.load(Ordering::SeqCst);
    let current = CURRENT.lock().unwrap();
    crate::test_support::with_allocations_denied(|| {
        for country in 1..=5 {
            assert!(append(
                &LOG.enter(),
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
    let mut entries = vec![Entry::default(); 5];
    LOG.enabled.store(true, Ordering::SeqCst);
    let (drained, _, _) = drain(&mut entries, &mut Vec::new()).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!((drained.copied, drained.pending), (5, 5));
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
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        std::ptr::from_ref::<Buffer>(lease.0) as *mut Buffer,
        LIVE.load(Ordering::SeqCst)
    );
    crate::test_support::with_allocations_denied(|| {
        assert!(append_buffer(
            &LOG,
            &SPILL,
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
    let mut entries = vec![Entry::default(); 2];
    let (drained, _, _) = drain(&mut entries, &mut Vec::new()).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!(drained.copied, 1);
    assert_eq!(entries[0].amount, -19);
    assert_eq!(LOG.epoch.load(Ordering::SeqCst), epoch);
    collect();
    release().unwrap();
}

#[test]
fn publication_between_empty_check_and_seal_survives_growth_and_collection() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    offer(1, 0);
    install_offered();
    offer(2, 0);
    let epoch = LOG.epoch.load(Ordering::SeqCst);
    {
        let mut domains = CURRENT.lock().unwrap();
        assert!(append(
            &LOG.enter(),
            Entry {
                amount: 17,
                ..Default::default()
            },
            None,
            &Memory(Vec::new())
        ));
        crate::test_support::with_allocations_denied(|| {
            install_prepared(&mut domains);
        });
    }
    collect();
    install_offered();
    assert!(CURRENT.lock().unwrap().retired.is_some());
    assert!(append(
        &LOG.enter(),
        Entry {
            amount: -23,
            ..Default::default()
        },
        None,
        &Memory(Vec::new())
    ));
    LOG.enabled.store(true, Ordering::SeqCst);
    let mut entries = vec![Entry::default(); 2];
    let (drained, _, _) = drain(&mut entries, &mut Vec::new()).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!((drained.copied, drained.pending), (2, 2));
    assert_eq!((entries[0].amount, entries[1].amount), (17, -23));
    assert_eq!(LOG.epoch.load(Ordering::SeqCst), epoch);
    install_offered();
    assert!(CURRENT.lock().unwrap().retired.is_none());
    collect();
    assert!(DISPOSED.load(Ordering::Acquire).is_null());
    release().unwrap();
}

#[test]
fn stale_publisher_can_fill_successor_before_the_boundary_publishes_it() {
    let log = Log::new(1);
    let spill = Spill::new();
    let old = Buffer::new(1, 0, Default::default()).unwrap();
    let mut fresh = Buffer::new(2, 0, Default::default()).unwrap();
    let memory = Memory(Vec::new());
    assert!(append_buffer(
        &log,
        &spill,
        &old,
        Entry {
            amount: 5,
            ..Default::default()
        },
        None,
        &memory
    ));
    old.next.store(&raw mut fresh, Ordering::Release);
    assert!(
        old.publishers
            .compare_exchange(0, usize::MAX, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    );
    let lease = publishing(&old).unwrap();
    assert!(append_buffer(
        &log,
        &spill,
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
    assert!(successor(&old).is_some());
    let mut entries = Vec::new();
    let (drained, _, _) = handover_buffers(
        &log,
        &spill,
        &fresh,
        Some(&old),
        engine_native_memory::walk::Buffer::Growing(&mut entries),
        engine_native_memory::walk::Buffer::Growing(&mut Vec::new()),
        None,
    )
    .unwrap();
    assert_eq!((drained.first, drained.cursor, drained.copied), (0, 2, 2));
    assert_eq!((entries[0].amount, entries[1].amount), (5, 9));
    assert_eq!(log.epoch.load(Ordering::SeqCst), epoch);
}

#[test]
fn a_nameless_receipt_refreshes_name_demand_for_a_named_callback_in_flight() {
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    offer(4, 0);
    install_offered();
    NAME_ATTEMPTS.store(20, Ordering::SeqCst);
    NAME_DEMAND.store(10, Ordering::Release);
    assert!(append(
        &LOG.enter(),
        Entry::default(),
        None,
        &Memory(Vec::new())
    ));
    assert_eq!(NAME_DEMAND.load(Ordering::Acquire), 20);
    assert_eq!(NAME_ATTEMPTS.load(Ordering::SeqCst), 20);
    NAME_CONSUMED.store(10, Ordering::Relaxed);
    raise(
        &NAME_DEMAND,
        attempted_since(20, &NAME_CONSUMED),
        Ordering::Release,
    );
    assert_eq!(NAME_DEMAND.load(Ordering::Acquire), 20);
    release().unwrap();
}

#[test]
fn concurrent_callbacks_and_short_drains_keep_all_receipts_and_name_bytes() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    NAME_STRIDE.store(6, Ordering::Release);
    offer(5000, 30_000);
    install_offered();
    LOG.enabled.store(true, Ordering::SeqCst);
    let epoch = LOG.epoch.load(Ordering::SeqCst);
    let done = AtomicUsize::new(0);
    let mut seen = vec![false; 5000];
    let mut output = vec![Entry::default(); 137];
    let mut names = vec![0; 137 * 6];
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
                        &LOG.enter(),
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
            let finished = done.load(Ordering::Acquire) == 8;
            let (drained, _, _) = drain(&mut output, &mut names).unwrap();
            if finished && drained.copied == 0 {
                assert_eq!(drained.pending, 0);
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

#[test]
fn spool_sizing_and_capacity_reads_never_refuse_the_hourly_drain_or_growth() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    offer(2, 0);
    install_offered();
    offer(4, 0);
    assert!(append(
        &LOG.enter(),
        Entry {
            amount: -5,
            ..Default::default()
        },
        None,
        &Memory(Vec::new())
    ));
    let refusals = (
        COUNTERS[6].load(Ordering::Relaxed),
        DRAIN_REFUSALS.load(Ordering::Relaxed),
    );
    let spool = SIZING.lock().unwrap();
    LOG.enabled.store(true, Ordering::SeqCst);
    let mut entries = vec![Entry::default(); 2];
    let (drained, _, _) = crate::test_support::with_allocations_denied(|| {
        assert_eq!(capacities(), (4, 0));
        let drained = drain(&mut entries, &mut Vec::new());
        install_offered();
        drained
    })
    .unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!((drained.copied, drained.pending), (1, 1));
    assert_eq!(entries[0].amount, -5);
    assert_eq!(LOG.capacity.load(Ordering::Acquire), 4);
    assert_eq!(OFFERED.load(Ordering::Acquire), std::ptr::null_mut());
    drop(spool);
    assert_eq!(
        (
            COUNTERS[6].load(Ordering::Relaxed),
            DRAIN_REFUSALS.load(Ordering::Relaxed)
        ),
        refusals
    );
    release().unwrap();
}

#[test]
fn refused_drain_is_named_and_counted() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    offer(1, 0);
    install_offered();
    let busy = COUNTERS[6].load(Ordering::Relaxed);
    let refused = DRAIN_REFUSALS.load(Ordering::Relaxed);
    let detach = CURRENT.lock().unwrap();
    assert_eq!(
        drain(&mut vec![Entry::default()], &mut Vec::new()).unwrap_err(),
        "fuel observation storage busy"
    );
    drop(detach);
    assert_eq!(COUNTERS[6].load(Ordering::Relaxed), busy + 1);
    assert_eq!(DRAIN_REFUSALS.load(Ordering::Relaxed), refused + 1);
    release().unwrap();
}

#[test]
fn unsealable_boundary_keeps_or_supersedes_the_prepared_offer_without_a_lock() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    offer(1, 0);
    install_offered();
    offer(2, 0);
    let prepared = OFFERED.load(Ordering::Acquire);
    {
        let mut domains = CURRENT.lock().unwrap();
        let live = domains.current.as_ref().unwrap();
        live.publishers.fetch_add(1, Ordering::AcqRel);
        crate::test_support::with_allocations_denied(|| install_prepared(&mut domains));
        assert_eq!(OFFERED.load(Ordering::Acquire), prepared);
        assert!(domains.retired.is_none());
        let live = domains.current.as_ref().unwrap();
        live.publishers.fetch_sub(1, Ordering::AcqRel);
    }
    let older = Box::new(Buffer::with_stride(1, 0, 0, Default::default()).unwrap());
    crate::test_support::with_allocations_denied(|| return_offer(older));
    assert_eq!(OFFERED.load(Ordering::Acquire), prepared);
    assert!(!DISPOSED.load(Ordering::Acquire).is_null());
    collect();
    assert!(DISPOSED.load(Ordering::Acquire).is_null());
    install_offered();
    assert_eq!(LOG.capacity.load(Ordering::Acquire), 2);
    release().unwrap();
}

#[test]
fn a_root_from_before_an_hourly_boundary_is_never_taken() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut pending = started(7);
    pending.stamp = stamp();
    assert_eq!(held(&pending), Some(pending.located));
    boundary();
    assert_eq!(held(&pending), None);
    pending.stamp = stamp();
    assert_eq!(held(&pending), Some(pending.located));
    pending.stamp.0 = pending.stamp.0.wrapping_add(1);
    assert_eq!(held(&pending), None);
}

#[test]
fn partial_drain_reclaims_only_consumed_root_slots_under_out_of_order_reservation() {
    let log = Log::new(4);
    let spill = Spill::new();
    let buffer = Buffer::new(4, 32, Default::default()).unwrap();
    assert_eq!(buffer.roots.len(), 4);
    let memory = Memory(Vec::new());
    let mapping = [(0_u64, 1_usize, 101_i64), (1, 0, 202), (2, 2, 303), (3, 3, 404)];
    log.cursor.store(4, Ordering::SeqCst);
    buffer.cursor.store(4, Ordering::Release);
    for (entry_index, root_pos, amount) in mapping {
        buffer.roots[root_pos]
            .owner
            .store(entry_index + 1, Ordering::Release);
        // SAFETY: Test owns this local `Buffer` exclusively.
        unsafe {
            *buffer.roots[root_pos].data.get() = RootData {
                amount,
                ..Default::default()
            };
            *buffer.entries[entry_index as usize].entry.get() = CompactEntry {
                root: entry_index + 10,
                payload: CompactPayload::Root {
                    country: (entry_index + 1) as u32,
                    index: root_pos as u32,
                },
            };
            *buffer.entries[entry_index as usize].name_required.get() = 0;
        }
        buffer.entries[entry_index as usize]
            .published
            .store(entry_index + 1, Ordering::Release);
    }

    let mut one = [Entry::default(); 1];
    let mut names = [0_u8; 32];
    let (drained, _, _) = handover_buffers(
        &log,
        &spill,
        &buffer,
        None,
        engine_native_memory::walk::Buffer::Exact(&mut one),
        engine_native_memory::walk::Buffer::Exact(&mut names),
        None,
    )
    .unwrap();
    assert_eq!((drained.copied, drained.pending), (1, 4));
    assert_eq!(one[0].amount, 101);
    assert_eq!(buffer.roots[1].owner.load(Ordering::Acquire), 0);
    assert_eq!(buffer.roots[0].owner.load(Ordering::Acquire), 2);

    assert!(append_buffer(
        &log,
        &spill,
        &buffer,
        Entry {
            root: 14,
            country: 5,
            amount: 505,
            ..Default::default()
        },
        None,
        &memory,
    ));
    assert_eq!(buffer.roots[1].owner.load(Ordering::Acquire), 5);
    assert_eq!(buffer.roots[0].owner.load(Ordering::Acquire), 2);

    let mut compact_out = [CompactEntry::default(); 4];
    let mut compact_roots = Vec::new();
    let (drained, _, _) = handover_compact(
        &log,
        &spill,
        &buffer,
        None,
        engine_native_memory::walk::Buffer::Exact(&mut compact_out),
        &mut compact_roots,
        engine_native_memory::walk::Buffer::Exact(&mut names),
        None,
    )
    .unwrap();
    assert_eq!((drained.copied, drained.pending), (4, 4));
    let amounts: Vec<i64> = compact_out
        .iter()
        .map(|entry| match entry.payload {
            CompactPayload::Root { index, .. } => compact_roots[index as usize].amount,
            other => panic!("expected root payload, got {other:?}"),
        })
        .collect();
    assert_eq!(amounts, [202, 303, 404, 505]);
    assert!(buffer
        .roots
        .iter()
        .all(|slot| slot.owner.load(Ordering::Acquire) == 0));
}
