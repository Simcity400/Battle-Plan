use characters_leaders_cabinet_live_lane::adapter::{
    build_lane_frame_from_adapter_input, AdapterInput, AdvisorRoot, CharacterPortraitInput,
    CharacterRoot, CountryLeaderRoleRoot, CountryPoliticsRoot, TraitDefinitionRoot, UnitLeaderRoot,
    CARMY_LEADER_ATTACK_OFF, CARMY_LEADER_DEFENSE_OFF, CARMY_LEADER_LOGISTICS_OFF,
    CARMY_LEADER_PLANNING_OFF, CUNIT_LEADER_EXPERIENCE_RAW_OFF,
};
use characters_leaders_cabinet_live_lane::domain47_characters_leaders_cabinet_live as compact;
use characters_leaders_cabinet_live_lane::extractor::{
    self, LaneMemory, ResolvedTypeRoots, CADVISOR_CHARACTER_PTR_OFF,
    CCHARACTER_COUNTRY_TAG_DB_ID_OFF, CCHARACTER_ID_OFF, CCHARACTER_TYPE_ID, CCHARACTER_TYPE_OFF,
    CCHARACTER_UNIT_LEADER_PTR_OFF, CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
    CCOUNTRY_POLITICAL_STATUS_OFF, CPOLITICAL_PARTY_LEADER_VEC_OFF,
    CPOLITICAL_STATUS_RULING_PARTY_OFF, CUNIT_LEADER_CHARACTER_ID_COPY_OFF, CUNIT_LEADER_ID_OFF,
    CUNIT_LEADER_SKILL_SOURCE_PTR_OFF, CUNIT_LEADER_SKILL_VALUE_OFF,
    CUNIT_LEADER_TRAIT_CAPACITY_OFF, CUNIT_LEADER_TRAIT_DATA_OFF, CUNIT_LEADER_TRAIT_SIZE_OFF,
    CUNIT_LEADER_TYPE_ID, CUNIT_LEADER_TYPE_OFF,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const CHAR_VT: u64 = 0x1000_1000;
const ARMY_VT: u64 = 0x1000_2000;
const NAVY_VT: u64 = 0x1000_3000;
const COUNTRY_LEADER_VT: u64 = 0x1000_4000;
const ADVISOR_VT: u64 = 0x1000_5000;
const TRAIT_VT: u64 = 0x1000_6000;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                "extractor/native/runs/characters_leaders_cabinet_live_20260701_adapter_fixture_v1",
            )
        });
    fs::create_dir_all(&out_dir).map_err(|err| format!("create out dir failed: {err}"))?;

    let mut mem = FakeMemory::default();
    let roots = roots();
    let input = fixture_input(&mut mem);
    let result = build_lane_frame_from_adapter_input(&mem, &roots, &input);
    let payload = extractor::encode_frame_payload(&result.frame);
    let record = extractor::encode_frame_record(1, 1946051909, &result.frame);
    let summary = extractor::decode_frame_payload(0, &payload);
    if !summary.parse_errors.is_empty() {
        return Err(format!(
            "adapter fixture compact decode failed: {:?}",
            summary.parse_errors
        ));
    }
    if result.frame.status_flags != 0 || summary.status_flags_or != 0 {
        return Err(format!(
            "adapter fixture emitted status flags frame={} summary={}",
            result.frame.status_flags, summary.status_flags_or
        ));
    }
    fs::write(out_dir.join("domain47_payload.bin"), &payload)
        .map_err(|err| format!("write payload failed: {err}"))?;
    fs::write(out_dir.join("domain47_record.bin"), &record)
        .map_err(|err| format!("write record failed: {err}"))?;

    let report = format!(
        "{{\n  \"result\": \"validated\",\n  \"domain\": \"{}\",\n  \"domain_id\": {},\n  \"payload_bytes\": {},\n  \"record_bytes\": {},\n  \"row_count_total\": {},\n  \"character_rows\": {},\n  \"unit_leader_rows\": {},\n  \"unit_leader_trait_rows\": {},\n  \"country_leader_role_rows\": {},\n  \"active_ruler_rows\": {},\n  \"advisor_roster_rows\": {},\n  \"appointed_advisor_rows\": {},\n  \"character_portrait_rows\": {},\n  \"status_flags_or\": {},\n  \"schema_flags_or\": {},\n  \"adapter_stats\": {},\n  \"proof_scope\": \"lane-local CollectorState-to-LaneFrame adapter fixture, not shared DLL capture\",\n  \"parse_errors\": []\n}}\n",
        compact::DOMAIN_NAME,
        compact::DOMAIN_ID,
        payload.len(),
        record.len(),
        summary.row_count_total,
        summary.character_rows,
        summary.unit_leader_rows,
        summary.trait_rows,
        summary.country_leader_role_rows,
        summary.active_ruler_rows,
        summary.advisor_roster_rows,
        summary.appointed_advisor_rows,
        summary.character_portrait_rows,
        summary.status_flags_or,
        summary.schema_flags_or,
        adapter_stats_json(&result.stats),
    );
    fs::write(out_dir.join("adapter_fixture_gate.json"), report.as_bytes())
        .map_err(|err| format!("write report failed: {err}"))?;
    print!("{report}");
    Ok(())
}

fn fixture_input(mem: &mut FakeMemory) -> AdapterInput {
    let character = 0x1000;
    let leader = 0x2000;
    let skill_source = 0x2100;
    let trait_data = 0x2200;
    let trait_def = 0x2300;
    let country_leader = 0x3000;
    let country = 0x4000;
    let status = 0x4100;
    let party = 0x4200;
    let leader_vec = 0x4300;
    let advisor = 0x5000;

    write_character(mem, character, 1458, 2);
    mem.write_u64(leader, ARMY_VT);
    mem.write_u32(leader + CUNIT_LEADER_TYPE_OFF, CUNIT_LEADER_TYPE_ID);
    mem.write_u32(leader + CUNIT_LEADER_ID_OFF, 500);
    mem.write_u32(leader + CUNIT_LEADER_CHARACTER_ID_COPY_OFF, 1458);
    mem.write_i32(leader + CUNIT_LEADER_EXPERIENCE_RAW_OFF, 42_000);
    mem.write_u64(leader + CUNIT_LEADER_SKILL_SOURCE_PTR_OFF, skill_source);
    mem.write_u32(skill_source + CUNIT_LEADER_SKILL_VALUE_OFF, 4);
    mem.write_u8(leader + CARMY_LEADER_ATTACK_OFF, 6);
    mem.write_u8(leader + CARMY_LEADER_DEFENSE_OFF, 7);
    mem.write_u8(leader + CARMY_LEADER_PLANNING_OFF, 5);
    mem.write_u8(leader + CARMY_LEADER_LOGISTICS_OFF, 8);
    mem.write_u64(leader + CUNIT_LEADER_TRAIT_DATA_OFF, trait_data);
    mem.write_u32(leader + CUNIT_LEADER_TRAIT_CAPACITY_OFF, 1);
    mem.write_u32(leader + CUNIT_LEADER_TRAIT_SIZE_OFF, 1);
    mem.write_u64(trait_data, trait_def);
    mem.write_u64(trait_def, TRAIT_VT);

    mem.write_u64(country_leader, COUNTRY_LEADER_VT);
    mem.write_u64(
        country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
        character,
    );
    mem.write_u64(country + CCOUNTRY_POLITICAL_STATUS_OFF, status);
    mem.write_u64(status + CPOLITICAL_STATUS_RULING_PARTY_OFF, party);
    mem.write_u64(party + CPOLITICAL_PARTY_LEADER_VEC_OFF, leader_vec);
    mem.write_u64(party + CPOLITICAL_PARTY_LEADER_VEC_OFF + 8, leader_vec + 8);
    mem.write_u64(leader_vec, country_leader);

    mem.write_u64(advisor, ADVISOR_VT);
    mem.write_u64(advisor + CADVISOR_CHARACTER_PTR_OFF, character);

    AdapterInput {
        elapsed_wall_ms: 77,
        characters: vec![CharacterRoot {
            character_addr: character,
            char_id: 1458,
            name: Some("Maxwell Aitken".to_string()),
            token: Some("ENG_maxwell_aitken".to_string()),
        }],
        unit_leaders: vec![UnitLeaderRoot {
            leader_addr: leader,
            country_tag_db_id: 2,
            leader_kind: compact::UNIT_LEADER_ARMY,
            name: Some("Maxwell Aitken".to_string()),
            token: Some("ENG_maxwell_aitken".to_string()),
        }],
        unit_leader_trait_tokens: vec![TraitDefinitionRoot {
            trait_addr: trait_def,
            token: "old_guard".to_string(),
        }],
        country_leader_roles: vec![CountryLeaderRoleRoot {
            country_leader_addr: country_leader,
            char_id: 1458,
            name: Some("Maxwell Aitken".to_string()),
            token: Some("ENG_maxwell_aitken".to_string()),
        }],
        countries: vec![CountryPoliticsRoot {
            country_addr: country,
            country_tag_db_id: 2,
            ruling_party: Some("democratic".to_string()),
            ruler_name: Some("Maxwell Aitken".to_string()),
            ruler_token: Some("ENG_maxwell_aitken".to_string()),
            active_idea_tokens: vec!["ENG_maxwell_aitken".to_string()],
        }],
        advisors: vec![AdvisorRoot {
            advisor_addr: advisor,
            idea_token: "ENG_maxwell_aitken".to_string(),
            slot: Some("political_advisor".to_string()),
            ledger: None,
        }],
        portraits: vec![CharacterPortraitInput {
            country_tag_db_id: 2,
            char_id: 1458,
            role: "civilian".to_string(),
            size: "large".to_string(),
            gfx: "GFX_portrait_ENG_maxwell_aitken".to_string(),
        }],
        ..AdapterInput::default()
    }
}

fn roots() -> ResolvedTypeRoots {
    ResolvedTypeRoots {
        ccharacter_vtable: CHAR_VT,
        carmy_leader_vtable: ARMY_VT,
        cnavy_leader_vtable: NAVY_VT,
        ccountry_leader_vtable: COUNTRY_LEADER_VT,
        cadvisor_vtable: ADVISOR_VT,
        cunit_leader_trait_vtable: TRAIT_VT,
    }
}

fn write_character(mem: &mut FakeMemory, addr: u64, char_id: u32, tag_db_id: u32) {
    mem.write_u64(addr, CHAR_VT);
    mem.write_u32(addr + CCHARACTER_TYPE_OFF, CCHARACTER_TYPE_ID);
    mem.write_u32(addr + CCHARACTER_ID_OFF, char_id);
    mem.write_u32(addr + CCHARACTER_COUNTRY_TAG_DB_ID_OFF, tag_db_id);
    mem.write_u64(addr + CCHARACTER_UNIT_LEADER_PTR_OFF, 0x2000);
}

fn adapter_stats_json(
    stats: &characters_leaders_cabinet_live_lane::adapter::AdapterBuildStats,
) -> String {
    format!(
        "{{\"character_roots_seen\":{},\"character_rows\":{},\"unit_leader_roots_seen\":{},\"unit_leader_rows\":{},\"unit_leader_trait_rows\":{},\"country_leader_roots_seen\":{},\"country_leader_role_rows\":{},\"country_roots_seen\":{},\"active_ruler_rows\":{},\"advisor_roots_seen\":{},\"advisor_roster_rows\":{},\"appointed_advisor_rows\":{},\"portrait_inputs_seen\":{},\"portrait_rows\":{},\"rejected_roots\":{},\"missing_trait_tokens\":{}}}",
        stats.character_roots_seen,
        stats.character_rows,
        stats.unit_leader_roots_seen,
        stats.unit_leader_rows,
        stats.unit_leader_trait_rows,
        stats.country_leader_roots_seen,
        stats.country_leader_role_rows,
        stats.country_roots_seen,
        stats.active_ruler_rows,
        stats.advisor_roots_seen,
        stats.advisor_roster_rows,
        stats.appointed_advisor_rows,
        stats.portrait_inputs_seen,
        stats.portrait_rows,
        stats.rejected_roots,
        stats.missing_trait_tokens,
    )
}

#[derive(Default)]
struct FakeMemory {
    bytes: BTreeMap<u64, u8>,
}

impl FakeMemory {
    fn write_u8(&mut self, addr: u64, value: u8) {
        self.bytes.insert(addr, value);
    }

    fn write_u32(&mut self, addr: u64, value: u32) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn write_i32(&mut self, addr: u64, value: i32) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn write_u64(&mut self, addr: u64, value: u64) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn write_bytes(&mut self, addr: u64, bytes: &[u8]) {
        for (idx, byte) in bytes.iter().enumerate() {
            self.bytes.insert(addr + idx as u64, *byte);
        }
    }
}

impl LaneMemory for FakeMemory {
    fn read_u8(&self, addr: u64) -> Option<u8> {
        self.bytes.get(&addr).copied()
    }
}
