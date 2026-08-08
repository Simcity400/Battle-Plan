use std::time::Instant;

use crate::adapter::{
    build_lane_frame_from_adapter_input_ref, AdapterBuildStats, AdapterInput, AdapterInputRef,
    AdvisorRoot, CharacterPortraitInput, CharacterRoot, CountryLeaderRoleRoot, CountryPoliticsRoot,
    TraitDefinitionRoot, UnitLeaderRoot,
};
use crate::domain47_characters_leaders_cabinet_live as compact;
use crate::extractor::{
    encode_frame_payload, LaneFrame, LaneMemory, ResolvedTypeRoots, DOMAIN_ID, DOMAIN_NAME,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeRootCache {
    pub roots: Option<ResolvedTypeRoots>,
    pub roots_validated: bool,
    pub skipped_root_rows: u32,
    /// Which row families the prewarm that adopted these roots actually
    /// OBSERVED.
    ///
    /// The character census and the unit leaders it reaches through
    /// `CCharacter+0xA8` come from an exact engine-owned manager roster; the
    /// country-leader-role, advisor and trait-definition roots come from the
    /// shared heap sweep, which needs seconds to complete its first pass. Those
    /// two sources fail independently, and this field is what lets a prewarm
    /// adopt the half it did read instead of discarding both.
    ///
    /// A clear bit means the frame walk must not emit that family AND the fold
    /// must not treat its absence as a deletion — see
    /// [`compact::FAMILY_CHARACTER`].
    ///
    /// Not `Default`-derived: zero would mean "observed nothing" and would turn
    /// every `..Default::default()` construction into a silent total gap.
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
        // BORROW the cached roots instead of deep-cloning them into an owned
        // `AdapterInput` (d47 critical-path work, 2026-08-03). The frame walk
        // only reads them, and the clone was thousands of per-hour `String`
        // allocations that were dropped again at the end of the same frame.
        // `adapter_input_from_cache` still exists and still produces the same
        // owned value for the fixture/benchmark entry points; both forms feed
        // the identical builder over the identical data.
        let adapter_input = adapter_input_ref_from_cache(cache, &frame_input);
        let mut adapter_result = build_lane_frame_from_adapter_input_ref(
            mem,
            &cache.roots.expect("checked by has_stable_roots"),
            &adapter_input,
        );
        // The frame can only have observed what the roots that fed it did. The
        // adapter builds every family it is given input for and cannot tell an
        // empty family from an unobserved one — that distinction lives in the
        // cache, which knows which prewarm sources succeeded, so it is applied
        // here rather than inferred from row counts.
        //
        // The active-ruler family is the exception, and it is decided HERE
        // because it is the only family whose source is not the root cache: it
        // is walked fresh from this frame's country politics roots. A country
        // whose ruler chain could not be read produces no row, and the fold
        // full-replaces `commanders.leader` for every family it is told was
        // observed — so declaring the family observed while one country's read
        // failed would DELETE that country's leader and call it current. The
        // frame says it did not observe the family instead, and the fold keeps
        // the last leader it was told about.
        //
        // A country that has no leader at all is not a failure and does not
        // reach this: `active_ruler_absent_roots` counts those separately.
        adapter_result.frame.known_families = if adapter_result.stats.active_ruler_unresolved_roots
            == 0
        {
            cache.known_families
        } else {
            // The rows go with the bit. A family declared unobserved carries no
            // rows — the decoder refuses a payload that says otherwise, and a
            // refused payload costs the WHOLE d47 hour, census included. So the
            // successfully-read rulers are dropped here rather than emitted
            // under a declaration that disowns them.
            //
            // Losing them is the honest price of a whole-family vocabulary: the
            // mask can say "this family was not observed" and cannot say "these
            // three countries of it were". Retaining the previous hour's
            // leaders for every country beats publishing a set that silently
            // omits the ones that failed.
            adapter_result.frame.active_rulers.clear();
            cache.known_families & !compact::FAMILY_ACTIVE_RULER
        };
        if cache.skipped_root_rows != 0
            || adapter_result.stats.has_rejections()
            || adapter_input.skipped_input_rows != 0
        {
            // A torn root, an adapter rejection or a skipped country row is
            // ROW-level loss inside families this frame did observe. The
            // family-presence mask cannot describe it — it is a whole-family
            // vocabulary — so this frame must stay OUTSIDE the expected-partial
            // tolerance, and `SOURCE_UNAVAILABLE` is what keeps it there.
            //
            // Before per-family admission this branch set `PARTIAL_DATA` alone,
            // which was harmless only because no tolerance covered it. It does
            // now, and a tolerance written for a declared family gap must never
            // absorb an undeclared row loss.
            let mut frame = LaneFrame::new(adapter_input.elapsed_wall_ms);
            frame.status_flags = adapter_input.status_flags
                | compact::STATUS_SOURCE_UNAVAILABLE
                | compact::STATUS_PARTIAL_DATA;
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

/// Allocation-free twin of [`adapter_input_from_cache`]: the same field-for-field
/// projection, borrowing the cache's root vectors and the frame's country vector
/// instead of cloning them. `adapter_input_ref_from_cache(c, &f)` is
/// `adapter_input_from_cache(c, f).as_ref()` by construction —
/// `adapter_input_ref_matches_owned_projection` pins that equality.
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
        // Row-level loss is not a family gap. It must keep the second failure
        // bit that holds it outside d47's expected-partial tolerance, and it
        // must declare no mask — there is nothing it can honestly name.
        assert_ne!(read.status_flags & compact::STATUS_SOURCE_UNAVAILABLE, 0);
        assert_eq!(decoded.family_presence_mask, compact::FAMILY_ALL);
        assert!(decoded.rows.is_empty());
    }

    /// A recycled CHARACTER must cost its own text and nothing else — checked
    /// at the SCHEDULED layer, which is where the cost is decided.
    ///
    /// The adapter-level tests cannot see this: `read_native_scheduled_frame`
    /// throws away the adapter's whole frame and substitutes a rowless one
    /// whenever `has_rejections()` holds, so an adapter that "degrades" a row
    /// and a wrapper that discards every row are indistinguishable one layer
    /// down. Codex 019fd6ea, finding 1: the first version of this change
    /// routed the degradation through `has_rejections`, and one recycled
    /// character out of thousands emptied the entire domain-47 frame.
    ///
    /// MUTATION: add `identity_degraded_rows` to `AdapterBuildStats::
    /// has_rejections` and this fails with `row_count == 0`.
    #[test]
    fn a_recycled_character_costs_its_own_text_and_not_the_frame() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        // The census read char 1458 here; a different character occupies the
        // storage now.
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
        // The frame survives, and it carries the character that is actually
        // there.
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
        // The census's text reached nothing.
        assert!(!read.payload.windows(14).any(|w| w == b"Maxwell Aitken"));
        // Declared, not silent.
        assert_ne!(read.status_flags & compact::STATUS_PARTIAL_DATA, 0);
    }

    /// A cache built from the character manager census while the heap sweep was
    /// still cold publishes its characters, and says on the wire which families
    /// it never looked at.
    ///
    /// This is the frame the whole change exists to produce. Before it, the
    /// identical cache state published a rowless
    /// `SOURCE_UNAVAILABLE|PARTIAL_DATA` record for every hour of a capture's
    /// first ~12 seconds.
    ///
    /// MUTATION: drop the `adapter_result.frame.known_families = cache.known_families`
    /// assignment in `read_native_scheduled_frame` and the payload claims to
    /// have observed the advisor and role families it never read.
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
            // The sweep-derived families are EMPTY because they were never
            // read, which is exactly the state the mask exists to describe.
            ..NativeRootCache::default()
        };

        // A real country politics root, so the bridge is available and the
        // status word is decided by the family gap alone. An empty `countries`
        // vector would set `BRIDGE_UNAVAILABLE` and the "only PARTIAL"
        // assertion below would be passing for the wrong reason.
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

        // PARTIAL and ONLY partial, against the SHARED failure mask rather than
        // a list of bits written here. Any other failure-class bit would take
        // this record out of d47's expected-partial tolerance and the fold
        // would discard the census this whole change exists to deliver — and a
        // hand-written list would not notice a bit added to the vocabulary
        // later.
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

    /// One country's ruler resolves and another's does not: the active-ruler
    /// family is declared unobserved, its rows go with the declaration, and the
    /// rest of the payload still publishes.
    ///
    /// This is the shape that made the first version of the fix worse than the
    /// bug. Clearing `FAMILY_ACTIVE_RULER` while leaving the successful rows in
    /// the frame produced a payload the canonical decoder refuses — rows for a
    /// declared-unobserved family — and the collector replaces a refused
    /// payload with a rowless `SOURCE_UNAVAILABLE|PARTIAL_DATA` record, losing
    /// the entire d47 hour including the character census.
    ///
    /// MUTATION: delete the `active_rulers.clear()` in
    /// `read_native_scheduled_frame` and this test fails on `parse_errors`.
    #[test]
    fn a_partly_unresolved_ruler_walk_drops_the_family_and_still_publishes() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 1458, 2);

        // A country whose whole chain reads.
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

        // And one whose political-status pointer is unreadable.
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

        // The payload is VALID. Before the rows were dropped alongside the bit,
        // this was a decoder refusal and the whole hour was lost.
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
        // The census still publishes: that is the whole point.
        assert!(decoded.rows.iter().any(|row| matches!(
            row,
            compact::DecodedDomain47Row::Character { char_id: 1458, .. }
        )));
        assert_eq!(
            read.status_flags & compact_contract::COMPACT_STATUS_FAILURE_MASK,
            compact::STATUS_PARTIAL_DATA
        );
    }

    /// Fixture with EVERY root family populated, so an equality assertion over
    /// the adapter input cannot pass vacuously.
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
        mem.write_u64(country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF, character);
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

    /// The borrowed projection must be the owned projection, field for field.
    ///
    /// MUTATION: point any field of `adapter_input_ref_from_cache` at a
    /// different cache vector (e.g. `advisors: &cache.portraits`… or simply
    /// swap `characters` and `country_leader_roles`), or drop
    /// `skipped_input_rows` — this equality fails.
    #[test]
    fn adapter_input_ref_matches_owned_projection() {
        let (_mem, cache, frame_input) = borrowed_input_fixture();
        let owned = adapter_input_from_cache(&cache, frame_input.clone());
        let borrowed = adapter_input_ref_from_cache(&cache, &frame_input);
        assert_eq!(borrowed, owned.as_ref());
        // Non-vacuity: every root family carries at least one row.
        assert!(!borrowed.characters.is_empty());
        assert!(!borrowed.unit_leaders.is_empty());
        assert!(!borrowed.unit_leader_trait_tokens.is_empty());
        assert!(!borrowed.country_leader_roles.is_empty());
        assert!(!borrowed.countries.is_empty());
        assert!(!borrowed.advisors.is_empty());
        assert!(!borrowed.portraits.is_empty());
        assert_eq!(borrowed.skipped_input_rows, 3);
    }

    /// End-to-end: the scheduled read (which now BORROWS the root cache) must
    /// produce the byte-identical payload, row count and adapter stats that the
    /// owned clone-then-build path produces.
    ///
    /// SCOPE, stated honestly: `build_lane_frame_from_adapter_input` DELEGATES
    /// to the `_ref` builder, so this test cannot detect a change inside the
    /// shared builder body — that is what the pre-existing adapter tests cover.
    /// What it does pin is everything the borrowed path does DIFFERENTLY: the
    /// `adapter_input_ref_from_cache` projection and the PARTIAL/clean wrapping
    /// in `read_native_scheduled_frame` that consumes it.
    ///
    /// MUTATION: make `adapter_input_ref_from_cache` hand the builder an empty
    /// `countries` slice (or any wrong/missing root family), or drop the
    /// `adapter_input.skipped_input_rows != 0` arm from the scheduled reader —
    /// the payload diverges and this fails.
    #[test]
    fn borrowed_scheduled_read_matches_owned_build() {
        let (mem, cache, frame_input) = borrowed_input_fixture();
        // The fixture deliberately carries skipped_input_rows, which makes the
        // scheduled reader emit the rowless-PARTIAL frame; compare the CLEAN
        // build too so the row-bearing path is covered.
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
            let expected_frame = if cache.skipped_root_rows != 0
                || owned_result.stats.has_rejections()
                || owned.skipped_input_rows != 0
            {
                let mut frame = LaneFrame::new(owned.elapsed_wall_ms);
                // Row-level loss inside an observed family is not a family gap,
                // so it must stay outside d47's expected-partial tolerance;
                // `SOURCE_UNAVAILABLE` is what holds it there.
                frame.status_flags = owned.status_flags
                    | compact::STATUS_SOURCE_UNAVAILABLE
                    | compact::STATUS_PARTIAL_DATA;
                frame
            } else {
                let mut frame = owned_result.frame.clone();
                // The fixture's country has no readable political-status chain,
                // so its ruler is unresolved and the active-ruler family cannot
                // be declared observed. Mirrored here rather than hard-coded so
                // the rule, not this fixture's shape, is what is being pinned.
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
            assert_eq!(read.adapter_stats, owned_result.stats);
            assert_eq!(read.status_flags, {
                let gate = compact::decode_payload(0, &read.payload);
                gate.status_flags_or | expected_frame.status_flags
            });
        }
    }

    /// Portrait roots are roots. Before the portrait feed existed they were
    /// the one root list the shared collector was forbidden to populate, and a
    /// cache holding nothing else was still "no stable roots"; now a prewarm
    /// that read portraits has read something, and `has_stable_roots` has to
    /// agree or the adoption gate discards the census that produced them.
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
