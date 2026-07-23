use std::time::Instant;

use crate::adapter::{
    build_lane_frame_from_adapter_input, AdapterBuildStats, AdapterInput, AdvisorRoot,
    CharacterPortraitInput, CharacterRoot, CountryLeaderRoleRoot, CountryPoliticsRoot,
    TraitDefinitionRoot, UnitLeaderRoot,
};
use crate::domain47_characters_leaders_cabinet_live as compact;
use crate::extractor::{
    encode_frame_payload, LaneFrame, LaneMemory, ResolvedTypeRoots, DOMAIN_ID, DOMAIN_NAME,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeRootCache {
    pub roots: Option<ResolvedTypeRoots>,
    pub roots_validated: bool,
    pub skipped_root_rows: u32,
    pub characters: Vec<CharacterRoot>,
    pub unit_leaders: Vec<UnitLeaderRoot>,
    pub unit_leader_trait_tokens: Vec<TraitDefinitionRoot>,
    pub country_leader_roles: Vec<CountryLeaderRoleRoot>,
    pub advisors: Vec<AdvisorRoot>,
    pub portraits: Vec<CharacterPortraitInput>,
}

impl NativeRootCache {
    pub fn stable_root_count(&self) -> usize {
        self.characters.len()
            + self.unit_leaders.len()
            + self.unit_leader_trait_tokens.len()
            + self.country_leader_roles.len()
            + self.advisors.len()
            + self.portraits.len()
    }

    pub fn has_stable_roots(&self) -> bool {
        self.roots_validated && self.roots.is_some() && self.stable_root_count() != 0
    }

    /// Roots still excluded from the shared collector. Advisor and
    /// unit-leader-trait roots were unblocked by the wave-3 870ebf
    /// revalidation (`WAVE3_CRACKS_870EBF_D04.md` §1); portrait rows remain
    /// excluded until a static portrait feed keyed to validated live
    /// characters exists.
    pub fn shared_collector_excluded_root_count(&self) -> usize {
        self.portraits.len()
    }

    pub fn shared_collector_excluded_roots_are_empty(&self) -> bool {
        self.shared_collector_excluded_root_count() == 0
    }

    pub fn debug_assert_shared_collector_excluded_roots_empty(&self) {
        debug_assert!(
            self.shared_collector_excluded_roots_are_empty(),
            "shared domain47 collector must not populate character_portrait roots until a static portrait feed keyed to validated live characters is revalidated"
        );
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeFrameInput {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub skipped_input_rows: u32,
    pub countries: Vec<CountryPoliticsRoot>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeScheduledRead {
    pub domain_id: u16,
    pub domain_name: &'static str,
    pub payload: Vec<u8>,
    pub read_time_us: u64,
    pub row_count: u32,
    pub data_units_total: u32,
    pub status_flags: u32,
    pub schema_flags: u32,
    pub adapter_stats: AdapterBuildStats,
    pub parse_errors: Vec<String>,
}

pub fn read_native_scheduled_frame<M: LaneMemory>(
    mem: &M,
    cache: &NativeRootCache,
    frame_input: NativeFrameInput,
) -> NativeScheduledRead {
    let started = Instant::now();
    let (frame, stats) = if cache.has_stable_roots() {
        let adapter_input = adapter_input_from_cache(cache, frame_input);
        let adapter_result = build_lane_frame_from_adapter_input(
            mem,
            &cache.roots.expect("checked by has_stable_roots"),
            &adapter_input,
        );
        if cache.skipped_root_rows != 0
            || adapter_result.stats.has_rejections()
            || adapter_input.skipped_input_rows != 0
        {
            let mut frame = LaneFrame::new(adapter_input.elapsed_wall_ms);
            frame.status_flags = adapter_input.status_flags | compact::STATUS_PARTIAL_DATA;
            (frame, adapter_result.stats)
        } else {
            (adapter_result.frame, adapter_result.stats)
        }
    } else {
        let mut frame = LaneFrame::new(frame_input.elapsed_wall_ms);
        frame.status_flags = frame_input.status_flags
            | compact::STATUS_SOURCE_UNAVAILABLE
            | compact::STATUS_PARTIAL_DATA;
        (frame, AdapterBuildStats::default())
    };

    let payload = encode_frame_payload(&frame);
    let gate = compact::decode_payload(0, &payload);
    let read_time_us = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
    NativeScheduledRead {
        domain_id: DOMAIN_ID,
        domain_name: DOMAIN_NAME,
        payload,
        read_time_us,
        row_count: gate.row_count_total.min(u32::MAX as u64) as u32,
        data_units_total: gate
            .data_units_total
            .values()
            .copied()
            .sum::<u64>()
            .min(u32::MAX as u64) as u32,
        status_flags: gate.status_flags_or | frame.status_flags,
        schema_flags: gate.schema_flags_or,
        adapter_stats: stats,
        parse_errors: gate.parse_errors,
    }
}

pub fn empty_native_scheduled_read(elapsed_wall_ms: u64, status_flags: u32) -> NativeScheduledRead {
    let mut frame = LaneFrame::new(elapsed_wall_ms);
    frame.status_flags = status_flags;
    let payload = encode_frame_payload(&frame);
    let gate = compact::decode_payload(0, &payload);
    NativeScheduledRead {
        domain_id: DOMAIN_ID,
        domain_name: DOMAIN_NAME,
        payload,
        read_time_us: 0,
        row_count: 0,
        data_units_total: 0,
        status_flags: gate.status_flags_or | frame.status_flags,
        schema_flags: gate.schema_flags_or,
        adapter_stats: AdapterBuildStats::default(),
        parse_errors: gate.parse_errors,
    }
}

pub fn adapter_input_from_cache(
    cache: &NativeRootCache,
    frame_input: NativeFrameInput,
) -> AdapterInput {
    AdapterInput {
        elapsed_wall_ms: frame_input.elapsed_wall_ms,
        status_flags: frame_input.status_flags,
        skipped_input_rows: frame_input.skipped_input_rows,
        characters: cache.characters.clone(),
        unit_leaders: cache.unit_leaders.clone(),
        unit_leader_trait_tokens: cache.unit_leader_trait_tokens.clone(),
        country_leader_roles: cache.country_leader_roles.clone(),
        countries: frame_input.countries,
        advisors: cache.advisors.clone(),
        portraits: cache.portraits.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::adapter::{
        CARMY_LEADER_ATTACK_OFF, CARMY_LEADER_DEFENSE_OFF, CARMY_LEADER_LOGISTICS_OFF,
        CARMY_LEADER_PLANNING_OFF, CUNIT_LEADER_EXPERIENCE_RAW_OFF,
    };
    use crate::extractor::{
        CADVISOR_CHARACTER_PTR_OFF, CCHARACTER_COUNTRY_TAG_DB_ID_OFF, CCHARACTER_ID_OFF,
        CCHARACTER_TYPE_ID, CCHARACTER_TYPE_OFF, CCHARACTER_UNIT_LEADER_PTR_OFF,
        CCOUNTRY_LEADER_CHARACTER_PTR_OFF, CCOUNTRY_POLITICAL_STATUS_OFF,
        CPOLITICAL_PARTY_LEADER_VEC_CAP_OFF, CPOLITICAL_PARTY_LEADER_VEC_OFF,
        CPOLITICAL_PARTY_LEADER_VEC_SIZE_OFF, CPOLITICAL_STATUS_RULING_PARTY_OFF,
        CUNIT_LEADER_CHARACTER_ID_COPY_OFF, CUNIT_LEADER_ID_OFF, CUNIT_LEADER_SKILL_SOURCE_PTR_OFF,
        CUNIT_LEADER_SKILL_VALUE_OFF, CUNIT_LEADER_TRAIT_CAPACITY_OFF, CUNIT_LEADER_TRAIT_DATA_OFF,
        CUNIT_LEADER_TRAIT_SIZE_OFF, CUNIT_LEADER_TYPE_ID, CUNIT_LEADER_TYPE_OFF,
    };

    const CHAR_VT: u64 = 0x1000_1000;
    const ARMY_VT: u64 = 0x1000_2000;
    const NAVY_VT: u64 = 0x1000_3000;
    const COUNTRY_LEADER_VT: u64 = 0x1000_4000;
    const ADVISOR_VT: u64 = 0x1000_5000;
    const TRAIT_VT: u64 = 0x1000_6000;

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

    #[test]
    fn scheduled_frame_uses_cached_roots_and_frame_politics() {
        let mut mem = FakeMemory::default();
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

        write_character(&mut mem, character, 1458, 2);
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
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_SIZE_OFF, 1);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_CAP_OFF, 1);
        mem.write_u64(leader_vec, country_leader);

        mem.write_u64(advisor, ADVISOR_VT);
        mem.write_u64(advisor + CADVISOR_CHARACTER_PTR_OFF, character);

        let cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            skipped_root_rows: 0,
            characters: vec![CharacterRoot {
                character_addr: character,
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
                country_tag_db_id: 2,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
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
        };
        let read = read_native_scheduled_frame(
            &mem,
            &cache,
            NativeFrameInput {
                elapsed_wall_ms: 88,
                countries: vec![CountryPoliticsRoot {
                    country_addr: country,
                    country_tag_db_id: 2,
                    ruling_party: Some("democratic".to_string()),
                    ruler_name: Some("Maxwell Aitken".to_string()),
                    ruler_token: Some("ENG_maxwell_aitken".to_string()),
                    active_idea_tokens: vec!["ENG_maxwell_aitken".to_string()],
                }],
                ..NativeFrameInput::default()
            },
        );
        assert_eq!(read.domain_id, 47);
        assert_eq!(read.status_flags, 0);
        assert!(read.parse_errors.is_empty(), "{:?}", read.parse_errors);
        assert_eq!(read.row_count, 8);
        assert_eq!(read.adapter_stats.appointed_advisor_rows, 1);
        assert_eq!(read.adapter_stats.portrait_rows, 1);
        let decoded = compact::decode_payload_rows(0, &read.payload).unwrap();
        assert!(decoded.rows.iter().any(|row| matches!(
            row,
            compact::DecodedDomain47Row::AppointedAdvisor {
                country_tag_db_id: 2,
                char_id: 1458,
                idea_token,
                ..
            } if idea_token == "ENG_maxwell_aitken"
        )));
    }

    #[test]
    fn skipped_root_rows_emit_rowless_partial_read() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 1458, 2);

        let cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            skipped_root_rows: 1,
            characters: vec![CharacterRoot {
                character_addr: character,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            ..NativeRootCache::default()
        };

        let read = read_native_scheduled_frame(&mem, &cache, NativeFrameInput::default());
        assert_ne!(read.status_flags & compact::STATUS_PARTIAL_DATA, 0);
        assert_eq!(read.row_count, 0);
        assert_eq!(read.data_units_total, 0);
        let decoded = compact::decode_payload_rows(0, &read.payload).unwrap();
        assert_ne!(decoded.status_flags & compact::STATUS_PARTIAL_DATA, 0);
        assert!(decoded.rows.is_empty());
    }

    #[test]
    fn shared_collector_excluded_root_guard_flags_blocked_roots() {
        let mut cache = NativeRootCache::default();
        assert!(cache.shared_collector_excluded_roots_are_empty());
        // Advisor + trait roots are unblocked (wave-3 870ebf revalidation)
        // and must not trip the excluded-root guard.
        cache.unit_leader_trait_tokens.push(TraitDefinitionRoot {
            trait_addr: 0x2300,
            token: "old_guard".to_string(),
        });
        cache.advisors.push(AdvisorRoot {
            advisor_addr: 0x5000,
            idea_token: "ENG_maxwell_aitken".to_string(),
            slot: Some("political_advisor".to_string()),
            ledger: None,
        });
        assert_eq!(cache.shared_collector_excluded_root_count(), 0);
        assert!(cache.shared_collector_excluded_roots_are_empty());
        // Portrait roots remain excluded until their static feed is validated.
        cache.portraits.push(CharacterPortraitInput {
            country_tag_db_id: 2,
            char_id: 1458,
            role: "civilian".to_string(),
            size: "large".to_string(),
            gfx: "GFX_portrait_ENG_maxwell_aitken".to_string(),
        });
        assert_eq!(cache.shared_collector_excluded_root_count(), 1);
        assert!(!cache.shared_collector_excluded_roots_are_empty());
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
}
