// Included by native_design_catalog/tests.rs; Carver owns that include line.
// Fixture-only expectations pinned to A27_FINAL_NAMING_HANDSHAKE.json (2030 bytes,
// SHA256 1797152b6006ee313bb367b45291b5976fd9808bdd243032e8b41014536c0e69).
// Requires Carver's coherent actual host + portable grammar + existing Fixture.
// No collector prototype, replacement sampler, slot fixture, or native call.
mod template_variant_input_tests {
    use super::*;
    use snapshot::design_catalog::{
        DesignByteObservation, DesignCatalogGapCause, DesignI32Observation,
        NativeDesignerTemplateInputs, PortableDesignCatalog,
    };

    #[derive(Clone, Copy, Debug)]
    enum Operand {
        Country289a,
        Country3abe,
        AppendVariantName,
    }
    const OPERANDS: [Operand; 3] = [
        Operand::Country289a,
        Operand::Country3abe,
        Operand::AppendVariantName,
    ];
    impl Operand {
        fn address(self) -> u64 {
            match self {
                Self::Country289a => TEMPLATE + 0x1ec,
                Self::Country3abe => TEMPLATE + 0x1f4,
                Self::AppendVariantName => VARIANT + 0x24,
            }
        }
        fn width(self) -> usize {
            if matches!(self, Self::AppendVariantName) {
                1
            } else {
                4
            }
        }
        fn name(self) -> &'static str {
            match self {
                Self::Country289a => "country_289a",
                Self::Country3abe => "country_3abe",
                Self::AppendVariantName => "append_variant_name",
            }
        }
        fn rows(self) -> &'static str {
            if matches!(self, Self::AppendVariantName) {
                "variants"
            } else {
                "division_templates"
            }
        }
        fn upserts(self) -> &'static str {
            if matches!(self, Self::AppendVariantName) {
                "variant_upserts"
            } else {
                "template_upserts"
            }
        }
        fn other_upserts(self) -> &'static str {
            if matches!(self, Self::AppendVariantName) {
                "template_upserts"
            } else {
                "variant_upserts"
            }
        }
        fn set(self, f: &mut Fixture, changed: bool) {
            match self {
                Self::Country289a => f.put(
                    self.address(),
                    &(if changed { i32::MIN } else { 0 }).to_le_bytes(),
                ),
                Self::Country3abe => f.put(
                    self.address(),
                    &(if changed { i32::MAX } else { 0 }).to_le_bytes(),
                ),
                Self::AppendVariantName => f.put(self.address(), &[if changed { 255 } else { 0 }]),
            }
        }
        fn expected(self, changed: bool) -> Value {
            match self {
                Self::Country289a => {
                    json!({"status":"measured","raw_i32":if changed { i32::MIN } else { 0 }})
                }
                Self::Country3abe => {
                    json!({"status":"measured","raw_i32":if changed { i32::MAX } else { 0 }})
                }
                Self::AppendVariantName => {
                    json!({"status":"measured","raw_u8":if changed { 255 } else { 0 }})
                }
            }
        }
        fn observation<'a>(self, raw: &'a Value) -> &'a Value {
            &raw[self.rows()][0]["definition"][self.name()]
        }
    }
    fn zero_inputs() -> Fixture {
        let mut f = fixture();
        for operand in OPERANDS {
            operand.set(&mut f, false);
        }
        f
    }
    fn decode(raw: &Value) -> PortableDesignCatalog {
        PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(raw).unwrap()).unwrap()
    }
    fn assert_projection(catalog: &PortableDesignCatalog, hour: u64, raw: &Value) {
        let inputs = catalog.native_designer_inputs_at_hour(hour).unwrap();
        assert_eq!(inputs.hour, hour);
        assert_eq!(inputs.templates.len(), 1);
        let template: &NativeDesignerTemplateInputs = &inputs.templates[0];
        assert_eq!(template.design_id, "1:52");
        assert_eq!(template.template_id, 1);
        let definition = &raw["division_templates"][0]["definition"];
        assert_eq!(
            template.country_289a,
            serde_json::from_value::<DesignI32Observation>(definition["country_289a"].clone())
                .unwrap()
        );
        assert_eq!(
            template.country_3abe,
            serde_json::from_value::<DesignI32Observation>(definition["country_3abe"].clone())
                .unwrap()
        );
        assert_eq!(inputs.variants.len(), 1);
        assert_eq!(inputs.variants[0].design_id, "1:70");
        assert_eq!(
            inputs.variants[0].append_variant_name,
            serde_json::from_value::<DesignByteObservation>(
                raw["variants"][0]["definition"]["append_variant_name"].clone()
            )
            .unwrap()
        );
    }
    fn assert_unavailable(operand: Operand, raw: &Value) {
        let observed = operand.observation(raw);
        assert_eq!(observed["status"], "unavailable");
        assert_eq!(observed["reason"]["kind"], "unreadable");
        assert!(!observed["reason"]["field"].as_str().unwrap().is_empty());
        assert!(observed.get("raw_i32").is_none());
        assert!(observed.get("raw_u8").is_none());
    }
    fn assert_only_row_operand_changed(before: &Value, after: &Value, operand: Operand) {
        assert_ne!(operand.observation(before), operand.observation(after));
        for rows in ["variants", "division_templates"] {
            let left = before[rows].clone();
            let mut right = after[rows].clone();
            if rows == operand.rows() {
                right[0]["definition"][operand.name()] =
                    left[0]["definition"][operand.name()].clone();
            }
            assert_eq!(left, right, "only {:?} may change a native row", operand);
        }
    }
    // Change the actual memory byte at its second observation, inside capture's
    // readset seal. Do not synthesize a CatalogFailure or edit captured JSON.
    fn torn_capture(f: &Fixture, date: u32, operand: Operand) -> CatalogFailure {
        let reads = Cell::new(0);
        let memory = |address: u64, length: usize| {
            let mut bytes = f.read(address, length)?;
            if address <= operand.address() && operand.address() < address + length as u64 {
                reads.set(reads.get() + 1);
                if reads.get() == 2 {
                    bytes[(operand.address() - address) as usize] ^= 1;
                }
            }
            Ok(bytes)
        };
        let failure = capture(&memory, BASE, date).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Torn);
        assert_eq!(
            reads.get(),
            2,
            "actual initial read and seal must reach {:?}",
            operand
        );
        failure
    }
    fn push_current(journal: &mut timeline::TimelineJournal, f: &Fixture, date: u32) -> Value {
        let first = capture(f, BASE, date).unwrap();
        let recheck = capture(f, BASE, date).unwrap();
        assert_eq!(first, recheck);
        journal.push(first.clone(), recheck).unwrap();
        first
    }
    fn entry_at(raw: &Value, hour: u64) -> &Value {
        let entries: Vec<_> = raw["captures"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["capture_hour_index"] == hour)
            .collect();
        assert_eq!(entries.len(), 1);
        entries[0]
    }

    #[test]
    fn actual_capture_preserves_independent_signed_countries_and_nonboolean_append_bytes() {
        for (first, second, append) in [
            (i32::MIN, i32::MAX, 255u8),
            (-1, 0, 2),
            (0, -1, 0),
            (1, i32::MIN, 128),
            (i32::MAX, 1, 1),
        ] {
            let mut f = fixture();
            f.put(TEMPLATE + 0x1ec, &first.to_le_bytes());
            f.put(TEMPLATE + 0x1f4, &second.to_le_bytes());
            f.put(VARIANT + 0x24, &[append]);
            let raw = capture(&f, BASE, DATE).unwrap();
            assert_eq!(
                Operand::Country289a.observation(&raw),
                &json!({"status":"measured","raw_i32":first})
            );
            assert_eq!(
                Operand::Country3abe.observation(&raw),
                &json!({"status":"measured","raw_i32":second})
            );
            assert_eq!(
                Operand::AppendVariantName.observation(&raw),
                &json!({"status":"measured","raw_u8":append})
            );
            assert_eq!(
                raw["division_templates"][0]["owner_tag"], "POR",
                "membership owner remains independent"
            );
            assert_projection(&decode(&raw), 100, &raw);
        }
    }
    #[test]
    fn actual_readset_observes_each_raw_operand_once_and_seals_it_once() {
        let f = zero_inputs();
        let calls = std::cell::RefCell::new(Vec::new());
        let memory = |address: u64, length: usize| {
            calls.borrow_mut().push((address, length));
            f.read(address, length)
        };
        capture(&memory, BASE, DATE).unwrap();
        let calls = calls.borrow();
        let mut scalar_bytes = 0;
        for operand in OPERANDS {
            let selected: Vec<_> = calls
                .iter()
                .copied()
                .filter(|(p, n)| *p <= operand.address() && operand.address() < *p + *n as u64)
                .collect();
            assert_eq!(selected, vec![(operand.address(), operand.width()); 2]);
            scalar_bytes += selected.iter().map(|(_, n)| n).sum::<usize>();
        }
        assert_eq!(
            scalar_bytes,
            16 + 2,
            "V3 extra scalar reads including seal, excluding existing identity reads"
        );
    }
    #[test]
    fn initial_unreadability_is_local_and_distinct_from_measured_zero_through_projection() {
        let baseline = capture(&zero_inputs(), BASE, DATE).unwrap();
        for operand in OPERANDS {
            for byte in 0..operand.width() {
                let mut f = zero_inputs();
                f.bytes.remove(&(operand.address() + byte as u64));
                let raw = capture(&f, BASE, DATE).unwrap();
                assert_unavailable(operand, &raw);
                assert_only_row_operand_changed(&baseline, &raw, operand);
                assert_projection(&decode(&raw), 100, &raw);
            }
        }
    }
    #[test]
    fn mutation_of_either_country_or_append_byte_at_real_seal_refuses_entire_capture() {
        let f = zero_inputs();
        for operand in OPERANDS {
            torn_capture(&f, DATE, operand);
        }
    }
    #[test]
    fn unreadability_at_real_seal_is_not_downgraded_to_a_local_field_observation() {
        let f = zero_inputs();
        for operand in OPERANDS {
            let reads = Cell::new(0);
            let memory = |address: u64, length: usize| {
                if address <= operand.address() && operand.address() < address + length as u64 {
                    reads.set(reads.get() + 1);
                    if reads.get() == 2 {
                        return Err("closing operand unreadable".to_owned());
                    }
                }
                f.read(address, length)
            };
            assert_eq!(
                capture(&memory, BASE, DATE).unwrap_err().kind,
                FailureKind::Unreadable
            );
            assert_eq!(reads.get(), 2);
        }
    }
    #[test]
    fn changing_only_one_operand_between_real_same_hour_censuses_rejects_journal_entry() {
        for operand in OPERANDS {
            let (journal_path, _) = journal_test_paths(operand.name());
            let mut journal =
                timeline::TimelineJournal::create(journal_path.clone(), test_binding()).unwrap();
            let mut f = zero_inputs();
            let first = capture(&f, BASE, DATE).unwrap();
            operand.set(&mut f, true);
            let recheck = capture(&f, BASE, DATE).unwrap();
            assert_only_row_operand_changed(&first, &recheck, operand);
            assert!(journal
                .push(first, recheck)
                .unwrap_err()
                .contains("recheck disagrees"));
            assert_eq!(journal.last_hour(), None);
            journal.finish().unwrap();
            let text = std::fs::read_to_string(&journal_path).unwrap();
            let lines: Vec<_> = text.lines().collect();
            assert_eq!(lines.len(), 1);
            assert_eq!(
                snapshot::parse_json_strict(lines[0].as_bytes()).unwrap()["kind"],
                "stream_binding"
            );
            std::fs::remove_file(journal_path).unwrap();
        }
    }
    #[test]
    fn each_operand_drives_actual_delta_unavailable_gap_and_complete_baseline_recovery() {
        for operand in OPERANDS {
            let (journal_path, output) = journal_test_paths(operand.name());
            let mut journal =
                timeline::TimelineJournal::create(journal_path.clone(), test_binding()).unwrap();
            let mut f = zero_inputs();
            let baseline = push_current(&mut journal, &f, DATE);
            assert_eq!(operand.observation(&baseline), &operand.expected(false));
            f.u32(STATE + DATE_OFFSET, DATE + 1);
            operand.set(&mut f, true);
            let changed = push_current(&mut journal, &f, DATE + 1);
            assert_eq!(operand.observation(&changed), &operand.expected(true));
            assert_only_row_operand_changed(&baseline, &changed, operand);
            f.u32(STATE + DATE_OFFSET, DATE + 2);
            f.bytes.remove(&operand.address());
            let unavailable = push_current(&mut journal, &f, DATE + 2);
            assert_unavailable(operand, &unavailable);
            assert_only_row_operand_changed(&changed, &unavailable, operand);
            f.u32(STATE + DATE_OFFSET, DATE + 3);
            operand.set(&mut f, true);
            let torn = torn_capture(&f, DATE + 3, operand);
            journal.refusal(Some(DATE + 3), &torn).unwrap();
            assert_eq!(journal.last_hour(), Some(102));
            f.u32(STATE + DATE_OFFSET, DATE + 4);
            operand.set(&mut f, false);
            let recovery = push_current(&mut journal, &f, DATE + 4);
            assert_eq!(operand.observation(&recovery), &operand.expected(false));
            f.u32(STATE + DATE_OFFSET, DATE + 5);
            let unchanged = push_current(&mut journal, &f, DATE + 5);
            journal.finish().unwrap();
            timeline::seal_journal(&journal_path, &output, 100, 105, &test_binding()).unwrap();
            let bytes = std::fs::read(&output).unwrap();
            let raw = snapshot::parse_json_strict(&bytes).unwrap();
            for (hour, captured) in [(101, &changed), (102, &unavailable)] {
                let entry = entry_at(&raw, hour);
                assert!(entry.get("snapshot").is_none());
                let delta = &entry["delta"];
                assert_eq!(delta[operand.upserts()].as_array().unwrap().len(), 1);
                assert_eq!(delta[operand.other_upserts()], json!([]));
                assert_eq!(delta["variant_tombstones"], json!([]));
                assert_eq!(delta["template_tombstones"], json!([]));
                assert_eq!(delta[operand.upserts()][0], captured[operand.rows()][0]);
            }
            assert_eq!(entry_at(&raw, 103)["gap"]["cause"], "torn");
            let resumed = entry_at(&raw, 104);
            assert!(resumed.get("delta").is_none());
            assert_eq!(resumed["snapshot"], recovery);
            assert_eq!(
                resumed["snapshot"]["division_templates"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(resumed["snapshot"]["variants"].as_array().unwrap().len(), 1);
            for key in [
                "variant_upserts",
                "template_upserts",
                "variant_tombstones",
                "template_tombstones",
            ] {
                assert_eq!(entry_at(&raw, 105)["delta"][key], json!([]));
            }
            let catalog = PortableDesignCatalog::decode_a27_live_json(&bytes).unwrap();
            assert_eq!(catalog.gaps.len(), 1);
            assert_eq!(catalog.gaps[0].first_hour_index, 103);
            assert_eq!(catalog.gaps[0].last_hour_index, 103);
            assert_eq!(catalog.gaps[0].cause, DesignCatalogGapCause::Torn);
            let reopened = PortableDesignCatalog::decode_a27_live_json(
                &catalog.encode_portable_json().unwrap(),
            )
            .unwrap();
            for current in [&catalog, &reopened] {
                assert!(current
                    .native_designer_inputs_at_hour(103)
                    .unwrap_err()
                    .contains("Torn"));
                for (hour, observed) in [
                    (100, &baseline),
                    (101, &changed),
                    (102, &unavailable),
                    (104, &recovery),
                    (105, &unchanged),
                ] {
                    assert_projection(current, hour, observed);
                }
            }
            std::fs::remove_file(journal_path).unwrap();
            std::fs::remove_file(output).unwrap();
        }
    }
}
