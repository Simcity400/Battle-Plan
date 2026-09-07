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
    f.u32(BASE + RUNTIME_DLC_MASK_RVA, 0);
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
fn full_panel_catalog_comes_from_real_capture_recheck_and_seal() {
    use snapshot::design_catalog::{DesignItemIconObservation, DesignTextObservation, DesignU32Observation};
    use std::io::Write;

    const FIRST_HOUR: u32 = 16_959_360;
    const MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;
    let (journal_path, output) = journal_test_paths("full-panel");
    let mut f = fixture();
    f.string(HEAP + 0x3000 + 32, "GER");
    f.u32(BASE + RUNTIME_DLC_MASK_RVA, 1 << 4);
    f.string(VARIANT + 0x458, ".");
    f.u32(STATE + DATE_OFFSET, DATE_EPOCH + FIRST_HOUR);
    let initial = capture(&f, BASE, DATE_EPOCH + FIRST_HOUR).unwrap();
    let initial_recheck = capture(&f, BASE, DATE_EPOCH + FIRST_HOUR).unwrap();
    use compact_contract::campaign_identity::{Cid1Fields, Cid1Observation, Cid1Witnesses, encode_cid1};
    let observation = |raw: &Value| Cid1Observation {
        game_unique_id: raw["authority"]["campaign"]["game_unique_id"].as_str().unwrap().into(),
        game_unique_seed: i32::try_from(raw["authority"]["campaign"]["game_unique_seed"].as_i64().unwrap()).unwrap(),
        game_hour_raw: i32::try_from(raw["authority"]["current_date_raw"].as_u64().unwrap()).unwrap(),
    };
    let fields = Cid1Fields::from_stable_observations(observation(&initial), observation(&initial_recheck),
        compact_contract::build_profile::COLLECTOR_PROFILE_ID.into(), Cid1Witnesses::default()).unwrap();
    let payload = encode_cid1(&fields).unwrap();
    let mut cid1_record = vec![0; compact_contract::COMPACT_RECORD_HEADER_LEN];
    cid1_record[..4].copy_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
    cid1_record[4..6].copy_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
    cid1_record[6..8].copy_from_slice(&(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    cid1_record[16..20].copy_from_slice(&((DATE_EPOCH + FIRST_HOUR) as i32).to_le_bytes());
    cid1_record[22..24].copy_from_slice(&compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER.to_le_bytes());
    cid1_record[24..28].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    cid1_record.extend_from_slice(&payload);
    let first_record = crate::duplex_session::identity_control::ValidatedFirstCompactRecord::identity_v1(0, &cid1_record).unwrap();
    let binding = custody::StreamBinding::from_cid1(0, first_record.validated_cid1().unwrap());
    assert_ne!(binding.cid1_payload_sha256, test_binding().cid1_payload_sha256);
    let mut journal = timeline::TimelineJournal::create(journal_path.clone(), binding.clone()).unwrap();
    for hour in FIRST_HOUR..=FIRST_HOUR + 1 {
        let date = DATE_EPOCH.checked_add(hour).unwrap();
        f.u32(STATE + DATE_OFFSET, date);
        let (first, recheck) = if hour == FIRST_HOUR {
            (initial.clone(), initial_recheck.clone())
        } else {
            (capture(&f, BASE, date).unwrap(), capture(&f, BASE, date).unwrap())
        };
        assert_eq!(first, recheck);
        assert_eq!(first["variants"].as_array().unwrap().len(), 1);
        assert_eq!(first["division_templates"].as_array().unwrap().len(), 1);
        journal.push(first, recheck).unwrap();
    }
    journal.finish().unwrap();
    timeline::seal_journal(&journal_path, &output, FIRST_HOUR.into(), (FIRST_HOUR + 1).into(), &binding).unwrap();
    assert!(std::fs::metadata(&output).unwrap().len() <= MAX_OUTPUT_BYTES);
    let bytes = std::fs::read(&output).unwrap();
    let raw: Value = snapshot::parse_json_strict(&bytes).unwrap();
    assert_eq!(raw["capture_count"], 2);
    assert_eq!(raw["authority"]["stream_binding"], serde_json::to_value(&binding).unwrap());
    assert!(raw["captures"][0].get("snapshot").is_some());
    assert!(raw["captures"][1].get("delta").is_some());
    let catalog = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(&bytes).unwrap();
    for hour in FIRST_HOUR..=FIRST_HOUR + 1 {
        let inputs = catalog.native_designer_inputs_at_hour(hour.into()).unwrap();
        assert_eq!(inputs.hour, u64::from(hour));
        assert_eq!(inputs.runtime_dlc_mask, DesignU32Observation::Measured { raw_u32: 16 });
        assert_eq!(inputs.countries.len(), 1);
        assert_eq!(inputs.countries[0].tag, "GER");
        assert_eq!(inputs.variants.len(), 1);
        let variant = &inputs.variants[0];
        assert_eq!(variant.equipment_definition_token, "ship_hull");
        assert_eq!(variant.equipment_token_raw, 17);
        assert_eq!(variant.model.creator_tag, "GER");
        assert_eq!(variant.model.explicit_entity, DesignTextObservation::Measured { value: String::new(), utf8_hex: String::new() });
        assert_eq!(variant.item_icon_override, DesignItemIconObservation::Measured {
            active: false, kind: 1,
            token: DesignTextObservation::Measured { value: ".".into(), utf8_hex: "2e".into() },
        });
        assert_eq!(variant.source_occurrences.iter().map(|row| row.source_list.as_str()).collect::<Vec<_>>(), vec!["A", "B"]);
        assert!(variant.source_occurrences.iter().all(|row| row.source_country_tag == "GER"));
        assert_eq!(variant.model.developed.len(), 1);
        assert_eq!(variant.model.developed[0].source_country_tag, "GER");
        assert_eq!(variant.model.developed[0].source_list, "developed");
        assert_eq!(catalog.records_at_hour(hour.into()).unwrap().len(), 2);
    }
    assert!(catalog.native_designer_inputs_at_hour(u64::from(FIRST_HOUR - 1)).is_err());
    let mut other_binding = binding.clone();
    other_binding.game_unique_seed += 1;
    let (_, rejected_output) = journal_test_paths("full-panel-wrong-campaign");
    assert!(timeline::seal_journal(&journal_path, &rejected_output, FIRST_HOUR.into(), (FIRST_HOUR + 1).into(), &other_binding).is_err());
    assert!(!rejected_output.exists());
    if let Some(path) = std::env::var_os("BATTLE_PLAN_A27_FULL_PANEL_OUTPUT").or_else(|| std::env::var_os("HOI4_A27_FULL_PANEL_OUTPUT")) {
        let path = std::path::PathBuf::from(path);
        let identity_path = path.with_extension("cid1.h4cr");
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        let mut identity_file = std::fs::OpenOptions::new().write(true).create_new(true).open(identity_path).unwrap();
        file.write_all(&bytes).unwrap();
        identity_file.write_all(&cid1_record).unwrap();
        file.sync_all().unwrap();
        identity_file.sync_all().unwrap();
    }
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

#[test]
fn serialized_variant_flag_preserves_all_bytes_and_unavailable_through_actual_capture() {
    for value in 0..=u8::MAX {
        let mut f = fixture();
        f.put(VARIANT + 0x41e, &[value]);
        let raw = capture(&f, BASE, DATE).unwrap();
        let observation = &raw["variants"][0]["definition"]["serialized_variant_flag_2b9d"];
        assert_eq!(observation, &json!({"status":"measured", "raw_u8":value}));
        let portable = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
            &serde_json::to_vec(&raw).unwrap(),
        ).unwrap();
        assert_eq!(portable.exact_native_catalog["variants"][0]["definition"]["serialized_variant_flag_2b9d"], *observation);
    }
    let mut f = fixture();
    f.bytes.remove(&(VARIANT + 0x41e));
    let raw = capture(&f, BASE, DATE).unwrap();
    let observation = &raw["variants"][0]["definition"]["serialized_variant_flag_2b9d"];
    assert_eq!(observation["status"], "unavailable");
    assert_eq!(observation["reason"]["kind"], "unreadable");
    assert!(!observation["reason"]["field"].as_str().unwrap().is_empty());
    assert!(observation.get("raw_u8").is_none());
    snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
        &serde_json::to_vec(&raw).unwrap(),
    ).unwrap();
}

#[test]
fn serialized_variant_flag_torn_or_over_budget_refuses_the_catalog() {
    let f = fixture();
    let count = Cell::new(0);
    let memory = |address, length| {
        let mut bytes = f.read(address, length)?;
        if address == VARIANT + 0x41e {
            assert_eq!(length, 1);
            count.set(count.get() + 1);
            if count.get() > 1 { bytes[0] ^= 1; }
        }
        Ok(bytes)
    };
    assert_eq!(capture(&memory, BASE, DATE).unwrap_err().kind, FailureKind::Torn);
    assert_eq!(count.get(), 2);
    let mut read = ReadSet::new(&f, BASE);
    read.bytes = MAX_READ_BYTES;
    assert_eq!(serialized_variant_flag_2b9d(&mut read, VARIANT).unwrap_err().kind, FailureKind::BoundExceeded);
}

#[test]
fn runtime_dlc_mask_preserves_each_bit_zero_and_unavailable_in_actual_census() {
    for raw_u32 in [0, u32::MAX].into_iter().chain((0..32).map(|bit| 1u32 << bit)) {
        let mut f = fixture();
        f.u32(BASE + RUNTIME_DLC_MASK_RVA, raw_u32);
        let count = Cell::new(0);
        let memory = |address, length| {
            if address == BASE + RUNTIME_DLC_MASK_RVA {
                assert_eq!(length, 4);
                count.set(count.get() + 1);
            }
            f.read(address, length)
        };
        let raw = capture(&memory, BASE, DATE).unwrap();
        assert_eq!(count.get(), 2, "one observation plus its seal witness");
        let expected = json!({"status":"measured", "raw_u32":raw_u32});
        assert_eq!(raw["runtime_dlc_mask"], expected);
        let portable = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
            &serde_json::to_vec(&raw).unwrap(),
        ).unwrap();
        assert_eq!(portable.exact_native_catalog["runtime_dlc_mask"], expected);
    }
    let mut f = fixture();
    f.bytes.remove(&(BASE + RUNTIME_DLC_MASK_RVA + 2));
    let raw = capture(&f, BASE, DATE).unwrap();
    assert_eq!(raw["runtime_dlc_mask"]["status"], "unavailable");
    assert_eq!(raw["runtime_dlc_mask"]["reason"]["kind"], "unreadable");
    assert!(!raw["runtime_dlc_mask"]["reason"]["field"].as_str().unwrap().is_empty());
    assert!(raw["runtime_dlc_mask"].get("raw_u32").is_none());
}

#[test]
fn runtime_dlc_mask_torn_or_budget_exhaustion_refuses_the_whole_census() {
    let f = fixture();
    let count = Cell::new(0);
    let memory = |address, length| {
        let mut bytes = f.read(address, length)?;
        if address == BASE + RUNTIME_DLC_MASK_RVA {
            count.set(count.get() + 1);
            if count.get() > 1 { bytes[0] ^= 1; }
        }
        Ok(bytes)
    };
    assert_eq!(capture(&memory, BASE, DATE).unwrap_err().kind, FailureKind::Torn);
    assert_eq!(count.get(), 2);
    let mut read = ReadSet::new(&f, BASE);
    read.bytes = MAX_READ_BYTES;
    assert_eq!(runtime_dlc_mask(&mut read).unwrap_err().kind, FailureKind::BoundExceeded);
}

#[test]
fn cached_stats_capture_all_signed_bits_at_existing_offsets_and_scales() {
    let fields = [
        ("speed_cache", 0x270, 100_000, i64::MIN),
        ("range", 0x278, 1_000, i64::MAX),
        ("air_attack", 0x280, 100_000, -(1_i64 << 32) - 17),
        ("agility", 0x288, 100_000, (1_i64 << 32) + 19),
        ("max_speed", 0x2f0, 100_000, 1_i64 << 53),
        ("reliability", 0x308, 100_000, (1_i64 << 53) + 1),
        ("air_defence", 0x318, 100_000, -1),
        ("thrust", 0x320, 100_000, 0),
        ("fuel_consumption", 0x328, 100_000, 1),
        ("build_cost_ic", 0x368, 100_000, i64::from(i32::MAX) + 1),
    ];
    let mut f = fixture();
    for (_, offset, _, raw) in fields { f.put(VARIANT + offset, &raw.to_le_bytes()); }
    let value = capture(&f, BASE, DATE).unwrap();
    let stats = value["variants"][0]["definition"]["cached_stats"].as_array().unwrap();
    assert_eq!(stats.len(), fields.len());
    let portable = snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
        &serde_json::to_vec(&value).unwrap()).unwrap();
    let inputs = portable.native_designer_inputs_at_hour(100).unwrap();
    assert_eq!(inputs.variants[0].cached_stats.len(), fields.len());
    for (ordinal, (name, _, scale, raw)) in fields.into_iter().enumerate() {
        assert_eq!(stats[ordinal], json!({"source_ordinal":ordinal,"stat_id":name,"raw_i64":raw,"scale":scale}));
        let exact = &inputs.variants[0].cached_stats[ordinal];
        assert_eq!((exact.stat_id.as_str(), exact.scale, exact.raw_i64, exact.source_ordinal),
            (name, scale, raw, ordinal as u32));
    }
}

#[test]
fn cached_stat_upper_bytes_are_required_and_part_of_the_same_seal() {
    for offset in [0x270, 0x278, 0x280, 0x288, 0x2f0, 0x308, 0x318, 0x320, 0x328, 0x368] {
        let mut missing = fixture();
        missing.bytes.remove(&(VARIANT + offset + 7));
        assert_eq!(capture(&missing, BASE, DATE).unwrap_err().kind, FailureKind::Unreadable);
        let f = fixture();
        let reads = Cell::new(0);
        let memory = |address, length| {
            let mut bytes = f.read(address, length)?;
            if address == VARIANT + offset && length == 8 {
                let n = reads.get(); reads.set(n + 1);
                if n > 0 { bytes[7] ^= 0x80; }
            }
            Ok(bytes)
        };
        assert_eq!(capture(&memory, BASE, DATE).unwrap_err().kind, FailureKind::Torn);
        assert!(reads.get() >= 2);
    }
}

fn mission_inputs(value: &Value) -> Vec<snapshot::design_catalog::DesignMissionStatOperand> {
    snapshot::design_catalog::PortableDesignCatalog::decode_a27_live_json(
        &serde_json::to_vec(value).unwrap()).unwrap().native_designer_inputs_at_hour(100).unwrap()
        .variants.into_iter().next().unwrap().mission_stat_operands
}

const MISSION_GROUPS: u64 = HEAP + 0x90000;
const MISSION_STATS: u64 = HEAP + 0x100000;

fn set_mission_groups(f: &mut Fixture, groups: &[(u32, Vec<(u32, i64)>)]) {
    f.u64(VARIANT + 0x370, MISSION_GROUPS);
    f.u32(VARIANT + 0x37c, groups.len() as u32);
    for address in VARIANT + 0x378..VARIANT + 0x37c { f.bytes.remove(&address); }
    for (ordinal, (key, stats)) in groups.iter().enumerate() {
        let group = MISSION_GROUPS + ordinal as u64 * 0x28;
        let data = MISSION_STATS + ordinal as u64 * 0x20000;
        f.u32(group, *key);
        f.u64(group + 0x10, data);
        f.u32(group + 0x1c, stats.len() as u32);
        for (ordinal, (key, value)) in stats.iter().enumerate() {
            f.u32(data + ordinal as u64 * 0x10, *key);
            f.put(data + ordinal as u64 * 0x10 + 8, &value.to_le_bytes());
        }
    }
}

#[test]
fn mission_operands_keep_first_group_and_stat_without_reading_unselected_payloads() {
    let mut f = fixture();
    f.put(VARIANT + 0x218, &i64::MIN.to_le_bytes());
    f.put(VARIANT + 0x220, &((1_i64 << 53) + 1).to_le_bytes());
    set_mission_groups(&mut f, &[
        (0x4000, vec![]),
        (0x1000, vec![(0x24, 77), (0x23, -9), (0x23, 999)]),
        (0x1000, vec![(0x23, 123)]),
        (0x2000, vec![(0x24, i64::MAX), (0x24, 111)]),
    ]);
    for address in MISSION_GROUPS + 0x10..MISSION_GROUPS + 0x20 { f.bytes.remove(&address); }
    for address in MISSION_STATS + 0x20000 + 8..MISSION_STATS + 0x20000 + 16 { f.bytes.remove(&address); }
    let value = capture(&f, BASE, DATE).unwrap();
    let rows = &value["variants"][0]["definition"]["mission_stat_operands"];
    assert_eq!(rows[0], json!({"group_key":4096,"stat_key":35,
        "fallback":{"status":"measured","raw_i64":i64::MIN},
        "override_lookup":{"status":"measured","raw_i64":-9,"group_count":4,
            "group_key_prefix":[16384,4096],"stat_count":3,"stat_key_prefix":[36,35]}}));
    assert_eq!(rows[1], json!({"group_key":8192,"stat_key":36,
        "fallback":{"status":"measured","raw_i64":(1_i64<<53)+1},
        "override_lookup":{"status":"measured","raw_i64":i64::MAX,"group_count":4,
            "group_key_prefix":[16384,4096,4096,8192],"stat_count":2,"stat_key_prefix":[36]}}));
    let inputs = mission_inputs(&value);
    assert_eq!(inputs[0].effective_raw_i64(), Ok(-9));
    assert_eq!(inputs[1].effective_raw_i64(), Ok(i64::MAX));
}

#[test]
fn mission_absence_is_a_complete_search_and_never_searches_a_later_duplicate_group() {
    let mut f = fixture();
    f.put(VARIANT + 0x218, &(-77_i64).to_le_bytes());
    set_mission_groups(&mut f, &[(0x1000, vec![(0x24, 17)]), (0x1000, vec![(0x23, 999)])]);
    let value = capture(&f, BASE, DATE).unwrap();
    let rows = &value["variants"][0]["definition"]["mission_stat_operands"];
    assert_eq!(rows[0]["override_lookup"], json!({"status":"stat_absent","group_count":2,
        "group_key_prefix":[4096],"stat_count":1,"stat_key_prefix":[36]}));
    assert_eq!(rows[1]["override_lookup"], json!({"status":"group_absent","group_count":2,
        "group_key_prefix":[4096,4096]}));
    assert_eq!(mission_inputs(&value)[0].effective_raw_i64(), Ok(-77));
    set_mission_groups(&mut f, &[(0x1000, vec![]), (0x1000, vec![(0x23, 999)])]);
    for address in MISSION_GROUPS + 0x10..MISSION_GROUPS + 0x18 { f.bytes.remove(&address); }
    let zero_stats = capture(&f, BASE, DATE).unwrap();
    assert_eq!(zero_stats["variants"][0]["definition"]["mission_stat_operands"][0]["override_lookup"],
        json!({"status":"stat_absent","group_count":2,"group_key_prefix":[4096],"stat_count":0,"stat_key_prefix":[]}));
    assert_eq!(mission_inputs(&zero_stats)[0].effective_raw_i64(), Ok(-77));
    set_mission_groups(&mut f, &[]);
    for address in VARIANT + 0x370..VARIANT + 0x378 { f.bytes.remove(&address); }
    let empty = capture(&f, BASE, DATE).unwrap();
    for row in empty["variants"][0]["definition"]["mission_stat_operands"].as_array().unwrap() {
        assert_eq!(row["override_lookup"], json!({"status":"group_absent","group_count":0,"group_key_prefix":[]}));
    }
}

#[test]
fn mission_fallback_and_override_unreadability_remain_independent() {
    let mut f = fixture();
    set_mission_groups(&mut f, &[(0x1000, vec![(0x23, 0)]), (0x2000, vec![(0x24, -1)])]);
    f.bytes.remove(&(VARIANT + 0x218 + 7));
    let value = capture(&f, BASE, DATE).unwrap();
    let rows = &value["variants"][0]["definition"]["mission_stat_operands"];
    assert_eq!(rows[0]["fallback"]["status"], "unavailable");
    assert_eq!(rows[0]["fallback"]["reason"]["kind"], "unreadable");
    assert_eq!(rows[0]["override_lookup"]["status"], "measured");
    assert_eq!(rows[0]["override_lookup"]["raw_i64"], 0);
    assert_eq!(mission_inputs(&value)[0].effective_raw_i64(), Ok(0));
    f.u64(VARIANT + 0x218, 55);
    f.bytes.remove(&(MISSION_STATS + 15));
    let value = capture(&f, BASE, DATE).unwrap();
    let rows = &value["variants"][0]["definition"]["mission_stat_operands"];
    assert_eq!(rows[0]["override_lookup"]["status"], "unavailable");
    assert_eq!(rows[0]["override_lookup"]["reason"]["kind"], "unreadable");
    assert!(rows[0]["override_lookup"].get("raw_i64").is_none());
    assert_eq!(rows[1]["override_lookup"]["raw_i64"], -1);
    assert_eq!(rows[0]["fallback"]["raw_i64"], 55);
    assert_eq!(mission_inputs(&value)[0].effective_raw_i64().unwrap_err().kind, "unreadable");
}

#[test]
fn mission_negative_counts_are_invalid_vectors_not_measured_absence() {
    for count in [-1_i32, i32::MIN] {
        let mut f = fixture();
        f.u32(VARIANT + 0x37c, count as u32);
        let value = capture(&f, BASE, DATE).unwrap();
        for row in value["variants"][0]["definition"]["mission_stat_operands"].as_array().unwrap() {
            assert_eq!(row["override_lookup"]["status"], "unavailable");
            assert_eq!(row["override_lookup"]["reason"]["kind"], "invalid_vector");
        }
        set_mission_groups(&mut f, &[(0x1000, vec![]), (0x2000, vec![(0x24, 11)])]);
        f.u32(MISSION_GROUPS + 0x1c, count as u32);
        let value = capture(&f, BASE, DATE).unwrap();
        let rows = &value["variants"][0]["definition"]["mission_stat_operands"];
        assert_eq!(rows[0]["override_lookup"]["reason"]["kind"], "invalid_vector");
        assert_eq!(rows[1]["override_lookup"]["raw_i64"], 11);
    }
}

#[test]
fn mission_key_prefix_bounds_refuse_without_truncating_or_inventing_absence() {
    let mut f = fixture();
    set_mission_groups(&mut f, &[(0x1000, vec![(0x23, 7)]), (0x2000, vec![(0x24, 8)])]);
    f.u32(VARIANT + 0x37c, i32::MAX as u32);
    f.u32(MISSION_GROUPS + 0x1c, i32::MAX as u32);
    let value = capture(&f, BASE, DATE).unwrap();
    let row = &value["variants"][0]["definition"]["mission_stat_operands"][0]["override_lookup"];
    assert_eq!(row["group_count"], i32::MAX);
    assert_eq!(row["stat_count"], i32::MAX);
    assert_eq!(row["stat_key_prefix"], json!([35]));
    for ordinal in 0..4096_u64 { f.u32(MISSION_GROUPS + ordinal * 0x28, 0x4000); }
    assert_eq!(capture(&f, BASE, DATE).unwrap_err().kind, FailureKind::BoundExceeded);
    set_mission_groups(&mut f, &[(0x1000, vec![])]);
    f.u32(MISSION_GROUPS + 0x1c, 4097);
    for ordinal in 0..4096_u64 { f.u32(MISSION_STATS + ordinal * 0x10, 0x24); }
    assert_eq!(capture(&f, BASE, DATE).unwrap_err().kind, FailureKind::BoundExceeded);
    let mut read = ReadSet::new(&f, BASE);
    read.bytes = MAX_READ_BYTES;
    assert_eq!(mission_stat_operands(&mut read, VARIANT).unwrap_err().kind, FailureKind::BoundExceeded);
}

#[test]
fn mission_headers_prefixes_and_raw_high_bytes_share_the_catalog_seal() {
    for (address, length) in [
        (VARIANT + 0x218, 8), (VARIANT + 0x220, 8), (VARIANT + 0x370, 8),
        (VARIANT + 0x37c, 4), (MISSION_GROUPS, 4), (MISSION_GROUPS + 0x10, 8),
        (MISSION_GROUPS + 0x1c, 4), (MISSION_STATS, 4), (MISSION_STATS + 8, 8),
    ] {
        let mut f = fixture();
        set_mission_groups(&mut f, &[(0x1000, vec![(0x23, 5)])]);
        let reads = Cell::new(0);
        let memory = |current_address, current_length| {
            let mut bytes = f.read(current_address, current_length)?;
            if current_address == address && current_length == length {
                let n = reads.get(); reads.set(n + 1);
                if n > 0 { bytes[length - 1] ^= 0x80; }
            }
            Ok(bytes)
        };
        assert_eq!(capture(&memory, BASE, DATE).unwrap_err().kind, FailureKind::Torn);
        assert!(reads.get() >= 2);
    }
}

#[test]
fn mission_first_match_at_key_4096_is_measured_without_reading_key_4097() {
    let mut f = fixture();
    let mut groups = vec![(0x4000, Vec::new()); 4096];
    groups[4094] = (0x1000, Vec::new());
    groups[4095] = (0x2000, Vec::new());
    set_mission_groups(&mut f, &groups);
    let value = capture(&f, BASE, DATE).unwrap();
    let rows = &value["variants"][0]["definition"]["mission_stat_operands"];
    assert_eq!(rows[1]["override_lookup"]["status"], "stat_absent");
    assert_eq!(rows[1]["override_lookup"]["group_key_prefix"].as_array().unwrap().len(), 4096);
    let mut stats = vec![(0x24, 0); 4096];
    stats[4095] = (0x23, i64::MIN);
    set_mission_groups(&mut f, &[(0x1000, stats)]);
    let value = capture(&f, BASE, DATE).unwrap();
    assert_eq!(value["variants"][0]["definition"]["mission_stat_operands"][0]["override_lookup"]["stat_key_prefix"].as_array().unwrap().len(), 4096);
    assert_eq!(mission_inputs(&value)[0].effective_raw_i64(), Ok(i64::MIN));
}
