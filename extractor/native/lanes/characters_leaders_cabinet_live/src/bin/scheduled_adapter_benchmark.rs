use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use characters_leaders_cabinet_live_lane::adapter::{
    AdvisorRoot, CharacterPortraitInput, CharacterRoot, CountryLeaderRoleRoot, CountryPoliticsRoot,
    TraitDefinitionRoot, UnitLeaderRoot, CARMY_LEADER_ATTACK_OFF, CARMY_LEADER_DEFENSE_OFF,
    CARMY_LEADER_LOGISTICS_OFF, CARMY_LEADER_PLANNING_OFF, CNAVY_LEADER_ATTACK_OFF,
    CNAVY_LEADER_COORDINATION_OFF, CNAVY_LEADER_DEFENSE_OFF, CNAVY_LEADER_MANEUVERING_OFF,
    CUNIT_LEADER_EXPERIENCE_RAW_OFF,
};
use characters_leaders_cabinet_live_lane::domain47_characters_leaders_cabinet_live as compact;
use characters_leaders_cabinet_live_lane::extractor::{
    LaneMemory, ResolvedTypeRoots, CADVISOR_CHARACTER_PTR_OFF, CCHARACTER_COUNTRY_TAG_DB_ID_OFF,
    CCHARACTER_ID_OFF, CCHARACTER_TYPE_ID, CCHARACTER_TYPE_OFF, CCHARACTER_UNIT_LEADER_PTR_OFF,
    CCOUNTRY_LEADER_CHARACTER_PTR_OFF, CCOUNTRY_POLITICAL_STATUS_OFF,
    CPOLITICAL_PARTY_LEADER_VEC_OFF, CPOLITICAL_STATUS_RULING_PARTY_OFF,
    CUNIT_LEADER_CHARACTER_ID_COPY_OFF, CUNIT_LEADER_ID_OFF, CUNIT_LEADER_SKILL_SOURCE_PTR_OFF,
    CUNIT_LEADER_SKILL_VALUE_OFF, CUNIT_LEADER_TRAIT_CAPACITY_OFF, CUNIT_LEADER_TRAIT_DATA_OFF,
    CUNIT_LEADER_TRAIT_SIZE_OFF, CUNIT_LEADER_TYPE_ID, CUNIT_LEADER_TYPE_OFF,
};
use characters_leaders_cabinet_live_lane::scheduled::{
    read_native_scheduled_frame, NativeFrameInput, NativeRootCache,
};

const CHAR_VT: u64 = 0x1000_1000;
const ARMY_VT: u64 = 0x1000_2000;
const NAVY_VT: u64 = 0x1000_3000;
const COUNTRY_LEADER_VT: u64 = 0x1000_4000;
const ADVISOR_VT: u64 = 0x1000_5000;
const TRAIT_VT: u64 = 0x1000_6000;

const DEFAULT_FRAMES: u32 = 24;

#[derive(Clone, Copy)]
struct Scale {
    characters: u32,
    unit_leaders: u32,
    unit_leader_traits: u32,
    trait_defs: u32,
    country_leader_roles: u32,
    countries: u32,
    advisors: u32,
    appointed: u32,
    portraits: u32,
}

impl Default for Scale {
    fn default() -> Self {
        Self {
            characters: 13_290,
            unit_leaders: 1_288,
            unit_leader_traits: 3_164,
            trait_defs: 194,
            country_leader_roles: 1_941,
            countries: 55,
            advisors: 8_378,
            appointed: 3,
            portraits: 128,
        }
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut frames = DEFAULT_FRAMES;
    let mut out_dir = PathBuf::from(
        "extractor/native/runs/characters_leaders_cabinet_live_20260701_scheduled_adapter_benchmark_v1",
    );
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--frames" => {
                idx += 1;
                frames = args
                    .get(idx)
                    .ok_or("--frames requires a value")?
                    .parse::<u32>()
                    .map_err(|err| format!("invalid --frames: {err}"))?;
            }
            "--out-dir" => {
                idx += 1;
                out_dir = PathBuf::from(args.get(idx).ok_or("--out-dir requires a value")?);
            }
            value => return Err(format!("unknown argument: {value}")),
        }
        idx += 1;
    }
    if frames == 0 {
        return Err("--frames must be nonzero".to_string());
    }
    fs::create_dir_all(&out_dir).map_err(|err| format!("create out dir failed: {err}"))?;

    let scale = Scale::default();
    let (mut mem, cache, frame_input) = build_live_scale_fixture(scale)?;
    let mut total_us = 0u128;
    let mut max_us = 0u64;
    let mut last_payload = Vec::new();
    let mut last_row_count = 0u32;
    let mut last_status_flags = 0u32;
    let mut last_schema_flags = 0u32;
    let mut last_parse_errors = Vec::<String>::new();
    let mut appointed_rows = 0u32;
    let mut portrait_rows = 0u32;
    for frame_idx in 0..frames {
        mutate_one_skill(&mut mem, frame_idx);
        let mut input = frame_input.clone();
        input.elapsed_wall_ms = u64::from(frame_idx) * 50;
        let read = read_native_scheduled_frame(&mem, &cache, input);
        total_us = total_us.saturating_add(u128::from(read.read_time_us));
        max_us = max_us.max(read.read_time_us);
        last_row_count = read.row_count;
        last_status_flags = read.status_flags;
        last_schema_flags = read.schema_flags;
        appointed_rows = read.adapter_stats.appointed_advisor_rows;
        portrait_rows = read.adapter_stats.portrait_rows;
        last_parse_errors = read.parse_errors.clone();
        last_payload = read.payload;
        if last_status_flags != 0 || !last_parse_errors.is_empty() {
            break;
        }
    }
    fs::write(out_dir.join("domain47_payload_last.bin"), &last_payload)
        .map_err(|err| format!("write payload failed: {err}"))?;
    let last_record = compact::encode_compact_data_record(1, 1946051909, &last_payload);
    fs::write(out_dir.join("domain47_record_last.bin"), &last_record)
        .map_err(|err| format!("write record failed: {err}"))?;
    let avg_us = (total_us / u128::from(frames)).min(u128::from(u64::MAX)) as u64;
    let expected_rows = scale.characters
        + scale.unit_leaders
        + scale.unit_leader_traits
        + scale.country_leader_roles
        + scale.countries
        + scale.advisors
        + scale.appointed
        + scale.portraits;
    let result = if last_status_flags == 0
        && last_parse_errors.is_empty()
        && last_row_count == expected_rows
    {
        "validated"
    } else {
        "failed"
    };
    let report = format!(
        concat!(
            "{{\n",
            "  \"result\": \"{}\",\n",
            "  \"domain\": \"{}\",\n",
            "  \"domain_id\": {},\n",
            "  \"frames\": {},\n",
            "  \"native_hot_loop_only\": true,\n",
            "  \"proof_scope\": \"Rust lane-local scheduled adapter benchmark using cached roots; not shared DLL capture\",\n",
            "  \"characters\": {},\n",
            "  \"unit_leaders\": {},\n",
            "  \"unit_leader_traits\": {},\n",
            "  \"country_leader_roles\": {},\n",
            "  \"countries\": {},\n",
            "  \"advisors\": {},\n",
            "  \"appointed_rows\": {},\n",
            "  \"portrait_rows\": {},\n",
            "  \"expected_rows\": {},\n",
            "  \"last_row_count\": {},\n",
            "  \"payload_last_bytes\": {},\n",
            "  \"record_last_bytes\": {},\n",
            "  \"avg_read_time_us\": {},\n",
            "  \"max_read_time_us\": {},\n",
            "  \"status_flags_or\": {},\n",
            "  \"schema_flags_or\": {},\n",
            "  \"parse_errors\": {}\n",
            "}}\n"
        ),
        result,
        compact::DOMAIN_NAME,
        compact::DOMAIN_ID,
        frames,
        scale.characters,
        scale.unit_leaders,
        scale.unit_leader_traits,
        scale.country_leader_roles,
        scale.countries,
        scale.advisors,
        appointed_rows,
        portrait_rows,
        expected_rows,
        last_row_count,
        last_payload.len(),
        last_record.len(),
        avg_us,
        max_us,
        last_status_flags,
        last_schema_flags,
        json_string_array(&last_parse_errors),
    );
    fs::write(
        out_dir.join("scheduled_adapter_benchmark.json"),
        report.as_bytes(),
    )
    .map_err(|err| format!("write report failed: {err}"))?;
    print!("{report}");
    if result == "validated" {
        Ok(())
    } else {
        Err("scheduled adapter benchmark failed".to_string())
    }
}

fn build_live_scale_fixture(
    scale: Scale,
) -> Result<(FakeMemory, NativeRootCache, NativeFrameInput), String> {
    let mut mem = FakeMemory::default();
    let mut characters = Vec::new();
    let mut unit_leaders = Vec::new();
    let mut trait_defs = Vec::new();
    let mut country_leader_roles = Vec::new();
    let mut advisors = Vec::new();
    let mut portraits = Vec::new();
    let mut countries = Vec::new();

    let character_base = 0x0100_0000u64;
    let leader_base = 0x0200_0000u64;
    let country_leader_base = 0x0300_0000u64;
    let advisor_base = 0x0400_0000u64;
    let trait_base = 0x0500_0000u64;
    let country_base = 0x0600_0000u64;
    let skill_base = 0x0700_0000u64;
    let trait_vec_base = 0x0800_0000u64;
    let status_base = 0x0900_0000u64;
    let party_base = 0x0A00_0000u64;
    let leader_vec_base = 0x0B00_0000u64;

    for idx in 0..scale.characters {
        let addr = character_base + u64::from(idx) * 0x200;
        let tag_db_id = idx % 365 + 1;
        write_character(&mut mem, addr, idx + 1, tag_db_id);
        characters.push(CharacterRoot {
            character_addr: addr,
            // The identity `write_character` just wrote at `addr`; a root
            // that disagreed would be refused as recycled and the benchmark
            // would time the refusal path.
            char_id: idx + 1,
            name: None,
            token: None,
        });
    }

    for idx in 0..scale.trait_defs {
        let addr = trait_base + u64::from(idx) * 0x80;
        mem.write_u64(addr, TRAIT_VT);
        trait_defs.push(TraitDefinitionRoot {
            trait_addr: addr,
            token: format!("trait_{idx}"),
        });
    }

    let base_traits = scale.unit_leader_traits / scale.unit_leaders.max(1);
    let extra_traits = scale.unit_leader_traits % scale.unit_leaders.max(1);
    for idx in 0..scale.unit_leaders {
        let addr = leader_base + u64::from(idx) * 0x1200;
        let skill_addr = skill_base + u64::from(idx) * 0x100;
        let trait_vec_addr = trait_vec_base + u64::from(idx) * 0x100;
        let character_idx = idx % scale.characters;
        let tag_db_id = character_idx % 365 + 1;
        let leader_kind = if idx % 5 == 0 {
            compact::UNIT_LEADER_NAVY
        } else {
            compact::UNIT_LEADER_ARMY
        };
        let vtable = if leader_kind == compact::UNIT_LEADER_NAVY {
            NAVY_VT
        } else {
            ARMY_VT
        };
        mem.write_u64(addr, vtable);
        mem.write_u32(addr + CUNIT_LEADER_TYPE_OFF, CUNIT_LEADER_TYPE_ID);
        mem.write_u32(addr + CUNIT_LEADER_ID_OFF, 10_000 + idx);
        mem.write_u32(addr + CUNIT_LEADER_CHARACTER_ID_COPY_OFF, character_idx + 1);
        mem.write_i32(addr + CUNIT_LEADER_EXPERIENCE_RAW_OFF, 40_000 + idx as i32);
        mem.write_u64(addr + CUNIT_LEADER_SKILL_SOURCE_PTR_OFF, skill_addr);
        mem.write_u32(skill_addr + CUNIT_LEADER_SKILL_VALUE_OFF, idx % 9 + 1);
        write_leader_stats(&mut mem, addr, leader_kind, idx);
        let trait_count = base_traits + u32::from(idx < extra_traits);
        mem.write_u64(addr + CUNIT_LEADER_TRAIT_DATA_OFF, trait_vec_addr);
        mem.write_u32(addr + CUNIT_LEADER_TRAIT_CAPACITY_OFF, trait_count);
        mem.write_u32(addr + CUNIT_LEADER_TRAIT_SIZE_OFF, trait_count);
        for trait_idx in 0..trait_count {
            let def_idx = (idx + trait_idx) % scale.trait_defs.max(1);
            let def_addr = trait_base + u64::from(def_idx) * 0x80;
            mem.write_u64(trait_vec_addr + u64::from(trait_idx) * 8, def_addr);
        }
        unit_leaders.push(UnitLeaderRoot {
            leader_addr: addr,
            country_tag_db_id: tag_db_id,
            leader_kind,
            name: None,
            token: None,
        });
    }

    for idx in 0..scale.country_leader_roles {
        let addr = country_leader_base + u64::from(idx) * 0x100;
        let character_idx = idx % scale.characters;
        let character_addr = character_base + u64::from(character_idx) * 0x200;
        mem.write_u64(addr, COUNTRY_LEADER_VT);
        mem.write_u64(addr + CCOUNTRY_LEADER_CHARACTER_PTR_OFF, character_addr);
        country_leader_roles.push(CountryLeaderRoleRoot {
            country_leader_addr: addr,
            char_id: character_idx + 1,
            name: None,
            token: None,
        });
    }

    for idx in 0..scale.countries {
        let country_addr = country_base + u64::from(idx) * 0x200;
        let status_addr = status_base + u64::from(idx) * 0x100;
        let party_addr = party_base + u64::from(idx) * 0x100;
        let leader_vec_addr = leader_vec_base + u64::from(idx) * 0x20;
        let country_leader_addr = country_leader_base + u64::from(idx) * 0x100;
        mem.write_u64(country_addr + CCOUNTRY_POLITICAL_STATUS_OFF, status_addr);
        mem.write_u64(status_addr + CPOLITICAL_STATUS_RULING_PARTY_OFF, party_addr);
        mem.write_u64(
            party_addr + CPOLITICAL_PARTY_LEADER_VEC_OFF,
            leader_vec_addr,
        );
        mem.write_u64(
            party_addr + CPOLITICAL_PARTY_LEADER_VEC_OFF + 8,
            leader_vec_addr + 8,
        );
        mem.write_u64(leader_vec_addr, country_leader_addr);
        countries.push(CountryPoliticsRoot {
            country_addr,
            country_tag_db_id: idx + 1,
            ruling_party: None,
            ruler_name: None,
            ruler_token: None,
            active_idea_tokens: active_tokens_for_country(idx + 1, scale.appointed),
        });
    }

    for idx in 0..scale.advisors {
        let addr = advisor_base + u64::from(idx) * 0x100;
        let character_idx = idx % scale.characters;
        let character_addr = character_base + u64::from(character_idx) * 0x200;
        mem.write_u64(addr, ADVISOR_VT);
        mem.write_u64(addr + CADVISOR_CHARACTER_PTR_OFF, character_addr);
        advisors.push(AdvisorRoot {
            advisor_addr: addr,
            idea_token: format!("advisor_token_{idx}"),
            slot: Some(advisor_slot(idx).to_string()),
            ledger: None,
        });
    }

    for idx in 0..scale.portraits {
        portraits.push(CharacterPortraitInput {
            country_tag_db_id: idx % 365 + 1,
            char_id: idx + 1,
            role: if idx % 5 == 0 { "army" } else { "civilian" }.to_string(),
            size: "large".to_string(),
            gfx: format!("GFX_portrait_scheduled_{idx}"),
        });
    }

    let cache = NativeRootCache {
        roots: Some(roots()),
        roots_validated: true,
        skipped_root_rows: 0,
        characters,
        unit_leaders,
        unit_leader_trait_tokens: trait_defs,
        country_leader_roles,
        advisors,
        portraits,
    };
    let frame_input = NativeFrameInput {
        countries,
        ..NativeFrameInput::default()
    };
    Ok((mem, cache, frame_input))
}

fn active_tokens_for_country(tag_db_id: u32, appointed: u32) -> Vec<String> {
    if tag_db_id <= appointed {
        vec![format!("advisor_token_{}", tag_db_id - 1)]
    } else {
        Vec::new()
    }
}

fn advisor_slot(idx: u32) -> &'static str {
    match idx % 6 {
        0 => "political_advisor",
        1 => "army_chief",
        2 => "navy_chief",
        3 => "air_chief",
        4 => "high_command",
        _ => "theorist",
    }
}

fn write_leader_stats(mem: &mut FakeMemory, addr: u64, leader_kind: u8, idx: u32) {
    if leader_kind == compact::UNIT_LEADER_NAVY {
        mem.write_u8(addr + CNAVY_LEADER_ATTACK_OFF, (idx % 9 + 1) as u8);
        mem.write_u8(addr + CNAVY_LEADER_DEFENSE_OFF, (idx % 8 + 1) as u8);
        mem.write_u8(addr + CNAVY_LEADER_MANEUVERING_OFF, (idx % 7 + 1) as u8);
        mem.write_u8(addr + CNAVY_LEADER_COORDINATION_OFF, (idx % 6 + 1) as u8);
    } else {
        mem.write_u8(addr + CARMY_LEADER_ATTACK_OFF, (idx % 9 + 1) as u8);
        mem.write_u8(addr + CARMY_LEADER_DEFENSE_OFF, (idx % 8 + 1) as u8);
        mem.write_u8(addr + CARMY_LEADER_PLANNING_OFF, (idx % 7 + 1) as u8);
        mem.write_u8(addr + CARMY_LEADER_LOGISTICS_OFF, (idx % 6 + 1) as u8);
    }
}

fn mutate_one_skill(mem: &mut FakeMemory, frame_idx: u32) {
    let skill_addr = 0x0700_0000u64 + CUNIT_LEADER_SKILL_VALUE_OFF;
    mem.write_u32(skill_addr, frame_idx % 9 + 1);
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
    mem.write_u64(addr + CCHARACTER_UNIT_LEADER_PTR_OFF, 0);
}

fn json_string_array(values: &[String]) -> String {
    let body = values
        .iter()
        .map(|value| format!("\"{}\"", json_escape(value)))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{body}]")
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[derive(Default)]
struct FakeMemory {
    bytes: HashMap<u64, u8>,
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
