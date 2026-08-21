use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapter::{
    build_lane_frame_from_adapter_input_ref, build_lane_frame_reusing_walk, AdapterBuildStats,
    AdapterInput, AdapterInputRef, AdvisorRoot, CachedPortraitIndex, CachedWalk,
    CharacterPortraitInput, CharacterRoot, CountryLeaderRoleRoot, CountryPoliticsRoot,
    TraitDefinitionRoot, UnitLeaderRoot, CACHEABLE_WALK_FAMILIES,
};
use crate::domain47_characters_leaders_cabinet_live as compact;
use crate::extractor::{
    encode_frame_payload, encode_frame_payload_over, LaneFrame, LaneMemory, ResolvedTypeRoots,
    DOMAIN_ID, DOMAIN_NAME,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeRootCache {
    pub roots: Option<ResolvedTypeRoots>,
    pub roots_validated: bool,
    pub skipped_root_rows: u32,
    pub known_families: u32,
    pub characters: Vec<CharacterRoot>,
    pub unit_leaders: Vec<UnitLeaderRoot>,
    pub unit_leader_trait_tokens: Vec<TraitDefinitionRoot>,
    pub country_leader_roles: Vec<CountryLeaderRoleRoot>,
    pub advisors: Vec<AdvisorRoot>,
    pub portraits: Vec<CharacterPortraitInput>,
}

impl Default for NativeRootCache {
    fn default() -> Self {
        Self {
            roots: None,
            roots_validated: false,
            skipped_root_rows: 0,
            known_families: compact::FAMILY_ALL,
            characters: Vec::new(),
            unit_leaders: Vec::new(),
            unit_leader_trait_tokens: Vec::new(),
            country_leader_roles: Vec::new(),
            advisors: Vec::new(),
            portraits: Vec::new(),
        }
    }
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
}

fn rowless_outcome_is_already_decided(
    cache: &NativeRootCache,
    frame_input: &NativeFrameInput,
) -> bool {
    cache.skipped_root_rows != 0 || frame_input.skipped_input_rows != 0
}

fn rowless_partial_frame(elapsed_wall_ms: u64, status_flags: u32) -> LaneFrame {
    let mut frame = LaneFrame::new(elapsed_wall_ms);
    frame.status_flags =
        status_flags | compact::STATUS_SOURCE_UNAVAILABLE | compact::STATUS_PARTIAL_DATA;
    frame
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeScheduledObservation {
    frame: LaneFrame,
    stats: AdapterBuildStats,
    observe: Duration,
    cached: Option<Arc<CachedWalk>>,
}

impl NativeScheduledObservation {
    pub fn adopt_cached_walk(&mut self, families: u32) -> Option<Arc<CachedWalk>> {
        let families = families & CACHEABLE_WALK_FAMILIES;
        let leader_pair = compact::FAMILY_UNIT_LEADER | compact::FAMILY_UNIT_LEADER_TRAIT;
        let families = if families & leader_pair == leader_pair {
            families
        } else {
            families & !leader_pair
        };
        if families == 0
            || self.cached.is_some()
            || self.stats.has_rejections()
            || self.stats.identity_degraded_rows != 0
            || (self.frame.characters.is_empty()
                && self.frame.advisor_roster.is_empty()
                && self.frame.country_leader_roles.is_empty()
                && self.frame.unit_leaders.is_empty())
        {
            return None;
        }
        let mut cached = CachedWalk {
            families,
            stats: self.stats.clone(),
            ..CachedWalk::default()
        };
        let mut known_keys: Vec<(u32, u32)> = Vec::new();
        if families & compact::FAMILY_CHARACTER != 0 {
            cached.characters = std::mem::take(&mut self.frame.characters);
            cached.character_identity = cached
                .characters
                .iter()
                .enumerate()
                .map(|(index, row)| ((row.country_tag_db_id, row.char_id), index as u32))
                .collect();
            cached
                .character_identity
                .sort_unstable_by_key(|(key, _)| *key);
            known_keys.extend(cached.character_identity.iter().map(|(key, _)| *key));
        }
        if families & compact::FAMILY_UNIT_LEADER != 0 {
            cached.unit_leaders = std::mem::take(&mut self.frame.unit_leaders);
            cached.unit_leader_traits = std::mem::take(&mut self.frame.unit_leader_traits);
            known_keys.extend(
                cached
                    .unit_leaders
                    .iter()
                    .map(|r| (r.country_tag_db_id, r.char_id)),
            );
        }
        if families & compact::FAMILY_COUNTRY_LEADER_ROLE != 0 {
            cached.country_leader_roles = std::mem::take(&mut self.frame.country_leader_roles);
            known_keys.extend(
                cached
                    .country_leader_roles
                    .iter()
                    .map(|r| (r.country_tag_db_id, r.char_id)),
            );
        }
        if families & compact::FAMILY_ADVISOR_ROSTER != 0 {
            cached.advisor_roster = std::mem::take(&mut self.frame.advisor_roster);
            known_keys.extend(
                cached
                    .advisor_roster
                    .iter()
                    .map(|r| (r.country_tag_db_id, r.char_id)),
            );
        }
        known_keys.sort_unstable();
        known_keys.dedup();
        cached.known_keys = known_keys;
        let cached_key = |p: &crate::extractor::CharacterPortraitRecord| {
            cached.knows(&(p.country_tag_db_id, p.char_id))
        };
        // Lane PH: the portrait phase's precomputed answer (see
        // `CachedWalk::portrait_index`). It is taken HERE, before the rows
        // move, and it is taken over `self.frame.character_portraits` because
        // on an adopting frame that list IS the input list: this method
        // refuses a walk with `has_rejections()`, an input portrait that was
        // not admitted increments `rejected_roots`, and an adopting frame is
        // always a FULL frame (no cache to skip a row), so every input row was
        // pushed. `stats.portrait_inputs_seen` is that input length.
        let mut portrait_unknown_keys: Vec<(u32, u32)> = self
            .frame
            .character_portraits
            .iter()
            .filter(|p| !cached_key(p))
            .map(|p| (p.country_tag_db_id, p.char_id))
            .collect();
        let portrait_unknown_rows = portrait_unknown_keys.len().min(u32::MAX as usize) as u32;
        portrait_unknown_keys.sort_unstable();
        portrait_unknown_keys.dedup();
        if self.frame.character_portraits.iter().all(cached_key) {
            cached.character_portraits = std::mem::take(&mut self.frame.character_portraits);
        } else {
            cached.character_portraits = self
                .frame
                .character_portraits
                .iter()
                .filter(|p| cached_key(p))
                .cloned()
                .collect();
        }
        cached.portrait_index = Some(CachedPortraitIndex {
            portrait_input_rows: self.stats.portrait_inputs_seen,
            portrait_unknown_keys,
            portrait_unknown_rows,
        });
        let cached = Arc::new(cached);
        self.cached = Some(cached.clone());
        Some(cached)
    }

    pub fn reused_walk_families(&self) -> u32 {
        self.cached.as_ref().map_or(0, |c| c.families)
    }

    /// Lane MX: unit-leader trait pointers this walk could not resolve to a
    /// definition token (`AdapterBuildStats::missing_trait_tokens`).
    pub fn missing_trait_tokens(&self) -> u32 {
        self.stats.missing_trait_tokens
    }
}

pub fn observe_native_scheduled_frame<M: LaneMemory>(
    mem: &M,
    cache: &NativeRootCache,
    frame_input: NativeFrameInput,
) -> NativeScheduledObservation {
    observe_native_scheduled_frame_reusing(mem, cache, frame_input, None)
}

pub fn observe_native_scheduled_frame_reusing<M: LaneMemory>(
    mem: &M,
    cache: &NativeRootCache,
    frame_input: NativeFrameInput,
    cached: Option<Arc<CachedWalk>>,
) -> NativeScheduledObservation {
    let started = Instant::now();
    let (frame, stats, cached) = observe_native_scheduled_parts(mem, cache, frame_input, cached);
    NativeScheduledObservation {
        frame,
        stats,
        observe: started.elapsed(),
        cached,
    }
}

fn duration_us(span: Duration) -> u64 {
    span.as_micros().min(u64::MAX as u128) as u64
}

pub fn finish_native_scheduled_read(
    observation: NativeScheduledObservation,
) -> NativeScheduledRead {
    let NativeScheduledObservation {
        frame,
        stats,
        observe,
        cached,
    } = observation;
    let compute_started = Instant::now();
    let payload = encode_frame_payload_over(&frame, cached.as_deref());
    let gate = compact::decode_payload(0, &payload);
    let read_time_us = duration_us(observe.saturating_add(compute_started.elapsed()));
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

pub fn read_native_scheduled_frame<M: LaneMemory>(
    mem: &M,
    cache: &NativeRootCache,
    frame_input: NativeFrameInput,
) -> NativeScheduledRead {
    finish_native_scheduled_read(observe_native_scheduled_frame(mem, cache, frame_input))
}

fn observe_native_scheduled_parts<M: LaneMemory>(
    mem: &M,
    cache: &NativeRootCache,
    frame_input: NativeFrameInput,
    cached: Option<Arc<CachedWalk>>,
) -> (LaneFrame, AdapterBuildStats, Option<Arc<CachedWalk>>) {
    if cache.has_stable_roots() && rowless_outcome_is_already_decided(cache, &frame_input) {
        (
            rowless_partial_frame(frame_input.elapsed_wall_ms, frame_input.status_flags),
            AdapterBuildStats::default(),
            None,
        )
    } else if cache.has_stable_roots() {
        let adapter_input = adapter_input_ref_from_cache(cache, &frame_input);
        let roots = &cache.roots.expect("checked by has_stable_roots");
        let cached = cached.filter(|c| c.families != 0);
        let mut adapter_result = match cached.as_deref() {
            Some(cached_walk) => {
                build_lane_frame_reusing_walk(mem, roots, &adapter_input, cached_walk)
            }
            None => build_lane_frame_from_adapter_input_ref(mem, roots, &adapter_input),
        };
        adapter_result.frame.known_families =
            if adapter_result.stats.active_ruler_unresolved_roots == 0 {
                cache.known_families
            } else {
                adapter_result.frame.active_rulers.clear();
                cache.known_families & !compact::FAMILY_ACTIVE_RULER
            };
        if adapter_result.stats.has_rejections() {
            let mut frame = LaneFrame::new(adapter_input.elapsed_wall_ms);
            frame.status_flags = adapter_input.status_flags
                | compact::STATUS_SOURCE_UNAVAILABLE
                | compact::STATUS_PARTIAL_DATA;
            (frame, adapter_result.stats, None)
        } else {
            (adapter_result.frame, adapter_result.stats, cached)
        }
    } else {
        (
            rowless_partial_frame(frame_input.elapsed_wall_ms, frame_input.status_flags),
            AdapterBuildStats::default(),
            None,
        )
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

pub fn adapter_input_ref_from_cache<'a>(
    cache: &'a NativeRootCache,
    frame_input: &'a NativeFrameInput,
) -> AdapterInputRef<'a> {
    AdapterInputRef {
        elapsed_wall_ms: frame_input.elapsed_wall_ms,
        status_flags: frame_input.status_flags,
        skipped_input_rows: frame_input.skipped_input_rows,
        characters: &cache.characters,
        unit_leaders: &cache.unit_leaders,
        unit_leader_trait_tokens: &cache.unit_leader_trait_tokens,
        country_leader_roles: &cache.country_leader_roles,
        countries: &frame_input.countries,
        advisors: &cache.advisors,
        portraits: &cache.portraits,
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

    #[derive(Clone, Default)]
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
            known_families: compact::FAMILY_ALL,
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
                gfx_encoding: compact::PortraitIdentityEncoding::Utf8,
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
                char_id: 1458,
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
        assert_ne!(read.status_flags & compact::STATUS_SOURCE_UNAVAILABLE, 0);
        assert_eq!(decoded.family_presence_mask, compact::FAMILY_ALL);
        assert!(decoded.rows.is_empty());
    }

    struct CountingMemory {
        inner: FakeMemory,
        reads: std::cell::Cell<u64>,
    }

    impl CountingMemory {
        fn new(inner: FakeMemory) -> Self {
            Self {
                inner,
                reads: std::cell::Cell::new(0),
            }
        }
    }

    impl LaneMemory for CountingMemory {
        fn read_u8(&self, addr: u64) -> Option<u8> {
            self.reads.set(self.reads.get().saturating_add(1));
            self.inner.read_u8(addr)
        }
    }

    #[test]
    fn a_predecided_rowless_frame_reads_no_engine_memory() {
        let cache_with = |skipped_root_rows: u32, character: u64| NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            skipped_root_rows,
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 1458,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            ..NativeRootCache::default()
        };
        let character = 0x1000;
        let memory = || {
            let mut mem = FakeMemory::default();
            write_character(&mut mem, character, 1458, 2);
            CountingMemory::new(mem)
        };
        let expected = empty_native_scheduled_read(
            0,
            compact::STATUS_SOURCE_UNAVAILABLE | compact::STATUS_PARTIAL_DATA,
        );

        for (case, cache, frame_input) in [
            (
                "torn or clock-refused roots",
                cache_with(1, character),
                NativeFrameInput::default(),
            ),
            (
                "a skipped country row",
                cache_with(0, character),
                NativeFrameInput {
                    skipped_input_rows: 1,
                    ..NativeFrameInput::default()
                },
            ),
        ] {
            let mem = memory();
            let read = read_native_scheduled_frame(&mem, &cache, frame_input);
            assert_eq!(
                mem.reads.get(),
                0,
                "{case}: the walk ran for a frame whose rowless outcome was already decided"
            );
            assert_eq!(read.payload, expected.payload, "{case}");
            assert_eq!(read.status_flags, expected.status_flags, "{case}");
            assert_eq!(read.row_count, 0, "{case}");
            assert_eq!(read.data_units_total, 0, "{case}");
            assert_eq!(read.schema_flags, expected.schema_flags, "{case}");
            assert_ne!(
                read.status_flags & compact::STATUS_PARTIAL_DATA,
                0,
                "{case}"
            );
            assert_ne!(
                read.status_flags & compact::STATUS_SOURCE_UNAVAILABLE,
                0,
                "{case}"
            );
        }

        let mem = memory();
        let read = read_native_scheduled_frame(
            &mem,
            &cache_with(0, character),
            NativeFrameInput::default(),
        );
        assert_ne!(
            mem.reads.get(),
            0,
            "the ordinary frame must still read the engine"
        );
        assert_ne!(read.row_count, 0, "the ordinary frame must still publish");
    }

    #[test]
    fn a_recycled_character_costs_its_own_text_and_not_the_frame() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 9999, 2);

        let cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            skipped_root_rows: 0,
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 1458,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            ..NativeRootCache::default()
        };

        let read = read_native_scheduled_frame(&mem, &cache, NativeFrameInput::default());
        assert_ne!(read.row_count, 0, "the frame was discarded, not degraded");
        let decoded = compact::decode_payload_rows(0, &read.payload).unwrap();
        assert!(decoded.rows.iter().any(|row| matches!(
            row,
            compact::DecodedDomain47Row::Character {
                char_id: 9999,
                country_tag_db_id: 2,
                name: None,
                token: None,
            }
        )));
        assert!(!read.payload.windows(14).any(|w| w == b"Maxwell Aitken"));
        assert_ne!(read.status_flags & compact::STATUS_PARTIAL_DATA, 0);
    }

    #[test]
    fn a_census_only_cache_publishes_its_characters_and_declares_the_rest_unknown() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 1458, 2);

        let cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            skipped_root_rows: 0,
            known_families: compact::FAMILY_CENSUS_ONLY,
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 1458,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            ..NativeRootCache::default()
        };

        let country = 0x4000;
        let status = 0x4100;
        let party = 0x4200;
        let leader_vec = 0x4300;
        let country_leader = 0x4400;
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
        let read = read_native_scheduled_frame(
            &mem,
            &cache,
            NativeFrameInput {
                countries: vec![CountryPoliticsRoot {
                    country_addr: country,
                    country_tag_db_id: 2,
                    ruling_party: Some("democratic".to_string()),
                    ruler_name: Some("Maxwell Aitken".to_string()),
                    ruler_token: Some("ENG_maxwell_aitken".to_string()),
                    active_idea_tokens: vec![],
                }],
                ..NativeFrameInput::default()
            },
        );
        assert!(read.parse_errors.is_empty(), "{:?}", read.parse_errors);
        assert_ne!(read.row_count, 0, "the character census must be published");

        assert_eq!(
            read.status_flags & compact_contract::COMPACT_STATUS_FAILURE_MASK,
            compact::STATUS_PARTIAL_DATA
        );

        let decoded = compact::decode_payload_rows(0, &read.payload).unwrap();
        assert_eq!(decoded.family_presence_mask, compact::FAMILY_CENSUS_ONLY);
        assert_eq!(
            compact::absent_family_names(decoded.family_presence_mask),
            vec![
                "unit_leader_traits",
                "country_leader_roles",
                "advisor_roster",
                "appointed_advisors"
            ]
        );
        assert!(decoded.rows.iter().any(|row| matches!(
            row,
            compact::DecodedDomain47Row::Character { char_id: 1458, .. }
        )));
        assert!(
            !decoded.rows.iter().any(|row| matches!(
                row,
                compact::DecodedDomain47Row::AdvisorRoster { .. }
                    | compact::DecodedDomain47Row::CountryLeaderRole { .. }
                    | compact::DecodedDomain47Row::UnitLeaderTrait { .. }
            )),
            "a family declared unobserved must carry no rows"
        );
    }

    #[test]
    fn a_partly_unresolved_ruler_walk_drops_the_family_and_still_publishes() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 1458, 2);

        let good_country = 0x4000;
        let status = 0x4100;
        let party = 0x4200;
        let leader_vec = 0x4300;
        let country_leader = 0x4400;
        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(
            country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
            character,
        );
        mem.write_u64(good_country + CCOUNTRY_POLITICAL_STATUS_OFF, status);
        mem.write_u64(status + CPOLITICAL_STATUS_RULING_PARTY_OFF, party);
        mem.write_u64(party + CPOLITICAL_PARTY_LEADER_VEC_OFF, leader_vec);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_SIZE_OFF, 1);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_CAP_OFF, 1);
        mem.write_u64(leader_vec, country_leader);

        let unreadable_country = 0x5000;

        let cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            known_families: compact::FAMILY_ALL,
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 1458,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            ..NativeRootCache::default()
        };
        let politics = |addr: u64, tag: u32| CountryPoliticsRoot {
            country_addr: addr,
            country_tag_db_id: tag,
            ruling_party: Some("democratic".to_string()),
            ruler_name: Some("Maxwell Aitken".to_string()),
            ruler_token: Some("ENG_maxwell_aitken".to_string()),
            active_idea_tokens: vec![],
        };
        let read = read_native_scheduled_frame(
            &mem,
            &cache,
            NativeFrameInput {
                countries: vec![politics(good_country, 2), politics(unreadable_country, 3)],
                ..NativeFrameInput::default()
            },
        );

        assert!(read.parse_errors.is_empty(), "{:?}", read.parse_errors);
        assert_eq!(read.adapter_stats.active_ruler_unresolved_roots, 1);

        let decoded = compact::decode_payload_rows(0, &read.payload).unwrap();
        assert_eq!(
            decoded.family_presence_mask & compact::FAMILY_ACTIVE_RULER,
            0,
            "a family with an unreadable member is not observed"
        );
        assert!(
            !decoded
                .rows
                .iter()
                .any(|row| matches!(row, compact::DecodedDomain47Row::ActiveRuler { .. })),
            "the rows must go with the declaration"
        );
        assert!(decoded.rows.iter().any(|row| matches!(
            row,
            compact::DecodedDomain47Row::Character { char_id: 1458, .. }
        )));
        assert_eq!(
            read.status_flags & compact_contract::COMPACT_STATUS_FAILURE_MASK,
            compact::STATUS_PARTIAL_DATA
        );
    }

    fn borrowed_input_fixture() -> (FakeMemory, NativeRootCache, NativeFrameInput) {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        let leader = 0x2000;
        let skill_source = 0x2100;
        let trait_data = 0x2200;
        let trait_def = 0x2300;
        let country_leader = 0x3000;
        let country = 0x4000;
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
        mem.write_u64(advisor, ADVISOR_VT);
        mem.write_u64(advisor + CADVISOR_CHARACTER_PTR_OFF, character);

        let cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            skipped_root_rows: 0,
            known_families: compact::FAMILY_ALL,
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
                gfx_encoding: compact::PortraitIdentityEncoding::Utf8,
            }],
        };
        let frame_input = NativeFrameInput {
            elapsed_wall_ms: 88,
            status_flags: 0,
            skipped_input_rows: 3,
            countries: vec![CountryPoliticsRoot {
                country_addr: country,
                country_tag_db_id: 2,
                ruling_party: Some("democratic".to_string()),
                ruler_name: Some("Maxwell Aitken".to_string()),
                ruler_token: Some("ENG_maxwell_aitken".to_string()),
                active_idea_tokens: vec!["ENG_maxwell_aitken".to_string()],
            }],
        };
        (mem, cache, frame_input)
    }

    #[test]
    fn adapter_input_ref_matches_owned_projection() {
        let (_mem, cache, frame_input) = borrowed_input_fixture();
        let owned = adapter_input_from_cache(&cache, frame_input.clone());
        let borrowed = adapter_input_ref_from_cache(&cache, &frame_input);
        assert_eq!(borrowed, owned.as_ref());
        assert!(!borrowed.characters.is_empty());
        assert!(!borrowed.unit_leaders.is_empty());
        assert!(!borrowed.unit_leader_trait_tokens.is_empty());
        assert!(!borrowed.country_leader_roles.is_empty());
        assert!(!borrowed.countries.is_empty());
        assert!(!borrowed.advisors.is_empty());
        assert!(!borrowed.portraits.is_empty());
        assert_eq!(borrowed.skipped_input_rows, 3);
    }

    #[test]
    fn reused_walk_matches_full_walk() {
        let (mem, cache, frame_input) = borrowed_input_fixture();
        let mut input = frame_input;
        input.skipped_input_rows = 0;
        let input_again = input.clone();
        let plain = observe_native_scheduled_frame(&mem, &cache, input.clone());
        assert_eq!(plain.reused_walk_families(), 0);
        let plain_read = finish_native_scheduled_read(plain.clone());
        let mut full = plain.clone();
        let cached = full
            .adopt_cached_walk(CACHEABLE_WALK_FAMILIES)
            .expect("the fixture walk is clean, so it caches");
        assert_eq!(cached.families, CACHEABLE_WALK_FAMILIES);
        assert!(!cached.characters.is_empty(), "fixture has characters");
        assert!(full.frame.characters.is_empty(), "moved, not cloned");
        assert_eq!(full.reused_walk_families(), CACHEABLE_WALK_FAMILIES);
        assert_eq!(
            finish_native_scheduled_read(full.clone()).payload,
            plain_read.payload
        );
        let reused =
            observe_native_scheduled_frame_reusing(&mem, &cache, input, Some(cached.clone()));
        assert_eq!(reused.reused_walk_families(), CACHEABLE_WALK_FAMILIES);
        assert!(reused.frame.characters.is_empty());
        assert!(reused.frame.advisor_roster.is_empty());
        assert!(reused.frame.country_leader_roles.is_empty());
        assert!(reused.frame.unit_leaders.is_empty());
        let full_read = finish_native_scheduled_read(full.clone());
        let reused_read = finish_native_scheduled_read(reused.clone());
        assert_eq!(reused_read.payload, full_read.payload);
        assert_eq!(reused_read.row_count, full_read.row_count);
        assert_eq!(reused_read.status_flags, full_read.status_flags);
        assert!(reused_read.parse_errors.is_empty());
        let without_timings = |mut stats: AdapterBuildStats| {
            stats.walk_characters_us = 0;
            stats.walk_unit_leaders_us = 0;
            stats.walk_country_leader_roles_us = 0;
            stats.walk_active_rulers_us = 0;
            stats.walk_advisors_us = 0;
            stats.walk_appointed_advisors_us = 0;
            stats.walk_portraits_us = 0;
            stats
        };
        assert_eq!(
            without_timings(reused_read.adapter_stats),
            without_timings(full_read.adapter_stats)
        );
        let none = observe_native_scheduled_frame_reusing(
            &mem,
            &cache,
            input_again.clone(),
            Some(Arc::new(CachedWalk::default())),
        );
        assert_eq!(none.reused_walk_families(), 0);
        let mut partial = plain.clone();
        let cached_partial = partial
            .adopt_cached_walk(compact::FAMILY_ADVISOR_ROSTER)
            .expect("advisor roster caches");
        assert_eq!(cached_partial.families, compact::FAMILY_ADVISOR_ROSTER);
        assert_eq!(
            finish_native_scheduled_read(partial.clone()).payload,
            plain_read.payload
        );
        let reused_partial = observe_native_scheduled_frame_reusing(
            &mem,
            &cache,
            input_again.clone(),
            Some(cached_partial),
        );
        assert_eq!(
            finish_native_scheduled_read(reused_partial).payload,
            plain_read.payload
        );
        let mut mem2 = mem.clone();
        write_character(&mut mem2, 0x6000, 7, 7);
        mem2.write_u64(0x7000, ADVISOR_VT);
        mem2.write_u64(0x7000 + CADVISOR_CHARACTER_PTR_OFF, 0x6000);
        let mut cache_two = cache.clone();
        cache_two.characters.push(CharacterRoot {
            character_addr: 0x6000,
            char_id: 7,
            name: Some("Other".to_string()),
            token: Some("OTHER_other".to_string()),
        });
        cache_two.advisors = vec![AdvisorRoot {
            advisor_addr: 0x7000,
            idea_token: "OTHER_theorist".to_string(),
            slot: Some("theorist".to_string()),
            ledger: None,
        }];
        cache_two.portraits.push(CharacterPortraitInput {
            country_tag_db_id: 7,
            char_id: 7,
            role: "civilian".to_string(),
            size: "large".to_string(),
            gfx: "GFX_portrait_other".to_string(),
            gfx_encoding: compact::PortraitIdentityEncoding::Utf8,
        });
        let plain_two = observe_native_scheduled_frame(&mem2, &cache_two, input_again.clone());
        let plain_two_read = finish_native_scheduled_read(plain_two.clone());
        assert_eq!(
            plain_two.frame.character_portraits.len(),
            2,
            "both admitted"
        );
        let mut adopting_two = plain_two.clone();
        let cached_two = adopting_two
            .adopt_cached_walk(compact::FAMILY_ADVISOR_ROSTER)
            .expect("advisor roster caches");
        assert_eq!(cached_two.known_keys, vec![(7, 7)]);
        assert_eq!(
            cached_two.character_portraits.len(),
            1,
            "the cached-key subset"
        );
        assert_eq!(
            adopting_two.frame.character_portraits.len(),
            2,
            "kept whole: a live-only admission"
        );
        assert_eq!(
            finish_native_scheduled_read(adopting_two).payload,
            plain_two_read.payload
        );
        let fallback = observe_native_scheduled_frame_reusing(
            &mem2,
            &cache_two,
            input_again.clone(),
            Some(cached_two),
        );
        let keys: Vec<(u32, u32)> = fallback
            .frame
            .character_portraits
            .iter()
            .map(|p| (p.country_tag_db_id, p.char_id))
            .collect();
        assert_eq!(keys, vec![(2, 1458), (7, 7)], "whole family, input order");
        let fallback_read = finish_native_scheduled_read(fallback);
        assert_eq!(fallback_read.payload, plain_two_read.payload);
        assert_eq!(fallback_read.adapter_stats.portrait_rows, 2);
        assert!(reused
            .clone()
            .adopt_cached_walk(CACHEABLE_WALK_FAMILIES)
            .is_none());
        let half = plain
            .clone()
            .adopt_cached_walk(compact::FAMILY_CHARACTER | compact::FAMILY_UNIT_LEADER)
            .expect("characters alone cache");
        assert_eq!(half.families, compact::FAMILY_CHARACTER);
        assert!(half.unit_leaders.is_empty());
    }

    /// Lane PH: the portrait index counts input ROWS, not distinct keys.
    ///
    /// A portrait key repeats across roles and sizes, so the row count and the
    /// distinct-key count differ, and only the row count reproduces what the
    /// row walk contributes to `rejected_roots` — the number reported on the
    /// wire as `domain47_adapter_rejected_roots`.
    /// `both_portrait_forms_agree_on_every_shape` cannot reach this: it stages
    /// its index directly, so the PRODUCTION precompute in `adopt_cached_walk`
    /// went untested until this existed, and the swap survived (Codex round 1,
    /// finding 2).
    ///
    /// THIS IS A DEFENSIVE UNIT TEST, NOT A PRODUCTION SHAPE, and the
    /// distinction is worth stating because the second review round caught the
    /// earlier comment claiming otherwise. Two things make the input below
    /// unproducible by today's collector:
    ///
    ///  * it gives a character a portrait without giving it a character root.
    ///    `attach_log.rs` builds the portrait list while enumerating character
    ///    roots and filters the two together, so every portrait key really is
    ///    a character-root key; adopt `FAMILY_CHARACTER` and that key is in
    ///    `known_keys`, and do not adopt it and the character walk re-admits
    ///    the key on the reusing frame. Either way `portrait_unknown_rows`
    ///    cannot currently do any work.
    ///  * the reusing frame has no countries at all, and an empty country
    ///    authority makes the collector return rowless before it reaches this
    ///    adapter.
    ///
    /// It is kept anyway, because BOTH of those invariants live in
    /// `attach_log.rs` — a different crate — and neither is enforced here.
    /// This crate's contract is "the index answers what the row walk would
    /// have answered", and that has to hold whatever list the collector hands
    /// over. The fixture uses the active ruler because it is the one family
    /// that feeds `known_characters` and can never feed `known_keys` (it is
    /// absent from `CACHEABLE_WALK_FAMILIES`), which is the cheapest way to
    /// construct the row/key gap at this seam.
    ///
    /// kills: counting `portrait_unknown_keys.len()` after the dedup instead of
    /// before it reports 1 rejection where the row walk reports 3.
    #[test]
    fn the_portrait_index_counts_rows_and_not_distinct_keys() {
        let mut mem = FakeMemory::default();
        let ruler_character = 0x1000;
        let advisor_character = 0x6000;
        let advisor = 0x7000;
        let country = 0x4000;
        let status = 0x4100;
        let party = 0x4200;
        let leader_vec = 0x4300;
        let country_leader = 0x4400;

        write_character(&mut mem, ruler_character, 1458, 2);
        write_character(&mut mem, advisor_character, 7, 7);
        mem.write_u64(advisor, ADVISOR_VT);
        mem.write_u64(advisor + CADVISOR_CHARACTER_PTR_OFF, advisor_character);
        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(
            country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
            ruler_character,
        );
        mem.write_u64(country + CCOUNTRY_POLITICAL_STATUS_OFF, status);
        mem.write_u64(status + CPOLITICAL_STATUS_RULING_PARTY_OFF, party);
        mem.write_u64(party + CPOLITICAL_PARTY_LEADER_VEC_OFF, leader_vec);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_SIZE_OFF, 1);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_CAP_OFF, 1);
        mem.write_u64(leader_vec, country_leader);

        let portrait = |tag: u32, id: u32, role: &str| CharacterPortraitInput {
            country_tag_db_id: tag,
            char_id: id,
            role: role.to_string(),
            size: "large".to_string(),
            gfx: format!("GFX_portrait_{tag}_{id}_{role}"),
            gfx_encoding: compact::PortraitIdentityEncoding::Utf8,
        };
        let cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            known_families: compact::FAMILY_ALL,
            // The RULER's character is not here on purpose.
            characters: vec![CharacterRoot {
                character_addr: advisor_character,
                char_id: 7,
                name: None,
                token: None,
            }],
            advisors: vec![AdvisorRoot {
                advisor_addr: advisor,
                idea_token: "OTHER_theorist".to_string(),
                slot: Some("theorist".to_string()),
                ledger: None,
            }],
            // THREE rows share the ruler's key; one names the advisor's.
            portraits: vec![
                portrait(2, 1458, "civilian"),
                portrait(2, 1458, "army"),
                portrait(2, 1458, "navy"),
                portrait(7, 7, "civilian"),
            ],
            ..NativeRootCache::default()
        };
        let with_country = || NativeFrameInput {
            countries: vec![CountryPoliticsRoot {
                country_addr: country,
                country_tag_db_id: 2,
                ruling_party: Some("democratic".to_string()),
                ruler_name: None,
                ruler_token: None,
                active_idea_tokens: vec![],
            }],
            ..NativeFrameInput::default()
        };

        let mut adopting = observe_native_scheduled_frame(&mem, &cache, with_country());
        assert_eq!(
            adopting.stats.rejected_roots, 0,
            "the adopting frame must admit every portrait, or the walk cannot cache"
        );
        assert_eq!(
            adopting.stats.active_ruler_rows, 1,
            "the ruler is what admits the three rows"
        );
        let cached = adopting
            .adopt_cached_walk(compact::FAMILY_CHARACTER | compact::FAMILY_ADVISOR_ROSTER)
            .expect("a clean walk caches");
        assert!(
            !cached.knows(&(2, 1458)),
            "the ruler's key can never enter `known_keys`"
        );
        let index = cached
            .portrait_index
            .as_ref()
            .expect("adoption builds the index");
        assert_eq!(index.portrait_input_rows, 4);
        assert_eq!(
            index.portrait_unknown_keys,
            vec![(2, 1458)],
            "one distinct key the cache does not know"
        );
        assert_eq!(
            index.portrait_unknown_rows, 3,
            "but THREE input rows carry it — this is the number the walk rejects"
        );

        // No country now, so `known_characters` is empty and the three rows
        // have nothing to admit them.
        let reused = observe_native_scheduled_frame_reusing(
            &mem,
            &cache,
            NativeFrameInput::default(),
            Some(cached.clone()),
        );
        assert_eq!(
            reused.stats.rejected_roots, 3,
            "the fast path must report one rejection per ROW, not per key"
        );
    }

    #[test]
    fn borrowed_scheduled_read_matches_owned_build() {
        let (mem, cache, frame_input) = borrowed_input_fixture();
        let mut clean_input = frame_input.clone();
        clean_input.skipped_input_rows = 0;

        for input in [frame_input, clean_input] {
            let read = read_native_scheduled_frame(&mem, &cache, input.clone());
            let owned = adapter_input_from_cache(&cache, input);
            let owned_result = crate::adapter::build_lane_frame_from_adapter_input(
                &mem,
                &cache.roots.expect("fixture has roots"),
                &owned,
            );
            let decided_before_the_walk =
                cache.skipped_root_rows != 0 || owned.skipped_input_rows != 0;
            let expected_frame = if decided_before_the_walk || owned_result.stats.has_rejections() {
                let mut frame = LaneFrame::new(owned.elapsed_wall_ms);
                frame.status_flags = owned.status_flags
                    | compact::STATUS_SOURCE_UNAVAILABLE
                    | compact::STATUS_PARTIAL_DATA;
                frame
            } else {
                let mut frame = owned_result.frame.clone();
                frame.known_families = if owned_result.stats.active_ruler_unresolved_roots == 0 {
                    cache.known_families
                } else {
                    cache.known_families & !compact::FAMILY_ACTIVE_RULER
                };
                frame
            };
            assert_eq!(
                read.payload,
                encode_frame_payload(&expected_frame),
                "payload must be byte-identical to the owned build"
            );
            let without_timings = |mut stats: AdapterBuildStats| {
                stats.walk_characters_us = 0;
                stats.walk_unit_leaders_us = 0;
                stats.walk_country_leader_roles_us = 0;
                stats.walk_active_rulers_us = 0;
                stats.walk_advisors_us = 0;
                stats.walk_appointed_advisors_us = 0;
                stats.walk_portraits_us = 0;
                stats
            };
            assert_eq!(
                without_timings(read.adapter_stats),
                if decided_before_the_walk {
                    AdapterBuildStats::default()
                } else {
                    without_timings(owned_result.stats)
                }
            );
            assert_eq!(read.status_flags, {
                let gate = compact::decode_payload(0, &read.payload);
                gate.status_flags_or | expected_frame.status_flags
            });
        }
    }

    #[test]
    fn portrait_roots_count_toward_the_adoptable_root_total() {
        let mut cache = NativeRootCache {
            roots: Some(roots()),
            roots_validated: true,
            ..NativeRootCache::default()
        };
        assert_eq!(cache.stable_root_count(), 0);
        assert!(!cache.has_stable_roots());
        cache.portraits.push(CharacterPortraitInput {
            country_tag_db_id: 2,
            char_id: 1458,
            role: "civilian".to_string(),
            size: "large".to_string(),
            gfx: "GFX_portrait_ENG_maxwell_aitken".to_string(),
            gfx_encoding: compact::PortraitIdentityEncoding::Utf8,
        });
        assert_eq!(cache.stable_root_count(), 1);
        assert!(cache.has_stable_roots());
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
