use super::*;

static EXCLUSIVE: Mutex<()> = Mutex::new(());

#[test]
fn detach_after_air_store_keeps_terminal_cleanup_live_after_admission_stops() {
    let _exclusive = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    PENDING.set(None);
    finish_air_store(1);
    LOG.enabled.store(true, Ordering::SeqCst);
    let mut registers: Registers = unsafe { std::mem::zeroed() };
    registers.rdx = 0x1200;
    // The AIR callback returns before the native displaced store. Its
    // independent interval guard must retain the terminal tap beyond return.
    unsafe { observe(&registers, 8) };
    assert_eq!(AIR_CONTAINER_WRITERS.load(Ordering::SeqCst), 1);
    assert_eq!(LOG.active.load(Ordering::SeqCst), 0);
    let mut hooks = Hooks {
        taps: Vec::new(),
        publication_attempted: false,
    };
    assert_eq!(hooks.set_enabled(false), Ok(false));
    assert!(!LOG.enabled.load(Ordering::SeqCst));
    // This call represents END after the original receipt store completed.
    // Disabled capture still releases its certificate guard.
    unsafe { observe(&registers, 1) };
    assert_eq!(AIR_CONTAINER_WRITERS.load(Ordering::SeqCst), 0);
    assert!(stop_admission_and_drained());
}

#[test]
fn detach_cannot_pass_an_admitted_callback_before_it_retains_the_store_guard() {
    let _exclusive = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    finish_air_store(1);
    LOG.enabled.store(true, Ordering::SeqCst);
    // The callback's admission check observed enabled but it has not yet
    // created its AIR guard when another thread begins detach.
    let call = LOG.enter();
    assert!(LOG.enabled.load(Ordering::SeqCst));
    assert!(!stop_admission_and_drained());
    AIR_CONTAINER_WRITER
        .with(|writer| *writer.borrow_mut() = Some(AirContainerWriter::new(0x1200)));
    drop(call);
    assert!(!stop_admission_and_drained());
    finish_air_store(1);
    assert!(stop_admission_and_drained());
}

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
            engine_native::walk::Buffer::Growing(&mut entries),
            engine_native::walk::Buffer::Growing(&mut names),
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
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    struct NullState;
    impl engine_native::Memory for NullState {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == engine_native::cgame_state::SINGLETON_RVA && output.len() == 8 {
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
    let unreadable = engine_native::memory::SliceMemory {
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
    // The next boundary retires the consumed predecessor; the spool frees it.
    crate::test_support::with_allocations_denied(install_offered);
    assert!(CURRENT.lock().unwrap().retired.is_none());
    assert!(!DISPOSED.load(Ordering::Acquire).is_null());
    collect();
    assert!(DISPOSED.load(Ordering::Acquire).is_null());
    release().unwrap();
}

struct Memory(Vec<u8>);
impl engine_native::Memory for Memory {
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
fn lost_reset_overflow_withholds_interval_and_retains_actual_growth_demand() {
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        located: native::Located {
            owner: 0,
            country: engine_native::ccountry::Country {
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

/// A root's location is taken by its own invocation's later calls only while
/// the root stands: in the current world, hour and log epoch, and not
/// refused (by a reset of its status, a nested award or a lost receipt). A
/// refused or stale root is read afresh.
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
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    offer(1, 1);
    install_offered();
    let mut memory = Memory(vec![0; 0x400]);
    let _ = name(&mut memory, b"recorded");
    let current = CURRENT.lock().unwrap();
    crate::test_support::with_allocations_denied(|| {
        for country in 1..=5000 {
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
    assert_eq!(ENTRY_DEMAND.load(Ordering::Acquire), 10_000);
    assert_eq!(NAME_DEMAND.load(Ordering::Acquire), 40_000);
    let unreadable = engine_native::memory::SliceMemory {
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
    crate::test_support::with_allocations_denied(install_offered);
    collect();
    for _ in 0..2 {
        assert!(append(&LOG.enter(), Entry::default(), None, &memory));
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
    let _guard = EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    release().unwrap();
    offer(5000, 0);
    install_offered();
    let epoch = LOG.epoch.load(Ordering::SeqCst);
    let current = CURRENT.lock().unwrap();
    crate::test_support::with_allocations_denied(|| {
        for country in 1..=5000 {
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
        lease.0 as *const Buffer as *mut Buffer,
        LIVE.load(Ordering::SeqCst)
    );
    crate::test_support::with_allocations_denied(|| {
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
        // install_offered observed quiet/empty before this native publication.
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
    let mut entries = [Entry::default(); 2];
    let (drained, _, _) = drain(&mut entries, &mut []).unwrap();
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
        engine_native::walk::Buffer::Growing(&mut entries),
        engine_native::walk::Buffer::Growing(&mut Vec::new()),
        None,
    )
    .unwrap();
    assert_eq!((drained.first, drained.cursor, drained.copied), (0, 2, 2));
    assert_eq!((entries[0].amount, entries[1].amount), (5, 9));
    assert_eq!(log.epoch.load(Ordering::SeqCst), epoch);
}

/// A named callback adds its bytes to the attempts before it raises the name
/// demand; a drain can consume earlier bytes in between. A nameless receipt
/// in that gap still raises the demand to every attempted byte, so the named
/// callback's late raise against the advanced consumption cannot lower what
/// sizing reads.
#[test]
fn a_nameless_receipt_refreshes_name_demand_for_a_named_callback_in_flight() {
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    offer(4, 0);
    install_offered();
    // Ten bytes consumed-pending, then a named callback's ten more bytes,
    // added but not yet raised.
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
    // The drain then consumes ten bytes and the named callback raises 20 - 10.
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
            // Whether every writer had returned BEFORE this drain: only then
            // does an empty drain prove nothing is left. Checking after it
            // let a writer append its last entries between the two and end
            // the loop with them unread.
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
    // The spool holds its sizing lock across the whole parked boundary.
    let spool = SIZING.lock().unwrap();
    LOG.enabled.store(true, Ordering::SeqCst);
    let mut entries = [Entry::default(); 2];
    let (drained, _, _) = crate::test_support::with_allocations_denied(|| {
        assert_eq!(capacities(), (4, 0));
        let drained = drain(&mut entries, &mut []);
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
fn refused_drain_is_named_and_counted_in_status() {
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
        drain(&mut [Entry::default()], &mut []).unwrap_err(),
        "fuel observation storage busy"
    );
    drop(detach);
    assert_eq!(COUNTERS[6].load(Ordering::Relaxed), busy + 1);
    assert_eq!(DRAIN_REFUSALS.load(Ordering::Relaxed), refused + 1);
    let status = serde_json::to_value(status()).unwrap();
    assert!(status["busy_refusals"].as_u64().unwrap() > busy);
    assert!(status["drain_refusals"].as_u64().unwrap() > refused);
    assert!(status["first_drain_refusal"].is_string());
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
        // A publisher still inside the live buffer prevents sealing.
        let live = domains.current.as_ref().unwrap();
        live.publishers.fetch_add(1, Ordering::AcqRel);
        crate::test_support::with_allocations_denied(|| install_prepared(&mut domains));
        assert_eq!(OFFERED.load(Ordering::Acquire), prepared);
        assert!(domains.retired.is_none());
        let live = domains.current.as_ref().unwrap();
        live.publishers.fetch_sub(1, Ordering::AcqRel);
    }
    // Sizing replaced the offer while the boundary held an older one.
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

/// Release: fuel update's begin and end taps for 1,500 countries through the
/// observer against a synthetic native world, the reset tap, and a recipient
/// receipt's recording (its unit reads are not modelled).
#[test]
#[ignore = "release per-call detour cost; run with --release --ignored --nocapture --test-threads=1"]
fn hook_cost_fuel_awards() {
    use engine_fuel::{ccountry_fuel_status as fuel, fuel_consumer_data};
    use engine_native::{ccountry, cgame_state};
    const COUNTRIES: usize = 1500;
    const GAME: usize = 0x1000;
    const TABLE: usize = 0x2000;
    const LINKS: usize = TABLE + (COUNTRIES + 1) * 8;
    const STATUSES: usize = (LINKS + (COUNTRIES + 1) * 4).div_ceil(0x100) * 0x100;
    const STATUS: usize = 0x200;
    const OBJECTS: usize = STATUSES + (COUNTRIES + 1) * STATUS;
    const OBJECT: usize = ccountry::FUEL_STATUS + 0x80;
    const _: () = assert!(0x100 + 3 * fuel_consumer_data::STRIDE <= STATUS);
    let _guard = EXCLUSIVE.lock().unwrap();
    release().unwrap();
    let mut storage = vec![0_u64; (OBJECTS + (COUNTRIES + 1) * OBJECT) / 8];
    let at = storage.as_ptr() as usize;
    let base = at - cgame_state::SINGLETON_RVA;
    let mut word = |offset: usize, value: u64| storage[offset / 8] = value;
    let array = |word: &mut dyn FnMut(usize, u64), offset: usize, data: usize, count: usize| {
        word(offset, (at + data) as u64);
        word(offset + 8, ((count as u64) << 32) | count as u64);
    };
    word(0, (at + GAME) as u64);
    array(
        &mut word,
        GAME + cgame_state::COUNTRIES,
        TABLE,
        COUNTRIES + 1,
    );
    array(
        &mut word,
        GAME + cgame_state::COUNTRY_LINKS,
        LINKS,
        COUNTRIES + 1,
    );
    let status = |country: usize| STATUSES + country * STATUS;
    for country in 1..=COUNTRIES {
        let object = OBJECTS + country * OBJECT;
        word(TABLE + country * 8, (at + object) as u64);
        word(object, (base + ccountry::VTABLE_RVA) as u64);
        // Id and instance both `country`.
        word(
            object + ccountry::ID,
            ((country as u64) << 32) | country as u64,
        );
        word(
            object + ccountry::FUEL_STATUS,
            (at + status(country)) as u64,
        );
        word(status(country), (base + fuel::VTABLE_RVA) as u64);
        word(status(country) + fuel::COUNTRY, (at + object) as u64);
        let consumers = status(country) + 0x100;
        array(&mut word, status(country) + fuel::CONSUMERS, consumers, 3);
    }
    let links = storage.as_mut_ptr().cast::<u32>();
    for country in 1..=COUNTRIES {
        unsafe { *links.add((LINKS + country * 4) / 4) = country as u32 };
    }
    BASE.store(base, Ordering::Relaxed);
    NAME_STRIDE.store(32, Ordering::Release);
    offer(32 * COUNTRIES, 32 * COUNTRIES * 32);
    install_offered();
    LOG.enabled.store(true, Ordering::SeqCst);
    let mut registers: Registers = unsafe { std::mem::zeroed() };
    let mut roots = || {
        for country in 1..=COUNTRIES {
            registers.rsi = at + status(country);
            unsafe { observe(&registers, 0) };
            registers.rbx = at + status(country);
            unsafe { observe(&registers, 1) };
        }
    };
    let root = crate::test_support::per_call_ns(2 * COUNTRIES, &mut roots);
    let reset = crate::test_support::per_call_ns(COUNTRIES, || {
        for country in 1..=COUNTRIES {
            registers.rsi = at + status(country);
            unsafe { observe(&registers, 9) };
        }
    });
    let mut memory = Memory(vec![0; 0x400]);
    name(&mut memory, b"1. Division");
    let receipt = crate::test_support::per_call_ns(COUNTRIES, || {
        let call = LOG.enter();
        for country in 1..=COUNTRIES {
            assert!(append(
                &call,
                Entry {
                    root: country as u64,
                    country: country as u32,
                    recipient: Some(native::Recipient::default()),
                    amount: -7,
                    ..Default::default()
                },
                Some(native::stored_name(&memory, 0x100, 2).unwrap()),
                &memory,
            ));
        }
    });
    let mut entries = Vec::new();
    let mut names = Vec::new();
    let (drained, _, _) = drain_growing(&mut entries, &mut names).unwrap();
    LOG.enabled.store(false, Ordering::SeqCst);
    assert_eq!(drained.copied, 3 * 8 * COUNTRIES);
    assert!(entries.iter().all(|entry| !entry.refused));
    println!(
        "hook_cost fuel_awards: begin+end {root:.1} ns/call, reset {reset:.1} ns/call, recipient receipt recording {receipt:.1} ns/call"
    );
    release().unwrap();
}

/// A root's located country is taken only in the world generation and
/// recording hour it was located in.
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
