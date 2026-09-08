const SLOT_ROWS: u64 = HEAP + 0x60000;
const SLOT_LEXER: u64 = HEAP + 0x100000;

fn fixture_with_copied_slots(rows: &[(u32, &str)]) -> Fixture {
    let mut f = fixture();
    f.u64(EQUIPMENT + 0x70, if rows.is_empty() { 0 } else { SLOT_ROWS });
    f.u32(EQUIPMENT + 0x78, rows.len() as u32);
    f.u32(EQUIPMENT + 0x7c, rows.len() as u32);
    f.put(BASE + 0x35c8b50, &[1]);
    f.u32(BASE + 0x35c8b54, 1000);
    f.u64(BASE + 0x35c8b80, SLOT_LEXER);
    f.u32(BASE + 0x35c8b8c, 1001);
    for (ordinal, &(raw, key)) in rows.iter().enumerate() {
        f.u32(SLOT_ROWS + ordinal as u64 * 0x50 + 8, raw);
        f.string(SLOT_LEXER + raw as u64 * 0x20, key);
    }
    f
}

#[test]
fn actual_capture_retains_all_copied_slots_including_unassigned_and_165() {
    use snapshot::design_catalog::{DesignSlotIdentityObservation, PortableDesignCatalog};
    let f = fixture_with_copied_slots(&[(700, "secondary"), (0x165, "none"), (500, "primary")]);
    let raw = capture(&f, BASE, DATE).unwrap();
    assert!(raw["variants"][0]["definition"]["modules"].as_array().unwrap().is_empty());
    let portable = PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).unwrap();
    let input = portable.native_designer_inputs_at_hour(100).unwrap();
    let DesignSlotIdentityObservation::Measured { slots, .. } = &input.variants[0].slot_identities else {
        panic!("complete copied census expected");
    };
    assert_eq!(slots.iter().map(|s| (s.source_ordinal, s.token_handle_raw)).collect::<Vec<_>>(),
        vec![(0, 700), (1, 0x165), (2, 500)]);
    assert_eq!(input.templates[0].design_id, "1:52");
    assert_eq!(input.templates[0].template_id, 1);
}

#[test]
fn actual_capture_copied_vector_empty_and_unavailable_are_distinct() {
    let f = fixture_with_copied_slots(&[]);
    let raw = capture(&f, BASE, DATE).unwrap();
    assert_eq!(raw["variants"][0]["definition"]["slot_identities"]["slots"], json!([]));
    let mut f = fixture_with_copied_slots(&[(500, "primary")]);
    f.u64(EQUIPMENT + 0x70, 0);
    let raw = capture(&f, BASE, DATE).unwrap();
    assert_eq!(raw["variants"][0]["definition"]["slot_identities"]["status"], "unavailable");
    assert_eq!(raw["variants"][0]["definition"]["slot_identities"]["reason"]["kind"], "invalid_vector");
}

#[test]
fn actual_capture_seals_owner_membership_slot_id_and_lexer_spans() {
    for address in [EQUIPMENT + 0x4d8, EQUIPMENT + 0x70, EQUIPMENT + 0x78,
        EQUIPMENT + 0x7c, SLOT_ROWS + 8, BASE + 0x35c8b50, BASE + 0x35c8b54,
        BASE + 0x35c8b80, BASE + 0x35c8b8c, SLOT_LEXER + 500 * 0x20] {
        let f = fixture_with_copied_slots(&[(500, "primary")]);
        let seen = Cell::new(0);
        let memory = |at, length| {
            let mut bytes = f.read(at, length)?;
            if at == address {
                let n = seen.get() + 1;
                seen.set(n);
                if n >= 2 { bytes[0] ^= 1; }
            }
            Ok(bytes)
        };
        assert_eq!(capture(&memory, BASE, DATE).unwrap_err().kind, FailureKind::Torn,
            "unsealed address {address:x}");
    }
}

#[test]
fn actual_capture_external_lexer_buffer_is_part_of_outer_seal() {
    let mut f = fixture_with_copied_slots(&[(500, "primary")]);
    let header = SLOT_LEXER + 500 * 0x20;
    let data = HEAP + 0x200000;
    let key = b"long_copied_primary_slot";
    f.zero(header, 32);
    f.u64(header, data);
    f.u64(header + 16, key.len() as u64);
    f.u64(header + 24, key.len() as u64);
    f.put(data, key);
    let seen = Cell::new(0);
    let memory = |at, length| {
        let mut bytes = f.read(at, length)?;
        if at == data {
            seen.set(seen.get() + 1);
            if seen.get() >= 2 { bytes[0] ^= 1; }
        }
        Ok(bytes)
    };
    assert_eq!(capture(&memory, BASE, DATE).unwrap_err().kind, FailureKind::Torn);
}

#[test]
fn actual_capture_unreadable_slot_text_does_not_recover_to_measured_on_close() {
    let f = fixture_with_copied_slots(&[(500, "primary")]);
    let header = SLOT_LEXER + 500 * 0x20;
    let seen = Cell::new(0);
    let memory = |at, length| {
        if at == header {
            seen.set(seen.get() + 1);
            if seen.get() == 1 { return Err("unreadable first opening".into()); }
        }
        f.read(at, length)
    };
    let raw = capture(&memory, BASE, DATE).unwrap();
    assert_eq!(raw["variants"][0]["definition"]["slot_identities"]["status"], "unavailable");
    assert_eq!(raw["variants"][0]["definition"]["slot_identities"]["reason"]["kind"], "unreadable");
    assert_eq!(seen.get(), 2);
}

#[test]
fn actual_capture_never_substitutes_ordinal_or_duplicate_lexer_identity() {
    for rows in [vec![(500, "primary"), (500, "primary")],
        vec![(500, "primary"), (501, "PRIMARY")]] {
        let f = fixture_with_copied_slots(&rows);
        let raw = capture(&f, BASE, DATE).unwrap();
        assert_eq!(raw["variants"][0]["definition"]["slot_identities"]["reason"]["kind"],
            "duplicate_identity");
    }
}

#[test]
fn shared_readset_closes_every_failed_and_repeated_attempt_after_budget_failure() {
    let f = fixture();
    let calls = std::cell::RefCell::new(Vec::new());
    let memory = |at, width| {
        calls.borrow_mut().push((at, width));
        if at == HEAP + 0x900000 { Err("unreadable".into()) } else { f.read(at, width) }
    };
    let budget = std::rc::Rc::new(std::cell::RefCell::new(
        slot_identity::Budget::new(24, 6, 100000)));
    let mut read = ReadSet::with_budget(&memory, BASE, budget.clone());
    read.u32(VARIANT + 0x18).unwrap();
    assert_eq!(read.u32(HEAP + 0x900000).unwrap_err().kind, FailureKind::Unreadable);
    read.u32(VARIANT + 0x18).unwrap();
    assert_eq!(read.u32(VARIANT + 0x18).unwrap_err().kind, FailureKind::BoundExceeded);
    assert_eq!(read.seal().unwrap_err().kind, FailureKind::BoundExceeded);
    assert_eq!(calls.borrow().len(), 6);
    assert_eq!(budget.borrow().read_bytes_reserved, 24);
    assert_eq!(budget.borrow().read_calls_reserved, 6);
}

#[test]
fn slot_changes_use_existing_raw_variant_delta_path() {
    use snapshot::design_catalog::PortableDesignCatalog;
    let f = fixture_with_copied_slots(&[(500, "primary")]);
    let first = capture(&f, BASE, DATE).unwrap();
    let mut later = fixture_with_copied_slots(&[(501, "primary")]);
    later.u32(STATE + DATE_OFFSET, DATE + 1);
    let second = capture(&later, BASE, DATE + 1).unwrap();
    let decode = |v: &Value| PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(v).unwrap()).unwrap();
    let delta = decode(&second).native_row_delta_from(&decode(&first)).unwrap();
    assert_eq!(delta["variant_upserts"].as_array().unwrap().len(), 1);
    assert!(delta["template_upserts"].as_array().unwrap().is_empty());
}

#[test]
fn actual_readset_second_acquisition_cannot_reset_failed_or_completed_work() {
    let f = fixture();
    let calls = Cell::new(0);
    let memory = |address, width| {
        calls.set(calls.get() + 1);
        f.read(address, width)
    };
    let budget = std::rc::Rc::new(std::cell::RefCell::new(
        slot_identity::Budget::new(8, 2, 100000)));
    {
        let mut first = ReadSet::with_budget(&memory, BASE, budget.clone());
        first.u32(VARIANT + 0x18).unwrap();
        first.seal().unwrap();
    }
    let spent = budget.borrow().allocation_bytes_reserved;
    let mut second = ReadSet::with_budget(&memory, BASE, budget.clone());
    assert_eq!(second.u32(VARIANT + 0x18).unwrap_err().kind, FailureKind::BoundExceeded);
    assert_eq!(second.seal().unwrap_err().kind, FailureKind::BoundExceeded);
    assert_eq!(calls.get(), 2);
    assert_eq!(budget.borrow().read_bytes_reserved, 8);
    assert!(budget.borrow().allocation_bytes_reserved >= spent);
}

#[test]
fn actual_capture_slot_preflight_budget_failure_refuses_and_closes_every_attempt() {
    // Full actual traffic fits each limited budget. The worst-case complete
    // copied-slot reservation does not: rejecting it must not publish a row
    // with local unavailable slots and a successful outer seal.
    let rows: Vec<_> = (0..256).map(|i| (500 + i, format!("slot_{i}"))).collect();
    let rows: Vec<_> = rows.iter().map(|(id, name)| (*id, name.as_str())).collect();
    let f = fixture_with_copied_slots(&rows);
    let measurement = readset_accounting::budget();
    capture_with_budget(&f, BASE, DATE, measurement.clone()).unwrap();
    let measured = measurement.borrow();
    let actual = (measured.read_bytes_reserved, measured.read_calls_reserved,
        measured.allocation_bytes_reserved);
    drop(measured);
    for (bytes, calls, allocation, expected_field) in [
        (actual.0, 2_000_000, 256 * 1024 * 1024, "complete census work"),
        (128 * 1024 * 1024, actual.1, 256 * 1024 * 1024, "complete census work"),
        (128 * 1024 * 1024, 2_000_000, actual.2, "complete census capacity"),
    ] {
        let budget = std::rc::Rc::new(std::cell::RefCell::new(
            slot_identity::Budget::new(bytes, calls, allocation)));
        let trace = std::cell::RefCell::new(Vec::new());
        let opening_count = Cell::new(0);
        let memory = |address, width| {
            trace.borrow_mut().push((address, width));
            if address == EQUIPMENT + 0x7c && opening_count.get() == 0 {
                opening_count.set(trace.borrow().len());
            }
            f.read(address, width)
        };
        let error = capture_with_budget(&memory, BASE, DATE, budget.clone()).unwrap_err();
        assert_eq!(error.kind, FailureKind::BoundExceeded);
        assert_eq!(error.field, expected_field);
        let trace = trace.borrow();
        let opened = opening_count.get();
        assert!(opened > 0);
        assert_eq!(trace.len(), 2 * opened, "no new opens after failed preflight");
        assert_eq!(&trace[..opened], &trace[opened..], "all funded closes in order");
        assert!(!trace.iter().any(|(at, _)| *at == SLOT_ROWS + 8));
        assert!(budget.borrow().read_bytes_reserved <= bytes);
        assert!(budget.borrow().read_calls_reserved <= calls);
        assert!(budget.borrow().allocation_bytes_reserved <= allocation);
    }
}

#[test]
fn actual_readset_trace_output_and_recipe_failures_are_sticky_and_close_failed_opens() {
    use slot_identity::{AcquisitionRead, RefusalKind};
    for mode in 0..3 {
        let f = fixture();
        let missing = HEAP + 0x900000;
        let trace = std::cell::RefCell::new(Vec::new());
        let memory = |address, width| {
            trace.borrow_mut().push((address, width));
            f.read(address, width)
        };
        let budget = readset_accounting::budget();
        let mut read = ReadSet::with_budget(&memory, BASE, budget.clone());
        read.u32(VARIANT + 0x18).unwrap();
        assert_eq!(read.u32(missing).unwrap_err().kind, FailureKind::Unreadable);
        match mode {
            0 => assert_eq!(read.qualify(0, 0, 0, 1_000_000).unwrap_err().kind,
                RefusalKind::TraceBound),
            1 => assert_eq!(read.reserve_allocation(usize::MAX).unwrap_err().kind,
                RefusalKind::AllocationBudget),
            _ => assert!(matches!(read.accounting.reserve_recipe(usize::MAX, 1, 0),
                Err(CatalogFailure { kind: FailureKind::BoundExceeded, .. }))),
        }
        assert_eq!(read.u32(VARIANT + 0x18).unwrap_err().kind, FailureKind::BoundExceeded);
        assert_eq!(read.seal().unwrap_err().kind, FailureKind::BoundExceeded);
        assert_eq!(*trace.borrow(), vec![
            (VARIANT + 0x18, 4), (missing, 4), (VARIANT + 0x18, 4), (missing, 4)]);
        assert_eq!(budget.borrow().read_bytes_reserved, 16);
        assert_eq!(budget.borrow().read_calls_reserved, 4);
    }
}

#[test]
fn actual_capture_high_bit_registered_ids_preserve_signed_operands_and_lexer_bounds() {
    use snapshot::design_catalog::{DesignByteObservation, DesignI32Observation,
        DesignSlotIdentityObservation, PortableDesignCatalog};
    for (template_id, variant_id) in [(0x80000000, u32::MAX), (u32::MAX, 0x80000000)] {
        let mut f = fixture_with_copied_slots(&[(0x165, "unused"), (500, "primary")]);
        for (kind, object, vtable, id) in [
            (52, TEMPLATE, TEMPLATE_VTABLE, template_id),
            (70, VARIANT, VARIANT_VTABLE, variant_id),
        ] {
            let root = f.registry(kind, object, vtable);
            let table = root + 0x100;
            f.zero(table, 13 * 24); // Keep the existing FF registry sentinel.
            let mixed = ((u64::from(kind) << 32) | u64::from(id)).wrapping_mul(0x49631a45);
            let hash = mixed as u32 ^ (mixed >> 32) as u32;
            let slot = table + u64::from(hash & 7) * 24;
            f.u32(slot, hash);
            f.put(slot + 4, &[1]);
            f.u32(slot + 8, kind);
            f.u32(slot + 12, id);
            f.u64(slot + 16, object);
            f.u32(object + 12, id);
        }
        f.put(TEMPLATE + 0x1ec, &i32::MIN.to_le_bytes());
        f.put(TEMPLATE + 0x1f4, &(-17_i32).to_le_bytes());
        f.put(VARIANT + 0x24, &[255]);
        let raw = capture(&f, BASE, DATE).unwrap();
        let catalog = PortableDesignCatalog::decode_a27_live_json(
            &serde_json::to_vec(&raw).unwrap()).unwrap();
        let inputs = catalog.native_designer_inputs_at_hour(100).unwrap();
        assert_eq!(inputs.templates[0].template_id, template_id);
        assert_eq!(inputs.templates[0].design_id, format!("{template_id}:52"));
        assert_eq!(inputs.variants[0].design_id, format!("{variant_id}:70"));
        assert_eq!(inputs.templates[0].country_289a, DesignI32Observation::Measured { raw_i32: i32::MIN });
        assert_eq!(inputs.templates[0].country_3abe, DesignI32Observation::Measured { raw_i32: -17 });
        assert_eq!(inputs.variants[0].append_variant_name, DesignByteObservation::Measured { raw_u8: 255 });
        let DesignSlotIdentityObservation::Measured { slots, .. } = &inputs.variants[0].slot_identities
            else { panic!("registered IDs must not change copied-slot admission"); };
        assert_eq!(slots.iter().map(|row| row.token_handle_raw).collect::<Vec<_>>(), vec![0x165, 500]);
        for object in [TEMPLATE, VARIANT] {
            for (offset, value) in [(0, 0_u64), (8, 0), (12, 0), (16, 0)] {
                let mut invalid = f.clone();
                match offset {
                    0 => invalid.u64(object, value),
                    16 => invalid.put(object + offset, &[value as u8]),
                    _ => invalid.u32(object + offset, value as u32),
                }
                assert_eq!(capture(&invalid, BASE, DATE).unwrap_err().kind, FailureKind::InvalidIdentity);
            }
        }
        // Registered IDs are u32. Lexer IDs retain their separate signed range.
        f.u32(SLOT_ROWS + 8, 0x80000000);
        let raw = capture(&f, BASE, DATE).unwrap();
        assert_eq!(raw["variants"][0]["definition"]["slot_identities"]["reason"]["kind"], "invalid_token_id");
        assert_eq!(raw["division_templates"][0]["id"], json!(template_id));
    }
}

#[test]
fn actual_readset_preflight_distinguishes_work_and_output_capacity() {
    use slot_identity::{AcquisitionRead, RefusalKind};
    let f = fixture();
    for (read_limit, allocation_limit, expected) in [
        (1, 100000, RefusalKind::ReadBudget),
        (100000, 100000, RefusalKind::AllocationBudget),
    ] {
        let budget = std::rc::Rc::new(std::cell::RefCell::new(
            slot_identity::Budget::new(read_limit, 1000, allocation_limit)));
        let mut read = ReadSet::with_budget(&f, BASE, budget.clone());
        let error = read.qualify(8, 2, 100001, 1).unwrap_err();
        assert_eq!(error.kind, expected);
        assert_eq!(budget.borrow().read_bytes_reserved, 0);
        assert_eq!(read.seal().unwrap_err().kind, FailureKind::BoundExceeded);
    }
}


#[test]
fn actual_capture_pair_preserves_current_binding_and_independent_labels() {
    let mut f = fixture_with_copied_slots(&[(500, "primary"), (0x165, "unused")]);
    f.string(VARIANT + 0x28, "  Vessel ");
    f.string(VARIANT + 0x48, " Mk II ");
    let (first, second) = capture_pair(&f, BASE, DATE).unwrap();
    assert_eq!(first, second);
    test_binding().validate_snapshot(&first).unwrap();
    let catalog = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
        &serde_json::to_vec(&first).unwrap()).unwrap();
    let inputs = catalog.native_designer_inputs_at_hour(100).unwrap();
    assert_eq!(inputs.variants[0].display_name, snapshot::design_catalog::DesignTextObservation::Measured {
        value: "  Vessel ".into(), utf8_hex: "202056657373656c20".into(),
    });
    assert_eq!(inputs.variants[0].variant_name, snapshot::design_catalog::DesignTextObservation::Measured {
        value: " Mk II ".into(), utf8_hex: "204d6b20494920".into(),
    });
    assert!(catalog.native_designer_inputs_at_hour(101).is_err());
}

#[test]
fn actual_capture_pair_cannot_reset_the_first_census_budget() {
    let f = fixture();
    let measurement = readset_accounting::budget();
    let first_calls = Cell::new(0);
    let measured_memory = |address, width| {
        first_calls.set(first_calls.get() + 1);
        f.read(address, width)
    };
    capture_with_budget(&measured_memory, BASE, DATE, measurement.clone()).unwrap();
    let bytes = measurement.borrow().read_bytes_reserved;
    let calls = measurement.borrow().read_calls_reserved;
    assert!(bytes > 0 && calls > 0);
    let bounded = std::rc::Rc::new(std::cell::RefCell::new(
        slot_identity::Budget::new(bytes, calls, 256 * 1024 * 1024)));
    let observed_calls = Cell::new(0);
    let memory = |address, width| {
        observed_calls.set(observed_calls.get() + 1);
        f.read(address, width)
    };
    assert_eq!(capture_pair_with_budget(&memory, BASE, DATE, bounded.clone()).unwrap_err().kind,
        FailureKind::BoundExceeded);
    assert!(first_calls.get() > 0 && first_calls.get() <= calls);
    assert_eq!(observed_calls.get(), first_calls.get());
    assert_eq!(bounded.borrow().read_bytes_reserved, bytes);
    assert_eq!(bounded.borrow().read_calls_reserved, calls);
}

#[test]
fn actual_capture_pair_refuses_second_census_hour_campaign_and_scalar_changes() {
    let first = fixture();
    let calls = Cell::new(0);
    capture(&|address, width| { calls.set(calls.get() + 1); first.read(address, width) }, BASE, DATE).unwrap();
    let boundary = calls.get();
    for change in 0..3 {
        let mut later = first.clone();
        let expected = match change {
            0 => { later.u32(STATE + DATE_OFFSET, DATE + 1); FailureKind::HourLabelMismatch }
            1 => { let mut other = test_binding(); other.game_unique_seed += 1;
                set_campaign(&mut later, &other); FailureKind::Torn }
            _ => { later.put(VARIANT + 0x24, &[255]); FailureKind::Torn }
        };
        let read_count = Cell::new(0);
        let memory = |address, width| {
            let previous = read_count.get(); read_count.set(previous + 1);
            if previous < boundary { first.read(address, width) } else { later.read(address, width) }
        };
        assert_eq!(capture_pair(&memory, BASE, DATE).unwrap_err().kind, expected, "change {change}");
        assert!(read_count.get() > boundary);
    }
}
