//! CGameState roots and stored session state.
//!
//! `local_player` copies playing and observer-view references together.
//! Its writer gates, lobby restore and observer selection are documented beside it.
//! Synthetic fixtures preserve raw words, reject short reads and verify the
//! constant guarded read cost.

use crate::{
    Memory,
    capture_text::{Owned, Value},
    msvc_string::Descriptor,
    read,
};

/// CGameState singleton.
pub const SINGLETON_RVA: usize = 0x0331_6e90;
/// Stored OS main-thread id initialized at executable startup.
/// Thread checks compare it with the current Windows thread id.
pub const MAIN_THREAD_ID_RVA: usize = 0x035c_ac94;

/// One original callback-entry numerical attempt; no native method is invoked.
pub fn read_main_thread_id(memory: &impl Memory, base: usize) -> Option<u32> {
    read(memory, base.checked_add(MAIN_THREAD_ID_RVA)?)
}
/// Play-session start Unix seconds.
pub const PLAY_SESSION_STARTED_AT_RVA: usize = 0x0331_7048;
/// CGameState CGameDate +0x460, raw i32 at +8.
pub const CURRENT_HOUR: usize = 0x468;
/// CGameState persisted u64 play time.
pub const PLAY_TIME: usize = 0x490;
/// Dense published variant pointer array, appended and saved in slot order from zero.
pub const EQUIPMENT_VARIANTS: usize = 0x708;
/// CGameState equipment-variant capacity, signed i32.
pub const EQUIPMENT_VARIANTS_CAPACITY: usize = 0x710;
/// CGameState equipment-variant count, signed i32.
pub const EQUIPMENT_VARIANTS_COUNT: usize = 0x714;
/// CGameState equipment-variant allocator.
pub const EQUIPMENT_VARIANTS_ALLOCATOR: usize = 0x718;
/// CGameState i32 seed.
pub const CAMPAIGN_SEED: usize = 0x720;
/// Stored campaign unique-id MSVC string.
pub const CAMPAIGN_ID: usize = 0x728;
/// CGameState speed index (0..4).
pub const SPEED: usize = 0x4bc;
/// CGameState `navy_id` allocation counter, signed i32.
pub const NAVY_ID: usize = 0x74c;
/// CGameState `land_combat_id` allocation counter, signed i32.
pub const LAND_COMBAT_ID: usize = 0x748;
/// Save-header serializer emits twenty-five consecutive fields.
pub const HEADER_SERIALIZER_RVA: usize = 0x001f_2a20;
/// Save-header loader restores twelve counter tokens to their matching
/// globals or setters. Token 0x2d4d instead loads +0x49c, not the id counter.
pub const HEADER_LOADER_RVA: usize = 0x001e_5550;
/// Tick marker, set before date advancement and cleared near return.
pub const TICK_BUSY: usize = 0x4c0;
/// CGameState CPdxArray<CCountry*>.
pub const COUNTRIES: usize = 0x310;
/// CGameState retained CPdxArray<i32> country slots.
pub const DAILY_COUNTRY_ROSTER: usize = 0x9a0;
/// CGameState country link indices.
pub const COUNTRY_LINKS: usize = 0x340;
/// Country-link tag strings, including released dynamic tags.
pub const COUNTRY_TAGS: usize = 0x358;
/// CGameState CPdxArray<CProvince*>.
pub const PROVINCES: usize = 0x2b0;

/// Stored u32 save version; save-oracle agreement remains unverified.
pub const SAVE_VERSION: usize = 0x58;
/// State MSVC-string candidate; writer and meaning remain unproven.
pub const SESSION_HEADER_TEXT: usize = 0x70;
/// CGameState retained save-header player array; the CHuman serializer preserves it separately from live players at +F8.
pub const SAVED_PLAYER_RECORDS: usize = 0xa8;
/// CGameState CIdeologyGroup* candidate.
pub const SESSION_IDEOLOGY_GROUP: usize = 0xc8;
/// State u32 candidate; this field does not establish the loaded DLC roster.
pub const SESSION_DLC_WORD: usize = 0xd4;
/// CGameState live CHuman pointer array; the world-clock reader and native player serializer distinguish it from saved records at +A8.
pub const PLAYER_RECORDS: usize = 0xf8;
/// CGameState embedded CCombatManager.
pub const COMBATS: usize = 0x260;
/// CGameState u32 province-vector capacity.
pub const PROVINCES_CAPACITY: usize = 0x2b8;
/// CGameState u32 province-vector count.
pub const PROVINCES_COUNT: usize = 0x2bc;
/// CGameState u32 country-vector capacity; CPdxArray layout and current guarded walker agree.
pub const COUNTRIES_CAPACITY: usize = 0x318;
/// CGameState u32 country-vector count.
pub const COUNTRIES_COUNT: usize = 0x31c;
/// CGameState major-country reference vector.
pub const MAJOR_COUNTRIES: usize = 0x328;
/// CGameState u32 major-country vector count.
pub const MAJOR_COUNTRIES_COUNT: usize = 0x334;
/// CGameState u32 country-link capacity; current guarded CPdxArray reader confirms +0x348.
pub const COUNTRY_LINKS_CAPACITY: usize = 0x348;
/// CGameState u32 country-link count; current guarded CPdxArray reader confirms +0x34c.
pub const COUNTRY_LINKS_COUNT: usize = 0x34c;
/// CGameState u32 country-tag string vector capacity.
pub const COUNTRY_TAGS_CAPACITY: usize = 0x360;
/// CGameState u32 country-tag string vector count.
pub const COUNTRY_TAGS_COUNT: usize = 0x364;
/// CGameState human-controller counts, native signed bytes; CHuman controller walk authenticates canonical country slots independently.
pub const HUMAN_STATUS: usize = 0x388;
/// CGameState human-status native signed array count; CHuman controller walk checks this separately from country count.
pub const HUMAN_STATUS_COUNT: usize = 0x394;
/// Per-canonical-country autonomous-control flags.
pub const AUTONOMOUS_STATUS: usize = 0x370;

/// Exercise-level AI-control gate, including live-player fallback.
pub fn autonomous_country(
    memory: &impl Memory,
    base: usize,
    state: usize,
    raw: u32,
) -> Result<bool, &'static str> {
    let reader = crate::ccountry::Reader::new(memory, base, state)?;
    let canonical = if raw == 0 {
        0
    } else {
        reader.resolve(raw)?.canonical_slot
    };
    AiControl::read(memory, state)?.autonomous(memory, base, &reader, raw, canonical)
}

/// Slots whose human-controller counts and autonomy flags one gather copies.
const CONTROL_CHUNK: usize = 256;

/// AI-control operands for many countries. Within the human-controller
/// counts, a country is AI-controlled exactly when its autonomy flag is
/// nonzero and count is zero. Past those counts, it is AI-controlled unless
/// a live human names it.
///
/// Slot-order census gathers up to `CONTROL_CHUNK` counts and flags in one
/// guarded copy. The first failure inside the counts, including gather
/// failure, fails control for the remainder of the sample. Covered operands
/// are never reread to hide a fault.
pub struct AiControl {
    state: usize,
    counts: crate::cpdx_array::Array,
    /// Read lazily when the first slot within controller counts needs it.
    flags: Option<crate::cpdx_array::Array>,
    first: u32,
    counts_len: u32,
    flags_len: u32,
    gathered_counts: [u8; CONTROL_CHUNK],
    gathered_flags: [u8; CONTROL_CHUNK],
    /// The first failure inside the counts, returned for every later slot.
    failed: Option<&'static str>,
}

impl AiControl {
    pub fn read(memory: &impl Memory, state: usize) -> Result<Self, &'static str> {
        let counts = state
            .checked_add(HUMAN_STATUS)
            .and_then(|p| crate::cpdx_array::Array::read(memory, p, 1))
            .ok_or("human-controller counts invalid")?;
        Ok(Self {
            state,
            counts,
            flags: None,
            first: 0,
            counts_len: 0,
            flags_len: 0,
            gathered_counts: [0; CONTROL_CHUNK],
            gathered_flags: [0; CONTROL_CHUNK],
            failed: None,
        })
    }

    /// Whether AI control applies to the country with raw tag `raw` and
    /// canonical slot `canonical`.
    pub fn autonomous<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        reader: &crate::ccountry::Reader<'_, M>,
        raw: u32,
        canonical: u32,
    ) -> Result<bool, &'static str> {
        use crate::{chuman, cpdx_array::Array};
        if canonical < self.counts.count {
            if let Some(reason) = self.failed {
                return Err(reason);
            }
            let result = self.inside_counts(memory, canonical);
            if let Err(reason) = result {
                self.failed = Some(reason);
            }
            return result;
        }
        let players = self
            .state
            .checked_add(PLAYER_RECORDS)
            .and_then(|p| Array::read(memory, p, chuman::STRIDE))
            .ok_or("live players invalid")?;
        for index in 0..players.count {
            let player = players
                .element(index, chuman::STRIDE)
                .ok_or("live player overflow")?;
            if read::<u64>(memory, player) != base.checked_add(chuman::VTABLE_RVA).map(|p| p as u64)
            {
                return Err("live player identity mismatch");
            }
            let tag = read::<u32>(memory, player + chuman::COUNTRY_TAG)
                .ok_or("live player tag unreadable")?;
            if tag == raw
                || (tag != 0 && raw != 0 && reader.resolve(tag)?.canonical_slot == canonical)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// AI-control verdict within controller counts from gathered operands.
    /// A slot past the flag count fails without additional memory reads.
    fn inside_counts(
        &mut self,
        memory: &impl Memory,
        canonical: u32,
    ) -> Result<bool, &'static str> {
        let flags = match self.flags {
            Some(flags) => flags,
            None => {
                let flags = self
                    .state
                    .checked_add(AUTONOMOUS_STATUS)
                    .and_then(|p| crate::cpdx_array::Array::read(memory, p, 1))
                    .ok_or("autonomous flags invalid")?;
                self.flags = Some(flags);
                flags
            }
        };
        if canonical < self.first || canonical - self.first >= self.counts_len {
            self.gather(memory, flags, canonical)?;
        }
        let offset = canonical - self.first;
        let flag = if offset < self.flags_len {
            self.gathered_flags[offset as usize]
        } else {
            flags
                .element(canonical, 1)
                .and_then(|p| read::<u8>(memory, p))
                .ok_or("autonomous flag unreadable")?
        };
        let humans = self.gathered_counts[offset as usize] as i8;
        if humans < 0 {
            return Err("negative human-controller count");
        }
        Ok(flag != 0 && humans == 0)
    }

    /// Copy counts and flags from `first` in one guarded call. Flag-count
    /// shortages fail at their individual check. Copy faults remain the original
    /// observation failure; covered operands are never reread.
    fn gather(
        &mut self,
        memory: &impl Memory,
        flags: crate::cpdx_array::Array,
        first: u32,
    ) -> Result<(), &'static str> {
        self.first = first;
        self.counts_len = 0;
        self.flags_len = 0;
        let counts_len = (self.counts.count - first).min(CONTROL_CHUNK as u32);
        let flags_len = flags.count.saturating_sub(first).min(counts_len);
        let counts_at = self
            .counts
            .element(first, 1)
            .ok_or("human-controller count unreadable")?;
        // An empty span is complete without its address being read.
        let flags_at = flags.element(first, 1).unwrap_or(0);
        let copied = memory.copy_scattered(
            [counts_at, flags_at],
            &mut [
                &mut self.gathered_counts[..counts_len as usize],
                &mut self.gathered_flags[..flags_len as usize],
            ],
        );
        if copied != 2 {
            return Err("human-controller counts or autonomy flags unreadable");
        }
        self.counts_len = counts_len;
        self.flags_len = flags_len;
        Ok(())
    }
}
/// Embedded technology-sharing vector data, not a manager pointer.
pub const TECH_SHARING_GROUPS: usize = 0x3a0;
/// CGameState technology-sharing vector u32 capacity.
pub const TECH_SHARING_GROUPS_CAPACITY: usize = 0x3a8;
/// CGameState technology-sharing vector u32 count.
pub const TECH_SHARING_GROUPS_COUNT: usize = 0x3ac;
/// CGameState CSupplySystem*.
pub const SUPPLY_SYSTEM: usize = 0x3d8;
/// Rail-manager pointer owning its province table.
pub const RAIL_MANAGER: usize = 0x3e0;
/// CGameState international-market manager pointer.
pub const MARKET_MANAGER: usize = 0x3e8;
/// CGameState CRaidSystem*.
pub const RAID_SYSTEM: usize = 0x3f0;
/// CGameState faction-system pointer.
pub const FACTION_SYSTEM: usize = 0x3f8;
/// Inline game date with its raw hour eight bytes later.
pub const CURRENT_DATE: usize = 0x460;
/// Start-date raw word; full representation remains unproven.
pub const START_DATE_RAW: usize = 0x4a8;
/// Tick-progress word, independent of the game-speed field.
pub const TICK_PROGRESS: usize = 0x4b8;
/// Sunk-convoy history vector with 0x18-byte rows.
pub const SUNK_CONVOY_HISTORY: usize = 0x5a8;
/// CGameState CWeatherManager*.
pub const WEATHER_MANAGER: usize = 0x688;
/// CGameState strategic-air manager pointer.
pub const STRATEGIC_AIR_MANAGER: usize = 0x690;
/// CGameState CStrategicNavyManager pointer.
pub const STRATEGIC_NAVY_MANAGER: usize = 0x698;
/// CGameState CStrategicOperativeManager*.
pub const STRATEGIC_OPERATIVE_MANAGER: usize = 0x6a0;
/// CGameState CCharacterManager*.
pub const CHARACTER_MANAGER: usize = 0x6a8;
/// CGameState CWorldThreat*.
pub const WORLD_THREAT: usize = 0x6b0;
/// Player-input eligibility byte, set before load completion.
/// It does not establish world readiness.
pub const PLAYER_INPUT_ELIGIBLE: usize = 0xa39;
/// Game-speed setter; passive capture never calls it.
pub const SET_SPEED_RVA: usize = 0x001e_dad0;
/// Exact game-speed setter entry bytes.
pub const SET_SPEED_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x48, 0x83, 0xec, 0x40, 0x4c, 0x89, 0x74, 0x24, 0x30, 0x85, 0xd2, 0x79, 0x04, 0x33,
];
/// CGameState generated input-eligibility setter.
pub const SET_INPUT_ELIGIBLE_RVA: usize = 0x001e_dc70;
/// Game-state constructor/reset initializes input state.
pub const RESET_RVA: usize = 0x001e_0190;
/// Load setup sets eligibility before remaining calls; it does not establish world readiness.
pub const LOAD_SETUP_RVA: usize = 0x00dc_5310;

/// CGameState global scripted flag-manager pointer.
pub const GLOBAL_FLAGS: usize = 0x258;
/// CGameState state pointer table.
pub const STATES: usize = 0x2c8;
/// CGameState state table count i32.
pub const STATES_COUNT: usize = 0x2d4;
/// CGameState strategic-region pointer array.
pub const STRATEGIC_REGIONS: usize = 0x2e0;
/// CGameState strategic-region array capacity.
pub const STRATEGIC_REGIONS_CAPACITY: usize = 0x2e8;
/// CGameState strategic-region native signed count.
pub const STRATEGIC_REGIONS_COUNT: usize = 0x2ec;
/// CGameState power-balance manager pointer.
pub const POWER_BALANCE_MANAGER: usize = 0x450;
/// CGameState next event instance u32.
pub const NEXT_EVENT_INSTANCE: usize = 0x498;
/// CGameState embedded peace-conference container.
pub const PEACE_CONFERENCES: usize = 0x4e0;
/// CGameState peace-conference data pointer.
pub const PEACE_CONFERENCES_DATA: usize = 0x4e8;
/// CGameState peace-conference count i32.
pub const PEACE_CONFERENCES_COUNT: usize = 0x4f4;
/// Combat-log manager pointer array indexed by canonical country slot,
/// with null slot zero. Serialization and combat-log writers use this ordering.
pub const COMBAT_LOGS: usize = 0x880;
/// Local playing country, as a raw tag reference. Tag-switch and controller
/// assignment commands update it only for the local player's key. Observer
/// commands switch it to reference zero.
pub const LOCAL_PLAYER_COUNTRY: usize = 0x520;
/// Observer's viewed country. A positive playing tag clears it, but any
/// player's observer switch can update it, so it can remain nonzero while
/// this client plays. Viewing uses the positive playing tag first, otherwise this.
pub const OBSERVED_COUNTRY: usize = 0x524;

/// The client's two stored references, independent of the human roster.
/// Commands, load, lobby and editing paths write them outside the hourly tick.
/// Sampling at hourly-update entry observes the same choice used by combat
/// management. Switching to the viewed country also refreshes combat state.
/// The save's player tag is the viewing choice. Read both references in one
/// eight-byte guarded copy, independent of country count and resolution.
pub fn local_player(memory: &impl Memory, state: usize) -> Result<[u32; 2], &'static str> {
    const { assert!(OBSERVED_COUNTRY == LOCAL_PLAYER_COUNTRY + 4) };
    state
        .checked_add(LOCAL_PLAYER_COUNTRY)
        .and_then(|address| read::<u64>(memory, address))
        .map(|words| [words as u32, (words >> 32) as u32])
        .ok_or("local player references unreadable")
}
/// CGameState inline fired-event set.
pub const FIRED_EVENTS: usize = 0x538;
/// CGameState pending-event vector.
pub const PENDING_EVENTS: usize = 0x560;
/// CGameState pending-event vector capacity u32.
pub const PENDING_EVENTS_CAPACITY: usize = 0x568;
/// CGameState pending-event vector count u32.
pub const PENDING_EVENTS_COUNT: usize = 0x56c;
/// CGameState sunk-ship history vector.
pub const SUNK_SHIP_HISTORY: usize = 0x590;
/// CGameState sunk-convoy history u32 capacity.
pub const SUNK_CONVOY_HISTORY_CAPACITY: usize = 0x5b0;
/// CGameState sunk-convoy history u32 count.
pub const SUNK_CONVOY_HISTORY_COUNT: usize = 0x5b4;
/// CGameState naval combat-result vector.
pub const NAVAL_COMBAT_RESULTS: usize = 0x5c0;
/// CGameState naval combat-result u32 count.
pub const NAVAL_COMBAT_RESULTS_COUNT: usize = 0x5cc;
/// CGameState global scripted variables object.
pub const GLOBAL_VARIABLES: usize = 0x980;
/// Fired-event set accessor.
pub const FIRED_EVENTS_ACCESSOR_RVA: usize = 0x001e_dac0;
/// Pending-event registry writer.
pub const REGISTER_PENDING_EVENT_RVA: usize = 0x001c_ab70;

/// CGameState fleet-id high-water counter u32.
pub const NEXT_FLEET_ID: usize = 0x74c;
/// Ship-registration gate byte suppresses counter growth when set;
/// its full meaning remains unproven.
pub const SHIP_REGISTRATION_GATE: usize = 0xa35;
/// Fleet-id minter uses type 61 and increments the fleet counter.
pub const MINT_FLEET_ID_RVA: usize = 0x001e_4040;
/// CGameState sunk-ship history capacity u32.
pub const SUNK_SHIP_HISTORY_CAPACITY: usize = 0x598;
/// CGameState sunk-ship history count u32.
pub const SUNK_SHIP_HISTORY_COUNT: usize = 0x59c;
/// CGameState retained naval-result capacity u32.
pub const NAVAL_COMBAT_RESULTS_CAPACITY: usize = 0x5c8;
/// CGameState province-keyed retained-result unordered map.
pub const NAVAL_RESULTS_BY_PROVINCE: usize = 0x5d8;
/// CGameState retained-result map list-head pointer.
pub const NAVAL_RESULTS_MAP_LIST_HEAD: usize = 0x5e0;
/// CGameState retained-result map node count u64 (MSVC list size).
pub const NAVAL_RESULTS_MAP_LIST_SIZE: usize = 0x5e8;
/// CGameState retained-result map buckets pointer.
pub const NAVAL_RESULTS_MAP_BUCKETS: usize = 0x5f0;
/// CGameState retained-result map bucket mask.
pub const NAVAL_RESULTS_MAP_MASK: usize = 0x608;
/// Retained naval-result insertion updates both containers.
pub const INSERT_NAVAL_RESULT_RVA: usize = 0x001c_a9c0;
/// Retained naval-result retirement sweep.
pub const SWEEP_NAVAL_RESULTS_RVA: usize = 0x001d_73b0;
/// Province-keyed retained naval-result lookup.
pub const FIND_NAVAL_RESULTS_RVA: usize = 0x001d_b500;
/// Retained naval-result removal updates both containers.
pub const ERASE_NAVAL_RESULT_RVA: usize = 0x001e_b700;
/// Attributed sunk-ship record append.
pub const APPEND_ATTRIBUTED_SUNK_SHIP_RVA: usize = 0x001d_2c90;
/// Combat-source sunk-ship record append.
pub const APPEND_COMBAT_SUNK_SHIP_RVA: usize = 0x001d_2d10;
/// Cause-based sunk-ship record append.
pub const APPEND_CAUSE_SUNK_SHIP_RVA: usize = 0x001d_2da0;
/// Entry bytes shared by the three CGameState sunk-ship append functions.
pub const APPEND_SUNK_SHIP_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
/// Sunk-convoy aggregate append or increment, keyed by month, killer and owner.
pub const AGGREGATE_SUNK_CONVOY_RVA: usize = 0x001c_ae90;
/// Game-state serializer.
pub const SERIALIZE_RVA: usize = 0x001f_2a20;
/// Game-state deserializer restores the complete unsigned play time.
pub const DESERIALIZE_RVA: usize = 0x001e_5550;
/// Country-control census accessor.
pub const COUNTRY_CENSUS_RVA: usize = 0x001e_1fe0;
/// Pending-event lookup.
pub const FIND_PENDING_EVENT_RVA: usize = 0x001d_a9f0;
/// Pending-event expiry.
pub const EXPIRE_PENDING_EVENTS_RVA: usize = 0x001f_0770;
/// Pending-event removal.
pub const REMOVE_PENDING_EVENT_RVA: usize = 0x001e_b860;
/// CGameState tutorial-highlight gate byte.
pub const TUTORIAL_HIGHLIGHT_GATE: usize = 0xa37;
/// Custom-highlight group vector with 0x20-byte records.
pub const CUSTOM_HIGHLIGHT_GROUPS: usize = 0xa10;
/// CGameState custom-highlight group count i32.
pub const CUSTOM_HIGHLIGHT_GROUP_COUNT: usize = 0xa1c;

/// Per-hour update boundary. Entry observes settled end-of-hour state:
/// all commands drained and tick_busy zero, before this tick increments the
/// date. Command drain uses server polymorphic dispatch; this fixed entry
/// fires once per game hour.
pub const HOURLY_UPDATE_RVA: usize = 0x001d_cef0;
/// Exact hourly-update entry bytes.
pub const HOURLY_UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x50, 0x10, 0x48, 0x89, 0x48, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41,
];
/// Daily update also runs during initialization; it is not a once-per-hour boundary.
pub const DAILY_CANDIDATE_RVA: usize = 0x001d_4390;
/// Exact daily-update entry bytes.
pub const DAILY_CANDIDATE_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac,
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Clock<T = Owned> {
    pub game_state: u64,
    pub hour: i32,
    pub play_time: u64,
    pub play_session_started_at: i64,
    pub seed: i32,
    pub campaign_id: T,
    pub speed: i32,
    pub tick_busy: u8,
}

impl<T> Clock<T> {
    pub fn with_campaign_id<U>(&self, campaign_id: U) -> Clock<U> {
        Clock {
            game_state: self.game_state,
            hour: self.hour,
            play_time: self.play_time,
            play_session_started_at: self.play_session_started_at,
            seed: self.seed,
            campaign_id,
            speed: self.speed,
            tick_busy: self.tick_busy,
        }
    }
}
impl Clock {
    pub fn borrowed(&self) -> Clock<&[u8]> {
        self.with_campaign_id(self.campaign_id.as_bytes())
    }
    pub fn copy_owned(&self) -> std::io::Result<Self> {
        let bytes = crate::allocation::copy_vec(self.campaign_id.as_bytes())?;
        Ok(self.with_campaign_id(Owned {
            len: bytes.len(),
            bytes,
        }))
    }
}

/// Campaign id is an unrestricted stored string. Header-only demand
/// prepares storage and never supplies a campaign observation.
pub fn campaign_id_len(memory: &impl Memory, base: usize) -> Result<usize, &'static str> {
    let state = singleton(memory, base).ok_or("clock game state unreadable")?;
    let address = state
        .checked_add(CAMPAIGN_ID)
        .ok_or("clock campaign address overflow")?;
    Ok(Descriptor::read(memory, address)?.len)
}

/// Read fresh native bytes into storage prepared before the callback.
pub fn read_clock_prepared<'a>(
    memory: &impl Memory,
    base: usize,
    campaign: &'a mut Owned,
) -> Result<Clock<&'a [u8]>, &'static str> {
    let state = singleton(memory, base).ok_or("clock game state unreadable")?;
    let at = |offset| state.checked_add(offset).ok_or("clock address overflow");
    let hour = read(memory, at(CURRENT_HOUR)?).ok_or("clock hour unreadable")?;
    let play_time = read(memory, at(PLAY_TIME)?).ok_or("clock play time unreadable")?;
    let play_session_started_at = base
        .checked_add(PLAY_SESSION_STARTED_AT_RVA)
        .and_then(|address| read(memory, address))
        .ok_or("clock load signal unreadable")?;
    let seed = read(memory, at(CAMPAIGN_SEED)?).ok_or("clock seed unreadable")?;
    campaign.read_prepared(memory, at(CAMPAIGN_ID)?)?;
    Ok(Clock {
        game_state: state as u64,
        hour,
        play_time,
        play_session_started_at,
        seed,
        campaign_id: campaign.as_bytes(),
        speed: read(memory, at(SPEED)?).ok_or("clock speed unreadable")?,
        tick_busy: read(memory, at(TICK_BUSY)?).ok_or("clock tick busy unreadable")?,
    })
}

pub fn read_clock(memory: &impl Memory, base: usize) -> Option<Clock> {
    let game_state = read::<u64>(memory, base.checked_add(SINGLETON_RVA)?)?;
    let ptr = usize::try_from(game_state).ok().filter(|ptr| *ptr != 0)?;
    let at = |offset| ptr.checked_add(offset);
    Some(Clock {
        game_state,
        hour: read(memory, at(CURRENT_HOUR)?)?,
        play_time: read(memory, at(PLAY_TIME)?)?,
        play_session_started_at: read(memory, base.checked_add(PLAY_SESSION_STARTED_AT_RVA)?)?,
        seed: read(memory, at(CAMPAIGN_SEED)?)?,
        campaign_id: Owned::read(memory, at(CAMPAIGN_ID)?, false).ok()?,
        speed: read(memory, at(SPEED)?)?,
        tick_busy: read(memory, at(TICK_BUSY)?)?,
    })
}

/// Live element counts of the three country registries, read once before the
/// hourly hook is installed so capture buffers can be sized to the loaded
/// content instead of a vanilla-sized guess. Header words only: no country
/// object is read and nothing here is recorded.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CountryRegistrySize {
    /// `CPdxArray<CCountry*>` count at +0x31c, including the reserved slot zero.
    pub countries: u32,
    /// Raw country-reference link count at +0x34c, including its zero sentinel.
    pub links: u32,
    /// `_CountryLinkTags` string count at +0x364.
    pub tags: u32,
}

impl CountryRegistrySize {
    /// The largest registry, which bounds every per-country table.
    pub fn slots(self) -> usize {
        self.countries.max(self.links).max(self.tags) as usize
    }
}

/// Reads the registry counts from a consistent array header. An unreadable
/// singleton or header returns `None`; a registry the game has not filled yet
/// (for example at the main menu) reads as zero.
pub fn read_country_registry_size(
    memory: &impl Memory,
    base: usize,
) -> Option<CountryRegistrySize> {
    let state = singleton(memory, base)?;
    Some(CountryRegistrySize {
        countries: array_count(memory, state, COUNTRIES)?,
        links: array_count(memory, state, COUNTRY_LINKS)?,
        tags: array_count(memory, state, COUNTRY_TAGS)?,
    })
}

/// Loaded strategic-region count from [`STRATEGIC_REGIONS`]. Like
/// [`read_country_registry_size`], only header words are read; an unfilled
/// roster reports zero.
pub fn read_strategic_region_count(memory: &impl Memory, base: usize) -> Option<u32> {
    array_count(memory, singleton(memory, base)?, STRATEGIC_REGIONS)
}

fn singleton(memory: &impl Memory, base: usize) -> Option<usize> {
    let game_state = read::<u64>(memory, base.checked_add(SINGLETON_RVA)?)?;
    usize::try_from(game_state).ok().filter(|ptr| *ptr != 0)
}

/// The count of the CPdxArray header at `state + array`.
fn array_count(memory: &impl Memory, state: usize, array: usize) -> Option<u32> {
    let header = read::<[u8; 16]>(memory, state.checked_add(array)?)?;
    let capacity = u32::from_le_bytes(header[8..12].try_into().ok()?);
    let count = u32::from_le_bytes(header[12..16].try_into().ok()?);
    // The CPdxArray header keeps count within its capacity, which is a
    // signed i32 in every growth path; anything else is a torn read.
    (count <= capacity && capacity <= i32::MAX as u32).then_some(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    #[test]
    fn local_player_keeps_both_words_and_refuses_short_reads() {
        for words in [[5_000_u32, 0], [0, 4_999], [0, 0], [u32::MAX, 1]] {
            let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
            let memory = SliceMemory {
                base: 0x1000 + LOCAL_PLAYER_COUNTRY,
                bytes: &bytes,
            };
            let counted = crate::memory::CountedMemory::new(SliceMemory { ..memory });
            crate::memory::ReadCounts::take_current();
            assert_eq!(local_player(&counted, 0x1000), Ok(words));
            let cost = crate::memory::ReadCounts::take_current();
            assert_eq!((cost.copy_calls, cost.bytes), (1, 8));
            for len in 0..8 {
                let short = SliceMemory {
                    bytes: &bytes[..len],
                    ..memory
                };
                assert!(local_player(&short, 0x1000).is_err());
            }
            assert!(local_player(&memory, usize::MAX).is_err());
        }
    }

    #[test]
    fn registry_sizes_read_the_country_and_region_headers_at_mod_scale() {
        struct RootMemory<'a>(&'a [u8]);
        impl Memory for RootMemory<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == SINGLETON_RVA && output.len() == 8 {
                    output.copy_from_slice(&0x1000_u64.to_le_bytes());
                    true
                } else {
                    SliceMemory {
                        base: 0x1000,
                        bytes: self.0,
                    }
                    .copy(address, output)
                }
            }
        }
        let mut bytes = [0_u8; 0x800];
        let mut header = |at: usize, capacity: u32, count: u32| {
            bytes[at..at + 8].copy_from_slice(&0x9000_u64.to_le_bytes());
            bytes[at + 8..at + 12].copy_from_slice(&capacity.to_le_bytes());
            bytes[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
        };
        // One country per province of a large mod, plus dynamic tags.
        header(COUNTRIES, 20_000, 15_001);
        header(COUNTRY_LINKS, 20_000, 15_101);
        header(COUNTRY_TAGS, 16_384, 15_001);
        header(STRATEGIC_REGIONS, 4_096, 2_500);
        let size = read_country_registry_size(&RootMemory(&bytes), 0).unwrap();
        assert_eq!(size.countries, 15_001);
        assert_eq!(size.links, 15_101);
        assert_eq!(size.tags, 15_001);
        assert_eq!(size.slots(), 15_101);
        assert_eq!(
            read_strategic_region_count(&RootMemory(&bytes), 0),
            Some(2_500)
        );
        // A torn header (count above capacity) is refused rather than trusted.
        bytes[COUNTRIES + 12..COUNTRIES + 16].copy_from_slice(&30_000_u32.to_le_bytes());
        assert_eq!(read_country_registry_size(&RootMemory(&bytes), 0), None);
        let regions = STRATEGIC_REGIONS + 12;
        bytes[regions..regions + 4].copy_from_slice(&5_000_u32.to_le_bytes());
        assert_eq!(read_strategic_region_count(&RootMemory(&bytes), 0), None);
    }

    #[test]
    fn persisted_play_time_is_read_with_its_full_native_width() {
        struct RootMemory<'a>(&'a [u8]);
        impl Memory for RootMemory<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == SINGLETON_RVA && output.len() == 8 {
                    output.copy_from_slice(&0x1000_u64.to_le_bytes());
                    true
                } else if address == PLAY_SESSION_STARTED_AT_RVA && output.len() == 8 {
                    output.copy_from_slice(&1_778_888_888_i64.to_le_bytes());
                    true
                } else {
                    SliceMemory {
                        base: 0x1000,
                        bytes: self.0,
                    }
                    .copy(address, output)
                }
            }
        }
        let mut bytes = [0_u8; 0x800];
        bytes[PLAY_TIME..PLAY_TIME + 8]
            .copy_from_slice(&9_223_372_036_854_775_809_u64.to_le_bytes());
        bytes[CURRENT_HOUR..CURRENT_HOUR + 4].copy_from_slice(&1936_i32.to_le_bytes());
        bytes[CAMPAIGN_ID..CAMPAIGN_ID + 3].copy_from_slice(b"abc");
        bytes[CAMPAIGN_ID + 16..CAMPAIGN_ID + 24].copy_from_slice(&3_u64.to_le_bytes());
        bytes[CAMPAIGN_ID + 24..CAMPAIGN_ID + 32].copy_from_slice(&15_u64.to_le_bytes());
        let clock = read_clock(&RootMemory(&bytes), 0).unwrap();
        assert_eq!(clock.play_time, 9_223_372_036_854_775_809);
        assert_eq!(clock.play_session_started_at, 1_778_888_888);
        assert_eq!(clock.hour, 1936);
        assert_eq!(clock.campaign_id.as_bytes(), b"abc");
    }

    struct ClockMemory {
        bytes: Vec<u8>,
        fault: Option<usize>,
    }
    impl Memory for ClockMemory {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self.fault.is_some_and(|fault| {
                (address..address.saturating_add(output.len())).contains(&fault)
            }) {
                return false;
            }
            if address == SINGLETON_RVA && output.len() == 8 {
                output.copy_from_slice(&0x1000_u64.to_le_bytes());
                true
            } else if address == PLAY_SESSION_STARTED_AT_RVA && output.len() == 8 {
                output.copy_from_slice(&19_i64.to_le_bytes());
                true
            } else {
                SliceMemory {
                    base: 0x1000,
                    bytes: &self.bytes,
                }
                .copy(address, output)
            }
        }
    }
    fn clock_memory(len: usize) -> ClockMemory {
        let mut bytes = vec![0; 0x2001 + len];
        bytes[CURRENT_HOUR..CURRENT_HOUR + 4].copy_from_slice(&42_i32.to_le_bytes());
        let payload = if len <= 15 { CAMPAIGN_ID } else { 0x2000 };
        bytes[payload..payload + len].fill(b'a');
        if len > 15 {
            bytes[CAMPAIGN_ID..CAMPAIGN_ID + 8].copy_from_slice(&0x3000_u64.to_le_bytes());
        }
        bytes[CAMPAIGN_ID + 16..CAMPAIGN_ID + 24].copy_from_slice(&(len as u64).to_le_bytes());
        bytes[CAMPAIGN_ID + 24..CAMPAIGN_ID + 32]
            .copy_from_slice(&(len.max(15) as u64).to_le_bytes());
        ClockMemory { bytes, fault: None }
    }

    #[test]
    fn campaign_clock_uses_native_stored_length_and_reads_fresh_prepared_bytes() {
        for len in [0, 15, 16, 127, 128, 129, 5_001] {
            let mut memory = clock_memory(len);
            let clock = read_clock(&memory, 0).unwrap();
            assert_eq!(clock.campaign_id.as_bytes(), vec![b'a'; len]);
            assert_eq!(campaign_id_len(&memory, 0), Ok(len));
            let mut prepared = Owned::default();
            prepared.prepare(len).unwrap();
            let before = prepared.allocated_bytes();
            assert_eq!(
                read_clock_prepared(&memory, 0, &mut prepared).unwrap(),
                clock.borrowed()
            );
            let payload = if len <= 15 { CAMPAIGN_ID } else { 0x2000 };
            if len != 0 {
                memory.bytes[payload + len - 1] = b'b';
                let fresh = read_clock_prepared(&memory, 0, &mut prepared).unwrap();
                assert_eq!(fresh.campaign_id.last(), Some(&b'b'));
            }
            assert_eq!(prepared.allocated_bytes(), before);
            memory.bytes[payload + len] = 1;
            assert!(read_clock(&memory, 0).is_none());
            assert!(read_clock_prepared(&memory, 0, &mut prepared).is_err());
        }
    }

    #[test]
    fn campaign_clock_preparation_shortage_and_payload_fault_stay_explicit() {
        let mut memory = clock_memory(5_001);
        let mut prepared = Owned::default();
        prepared.prepare(128).unwrap();
        assert_eq!(
            read_clock_prepared(&memory, 0, &mut prepared),
            Err("string exceeds prepared native length")
        );
        prepared
            .prepare(campaign_id_len(&memory, 0).unwrap())
            .unwrap();
        assert_eq!(
            read_clock_prepared(&memory, 0, &mut prepared)
                .unwrap()
                .campaign_id
                .len(),
            5_001
        );
        memory.fault = Some(0x3000 + 5_000);
        assert!(read_clock(&memory, 0).is_none());
        assert!(read_clock_prepared(&memory, 0, &mut prepared).is_err());
    }
}
