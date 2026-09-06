use super::*;
use std::cell::Cell;

const BASE: u64 = 0x140000000;
const HEAP: u64 = 0x10000000000;
const STATE: u64 = HEAP + 0x1000;
const COUNTRY: u64 = HEAP + 0x4000;
const MANAGER: u64 = HEAP + 0x7000;
const VARIANT: u64 = HEAP + 0x10000;
const TEMPLATE: u64 = HEAP + 0x12000;
const EQUIPMENT: u64 = HEAP + 0x14000;
const SUBUNIT: u64 = HEAP + 0x16000;
const DATE: u32 = DATE_EPOCH + 100;

#[derive(Clone, Default)]
struct Fixture {
    bytes: BTreeMap<u64, u8>,
}
impl Fixture {
    fn put(&mut self, address: u64, bytes: &[u8]) {
        for (i, byte) in bytes.iter().enumerate() {
            self.bytes.insert(address + i as u64, *byte);
        }
    }
    fn zero(&mut self, address: u64, length: usize) {
        self.put(address, &vec![0; length]);
    }
    fn u32(&mut self, address: u64, value: u32) {
        self.put(address, &value.to_le_bytes());
    }
    fn u64(&mut self, address: u64, value: u64) {
        self.put(address, &value.to_le_bytes());
    }
    fn string(&mut self, address: u64, value: &str) {
        assert!(value.len() < 16);
        self.zero(address, 32);
        self.put(address, value.as_bytes());
        self.u64(address + 16, value.len() as u64);
        self.u64(address + 24, 15);
    }
    fn vector(
        &mut self,
        owner: u64,
        data_offset: u64,
        count_offset: u64,
        data: u64,
        values: &[u64],
    ) {
        self.u64(owner + data_offset, data);
        self.u32(owner + count_offset - 4, values.len() as u32);
        self.u32(owner + count_offset, values.len() as u32);
        for (i, value) in values.iter().enumerate() {
            self.u64(data + i as u64 * 8, *value);
        }
    }
    fn registry(&mut self, kind: u32, object: u64, vtable: u64) -> u64 {
        let root = HEAP + 0x20000 + u64::from(kind) * 0x1000;
        let table = root + 0x100;
        self.u64(BASE + REFERENCE_TABLES_RVA + u64::from(kind) * 8, root);
        self.zero(root, 32);
        self.u64(root + 8, table);
        self.u32(root + 16, 1);
        self.u32(root + 20, 7);
        self.put(root + 24, &[5]);
        self.zero(table, 14 * 24);
        self.put(table + 13 * 24 + 4, &[255]);
        let mixed = ((u64::from(kind) << 32) | 1).wrapping_mul(0x49631a45);
        let hash = mixed as u32 ^ (mixed >> 32) as u32;
        let slot = table + u64::from(hash & 7) * 24;
        self.u32(slot, hash);
        self.put(slot + 4, &[1]);
        self.u32(slot + 8, kind);
        self.u32(slot + 12, 1);
        self.u64(slot + 16, object);
        self.u64(object, BASE + vtable);
        self.u32(object + 8, kind);
        self.u32(object + 12, 1);
        self.put(object + 16, &[1]);
        root
    }
}
impl Memory for Fixture {
    fn read(&self, address: u64, length: usize) -> std::result::Result<Vec<u8>, String> {
        (0..length)
            .map(|i| {
                self.bytes
                    .get(&(address + i as u64))
                    .copied()
                    .ok_or("unreadable".into())
            })
            .collect()
    }
}
fn test_binding() -> custody::StreamBinding {
    custody::StreamBinding {
        logical_stream_id: 0,
        game_unique_id: "a4b17fed-ed12-4361-9064-fb6576638110".into(),
        game_unique_seed: -1234,
        cid1_payload_sha256: "a".repeat(64),
    }
}
fn set_campaign(f: &mut Fixture, binding: &custody::StreamBinding) {
    use compact_contract::build_profile as profile;
    f.u32(
        STATE + profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF as u64,
        binding.game_unique_seed as u32,
    );
    let object = STATE + profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF as u64;
    let chars = HEAP + 0x1f000;
    f.zero(object, 32);
    f.u64(object, chars);
    f.u64(object + 16, binding.game_unique_id.len() as u64);
    f.u64(object + 24, binding.game_unique_id.len() as u64);
    f.put(chars, binding.game_unique_id.as_bytes());
}
fn fixture() -> Fixture {
    let mut f = Fixture::default();
    f.u64(BASE + GAME_STATE_RVA, STATE);
    f.zero(STATE, 0x500);
    f.u32(STATE + DATE_OFFSET, DATE);
    set_campaign(&mut f, &test_binding());
    let db = HEAP + 0x2000;
    let tags = HEAP + 0x3000;
    f.u64(BASE + 0x33189c8, db);
    f.zero(db, 0x90);
    f.u64(db, tags);
    f.u32(db + 8, 2);
    f.u32(db + 12, 2);
    f.u32(db + 0x88, 2);
    f.string(tags, "---");
    f.string(tags + 32, "POR");
    f.vector(STATE, 0x310, 0x31c, HEAP + 0x3800, &[COUNTRY]);
    f.zero(COUNTRY, 0x1100);
    f.u64(COUNTRY, BASE + 0x27d0e90);
    f.u32(COUNTRY + 8, 1);
    f.u64(COUNTRY + 0xf68, MANAGER);
    f.zero(MANAGER, 0x100);
    f.vector(COUNTRY, 0x1b8, 0x1c4, HEAP + 0x8000, &[TEMPLATE]);
    f.vector(MANAGER, 0xa0, 0xac, HEAP + 0x8100, &[VARIANT]);
    f.vector(MANAGER, 0xe8, 0xf4, HEAP + 0x8200, &[VARIANT]);
    f.vector(MANAGER, 0xb8, 0xc4, HEAP + 0x8300, &[VARIANT]);
    f.zero(VARIANT, 0x500);
    f.registry(70, VARIANT, VARIANT_VTABLE);
    f.u32(VARIANT + 0x18, 17);
    f.u32(VARIANT + 0x1c, 1);
    f.u64(VARIANT + 0x3f0, EQUIPMENT);
    f.string(VARIANT + 0x28, "Vessel");
    f.string(VARIANT + 0x48, "");
    f.string(VARIANT + 0x430, "");
    f.u32(VARIANT + 0x454, 1);
    f.string(VARIANT + 0x458, "");
    f.u64(VARIANT + 0xb0, BASE + 0x306d170);
    f.u64(VARIANT + 0x68, BASE + 0x27ab628);
    f.zero(EQUIPMENT, 64);
    f.u64(EQUIPMENT, BASE + 0x29218a0);
    f.u32(EQUIPMENT + 8, 17);
    f.string(EQUIPMENT + 0x18, "ship_hull");
    f.zero(TEMPLATE, 0x240);
    f.registry(52, TEMPLATE, TEMPLATE_VTABLE);
    f.string(TEMPLATE + 0x20, "Foot");
    f.u32(TEMPLATE + 0x60, (2 << 16) | 2);
    f.vector(
        TEMPLATE,
        0x68,
        0x74,
        HEAP + 0x18000,
        &[SUBUNIT, 0, SUBUNIT, 0],
    );
    f.zero(SUBUNIT, 64);
    f.u64(SUBUNIT, BASE + 0x2773ba8);
    f.put(SUBUNIT + 0x10, &[1]);
    f.u32(SUBUNIT + 8, 9);
    f.string(SUBUNIT + 0x18, "infantry");
    f
}

#[test]
fn native_census_admits_through_portable_contract_and_preserves_sources_and_placeholders() {
    let f = fixture();
    let value = capture(&f, BASE, DATE).unwrap();
    let portable = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
        &serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert_eq!(portable.records.len(), 2);
    assert_eq!(portable.capture_hour_index, 100);
    let variant = &value["variants"][0];
    assert_eq!(variant["source_occurrences"][0]["source_list"], "A");
    assert_eq!(variant["source_occurrences"][1]["source_list"], "B");
    assert_eq!(
        variant["developed_source_occurrences"][0]["source_list"],
        "developed"
    );
    assert_eq!(
        variant["definition"]["explicit_entity"],
        json!({"status":"measured","value":"","utf8_hex":""})
    );
    let definition = &value["division_templates"][0]["definition"];
    assert_eq!(definition["regiment_placeholder_ordinals"], json!([1, 3]));
    assert_eq!(definition["regiments"][1]["x"], 1);
    assert_eq!(definition["regiments"][1]["y"], 0);
}
#[test]
fn registry_keeps_a_variant_without_owned_developed_or_ui_occurrences() {
    let mut f = fixture();
    for count in [0xac, 0xf4, 0xc4] {
        f.u32(MANAGER + count, 0);
    }
    let value = capture(&f, BASE, DATE).unwrap();
    assert_eq!(value["variants"].as_array().unwrap().len(), 1);
    assert_eq!(value["variants"][0]["source_occurrences"], json!([]));
    assert_eq!(
        value["variants"][0]["developed_source_occurrences"],
        json!([])
    );
    assert_eq!(
        value["ui_occurrence_counts"]["catalog_variants_without_occurrence"],
        1
    );
}
#[test]
fn explicit_entity_unreadable_is_not_measured_empty() {
    let mut f = fixture();
    f.bytes.remove(&(VARIANT + 0x430));
    let value = capture(&f, BASE, DATE).unwrap();
    let entity = &value["variants"][0]["definition"]["explicit_entity"];
    assert_eq!(entity["status"], "unavailable");
    assert!(entity.get("value").is_none());
}
#[test]
fn registry_population_mismatch_is_a_named_refusal() {
    let mut f = fixture();
    let root = HEAP + 0x20000 + 70 * 0x1000;
    f.u32(root + 16, 2);
    assert_eq!(
        capture(&f, BASE, DATE).unwrap_err().kind,
        FailureKind::RegistryCountMismatch
    );
}
#[test]
fn nonnull_bad_subunit_is_not_a_placeholder() {
    let mut f = fixture();
    f.u64(HEAP + 0x18000 + 8, HEAP + 0xdead0);
    assert_eq!(
        capture(&f, BASE, DATE).unwrap_err().kind,
        FailureKind::Unreadable
    );
}
#[test]
fn later_hour_never_receives_the_requested_label() {
    let f = fixture();
    assert_eq!(
        capture(&f, BASE, DATE - 1).unwrap_err().kind,
        FailureKind::HourLabelMismatch
    );
}
#[test]
fn changed_field_at_final_seal_refuses_the_whole_census() {
    let f = fixture();
    let reads = Cell::new(0);
    let memory = |address, length| {
        let mut bytes = f.read(address, length)?;
        if address == VARIANT + 0x28 {
            let n = reads.get();
            reads.set(n + 1);
            if n > 0 {
                bytes[0] ^= 1;
            }
        }
        Ok(bytes)
    };
    assert_eq!(
        capture(&memory, BASE, DATE).unwrap_err().kind,
        FailureKind::Torn
    );
}

#[test]
fn registered_template_without_country_membership_keeps_complete_composition() {
    let mut f = fixture();
    f.u32(COUNTRY + 0x1c4, 0);
    let value = capture(&f, BASE, DATE).unwrap();
    assert_eq!(
        value["division_templates"][0]["country_source_occurrences"],
        json!([])
    );
    assert!(value["division_templates"][0]["owner_tag"].is_null());
    assert_eq!(value["completeness"]["owner"], false);
    let portable = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
        &serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let definition = portable.get("1:52").unwrap();
    assert_eq!(definition.owner, None);
    assert_eq!(
        definition.owner_completeness,
        snapshot::design_catalog::FieldCompleteness::Unknown
    );
    assert_eq!(
        definition.definition_completeness,
        snapshot::design_catalog::FieldCompleteness::Complete
    );
    assert_eq!(
        portable.division_composition_at_hour(100).unwrap()["1:52"]
            .regiments
            .len(),
        2
    );
    let bytes = portable.encode_portable_json().unwrap();
    let reopened =
        snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(&bytes).unwrap();
    assert_eq!(
        reopened.division_composition_at_hour(100).unwrap(),
        portable.division_composition_at_hour(100).unwrap()
    );
}

#[test]
fn repeated_country_template_occurrences_preserve_order_without_inventing_an_owner() {
    let mut f = fixture();
    f.vector(COUNTRY, 0x1b8, 0x1c4, HEAP + 0x19000, &[TEMPLATE, TEMPLATE]);
    let value = capture(&f, BASE, DATE).unwrap();
    let sources = value["division_templates"][0]["country_source_occurrences"]
        .as_array()
        .unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0]["country_vector_ordinal"], 0);
    assert_eq!(sources[1]["country_vector_ordinal"], 1);
    assert_eq!(value["division_templates"].as_array().unwrap().len(), 1);
    snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
        &serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
}

#[test]
fn variant_model_projection_distinguishes_empty_from_unavailable_and_keeps_membership() {
    let mut f = fixture();
    let value = capture(&f, BASE, DATE).unwrap();
    let portable = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
        &serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let model = portable.variant_model_inputs_at_hour(100, 1).unwrap();
    assert_eq!(
        model.explicit_entity,
        snapshot::design_catalog::DesignTextObservation::Measured {
            value: String::new(),
            utf8_hex: String::new()
        }
    );
    assert_eq!(model.owned.len(), 1);
    assert_eq!(model.owned[0].source_list, "A");
    assert_eq!(model.developed.len(), 1);
    assert_eq!(model.developed[0].source_list, "developed");
    f.bytes.remove(&(VARIANT + 0x430));
    let unavailable = capture(&f, BASE, DATE).unwrap();
    let portable = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
        &serde_json::to_vec(&unavailable).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        portable
            .variant_model_inputs_at_hour(100, 1)
            .unwrap()
            .explicit_entity,
        snapshot::design_catalog::DesignTextObservation::Unavailable { .. }
    ));
    assert!(portable.variant_model_inputs_at_hour(101, 1).is_err());
}

fn journal_test_paths(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let stem = format!(
        "cap-native-a27-{label}-{}-{stamp}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let root = std::env::temp_dir();
    (
        root.join(format!("{stem}.jsonl")),
        root.join(format!("{stem}.json")),
    )
}

#[test]
fn native_journal_seals_real_censuses_rebased_to_the_recorded_spine() {
    let (journal_path, output) = journal_test_paths("seal");
    let mut journal =
        timeline::TimelineJournal::create(journal_path.clone(), test_binding()).unwrap();
    let mut f = fixture();
    for offset in 0..3 {
        f.u32(STATE + DATE_OFFSET, DATE + offset);
        f.string(VARIANT + 0x430, if offset == 2 { "ship_model" } else { "" });
        let first = capture(&f, BASE, DATE + offset).unwrap();
        let recheck = capture(&f, BASE, DATE + offset).unwrap();
        journal.push(first, recheck).unwrap();
    }
    journal.finish().unwrap();
    timeline::seal_journal(&journal_path, &output, 101, 102, &test_binding()).unwrap();
    let bytes = std::fs::read(&output).unwrap();
    let raw: Value = snapshot::parse_json_strict(&bytes).unwrap();
    assert_eq!(raw["capture_count"], 2);
    assert!(raw["captures"][0].get("snapshot").is_some());
    assert!(raw["captures"][0].get("delta").is_none());
    assert!(raw["captures"][1].get("delta").is_some());
    let catalog =
        snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(&bytes).unwrap();
    assert_eq!(catalog.capture_hour_index, 101);
    assert_eq!(catalog.temporal_deltas.len(), 1);
    assert_eq!(
        catalog
            .variant_model_inputs_at_hour(102, 1)
            .unwrap()
            .explicit_entity,
        snapshot::design_catalog::DesignTextObservation::Measured {
            value: "ship_model".into(),
            utf8_hex: hex(b"ship_model")
        }
    );
    assert!(catalog.variant_model_inputs_at_hour(100, 1).is_err());
    std::fs::remove_file(journal_path).unwrap();
    std::fs::remove_file(output).unwrap();
}

#[test]
fn independent_same_hour_disagreement_never_enters_the_journal() {
    let (journal_path, _) = journal_test_paths("disagreement");
    let mut journal =
        timeline::TimelineJournal::create(journal_path.clone(), test_binding()).unwrap();
    let mut f = fixture();
    let first = capture(&f, BASE, DATE).unwrap();
    f.string(VARIANT + 0x430, "changed");
    let second = capture(&f, BASE, DATE).unwrap();
    assert!(journal
        .push(first, second)
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

#[test]
fn journal_hash_tamper_refuses_publication() {
    let (journal_path, output) = journal_test_paths("hash");
    let mut journal =
        timeline::TimelineJournal::create(journal_path.clone(), test_binding()).unwrap();
    let f = fixture();
    let first = capture(&f, BASE, DATE).unwrap();
    journal.push(first.clone(), first).unwrap();
    journal.finish().unwrap();
    let text = std::fs::read_to_string(&journal_path).unwrap();
    let lines: Vec<_> = text.lines().collect();
    let mut entry = snapshot::parse_json_strict(lines[1].as_bytes()).unwrap();
    entry["snapshot"]["variants"][0]["definition"]["display_name"] = json!("tampered");
    std::fs::write(&journal_path, format!("{}\n{}\n", lines[0], entry)).unwrap();
    assert!(
        timeline::seal_journal(&journal_path, &output, 100, 100, &test_binding())
            .unwrap_err()
            .contains("hash mismatch")
    );
    assert!(!output.exists());
    std::fs::remove_file(journal_path).unwrap();
    std::fs::remove_file(output.with_extension("sealing.json")).unwrap();
}

#[test]
fn engine_null_subunit_preserves_ordinal_but_unexpected_vtable_is_refused() {
    let mut f = fixture();
    let null = HEAP + 0x1a000;
    f.u64(null, BASE + 0x292d880);
    f.put(null + 0x10, &[0]);
    f.u64(HEAP + 0x18000 + 8, null);
    let value = capture(&f, BASE, DATE).unwrap();
    assert_eq!(
        value["division_templates"][0]["definition"]["regiment_placeholder_ordinals"],
        json!([1, 3])
    );
    let catalog = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
        &serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert_eq!(
        catalog.division_composition_at_hour(100).unwrap()["1:52"]
            .regiments
            .len(),
        2
    );
    f.u64(null, BASE + 0x292d888);
    assert_eq!(
        capture(&f, BASE, DATE).unwrap_err().kind,
        FailureKind::InvalidIdentity
    );
    f.u64(null, BASE + 0x292d880);
    f.put(null + 0x10, &[1]);
    assert_eq!(
        capture(&f, BASE, DATE).unwrap_err().kind,
        FailureKind::InvalidIdentity
    );
}

#[test]
fn refused_and_missing_hours_seal_with_exact_queries_and_complete_recovery() {
    use snapshot::design_catalog::{
        DesignCatalogGapCause, DesignTextObservation, PortableDesignCatalog,
    };
    let (journal_path, output) = journal_test_paths("gaps");
    let mut journal =
        timeline::TimelineJournal::create(journal_path.clone(), test_binding()).unwrap();
    let mut f = fixture();
    f.u32(STATE + DATE_OFFSET, DATE + 1);
    let first = capture(&f, BASE, DATE + 1).unwrap();
    journal.push(first.clone(), first).unwrap();
    journal
        .refusal(
            Some(DATE + 2),
            &failure(FailureKind::Torn, "membership changed"),
        )
        .unwrap();
    f.u32(STATE + DATE_OFFSET, DATE + 4);
    f.string(VARIANT + 0x430, "after_gap");
    let resumed = capture(&f, BASE, DATE + 4).unwrap();
    journal.push(resumed.clone(), resumed).unwrap();
    journal
        .refusal(
            Some(DATE + 5),
            &failure(FailureKind::HourLabelMismatch, "closing date"),
        )
        .unwrap();
    journal.finish().unwrap();
    timeline::seal_journal(&journal_path, &output, 100, 106, &test_binding()).unwrap();
    let raw = snapshot::parse_json_strict(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(raw["same_hour_recheck_count"], 2);
    let recovery = raw["captures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["capture_hour_index"] == 104)
        .unwrap();
    assert!(recovery.get("snapshot").is_some());
    assert!(recovery.get("delta").is_none());
    let catalog =
        PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).unwrap();
    assert_eq!(catalog.coverage_range().unwrap(), (100, 106));
    catalog
        .validate_replay_hours(&(100..=106).collect::<Vec<_>>())
        .unwrap();
    assert_eq!(
        catalog
            .gaps
            .iter()
            .map(|gap| (gap.first_hour_index, gap.last_hour_index))
            .collect::<Vec<_>>(),
        vec![(100, 100), (102, 102), (103, 103), (105, 105), (106, 106)]
    );
    assert_eq!(catalog.gaps[1].cause, DesignCatalogGapCause::Torn);
    assert_eq!(
        catalog.gaps[3].cause,
        DesignCatalogGapCause::HourLabelMismatch
    );
    for hour in [100, 102, 103, 105, 106] {
        assert!(catalog
            .records_at_hour(hour)
            .unwrap_err()
            .contains("A27 catalog gap"));
        assert!(catalog
            .division_composition_at_hour(hour)
            .unwrap_err()
            .contains("A27 catalog gap"));
        assert!(catalog
            .variant_model_inputs_at_hour(hour, 1)
            .unwrap_err()
            .contains("A27 catalog gap"));
    }
    assert_eq!(catalog.records_at_hour(101).unwrap().len(), 2);
    assert_eq!(catalog.records_at_hour(104).unwrap().len(), 2);
    assert_eq!(
        catalog
            .variant_model_inputs_at_hour(104, 1)
            .unwrap()
            .explicit_entity,
        DesignTextObservation::Measured {
            value: "after_gap".into(),
            utf8_hex: hex(b"after_gap")
        }
    );
    assert_eq!(catalog.temporal_deltas[0].variant_upserts.len(), 1);
    assert_eq!(catalog.temporal_deltas[0].template_upserts.len(), 1);
    let mut encoded =
        snapshot::parse_json_strict(&catalog.encode_portable_json().unwrap()).unwrap();
    let reopened =
        PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&encoded).unwrap())
            .unwrap();
    assert_eq!(reopened.gaps, catalog.gaps);
    assert_eq!(
        reopened.records_at_hour(104).unwrap(),
        catalog.records_at_hour(104).unwrap()
    );
    encoded["temporal_deltas"][0]["template_upserts"] = json!([]);
    assert!(
        PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&encoded).unwrap())
            .unwrap_err()
            .contains("complete baseline")
    );
    std::fs::remove_file(journal_path).unwrap();
    std::fs::remove_file(output).unwrap();
}

#[test]
fn no_successful_census_still_seals_unknown_hours_without_an_empty_measurement() {
    use snapshot::design_catalog::PortableDesignCatalog;
    let (journal_path, output) = journal_test_paths("all-gaps");
    let mut journal =
        timeline::TimelineJournal::create(journal_path.clone(), test_binding()).unwrap();
    journal
        .refusal(None, &failure(FailureKind::Unreadable, "clock root"))
        .unwrap();
    journal.finish().unwrap();
    timeline::seal_journal(&journal_path, &output, 100, 102, &test_binding()).unwrap();
    let raw = snapshot::parse_json_strict(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(raw["same_hour_recheck_count"], 0);
    assert!(raw["captures"][0].get("snapshot").is_none());
    let catalog =
        PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).unwrap();
    assert!(catalog.exact_native_catalog.is_null());
    assert!(catalog.records.is_empty());
    assert_eq!(catalog.gaps.len(), 1);
    catalog.validate_replay_hours(&[100, 101, 102]).unwrap();
    let reopened =
        PortableDesignCatalog::decode_a27_live_json(&catalog.encode_portable_json().unwrap())
            .unwrap();
    for hour in 100..=102 {
        assert!(reopened
            .records_at_hour(hour)
            .unwrap_err()
            .contains("catalog gap"));
    }
    std::fs::remove_file(journal_path).unwrap();
    std::fs::remove_file(output).unwrap();
}

#[test]
fn native_required_fields_cannot_be_omitted_or_claimed_as_old_corpus_defaults() {
    use snapshot::design_catalog::PortableDesignCatalog;
    let value = capture(&fixture(), BASE, DATE).unwrap();
    for (array, field) in [
        ("division_templates", "country_source_occurrences"),
        ("variants", "developed_source_occurrences"),
    ] {
        let mut missing = value.clone();
        missing[array][0].as_object_mut().unwrap().remove(field);
        assert!(PortableDesignCatalog::decode_a27_live_json(
            &serde_json::to_vec(&missing).unwrap()
        )
        .unwrap_err()
        .contains(field));
    }
    let mut missing = value;
    missing["variants"][0]["definition"]
        .as_object_mut()
        .unwrap()
        .remove("explicit_entity");
    assert!(
        PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&missing).unwrap())
            .unwrap_err()
            .contains("explicit_entity")
    );
}

#[test]
fn campaign_change_at_same_hour_is_refused_until_the_new_stream_is_acknowledged() {
    let mut f = fixture();
    let old = test_binding();
    let mut next = old.clone();
    next.logical_stream_id = 1;
    next.game_unique_id = "11111111-1111-4111-8111-111111111111".into();
    next.game_unique_seed = 991;
    next.cid1_payload_sha256 = "b".repeat(64);
    let mut gate = custody::ObservationGate::default();
    let (date, pair) = observe_campaign(&f, BASE).unwrap();
    gate.observe(&old, date, &pair).unwrap();
    let (old_path, old_output) = journal_test_paths("old-campaign");
    let mut journal = timeline::TimelineJournal::create(old_path.clone(), old.clone()).unwrap();
    let value = capture(&f, BASE, DATE).unwrap();
    journal.push(value.clone(), value).unwrap();
    journal.finish().unwrap();
    set_campaign(&mut f, &next);
    f.string(VARIANT + 0x430, "new_campaign");
    let (date, pair) = observe_campaign(&f, BASE).unwrap();
    assert!(gate.observe(&old, date, &pair).is_err());
    gate.observe(&next, date, &pair).unwrap();
    let value = capture(&f, BASE, DATE).unwrap();
    assert!(old.validate_snapshot(&value).is_err());
    let (next_path, next_output) = journal_test_paths("new-campaign");
    let mut journal = timeline::TimelineJournal::create(next_path.clone(), next.clone()).unwrap();
    journal.push(value.clone(), value).unwrap();
    journal.finish().unwrap();
    assert!(
        timeline::seal_journal(&old_path, &next_output, 100, 100, &next)
            .unwrap_err()
            .contains("different acknowledged stream")
    );
    for (path, output, binding, entity) in [
        (&old_path, &old_output, &old, ""),
        (&next_path, &next_output, &next, "new_campaign"),
    ] {
        timeline::seal_journal(path, output, 100, 100, binding).unwrap();
        let catalog = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
            &std::fs::read(output).unwrap(),
        )
        .unwrap();
        assert_eq!(
            catalog
                .variant_model_inputs_at_hour(100, 1)
                .unwrap()
                .explicit_entity,
            snapshot::design_catalog::DesignTextObservation::Measured {
                value: entity.into(),
                utf8_hex: hex(entity.as_bytes())
            }
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(output).unwrap();
    }
}

#[test]
fn rollback_overlapping_hours_starts_a_new_bound_baseline_and_never_reenters_old_stream() {
    let mut f = fixture();
    let old = test_binding();
    let mut next = old.clone();
    next.logical_stream_id = 1;
    next.cid1_payload_sha256 = "c".repeat(64);
    let mut gate = custody::ObservationGate::default();
    let (_, pair) = observe_campaign(&f, BASE).unwrap();
    gate.observe(&old, DATE + 2, &pair).unwrap();
    assert!(gate.observe(&old, DATE, &pair).is_err());
    assert!(gate.observe(&old, DATE + 3, &pair).is_err());
    gate.observe(&next, DATE, &pair).unwrap();
    let mut paths = Vec::new();
    for (binding, label, entity) in [
        (&old, "before-rollback", "before"),
        (&next, "after-rollback", "after"),
    ] {
        let (path, output) = journal_test_paths(label);
        let mut journal = timeline::TimelineJournal::create(path.clone(), binding.clone()).unwrap();
        f.string(VARIANT + 0x430, entity);
        for offset in 0..3 {
            f.u32(STATE + DATE_OFFSET, DATE + offset);
            let value = capture(&f, BASE, DATE + offset).unwrap();
            journal.push(value.clone(), value).unwrap();
        }
        journal.finish().unwrap();
        timeline::seal_journal(&path, &output, 100, 102, binding).unwrap();
        let raw = snapshot::parse_json_strict(&std::fs::read(&output).unwrap()).unwrap();
        assert!(raw["captures"][0]["snapshot"].is_object());
        assert_eq!(
            raw["authority"]["stream_binding"]["logical_stream_id"],
            binding.logical_stream_id
        );
        let catalog = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
            &serde_json::to_vec(&raw).unwrap(),
        )
        .unwrap();
        for hour in 100..=102 {
            assert_eq!(
                catalog
                    .variant_model_inputs_at_hour(hour, 1)
                    .unwrap()
                    .explicit_entity,
                snapshot::design_catalog::DesignTextObservation::Measured {
                    value: entity.into(),
                    utf8_hex: hex(entity.as_bytes())
                }
            );
        }
        paths.extend([path, output]);
    }
    for path in paths {
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn campaign_pair_change_during_the_census_is_a_typed_torn_observation() {
    let f = fixture();
    let count = Cell::new(0);
    let memory = |address, length| {
        let mut bytes = f.read(address, length)?;
        if address
            == STATE
                + compact_contract::build_profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF as u64
        {
            count.set(count.get() + 1);
            if count.get() > 1 {
                bytes[0] ^= 1;
            }
        }
        Ok(bytes)
    };
    assert_eq!(
        capture(&memory, BASE, DATE).unwrap_err().kind,
        FailureKind::Torn
    );
}

#[test]
fn country_pointer_order_is_independent_of_registry_order_and_survives_portable_admission() {
    let mut f = fixture();
    let other_country = HEAP + 0x90000;
    let country_bytes = f.read(COUNTRY, 0x1100).unwrap();
    f.put(other_country, &country_bytes);
    f.u32(other_country + 8, 2);
    let tag_db = HEAP + 0x2000;
    f.u32(tag_db + 8, 3);
    f.u32(tag_db + 12, 3);
    f.u32(tag_db + 0x88, 3);
    f.string(HEAP + 0x3000 + 64, "ITA");
    f.vector(
        STATE,
        0x310,
        0x31c,
        HEAP + 0x3800,
        &[other_country, COUNTRY],
    );
    let raw = capture(&f, BASE, DATE).unwrap();
    assert_eq!(
        raw["schema"],
        snapshot::design_catalog::A27_LIVE_CATALOG_SCHEMA
    );
    assert_eq!(
        raw["countries"],
        json!([
            {"registry_index":0,"tag":"ITA","designer_pointer_order_ordinal":1},
            {"registry_index":1,"tag":"POR","designer_pointer_order_ordinal":0},
        ])
    );
    let portable = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
        &serde_json::to_vec(&raw).unwrap(),
    )
    .unwrap();
    assert_eq!(portable.exact_native_catalog["countries"], raw["countries"]);
    f.vector(
        STATE,
        0x310,
        0x31c,
        HEAP + 0x3800,
        &[COUNTRY, other_country],
    );
    let reordered = capture(&f, BASE, DATE).unwrap();
    assert_eq!(
        reordered["countries"],
        json!([
            {"registry_index":0,"tag":"POR","designer_pointer_order_ordinal":0},
            {"registry_index":1,"tag":"ITA","designer_pointer_order_ordinal":1},
        ])
    );
}

#[test]
fn item_icon_override_preserves_raw_identity_without_reading_runtime_resource() {
    for (active, kind, token) in [
        (0, 1, ""),
        (1, 0, "GFX_test"),
        (1, 1, "."),
        (1, 1, "gfx/icon.dds"),
        (1, 1, "gfx/interface/custom_icon.dds"),
    ] {
        let mut f = fixture();
        f.put(VARIANT + 0x450, &[active]);
        f.u32(VARIANT + 0x454, kind);
        if token.len() < 16 {
            f.string(VARIANT + 0x458, token);
        } else {
            f.u64(VARIANT + 0x458, HEAP + 0x9000);
            f.u64(VARIANT + 0x468, token.len() as u64);
            f.u64(VARIANT + 0x470, token.len() as u64);
            f.put(HEAP + 0x9000, token.as_bytes());
        }
        let memory = |address, length| {
            assert!(address + length as u64 <= VARIANT + 0x478 || address >= VARIANT + 0x480);
            f.read(address, length)
        };
        let raw = capture(&memory, BASE, DATE).unwrap();
        let icon = &raw["variants"][0]["definition"]["item_icon_override"];
        assert_eq!(
            icon,
            &json!({"status":"measured", "active":active != 0, "kind":kind,
            "token":{"status":"measured", "value":token, "utf8_hex":hex(token.as_bytes())}})
        );
        let portable = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
            &serde_json::to_vec(&raw).unwrap(),
        )
        .unwrap();
        assert_eq!(
            portable.exact_native_catalog["variants"][0]["definition"]["item_icon_override"],
            *icon
        );
    }
}

#[test]
fn item_icon_unavailable_token_retains_measured_header_and_invalid_header_is_named() {
    for invalid_string in [false, true] {
        let mut f = fixture();
        f.put(VARIANT + 0x450, &[1]);
        if invalid_string {
            f.u64(VARIANT + 0x468, 4097);
        } else {
            f.bytes.remove(&(VARIANT + 0x458));
        }
        let raw = capture(&f, BASE, DATE).unwrap();
        let icon = &raw["variants"][0]["definition"]["item_icon_override"];
        assert_eq!(icon["status"], "measured");
        assert_eq!(icon["active"], true);
        assert_eq!(icon["kind"], 1);
        assert_eq!(icon["token"]["status"], "unavailable");
        assert_eq!(
            icon["token"]["reason"]["kind"],
            if invalid_string {
                "invalid_string"
            } else {
                "unreadable"
            }
        );
        assert!(icon["token"].get("value").is_none());
    }
    for (active, kind) in [(2, 1), (1, 2)] {
        let mut f = fixture();
        f.put(VARIANT + 0x450, &[active]);
        f.u32(VARIANT + 0x454, kind);
        let raw = capture(&f, BASE, DATE).unwrap();
        let icon = &raw["variants"][0]["definition"]["item_icon_override"];
        assert_eq!(icon["status"], "unavailable");
        assert_eq!(icon["reason"]["kind"], "invalid_identity");
        assert!(!icon["reason"]["field"].as_str().unwrap().is_empty());
        assert!(icon.get("token").is_none());
    }
    let mut f = fixture();
    f.bytes.remove(&(VARIANT + 0x454));
    let raw = capture(&f, BASE, DATE).unwrap();
    assert_eq!(
        raw["variants"][0]["definition"]["item_icon_override"]["reason"]["kind"],
        "unreadable"
    );
}

#[test]
fn item_icon_torn_header_or_token_refuses_census_and_budget_is_not_field_unknown() {
    for offset in [0x450, 0x454, 0x458] {
        let f = fixture();
        let count = Cell::new(0);
        let memory = |address, length| {
            let mut bytes = f.read(address, length)?;
            if address == VARIANT + offset {
                count.set(count.get() + 1);
                if count.get() > 1 {
                    bytes[0] ^= 1;
                }
            }
            Ok(bytes)
        };
        assert_eq!(
            capture(&memory, BASE, DATE).unwrap_err().kind,
            FailureKind::Torn
        );
    }
    let f = fixture();
    let mut read = ReadSet::new(&f, BASE);
    read.bytes = MAX_READ_BYTES;
    assert_eq!(
        item_icon_override(&mut read, VARIANT).unwrap_err().kind,
        FailureKind::BoundExceeded
    );
}
