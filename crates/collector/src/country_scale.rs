//! Country-proportional capture sizing.
//!
//! Tables with one (or a fixed few) rows per country slot are sized from the
//! live country registry instead of a vanilla-sized default, so the first hour
//! of a campaign with tens of thousands of countries is not truncated. The
//! registry is read once before the hook is installed and again by the spool
//! thread while it idles, which sees a save load (the game rebuilds the
//! registry in `CGameState::reset`) before its first hourly tick. Allocation
//! stays off the game thread. Capacities only ever rise; the ordinary
//! truncation growth still covers anything these estimates miss.

use crate::{extended, map_capture::MapCapacities, sampling};
use engine::{Memory, cgame_state, country_memory::Cache};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Largest registry observed this attachment, for raw-copy jobs whose pools
/// live outside the frame. Only rises.
static SLOTS_HINT: AtomicUsize = AtomicUsize::new(0);

/// Record an observed registry size for job pools sized later.
pub fn note_slots(slots: usize) {
    SLOTS_HINT.fetch_max(slots, Ordering::Relaxed);
}

/// Tests that raise the registry hint, or measure buffers it sizes, hold
/// this so a parallel test cannot move it mid-measurement.
#[cfg(test)]
pub(crate) fn hint_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Country census rows a raw-copy job needs for the largest observed registry.
pub fn census_rows(minimum: usize) -> usize {
    rows(SLOTS_HINT.load(Ordering::Relaxed), 1).max(minimum)
}

/// Off the game thread: reserve a census vector for the observed registry.
/// Returns whether the allocation grew.
pub fn reserve_census<T>(census: &mut Vec<T>) -> bool {
    reserve_rows(census, census_rows(0))
}

fn reserve_rows<T>(census: &mut Vec<T>, target: usize) -> bool {
    if census.capacity() >= target {
        return false;
    }
    census.reserve_exact(target - census.len());
    true
}

/// The engine's own ceiling on canonical country slots. `CSupplySystem` stores
/// the controller and capital slots of every province as u16 (0x140eb753d,
/// 0x140eb7177) and the AI volunteer code indexes `CGameState+0x310` with a
/// u16 (0x141a64833), so a slot at or above 65,536 cannot work in the game.
/// A larger registry count is treated as a torn read of the sizing hint, not
/// as a limit on recorded data: tables still grow past it on demand.
pub const ENGINE_SLOT_CEILING: usize = 1 << 16;

/// Smallest country authentication cache; vanilla fits in it.
pub const MIN_COUNTRY_CELLS: usize = 8192;

/// Live country slots (the largest of the country, link and tag registries),
/// or `None` when no consistent registry is readable.
pub fn observe(memory: &impl Memory, base: usize) -> Option<usize> {
    let slots = cgame_state::read_country_registry_size(memory, base)?.slots();
    (slots <= ENGINE_SLOT_CEILING).then_some(slots)
}

/// Live dense CountryIntel record total, or `None` when no consistent
/// census is readable. Header words only; nothing observed is recorded.
pub fn observe_country_intel(memory: &impl Memory, base: usize) -> Option<usize> {
    // The same ceiling as `observe`: a registry past the engine's u16 slot
    // limit is a torn read, and no owner's vector is longer than the registry.
    let slots = observe(memory, base)?;
    let game_state = engine::read::<u64>(memory, base.checked_add(cgame_state::SINGLETON_RVA)?)?;
    let state = usize::try_from(game_state)
        .ok()
        .filter(|state| *state != 0)?;
    let rows = engine::ccountry_intel::native_rows(memory, base, state)?;
    Some(rows.min(slots.saturating_mul(slots)))
}

/// The raid arenas whose per-country demand content decides, not the slot
/// count: the raid type tokens of every raid parent and of the player cache,
/// and every country's convoy reservation words.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RaidArenas {
    pub type_tokens: usize,
    pub reservation_words: usize,
    pub facility_rows: usize,
    pub air_regions: usize,
    pub counteroperatives: usize,
}

/// Live raid arena demand, or `None` when no raid system or country census
/// is readable (at the main menu, say). Header words and the raid parents'
/// batched records only; nothing observed is recorded.
pub fn observe_raid_arenas(memory: &impl Memory, base: usize) -> Option<RaidArenas> {
    let game_state = engine::read::<u64>(memory, base.checked_add(cgame_state::SINGLETON_RVA)?)?;
    let state = usize::try_from(game_state)
        .ok()
        .filter(|state| *state != 0)?;
    let parents = engine::craid_system::required_type_tokens(memory, base, state).ok()?;
    let player = engine::cplayer_status_cache::required_tokens(memory, base, state).unwrap_or(0);
    let reservation_words =
        engine::raid_launch_inputs::required_reservations(memory, base, state).ok()?;
    // An empty destination counts native roster occurrences without reading
    // the programs or resolving active projects. This is only a sizing hint.
    let facility_rows = match engine::raid_facilities::walk(memory, base, state, &mut []) {
        engine::walk::Outcome::Rows { len } => len,
        engine::walk::Outcome::Truncated { unread_rows, .. } => unread_rows as usize,
        engine::walk::Outcome::Unavailable { .. } => 0,
    };
    let air_regions = state
        .checked_add(cgame_state::STRATEGIC_REGIONS)
        .and_then(|at| engine::cpdx_array::Array::read(memory, at, 8))
        .map_or(0, |array| array.count as usize);
    Some(RaidArenas {
        type_tokens: parents.checked_add(player)?,
        reservation_words,
        facility_rows,
        air_regions,
        counteroperatives: engine::raid_intel_inputs::required_operatives(memory, base, state)
            .unwrap_or(0),
    })
}

/// Live character and script flag demand: the Characters rows and text
/// bytes, and the flags of every ScriptFlags scope (global, countries, states
/// and characters).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharacterDemand {
    pub characters: engine::ccharacter::Demand,
    pub flags: usize,
}

/// Live character roster and flag demand, or `None` when either census is
/// unreadable (at the main menu, say). Header words, character views and
/// template headers only; nothing observed is recorded.
pub fn observe_characters(memory: &impl Memory, base: usize) -> Option<CharacterDemand> {
    let game_state = engine::read::<u64>(memory, base.checked_add(cgame_state::SINGLETON_RVA)?)?;
    let state = usize::try_from(game_state)
        .ok()
        .filter(|state| *state != 0)?;
    Some(CharacterDemand {
        characters: engine::ccharacter::demand(memory, base, state).ok()?,
        flags: extended::intel::script_flag_demand(memory, base, state)?,
    })
}

/// Live demand of the political and character tables whose per-country
/// sizes content and play decide: CountryCharacters' active-character
/// records, PartyLeaders' party leaders, CountryAppearance's leader trait
/// tokens, CountryCivilWars' entries, AdvisorTokens' keys and bytes, and
/// FactionAdvisors' occupied slots and token bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoliticsDemand {
    pub active_characters: usize,
    pub party_leaders: usize,
    pub leader_traits: usize,
    pub civil_wars: usize,
    pub advisor_tokens: usize,
    pub advisor_token_bytes: usize,
    pub faction_advisors: usize,
    pub faction_advisor_bytes: usize,
}

impl PoliticsDemand {
    /// Whether any field exceeds `seen`'s.
    pub fn exceeds(&self, seen: &Self) -> bool {
        self.active_characters > seen.active_characters
            || self.party_leaders > seen.party_leaders
            || self.leader_traits > seen.leader_traits
            || self.civil_wars > seen.civil_wars
            || self.advisor_tokens > seen.advisor_tokens
            || self.advisor_token_bytes > seen.advisor_token_bytes
            || self.faction_advisors > seen.faction_advisors
            || self.faction_advisor_bytes > seen.faction_advisor_bytes
    }

    /// The field-wise maximum.
    pub fn max(self, other: Self) -> Self {
        Self {
            active_characters: self.active_characters.max(other.active_characters),
            party_leaders: self.party_leaders.max(other.party_leaders),
            leader_traits: self.leader_traits.max(other.leader_traits),
            civil_wars: self.civil_wars.max(other.civil_wars),
            advisor_tokens: self.advisor_tokens.max(other.advisor_tokens),
            advisor_token_bytes: self.advisor_token_bytes.max(other.advisor_token_bytes),
            faction_advisors: self.faction_advisors.max(other.faction_advisors),
            faction_advisor_bytes: self.faction_advisor_bytes.max(other.faction_advisor_bytes),
        }
    }
}

/// Live political table demand, or `None` without a game state (at the main
/// menu, say). Each count is the table's own walk with no row storage, which
/// reads array headers (and, for text, string headers) only; a census that
/// is unreadable adds nothing, and its hourly walk reports it. Nothing
/// observed is recorded.
pub fn observe_politics(memory: &impl Memory, base: usize) -> Option<PoliticsDemand> {
    let game_state = engine::read::<u64>(memory, base.checked_add(cgame_state::SINGLETON_RVA)?)?;
    let state = usize::try_from(game_state)
        .ok()
        .filter(|state| *state != 0)?;
    let demand = |outcome: engine::walk::Outcome| match outcome {
        engine::walk::Outcome::Rows { len } => len,
        engine::walk::Outcome::Truncated { len, unread_rows } => {
            len.saturating_add(unread_rows as usize)
        }
        engine::walk::Outcome::Unavailable { .. } => 0,
    };
    let tokens = engine::advisor_tokens::walk(memory, base, state, &mut [], &mut []);
    let factions = engine::cfaction::walk_advisors(memory, base, state, &mut [], &mut []);
    Some(PoliticsDemand {
        active_characters: demand(engine::ccountry_characters::walk_active(
            memory,
            base,
            state,
            &mut [],
        )),
        party_leaders: engine::cpolitical_status::party_leader_demand(memory, base, state)
            .unwrap_or(0),
        leader_traits: engine::ccountry::leader_trait_demand(memory, base, state).unwrap_or(0),
        civil_wars: demand(engine::ccountry::walk_civil_wars(
            memory,
            base,
            state,
            &mut [],
        )),
        advisor_tokens: demand(tokens.outcome),
        advisor_token_bytes: tokens.required_bytes,
        faction_advisors: demand(factions.outcome),
        faction_advisor_bytes: factions.required_bytes,
    })
}

/// Rows for `per_country` rows per slot, with a third of headroom so the
/// three-quarters growth rule does not double a fresh buffer on its first hour.
pub fn rows(slots: usize, per_country: usize) -> usize {
    slots
        .saturating_mul(per_country)
        .saturating_mul(4)
        .div_ceil(3)
}

/// Raise `capacity` to at least `required`; never lowers it.
pub fn raise(capacity: &mut usize, required: usize) {
    *capacity = (*capacity).max(required);
}

/// Every capture capacity the frame pool is built from.
#[derive(Clone, Copy, PartialEq)]
pub struct Sizing {
    pub core: [usize; 12],
    pub map: MapCapacities,
    pub extended: extended::Capacities,
    pub country_cells: usize,
    /// DLC and mod list lengths for the once-only content roster sample.
    /// The launcher fixes them before a campaign exists, so one observation
    /// before hooking is enough.
    pub content: engine::cdlc_manager::Counts,
}

impl Sizing {
    pub fn for_slots(slots: usize) -> Self {
        let mut sizing = Self {
            core: sampling::INITIAL_CAPACITIES,
            map: MapCapacities::default(),
            extended: extended::Capacities::default(),
            country_cells: MIN_COUNTRY_CELLS,
            content: engine::cdlc_manager::Counts::default(),
        };
        sizing.raise_for_slots(slots);
        sizing
    }

    /// Raise every country-proportional capacity for `slots` country slots.
    pub fn raise_for_slots(&mut self, slots: usize) {
        sampling::raise_for_countries(&mut self.core, slots);
        self.map.raise_for_countries(slots);
        self.extended.raise_for_countries(slots);
        raise(
            &mut self.country_cells,
            Cache::cells_for_countries(slots, MIN_COUNTRY_CELLS),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The #63 attach hour: 440 countries whose raid type lists need more
    /// tokens than the default arena (it read 336 parents), and more
    /// convoy reservation words than the default 512. The observed demand
    /// raises both arenas, so the first hour's packs hold every row.
    #[test]
    fn observed_raid_demand_presizes_both_raid_arenas() {
        let demand = RaidArenas {
            type_tokens: 5_400,
            reservation_words: 1_900,
            facility_rows: 10_000,
            counteroperatives: 5000 * 11,
            air_regions: 700,
        };
        let mut sizing = Sizing::for_slots(440);
        let default_tokens = sizing.extended.projects.raid_type_tokens;
        assert!(default_tokens < demand.type_tokens);
        sizing.extended.raise_for_raid_arenas(demand);
        assert!(sizing.extended.projects.raid_type_tokens >= demand.type_tokens);
        assert!(sizing.extended.projects.rows[16] >= demand.facility_rows);
        assert!(sizing.extended.air.scores.regions >= demand.air_regions);
        assert!(
            sizing
                .extended
                .raid_launch_inputs
                .holds_reservations(demand.reservation_words)
        );
        // Raising never lowers a capacity.
        let before = sizing.extended;
        sizing.extended.raise_for_raid_arenas(RaidArenas::default());
        assert!(sizing.extended == before);
    }

    /// The political demand walks the live registry (here five thousand
    /// countries without a character object) and raising by it never lowers.
    #[test]
    fn observed_politics_demand_reads_the_registry_and_only_raises() {
        let demand = observe_politics(&world(), BASE).unwrap();
        assert_eq!(demand, PoliticsDemand::default());
        let mut sizing = Sizing::for_slots(COUNTRIES + 1);
        let before = sizing.extended;
        sizing.extended.raise_for_politics(demand);
        assert!(sizing.extended == before);
        sizing.extended.raise_for_politics(PoliticsDemand {
            active_characters: 1 << 20,
            ..Default::default()
        });
        assert!(sizing.extended != before);
        let before = sizing.extended;
        sizing.extended.raise_for_politics(PoliticsDemand {
            party_leaders: 1 << 20,
            ..Default::default()
        });
        assert!(sizing.extended != before);
        let before = sizing.extended;
        sizing.extended.raise_for_politics(PoliticsDemand {
            leader_traits: 1 << 20,
            ..Default::default()
        });
        assert!(sizing.extended != before);
        let before = sizing.extended;
        sizing.extended.raise_for_politics(PoliticsDemand {
            civil_wars: 1 << 20,
            ..Default::default()
        });
        assert!(sizing.extended != before);
        let seen = PoliticsDemand {
            active_characters: 5,
            party_leaders: 3,
            leader_traits: 2,
            civil_wars: 1,
            advisor_tokens: 4,
            advisor_token_bytes: 40,
            faction_advisors: 2,
            faction_advisor_bytes: 20,
        };
        let before = sizing.extended;
        sizing.extended.raise_for_politics(PoliticsDemand {
            faction_advisors: 1 << 16,
            faction_advisor_bytes: 1 << 20,
            ..Default::default()
        });
        assert!(sizing.extended != before);
        let before = sizing.extended;
        sizing.extended.raise_for_politics(PoliticsDemand {
            advisor_tokens: 1 << 16,
            advisor_token_bytes: 1 << 20,
            ..Default::default()
        });
        assert!(sizing.extended != before);
        assert!(!seen.exceeds(&seen) && seen.max(demand) == seen);
    }
    use crate::extended::{country_appearance, country_counters};
    use engine::{
        ccountry, ccountry_intel, cgame_state as gs, country_intel_row, msvc_string, walk::Outcome,
    };

    /// One country per state of a large modded map: the owner's design target.
    const COUNTRIES: usize = 5_000;
    const BASE: usize = 0x1_0000_0000;
    const STATE: usize = 0x10_0000;
    const TABLE: usize = 0x20_0000;
    const LINKS: usize = 0x30_0000;
    const TAGS: usize = 0x40_0000;
    const DATABASE: usize = 0x50_0000;
    const DIPLOMACY: usize = 0x60_0000;
    const OBJECTS: usize = 0x100_0000;
    const OBJECT: usize = 0x2000;
    const INTEL_TABLES: usize = 0x5000_0000;
    const INTEL_RECORDS: usize = 0x6000_0000;
    const BLOC_LINKS: usize = 0x7000_0000;
    // Country air objects, their region history tables, windows and records.
    const AIR_MANAGER: usize = 0x80_0000;
    const AIR_ROOTS: usize = 0x90_0000;
    const AIR_OBJECTS: usize = 0x8000_0000;
    const AIR_OBJECT: usize = 0x200;
    const AIR_TABLES: usize = 0x9000_0000;
    const HISTORIES: usize = 0xa000_0000;
    const WINDOWS: usize = 0xb000_0000;
    const RECORD_SLOTS: usize = 0xc000_0000;
    const RECORDS: usize = 0xf000_0000;
    const VALUES: usize = 0xf800_0000;
    /// Strategic region slots in every country's history table.
    const REGION_SLOTS: usize = 256;
    const HISTORY: usize = 0x40;
    const RECORD: usize = 0x10;
    const VALUE_FIELDS: usize = 0x300;

    /// Computes every byte on demand, so thousands of countries need no
    /// multi-megabyte fixture: slot `s` is object `OBJECTS + s * OBJECT` with
    /// native id `s`, every link id resolves to itself, and all other fields
    /// read as zero (with valid empty strings).
    struct World {
        slots: usize,
        /// Every country owns a dense intel vector over every slot with all
        /// four lanes nonzero: the worst case for the sparse table.
        intel: bool,
        /// Every country's shared diplomacy lists this many enemies and
        /// wartime allies: a world split into two warring blocs.
        bloc: Option<(u32, u32)>,
        air: Option<AirWorld>,
        /// This slot's country lives in a second object, past the others.
        relocated: Option<usize>,
    }

    /// One country slot in every `every` has a window-0 history of `ring`
    /// entries in each of `regions`, wrapped at `current`; every third entry
    /// is nonzero.
    struct AirWorld {
        every: usize,
        regions: Vec<u32>,
        ring: usize,
        current: u64,
    }

    impl AirWorld {
        /// The holder's offset in its block of `every` slots moves by one
        /// per block, so holders spread over every history part rather than
        /// following one stride.
        fn holds(&self, slot: usize) -> bool {
            slot % self.every == slot / self.every % self.every
        }

        fn has(&self, slot: usize, region: usize) -> bool {
            self.holds(slot) && self.regions.contains(&(region as u32))
        }
    }

    impl World {
        fn byte(&self, address: usize) -> u8 {
            let slots = self.slots as u64;
            let word = |at: usize, value: u64| (value >> ((address - at) * 8)) as u8;
            let header = |at: usize, pointer: usize| match address - at {
                0..8 => word(at, pointer as u64),
                8..12 => word(at + 8, slots),
                _ => word(at + 12, slots),
            };
            let singleton = BASE + gs::SINGLETON_RVA;
            if (singleton..singleton + 8).contains(&address) {
                return word(singleton, STATE as u64);
            }
            // Naming operands: no dynamic countries, one shared diplomacy.
            let database = BASE + engine::ccountry_database::SINGLETON_RVA;
            if (database..database + 8).contains(&address) {
                return word(database, DATABASE as u64);
            }
            let boundary = DATABASE + engine::ccountry_database::DYNAMIC_TAG_BOUNDARY;
            if (boundary..boundary + 4).contains(&address) {
                return word(boundary, slots);
            }
            if let Some(air) = &self.air
                && let Some(value) = self.air_byte(air, address)
            {
                return value;
            }
            if (DIPLOMACY..DIPLOMACY + 8).contains(&address) {
                return word(
                    DIPLOMACY,
                    (BASE + engine::cdiplomacy_status::VTABLE_RVA) as u64,
                );
            }
            if let Some((enemies, allies)) = self.bloc {
                for (field, count) in [
                    (engine::cdiplomacy_status::ENEMY_COUNTRIES, enemies),
                    (engine::cdiplomacy_status::WARTIME_ALLIES, allies),
                ] {
                    let at = DIPLOMACY + field;
                    if (at..at + 16).contains(&address) {
                        return match address - at {
                            0..8 => word(at, BLOC_LINKS as u64),
                            8..12 => word(at + 8, u64::from(count)),
                            _ => word(at + 12, u64::from(count)),
                        };
                    }
                }
                // Both lists name slots 1.. in order.
                if (BLOC_LINKS..BLOC_LINKS + self.slots * 4).contains(&address) {
                    let index = (address - BLOC_LINKS) / 4;
                    return word(BLOC_LINKS + index * 4, index as u64 + 1);
                }
            }
            for (field, pointer) in [
                (gs::COUNTRIES, TABLE),
                (gs::COUNTRY_LINKS, LINKS),
                (gs::COUNTRY_TAGS, TAGS),
            ] {
                if (STATE + field..STATE + field + 16).contains(&address) {
                    return header(STATE + field, pointer);
                }
            }
            if (TABLE + 8..TABLE + self.slots * 8).contains(&address) {
                let slot = (address - TABLE) / 8;
                let object = if self.relocated == Some(slot) {
                    self.slots
                } else {
                    slot
                };
                return word(TABLE + slot * 8, (OBJECTS + object * OBJECT) as u64);
            }
            if (LINKS + 4..LINKS + self.slots * 4).contains(&address) {
                let id = (address - LINKS) / 4;
                return word(LINKS + id * 4, id as u64);
            }
            let objects = self.slots + usize::from(self.relocated.is_some());
            if (OBJECTS + OBJECT..OBJECTS + objects * OBJECT).contains(&address) {
                let index = (address - OBJECTS) / OBJECT;
                let object = OBJECTS + index * OBJECT;
                let slot = match self.relocated {
                    Some(slot) if index == self.slots => slot,
                    _ => index,
                };
                let offset = address - object;
                let capacity = ccountry::COSMETIC_TAG + msvc_string::CAPACITY;
                let culture = ccountry::GRAPHICAL_CULTURE + msvc_string::CAPACITY;
                return match offset {
                    0..8 => word(object, (BASE + ccountry::VTABLE_RVA) as u64),
                    o if (ccountry::ID..ccountry::ID + 4).contains(&o) => {
                        word(object + ccountry::ID, slot as u64)
                    }
                    o if (capacity..capacity + 8).contains(&o) => {
                        word(object + capacity, msvc_string::INLINE_CAPACITY)
                    }
                    o if (culture..culture + 8).contains(&o) => {
                        word(object + culture, msvc_string::INLINE_CAPACITY)
                    }
                    o if (ccountry::DIPLOMACY_STATUS..ccountry::DIPLOMACY_STATUS + 8)
                        .contains(&o) =>
                    {
                        word(object + ccountry::DIPLOMACY_STATUS, DIPLOMACY as u64)
                    }
                    o if self.intel
                        && (ccountry::INTEL_TABLE..ccountry::INTEL_TABLE + 8).contains(&o) =>
                    {
                        word(
                            object + ccountry::INTEL_TABLE,
                            (INTEL_TABLES + slot * 0x20) as u64,
                        )
                    }
                    _ => 0,
                };
            }
            if self.intel && (INTEL_TABLES..INTEL_TABLES + self.slots * 0x20).contains(&address) {
                let table = INTEL_TABLES + (address - INTEL_TABLES) / 0x20 * 0x20;
                return match address - table {
                    0..8 => word(table, (BASE + ccountry_intel::VTABLE_RVA) as u64),
                    0x10..0x18 => word(table + 0x10, INTEL_RECORDS as u64),
                    0x18..0x1c => word(table + 0x18, slots),
                    0x1c..0x20 => word(table + 0x1c, slots),
                    _ => 0,
                };
            }
            if self.intel && self.in_records(address) {
                return 1;
            }
            0
        }

        fn air_byte(&self, air: &AirWorld, address: usize) -> Option<u8> {
            let word = |at: usize, value: u64| (value >> ((address - at) * 8)) as u8;
            let header = |at: usize, data: usize, count: usize| match address - at {
                0..8 => word(at, data as u64),
                8..12 => word(at + 8, count as u64),
                _ => word(at + 12, count as u64),
            };
            let (slots, ring) = (self.slots, air.ring);
            let pair = |at: usize, base: usize, stride: usize| {
                let index = (at - base) / stride;
                (
                    index / REGION_SLOTS,
                    index % REGION_SLOTS,
                    base + index * stride,
                )
            };
            Some(match address {
                a if (STATE + gs::STRATEGIC_AIR_MANAGER..STATE + gs::STRATEGIC_AIR_MANAGER + 8)
                    .contains(&a) =>
                {
                    word(STATE + gs::STRATEGIC_AIR_MANAGER, AIR_MANAGER as u64)
                }
                a if (AIR_MANAGER..AIR_MANAGER + 8).contains(&a) => word(
                    AIR_MANAGER,
                    (BASE + engine::cstrategic_air_manager::VTABLE_RVA) as u64,
                ),
                a if (AIR_MANAGER + 0x30..AIR_MANAGER + 0x40).contains(&a) => {
                    match a - AIR_MANAGER - 0x30 {
                        0..8 => word(AIR_MANAGER + 0x30, AIR_ROOTS as u64),
                        8..12 => word(AIR_MANAGER + 0x38, slots as u64),
                        _ => word(AIR_MANAGER + 0x3c, slots as u64),
                    }
                }
                a if (AIR_ROOTS + 8..AIR_ROOTS + slots * 8).contains(&a) => {
                    let slot = (a - AIR_ROOTS) / 8;
                    word(
                        AIR_ROOTS + slot * 8,
                        (AIR_OBJECTS + slot * AIR_OBJECT) as u64,
                    )
                }
                a if (AIR_OBJECTS + AIR_OBJECT..AIR_OBJECTS + slots * AIR_OBJECT).contains(&a) => {
                    let slot = (a - AIR_OBJECTS) / AIR_OBJECT;
                    let object = AIR_OBJECTS + slot * AIR_OBJECT;
                    let table = engine::cstrategic_air::HISTORY_TABLE;
                    match a - object {
                        0..8 => word(object, (BASE + engine::cstrategic_air::VTABLE_RVA) as u64),
                        o if (0x90..0x94).contains(&o) => word(object + 0x90, slot as u64),
                        o if (table..table + 8).contains(&o) => word(
                            object + table,
                            (AIR_TABLES + slot * REGION_SLOTS * 8) as u64,
                        ),
                        o if (table + 8..table + 16).contains(&o) => {
                            let at = object + table + 8;
                            match a - at {
                                0..4 => word(at, REGION_SLOTS as u64),
                                _ => word(at + 4, REGION_SLOTS as u64),
                            }
                        }
                        _ => 0,
                    }
                }
                a if (AIR_TABLES..AIR_TABLES + slots * REGION_SLOTS * 8).contains(&a) => {
                    let (slot, region, at) = pair(a, AIR_TABLES, 8);
                    let history = HISTORIES + (slot * REGION_SLOTS + region) * HISTORY;
                    word(
                        at,
                        if air.has(slot, region) {
                            history as u64
                        } else {
                            0
                        },
                    )
                }
                a if (HISTORIES..HISTORIES + slots * REGION_SLOTS * HISTORY).contains(&a) => {
                    let (slot, region, at) = pair(a, HISTORIES, HISTORY);
                    let window = WINDOWS + (slot * REGION_SLOTS + region) * HISTORY;
                    match a - at {
                        0..8 => word(at, (BASE + engine::cloop_history::VTABLE_RVA) as u64),
                        0x10..0x18 => word(at + 0x10, window as u64),
                        0x34..0x38 => word(
                            at + 0x34,
                            u64::from(engine::cloop_history::air_stats::FIELD_COUNT),
                        ),
                        _ => 0,
                    }
                }
                a if (WINDOWS..WINDOWS + slots * REGION_SLOTS * HISTORY).contains(&a) => {
                    let (slot, region, at) = pair(a, WINDOWS, HISTORY);
                    let records = RECORD_SLOTS + (slot * REGION_SLOTS + region) * ring * 8;
                    match a - at {
                        0..8 => word(at, (BASE + engine::loop_history_window::VTABLE_RVA) as u64),
                        8..24 => header(at + 8, records, ring),
                        0x20..0x24 => word(at + 0x20, ring as u64),
                        0x24..0x28 => word(at + 0x24, air.current),
                        0x28 => 1,
                        _ => 0,
                    }
                }
                a if (RECORD_SLOTS..RECORD_SLOTS + slots * REGION_SLOTS * ring * 8)
                    .contains(&a) =>
                {
                    let entry = (a - RECORD_SLOTS) / 8;
                    word(
                        RECORD_SLOTS + entry * 8,
                        (RECORDS + entry % ring * RECORD) as u64,
                    )
                }
                a if (RECORDS..RECORDS + ring * RECORD).contains(&a) => {
                    let entry = (a - RECORDS) / RECORD;
                    let at = RECORDS + entry * RECORD;
                    match a - at {
                        0..8 => word(at, (VALUES + entry * VALUE_FIELDS) as u64),
                        _ => 0,
                    }
                }
                a if (VALUES..VALUES + ring * VALUE_FIELDS).contains(&a) => {
                    let entry = (a - VALUES) / VALUE_FIELDS;
                    let at = VALUES + entry * VALUE_FIELDS;
                    let channel = (a - at) / 8;
                    let value = if entry.is_multiple_of(3) && [3, 43].contains(&channel) {
                        100_000 * (entry as u64 + 1)
                    } else {
                        0
                    };
                    word(at + channel * 8, value)
                }
                _ => return None,
            })
        }

        fn in_records(&self, address: usize) -> bool {
            (INTEL_RECORDS..INTEL_RECORDS + self.slots * country_intel_row::STRIDE)
                .contains(&address)
        }
    }

    impl Memory for World {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            // Record batches are the bulk of an intel walk; fill them directly.
            if self.in_records(address) && self.in_records(address + output.len() - 1) {
                output.fill(u8::from(self.intel));
                return true;
            }
            // So are the bloc lists' entries in a bloc-war walk.
            if self.bloc.is_some()
                && (BLOC_LINKS..BLOC_LINKS + self.slots * 4).contains(&address)
                && address + output.len() <= BLOC_LINKS + self.slots * 4
                && (address - BLOC_LINKS).is_multiple_of(4)
                && output.len().is_multiple_of(4)
            {
                let first = (address - BLOC_LINKS) / 4;
                for (index, entry) in output.chunks_exact_mut(4).enumerate() {
                    entry.copy_from_slice(&(first as u32 + index as u32 + 1).to_le_bytes());
                }
                return true;
            }
            for (index, byte) in output.iter_mut().enumerate() {
                *byte = self.byte(address + index);
            }
            true
        }
    }

    fn world() -> World {
        // Slot zero is the engine's reserved placeholder.
        World {
            slots: COUNTRIES + 1,
            intel: false,
            bloc: None,
            air: None,
            relocated: None,
        }
    }

    /// One country per state of the vanilla map, every one alive and every
    /// intel record nonzero: the first hour's sparse CountryIntel must fit.
    #[test]
    fn country_intel_sizing_holds_the_first_hour_of_fifteen_hundred_live_countries() {
        const LIVE: usize = 1_500;
        let memory = World {
            slots: LIVE + 1,
            intel: true,
            bloc: None,
            air: None,
            relocated: None,
        };
        let native = observe_country_intel(&memory, BASE).unwrap();
        assert_eq!(native, LIVE * (LIVE + 1));
        let mut sizing = Sizing::for_slots(observe(&memory, BASE).unwrap());
        let vanilla = sizing.extended.intel.rows[15];
        sizing.extended.raise_for_country_intel(native);
        let capacity = sizing.extended.intel.rows[15];
        assert!(capacity > vanilla);
        let mut rows = vec![engine::country_intel_row::Row::default(); capacity];
        let mut scratch = vec![0_u8; 16_384];
        assert_eq!(
            ccountry_intel::walk(&memory, BASE, STATE, &mut rows, &mut scratch),
            Outcome::Rows { len: native }
        );
        // The fixed default truncated this hour.
        let mut small = vec![engine::country_intel_row::Row::default(); vanilla];
        assert!(matches!(
            ccountry_intel::walk(&memory, BASE, STATE, &mut small, &mut scratch),
            Outcome::Truncated { .. }
        ));
        // Presized with growth headroom: at the observed fill (about 0.9 of
        // the dense total among live countries) the three-quarters rule does
        // not double this buffer after its first full hour.
        assert!(native * 9 / 10 < capacity - capacity / 4);
    }

    #[test]
    fn registry_sizing_captures_the_first_hour_of_five_thousand_countries() {
        let memory = world();
        let slots = observe(&memory, BASE).expect("synthetic registry is consistent");
        assert_eq!(slots, COUNTRIES + 1);
        let sizing = Sizing::for_slots(slots);
        let vanilla = Sizing::for_slots(0);

        // Core tables: the link census fits on the first hour; the old fixed
        // capacity truncated it.
        let mut frame =
            sampling::Captured::with_capacities(&sizing.core, &sizing.map, &sizing.extended);
        assert!(frame.countries.len() >= COUNTRIES);
        assert_eq!(
            ccountry::walk_links(&memory, BASE, STATE, &mut frame.links),
            Outcome::Rows { len: COUNTRIES }
        );
        let mut fixed = vec![ccountry::LinkRow::default(); vanilla.core[2]];
        assert!(matches!(
            ccountry::walk_links(&memory, BASE, STATE, &mut fixed),
            Outcome::Truncated { .. }
        ));

        // Per-country packs publish complete rows on the first capture.
        let mut appearance =
            country_appearance::Captured::with_capacities(&sizing.extended.country_appearance);
        appearance.capture(&memory, BASE, STATE);
        assert_eq!(appearance.outcomes()[0], Outcome::Rows { len: COUNTRIES });
        let mut counters =
            country_counters::Captured::with_capacities(&sizing.extended.country_counters);
        counters.capture(&memory, BASE, STATE);
        assert_eq!(counters.outcomes()[0], Outcome::Rows { len: COUNTRIES });
        let mut small =
            country_appearance::Captured::with_capacities(&vanilla.extended.country_appearance);
        small.capture(&memory, BASE, STATE);
        assert!(matches!(small.outcomes()[0], Outcome::Truncated { .. }));

        // The country authentication cache holds the whole census.
        let mut cache = std::sync::Arc::new(Cache::with_capacity(sizing.country_cells));
        let snapshot = Cache::prepare_shared(&mut cache, &memory, BASE, STATE).unwrap();
        assert_eq!(
            snapshot.view(&memory).seed_status(),
            engine::country_memory::SeedStatus::Ready
        );
    }

    /// Notifications a test controls, lock-free like the hook registry: one
    /// epoch, a revision the scenario bumps, and per-country sequences (the
    /// shared diplomacy status never changes).
    struct Notified {
        revision: std::sync::atomic::AtomicU64,
        countries: Vec<std::sync::atomic::AtomicU64>,
        /// Registrations the watch table refuses: 0 none, a slot + 1, or
        /// [`REFUSE_ALL`] for a table held for the next full hour.
        refused: std::sync::atomic::AtomicUsize,
    }

    const REFUSE_ALL: usize = usize::MAX;

    impl crate::relation_changes::Changes for Notified {
        fn quiet(&self) -> Option<(u64, u64)> {
            Some((1, self.revision.load(Ordering::Relaxed)))
        }
        fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
            if receiver == DIPLOMACY {
                return Some((1, 0));
            }
            let slot = receiver.checked_sub(OBJECTS)? / OBJECT;
            let refused = self.refused.load(Ordering::Relaxed);
            if refused == REFUSE_ALL || refused == slot + 1 {
                return None;
            }
            let sequence = self.countries.get(slot)?.load(Ordering::Relaxed);
            (receiver != 0).then_some((1, sequence))
        }
    }

    impl Notified {
        fn new(slots: usize) -> Self {
            Self {
                revision: Default::default(),
                countries: (0..slots).map(|_| Default::default()).collect(),
                refused: Default::default(),
            }
        }

        /// A writer touched these countries.
        fn touch(&self, slots: impl IntoIterator<Item = usize>) {
            self.revision.fetch_add(1, Ordering::Relaxed);
            for slot in slots {
                self.countries[slot].fetch_add(2, Ordering::Relaxed);
            }
        }
    }

    /// One relations hour: the parts' guarded calls and wall time (the
    /// game-thread stall with a free worker per part), the countries copied,
    /// then the decode thread's rows and time.
    struct RelationsHour {
        calls: usize,
        wall: std::time::Duration,
        copied: usize,
        held: usize,
        outcome: Outcome,
        decode: std::time::Duration,
    }

    /// Counts guarded calls from every part's thread.
    struct Counting<'a>(&'a World, std::sync::atomic::AtomicUsize);

    impl Memory for Counting<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.1.fetch_add(1, Ordering::Relaxed);
            self.0.copy(address, output)
        }
        fn copy_records(
            &self,
            sources: engine::memory::RecordSources<'_>,
            layout: &engine::memory::RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.1.fetch_add(1, Ordering::Relaxed);
            self.0.copy_records(sources, layout, destination)
        }
        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.1.fetch_add(1, Ordering::Relaxed);
            for (done, (address, destination)) in addresses
                .into_iter()
                .zip(destinations.iter_mut())
                .enumerate()
            {
                if !destination.is_empty() && !self.0.copy(address, destination) {
                    return done;
                }
            }
            N
        }
    }

    fn relations_hour(
        world: &World,
        changes: &Notified,
        hour: &mut crate::extended::air_relations::hour::Hour,
        at: i32,
        threads: bool,
    ) -> RelationsHour {
        let memory = Counting(world, Default::default());
        let started = std::time::Instant::now();
        let wall = hour.run(&memory, BASE, STATE, changes, at, threads);
        let decode = started.elapsed() - wall;
        RelationsHour {
            calls: memory.1.load(Ordering::Relaxed),
            wall,
            copied: hour.staged_countries(),
            held: hour.held.rows(),
            outcome: hour.emitted(),
            decode,
        }
    }

    /// `emitted` is exactly the rows of `slots` in a bloc world: each
    /// country's wartime allies, then its enemies, both lists naming slots
    /// 1.. in order.
    fn assert_bloc_rows(
        emitted: &[engine::air_participant_vectors::Row],
        slots: &[u32],
        (enemies, allies): (u32, u32),
    ) {
        use engine::air_participant_vectors::{ALLY, ENEMY, Row};
        let expected = slots.iter().flat_map(|&country| {
            [(ALLY, allies), (ENEMY, enemies)]
                .into_iter()
                .flat_map(move |(kind, count)| {
                    (0..count).map(move |ordinal| Row {
                        country,
                        kind,
                        ordinal,
                        other: ordinal + 1,
                    })
                })
        });
        let mut len = 0;
        for (index, row) in expected.enumerate() {
            assert_eq!(emitted.get(index), Some(&row), "row {index}");
            len += 1;
        }
        assert_eq!(emitted.len(), len);
    }

    /// Two blocs forming then going to war: the first hour (every country
    /// with its bloc's allies), the declaration touching every country (its
    /// copies outgrow the registry-sized staging, which grows in the hour),
    /// a quiet hour at war, and one more declaration touching two countries,
    /// whose vectors stay the same. Each hour runs its parts on their own
    /// threads and, on a second domain, one after another; both copy the
    /// same countries, hold every country's rows and emit exactly the first
    /// two hours' changed countries and the later hours' slices.
    fn bloc_war_relations(countries: usize) {
        use crate::extended::air_relations::{PARTS, hour::Hour};
        let enemies = countries as u32 / 2;
        let mut world = World {
            slots: countries + 1,
            bloc: Some((0, enemies - 1)),
            ..world()
        };
        let changes = Notified::new(countries + 1);
        let rows = crate::country_scale::rows(countries + 1, 8);
        let mut hour = Hour::new(rows, rows * 4 / PARTS);
        let mut serial = Hour::new(rows, rows * 4 / PARTS);
        let serial_changes = Notified::new(countries + 1);
        // Spawning the part threads, which pool jobs do not pay.
        let started = std::time::Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..PARTS {
                scope.spawn(|| ());
            }
        });
        let spawn = started.elapsed();
        // Every country is copied on the first hour and at the declaration;
        // the quiet hour copies nothing and the next copies the two touched.
        // The worst held-table case: a table held for the next full hour
        // refuses every receiver a writer touched (its old registrations
        // were dropped by the epoch before). Each hour then copies every
        // country afresh, quiet or not, and the rows stay exact.
        for (at, (label, touched, copied)) in [
            ("first hour", vec![], countries),
            ("bloc declaration", (1..=countries).collect(), countries),
            ("quiet hour", vec![], 0),
            ("single declaration", vec![1, 2], 2),
            ("held table", (1..=countries).collect(), countries),
            ("held table, quiet again", vec![], countries),
        ]
        .into_iter()
        .enumerate()
        {
            if label == "bloc declaration" {
                world.bloc = Some((enemies, enemies - 1));
            }
            if label == "held table" {
                changes.refused.store(REFUSE_ALL, Ordering::Relaxed);
                serial_changes.refused.store(REFUSE_ALL, Ordering::Relaxed);
            }
            if !touched.is_empty() {
                changes.touch(touched.iter().copied());
                serial_changes.touch(touched);
            }
            let result = relations_hour(&world, &changes, &mut hour, at as i32, true);
            let one = relations_hour(&world, &serial_changes, &mut serial, at as i32, false);
            let emitted = match result.outcome {
                Outcome::Rows { len } => len.to_string(),
                other => format!("{other:?}"),
            };
            eprintln!(
                "relations, {countries} countries, {label}: {} countries copied in {} guarded calls, {:?} wall over {PARTS} parts ({spawn:?} of it spawning), {:?} on one thread; decode {:?}, {emitted} of {} rows emitted",
                result.copied, result.calls, result.wall, one.wall, result.decode, result.held
            );
            // The first hour is a full hour and the declaration changes every
            // country; later hours emit their slice only.
            let slice: Vec<u32> = (1..=countries as u32)
                .filter(|&slot| {
                    at < 2 || slot % replay::cadence::STAGGER_PERIOD as u32 == at as u32
                })
                .collect();
            let bloc = world.bloc.unwrap();
            let per_country = (bloc.0 + bloc.1) as usize;
            for (run, pack) in [(&result, &hour), (&one, &serial)] {
                assert_eq!(run.copied, copied, "{label}");
                assert_eq!(run.held, countries * per_country, "{label}");
                assert_eq!(
                    run.outcome,
                    Outcome::Rows {
                        len: slice.len() * per_country
                    },
                    "{label}"
                );
                assert_bloc_rows(pack.emitted_rows(), &slice, bloc);
            }
            hour.grow();
            serial.grow();
        }
    }

    #[test]
    fn relations_are_copied_only_for_touched_countries_and_emitted_by_slice() {
        use crate::extended::air_relations::hour::Hour;
        let mut world = World {
            slots: 49,
            bloc: Some((24, 23)),
            ..world()
        };
        let changes = Notified::new(49);
        let mut hour = Hour::new(4096, 4096);
        let first = relations_hour(&world, &changes, &mut hour, 0, true);
        assert_eq!(first.copied, 48);
        assert_eq!(first.outcome, Outcome::Rows { len: 48 * 47 });
        // A quiet hour copies nothing and emits one slice: slots 1 and 25.
        let quiet = relations_hour(&world, &changes, &mut hour, 1, true);
        // Only the slot checks read: this world has no roster cache, so each
        // country lookup costs reads that production serves from the cache.
        // Verify mode's fresh copies are reads too.
        if !cfg!(feature = "relation_verify") {
            assert!(quiet.calls <= 8 * 48, "{}", quiet.calls);
        }
        assert_eq!(quiet.copied, 0);
        assert_eq!(quiet.outcome, Outcome::Rows { len: 2 * 47 });
        // A touched country whose vectors did not change is copied, not emitted.
        changes.touch([3]);
        let unchanged = relations_hour(&world, &changes, &mut hour, 2, true);
        assert_eq!(unchanged.copied, 1);
        assert_eq!(unchanged.outcome, Outcome::Rows { len: 2 * 47 });
        // A changed one is emitted at once, outside its slice.
        world.bloc = Some((24, 22));
        changes.touch([5]);
        let changed = relations_hour(&world, &changes, &mut hour, 3, true);
        assert_eq!(changed.copied, 1);
        assert_eq!(changed.outcome, Outcome::Rows { len: 2 * 47 + 46 });
        // A country object replaced at its slot is copied afresh, although
        // no writer touched it.
        world.relocated = Some(7);
        changes.touch([3]);
        let replaced = relations_hour(&world, &changes, &mut hour, 4, true);
        assert_eq!(replaced.copied, 2);
        // A staging buffer sized for the steady state grows; the hour stands.
        world.bloc = Some((24, 23));
        let mut small = Hour::new(4096, 64);
        let grown = relations_hour(&world, &Notified::new(49), &mut small, 0, false);
        assert_eq!(grown.outcome, Outcome::Rows { len: 48 * 47 });
    }

    /// A receiver the watch table refused is read afresh every hour, also
    /// when no writer moved the domain revision; the others are reused.
    #[test]
    fn a_refused_receiver_is_copied_every_hour() {
        use crate::extended::air_relations::hour::Hour;
        let world = World {
            slots: 49,
            bloc: Some((24, 23)),
            ..world()
        };
        let changes = Notified::new(49);
        changes.refused.store(3 + 1, Ordering::Relaxed);
        let mut hour = Hour::new(4096, 4096);
        assert_eq!(
            relations_hour(&world, &changes, &mut hour, 0, true).copied,
            48
        );
        for at in 1..3 {
            assert_eq!(
                relations_hour(&world, &changes, &mut hour, at, true).copied,
                1
            );
        }
        changes.touch([5]);
        assert_eq!(
            relations_hour(&world, &changes, &mut hour, 3, true).copied,
            2
        );
    }

    /// `forget_unpublished_families` skips this family, whose fingerprints
    /// live in the decode step, so an hour the spool withheld is recovered
    /// only by the full hour the spool then forces (the hours captured in
    /// between have no complete rows to merge into and are withheld too).
    /// The withheld hour's changes are already in the fingerprints, so a
    /// later non-full hour does not emit them again; the full hour emits
    /// every held country, exactly the rows a fresh read of the same world
    /// emits.
    #[test]
    fn a_forced_full_hour_re_emits_an_unpublished_hours_changes_exactly() {
        use crate::extended::air_relations::hour::Hour;
        let mut world = World {
            slots: 49,
            bloc: Some((24, 23)),
            ..world()
        };
        let changes = Notified::new(49);
        let mut hour = Hour::new(4096, 4096);
        relations_hour(&world, &changes, &mut hour, 0, true);
        // Every country changes at hour 1, whose frame the spool withholds.
        world.bloc = Some((24, 22));
        changes.touch(1..=48);
        let withheld = relations_hour(&world, &changes, &mut hour, 1, true);
        assert_eq!(withheld.outcome, Outcome::Rows { len: 48 * 46 });
        // Hour 2 was captured before the recovery: only its slice, slots 2
        // and 26.
        let between = relations_hour(&world, &changes, &mut hour, 2, true);
        assert_eq!(between.outcome, Outcome::Rows { len: 2 * 46 });
        // The recovery hour copies nothing new yet emits every held row.
        hour.force_full = true;
        let recovered = relations_hour(&world, &changes, &mut hour, 3, true);
        assert_eq!(recovered.copied, 0);
        assert_eq!(recovered.outcome, Outcome::Rows { len: 48 * 46 });
        let mut fresh = Hour::new(4096, 4096);
        relations_hour(&world, &Notified::new(49), &mut fresh, 0, true);
        assert_eq!(hour.emitted_rows(), fresh.emitted_rows());
    }

    #[test]
    fn a_relation_vector_longer_than_the_country_links_fails_the_hour() {
        use crate::extended::air_relations::hour::Hour;
        let world = World {
            slots: 49,
            bloc: Some((0, 50)),
            ..world()
        };
        let mut hour = Hour::new(4096, 4096);
        let result = relations_hour(&world, &Notified::new(49), &mut hour, 0, false);
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "air participant vector longer than the country links"
            }
        );
    }

    /// The bloc-war hours of the scale matrix, at a size the ordinary test
    /// run affords.
    #[test]
    fn bloc_war_relations_at_200_countries() {
        bloc_war_relations(200);
    }

    #[cfg(feature = "relation_verify")]
    #[test]
    fn verify_counts_a_change_no_writer_announced() {
        use crate::extended::air_relations::hour::Hour;
        let mut world = World {
            slots: 49,
            bloc: Some((24, 23)),
            ..world()
        };
        let changes = Notified::new(49);
        let mut hour = Hour::new(4096, 4096);
        relations_hour(&world, &changes, &mut hour, 0, true);
        let first = hour.verification();
        assert_eq!((first.countries, first.mismatches), (48, 0));
        // An announced change is copied, and the held rows stay exact.
        world.bloc = Some((24, 22));
        changes.touch(1..=48);
        relations_hour(&world, &changes, &mut hour, 1, true);
        assert_eq!(hour.verification().mismatches, 0);
        // An unannounced one keeps stale rows, which the fresh copy exposes.
        world.bloc = Some((24, 21));
        relations_hour(&world, &changes, &mut hour, 2, true);
        let stale = hour.verification();
        assert_eq!((stale.countries, stale.mismatches), (48, 48));
    }

    /// The scale matrix: `cargo test -p collector --release -- --ignored`.
    #[test]
    #[ignore]
    fn bloc_war_relations_at_1500_and_5000_countries() {
        bloc_war_relations(1_500);
        bloc_war_relations(5_000);
    }

    /// Every writer call as the hooks make it, on a watch domain of its own.
    fn touch(
        watches: &crate::history_changes::Watches,
        site: usize,
        receivers: impl IntoIterator<Item = usize>,
        calls: u64,
    ) {
        for receiver in receivers {
            for _ in 0..calls {
                drop(watches.notify(receiver, site));
            }
        }
    }

    /// The real watch domain, less receivers its table could not hold.
    struct Blinded<'a> {
        watches: &'a crate::history_changes::Watches,
        blind: std::collections::BTreeSet<usize>,
    }

    impl crate::history_changes::Changes for Blinded<'_> {
        fn quiet(&self) -> Option<(u64, u64)> {
            self.watches.quiet()
        }
        fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
            (!self.blind.contains(&receiver))
                .then(|| self.watches.sequence(receiver))
                .flatten()
        }
        fn site(&self, index: usize) -> Option<(u64, u64)> {
            self.watches.site(index)
        }
    }

    /// One naval-patrol history hour: the parts' guarded calls and wall
    /// time, the countries copied, the rows held, the table and the decode
    /// thread's time.
    struct HistoriesHour {
        calls: usize,
        wall: std::time::Duration,
        held: usize,
        outcome: Outcome,
        decode: std::time::Duration,
    }

    #[allow(clippy::too_many_arguments)]
    fn histories_hour(
        world: &World,
        regions: &[u64],
        changes: &(impl crate::history_changes::Changes + Sync),
        hour: &mut crate::extended::air_histories::hour::Hour,
        frame: &mut crate::extended::air_histories::Captured,
        at: i32,
        threads: bool,
    ) -> HistoriesHour {
        // The parts read through the callback's roster, as in production.
        let mut cache = std::sync::Arc::new(Cache::with_capacity(Cache::cells_for_countries(
            world.slots,
            MIN_COUNTRY_CELLS,
        )));
        let snapshot = Cache::prepare_shared(&mut cache, world, BASE, STATE).unwrap();
        let counting = Counting(world, Default::default());
        let memory = snapshot.roster_view(&counting);
        let (wall, decode, outcome) =
            hour.run(frame, &memory, BASE, STATE, regions, changes, at, threads);
        HistoriesHour {
            calls: counting.1.load(Ordering::Relaxed),
            wall,
            held: hour.held_rows(),
            outcome,
            decode,
        }
    }

    fn history(slot: usize, region: u32) -> usize {
        HISTORIES + (slot * REGION_SLOTS + region as usize) * HISTORY
    }

    /// Every country slot divisible by `every` has a 30-record window in each
    /// of `patrolled` regions, a third of the records nonzero: the first
    /// hour, a quiet hour, one country's combat, the daily advance of every
    /// window, and one more region patrolled. Each hour runs its parts on
    /// their own threads and, on a second domain, one after another. Buffers
    /// are presized from the registry and grown on each recycle, so once the
    /// parts' certificates took the capacities the first hour raised, the
    /// combat and advance hours stage without growing one.
    fn naval_patrol_histories(countries: usize, every: usize, patrolled: u32) -> Vec<usize> {
        use crate::extended::air_histories::{
            PARTS,
            hour::{self, Hour},
        };
        use crate::history_changes::Watches;
        let mut regions: Vec<u64> = (0..patrolled)
            .map(|region| u64::from(3 + region * 7))
            .collect();
        let mut world = World {
            slots: countries + 1,
            air: Some(AirWorld {
                every,
                // One more region holds histories nobody patrols yet.
                regions: (0..=patrolled).map(|region| 3 + region * 7).collect(),
                ring: 30,
                current: 3,
            }),
            ..world()
        };
        let air = world.air.as_ref().unwrap();
        let held: Vec<usize> = (1..=countries).filter(|&slot| air.holds(slot)).collect();
        // Presized for every region of the content, as the runtime does.
        let (watches, serial_watches) = (
            Watches::enabled(countries + 1, air.regions.len()),
            Watches::enabled(countries + 1, air.regions.len()),
        );
        let mut capacity = hour::capacities(countries + 1);
        let mut serial_capacity = capacity;
        let (mut frame, mut serial_frame) = (hour::frame(&capacity), hour::frame(&capacity));
        let (mut hour, mut serial) = (Hour::new(), Hour::new());
        // Spawning the part threads, which pool jobs do not pay.
        let started = std::time::Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..PARTS {
                scope.spawn(|| ());
            }
        });
        let spawn = started.elapsed();
        let mut calls = Vec::new();
        for (at, label) in [
            "first hour",
            "quiet hour",
            "one country's combat",
            "daily advance",
            "one more region",
        ]
        .into_iter()
        .enumerate()
        {
            match label {
                "one country's combat" => {
                    let air = [AIR_OBJECTS + held[0] * AIR_OBJECT];
                    touch(&watches, 0, air, 3);
                    touch(&serial_watches, 0, air, 3);
                }
                "daily advance" => {
                    let air = world.air.as_mut().unwrap();
                    air.current = 4;
                    let histories: Vec<_> = held
                        .iter()
                        .flat_map(|&slot| {
                            air.regions.iter().map(move |&region| history(slot, region))
                        })
                        .collect();
                    touch(&watches, 1, histories.iter().copied(), 1);
                    touch(&serial_watches, 1, histories, 1);
                }
                "one more region" => regions.push(u64::from(3 + patrolled * 7)),
                _ => {}
            }
            let allocated = hour.allocated(&frame);
            let result = histories_hour(
                &world, &regions, &watches, &mut hour, &mut frame, at as i32, true,
            );
            let one = histories_hour(
                &world,
                &regions,
                &serial_watches,
                &mut serial,
                &mut serial_frame,
                at as i32,
                false,
            );
            assert_eq!(result.held, held.len() * regions.len() * 10);
            assert_eq!(one.held, result.held);
            if (2..=3).contains(&at) {
                assert_eq!(hour.allocated(&frame), allocated, "{label} grew a buffer");
            }
            let counts = Hour::counts(&frame);
            let emitted = match result.outcome {
                Outcome::Rows { len } => len.to_string(),
                other => format!("{other:?}"),
            };
            let refused = watches.status()["refused_registrations"].clone();
            eprintln!(
                "naval patrol, {countries} countries, {patrolled} regions, 1/{every} with history, {label}: {} guarded calls; parts wall {:?} on {PARTS} threads ({spawn:?} of it spawning); serial {:?}, pool of 3 {:?}, pool of 8 {:?}; decode {:?}; {emitted} of {} rows emitted; {} read, {} advanced, {} refreshed; {refused} refused watches",
                result.calls,
                result.wall,
                one.wall,
                one.wall / 3,
                one.wall / PARTS as u32,
                result.decode,
                result.held,
                counts.read,
                counts.advanced,
                counts.refreshed,
            );
            // The spool's recycle: grow the frames and offer the watches.
            watches.grow(Hour::grow(&mut frame, &mut capacity));
            serial_watches.grow(Hour::grow(&mut serial_frame, &mut serial_capacity));
            calls.push(result.calls);
        }
        // No hour, not even one wanting a new region, installed a larger
        // table and so started an epoch that copies everything again.
        for watches in [&watches, &serial_watches] {
            assert_eq!(watches.status()["installs"], 1);
            assert_eq!(watches.status()["refused_registrations"], 0);
        }
        let mut fresh = Hour::new();
        let unobserved = crate::history_changes::Unobserved;
        histories_hour(
            &world,
            &regions,
            &unobserved,
            &mut fresh,
            &mut hour::frame(&capacity),
            0,
            false,
        );
        assert!(fresh.same(&hour) && fresh.same(&serial));
        calls
    }

    #[test]
    fn naval_patrol_histories_are_read_where_written() {
        use crate::extended::air_histories::PARTS;
        let calls = naval_patrol_histories(100, 5, 2);
        // `history_verify` also copies everything every hour.
        if cfg!(feature = "history_verify") {
            return;
        }
        // First hour: gathers batched across countries and histories, the
        // air objects authenticated 64 to a gather.
        assert!(calls[0] <= 12 * PARTS, "{calls:?}");
        // A quiet hour copies only each part's air object slots.
        assert_eq!(calls[1], 3 * PARTS);
        // One country re-read; the advance of 40 histories without a read.
        assert!(calls[2] <= 3 * PARTS + 12, "{calls:?}");
        assert_eq!(calls[3], 3 * PARTS);
    }

    /// A history whose advance count was missed is re-read on its own.
    #[test]
    fn an_unobserved_advance_rereads_only_that_history() {
        use crate::extended::air_histories::hour::{self, Hour};
        let world = |current| World {
            slots: 3,
            air: Some(AirWorld {
                every: 1,
                regions: vec![3],
                ring: 30,
                current,
            }),
            ..world()
        };
        let watches = crate::history_changes::Watches::enabled(3, 1);
        let mut changes = Blinded {
            watches: &watches,
            blind: Default::default(),
        };
        let (mut hour, mut frame) = (Hour::new(), hour::frame(&hour::capacities(3)));
        histories_hour(&world(3), &[3], &changes, &mut hour, &mut frame, 0, false);
        changes.blind.insert(history(1, 3));
        touch(&watches, 1, [history(1, 3), history(2, 3)], 1);
        let rows = histories_hour(&world(4), &[3], &changes, &mut hour, &mut frame, 1, true).held;
        let counts = Hour::counts(&frame);
        assert_eq!((counts.advanced, counts.refreshed, rows), (1, 1, 20));
        let mut fresh = Hour::new();
        let unobserved = crate::history_changes::Unobserved;
        histories_hour(
            &world(4),
            &[3],
            &unobserved,
            &mut fresh,
            &mut frame,
            0,
            false,
        );
        assert!(fresh.same(&hour));
    }

    /// A quiet hour, with no writer call anywhere, skips a part only when
    /// every receiver it holds is watched: a history or country air object
    /// whose watch was refused is read afresh every hour, and the watched
    /// rest is reused.
    #[test]
    fn a_quiet_hour_rereads_every_unwatched_receiver() {
        use crate::extended::air_histories::{
            Counts,
            hour::{self, Hour},
        };
        let world = World {
            slots: 3,
            air: Some(AirWorld {
                every: 1,
                regions: vec![3],
                ring: 30,
                current: 3,
            }),
            ..world()
        };
        let unobserved = crate::history_changes::Unobserved;
        let mut fresh = Hour::new();
        histories_hour(
            &world,
            &[3],
            &unobserved,
            &mut fresh,
            &mut hour::frame(&hour::capacities(3)),
            0,
            false,
        );
        for (blind, counts) in [
            (
                None,
                Counts {
                    reused: 2,
                    quiet: true,
                    ..Counts::default()
                },
            ),
            (
                Some(history(1, 3)),
                Counts {
                    reused: 2,
                    refreshed: 1,
                    ..Counts::default()
                },
            ),
            (
                Some(AIR_OBJECTS + AIR_OBJECT),
                Counts {
                    reused: 1,
                    read: 1,
                    ..Counts::default()
                },
            ),
        ] {
            let watches = crate::history_changes::Watches::enabled(3, 1);
            let changes = Blinded {
                watches: &watches,
                blind: blind.into_iter().collect(),
            };
            let (mut hour, mut frame) = (Hour::new(), hour::frame(&hour::capacities(3)));
            histories_hour(&world, &[3], &changes, &mut hour, &mut frame, 0, false);
            for at in 1..3 {
                let quiet = histories_hour(&world, &[3], &changes, &mut hour, &mut frame, at, true);
                assert_eq!(Hour::counts(&frame), counts, "{blind:x?} at {at}");
                assert_eq!(quiet.held, 20);
                assert!(fresh.same(&hour));
            }
        }
    }

    /// Whichever part fails after its hour staged an advance, the hour is
    /// published unavailable and none of its copies is applied, and every
    /// part's certificates are withdrawn: the next hour, staged before the
    /// failed one is decoded, copies every country afresh.
    #[test]
    fn a_failed_part_withdraws_the_hour_and_the_next_copies_afresh() {
        use crate::extended::air_histories::{
            PARTS,
            hour::{self, Hour},
        };
        // Two countries with a history in every part.
        let slots = 2 * PARTS + 1;
        let world = |current| World {
            slots,
            air: Some(AirWorld {
                every: 1,
                regions: vec![3],
                ring: 30,
                current,
            }),
            ..world()
        };
        let histories: Vec<_> = (1..slots).map(|slot| history(slot, 3)).collect();
        let capacity = hour::capacities(slots);
        for part in 0..PARTS {
            let watches = crate::history_changes::Watches::enabled(slots, 1);
            let memory = world(4);
            let (mut hour, mut frame) = (Hour::new(), hour::frame(&capacity));
            histories_hour(&world(3), &[3], &watches, &mut hour, &mut frame, 0, true);
            // Hour 1 stages the daily advance of every history; one part fails.
            touch(&watches, 1, histories.iter().copied(), 1);
            let (mut failed, mut next) = (hour::frame(&capacity), hour::frame(&capacity));
            hour.supply(&mut failed, &[3], &watches);
            hour.stage(&mut failed, &memory, BASE, STATE, &watches, true);
            hour.fail(&mut failed, part);
            hour.settle(&failed);
            // Hour 2 is quiet but stages before hour 1 is decoded.
            hour.supply(&mut next, &[3], &watches);
            hour.stage(&mut next, &memory, BASE, STATE, &watches, true);
            hour.settle(&next);
            assert_eq!(Hour::copied(&next), (slots - 1) as u64, "part {part}");
            assert_eq!(
                hour.decode(&mut failed, 1),
                Outcome::Unavailable {
                    reason: "naval-patrol history acquisition failed"
                },
                "part {part}"
            );
            assert_eq!(
                hour.decode(&mut next, 2),
                Outcome::Rows {
                    len: (slots - 1) * 10
                }
            );
            let mut fresh = Hour::new();
            let unobserved = crate::history_changes::Unobserved;
            histories_hour(&memory, &[3], &unobserved, &mut fresh, &mut next, 0, false);
            assert!(fresh.same(&hour), "part {part}");
        }
    }

    /// A part the pool refused never runs: its hour is published unavailable
    /// and the next hour, though quiet, copies every country afresh.
    #[test]
    fn a_refused_part_fails_the_hour_and_the_next_copies_afresh() {
        use crate::extended::air_histories::{
            PARTS,
            hour::{self, Hour},
        };
        let slots = 2 * PARTS + 1;
        let world = World {
            slots,
            air: Some(AirWorld {
                every: 1,
                regions: vec![3],
                ring: 30,
                current: 3,
            }),
            ..world()
        };
        let capacity = hour::capacities(slots);
        let watches = crate::history_changes::Watches::enabled(slots, 1);
        let (mut hour, mut frame) = (Hour::new(), hour::frame(&capacity));
        histories_hour(&world, &[3], &watches, &mut hour, &mut frame, 0, true);
        let mut refused = hour::frame(&capacity);
        hour.supply(&mut refused, &[3], &watches);
        hour.stage_refusing(&mut refused, &world, BASE, STATE, &watches, 1);
        hour.settle(&refused);
        assert_eq!(
            hour.decode(&mut refused, 1),
            Outcome::Unavailable {
                reason: "naval-patrol history worker admission failed"
            }
        );
        let next = histories_hour(&world, &[3], &watches, &mut hour, &mut frame, 2, true);
        assert_eq!(Hour::copied(&frame), (slots - 1) as u64);
        assert_eq!(
            next.outcome,
            Outcome::Rows {
                len: (slots - 1) * 10
            }
        );
    }

    /// The scale matrix: `cargo test -p collector --release -- --ignored`.
    #[test]
    #[ignore]
    fn naval_patrol_histories_at_1500_and_5000_countries() {
        for countries in [1_500, 5_000] {
            for (every, patrolled) in [(1, 8), (1, 32), (20, 8), (20, 32)] {
                naval_patrol_histories(countries, every, patrolled);
            }
        }
    }

    #[test]
    fn per_frame_memory_grows_linearly_with_countries() {
        // The registry hint is process-wide; hold it still while measuring.
        let _hint = crate::country_scale::hint_guard();
        let bytes = |slots| {
            let sizing = Sizing::for_slots(slots);
            sampling::Captured::with_capacities(&sizing.core, &sizing.map, &sizing.extended)
                .allocated_bytes()
        };
        let vanilla = bytes(0);
        let per_state = bytes(1_501);
        let large = bytes(5_001);
        assert!(vanilla <= per_state && per_state <= large);
        // Linear, not quadratic: about 16 KiB per country slot per frame
        // (measured 2026-09-28 after merging main's fuel and volunteer
        // tables with the raid and political tables; main alone is 16,519 B
        // per slot between 1,500 and 5,000 slots), dominated by the project
        // pools and the per-slot research, party and raid rows.
        assert!(
            large - per_state <= 3_500 * 20 * 1024,
            "vanilla {vanilla}, 1,501 slots {per_state}, 5,001 slots {large}"
        );
    }

    /// Attaching at the main menu builds vanilla-sized frames; the spool then
    /// grows them when a save fills the registry. That path must reach every
    /// buffer a frame built for the registry would have.
    #[test]
    fn growing_a_menu_sized_frame_reaches_the_registry_sizing() {
        let menu = Sizing::for_slots(0);
        let mut loaded = menu;
        loaded.raise_for_slots(COUNTRIES + 1);
        let mut frame = sampling::Captured::with_capacities(&menu.core, &menu.map, &menu.extended);
        let (mut core, mut map, mut extended) = (loaded.core, loaded.map, loaded.extended);
        frame.grow(&mut core);
        frame.map.grow(&mut map);
        frame.extended.grow(&mut extended);
        // Growth of an unused frame follows the raised capacities exactly
        // (combat_details adjusts its own capacities on every growth, so the
        // extended packs are compared by allocation below).
        assert_eq!(core, loaded.core);
        assert!(map == loaded.map);
        let built =
            sampling::Captured::with_capacities(&loaded.core, &loaded.map, &loaded.extended);
        assert!(frame.countries.len() >= built.countries.len());
        assert!(frame.links.len() >= built.links.len());
        for ((name, grown), (_, expected)) in frame
            .extended
            .allocated_by_pack()
            .into_iter()
            .zip(built.extended.allocated_by_pack())
        {
            assert!(
                grown >= expected,
                "{name}: grew to {grown}, built {expected}"
            );
        }
    }

    #[test]
    fn a_larger_country_cache_is_handed_to_the_hook_without_game_thread_allocation() {
        use std::sync::atomic::Ordering;
        let shared = sampling::Shared::new(BASE, 0, 1);
        let installed = shared.country_cache_cells.load(Ordering::Acquire);
        let cells = Cache::cells_for_countries(COUNTRIES + 1, MIN_COUNTRY_CELLS);
        assert!(cells > installed);
        shared.offer_country_cache(cells);
        // Offering the same cells again does not allocate a second cache.
        let pool = shared.pool_bytes.load(Ordering::Relaxed);
        shared.offer_country_cache(cells);
        assert_eq!(shared.pool_bytes.load(Ordering::Relaxed), pool);
        shared.install_offered_country_cache();
        assert_eq!(shared.country_cache_cells.load(Ordering::Acquire), cells);
        assert!(shared.country_cache_swap.lock().unwrap().retired.is_some());
        // The spool thread frees the retired cache on its next offer.
        shared.offer_country_cache(0);
        assert!(shared.country_cache_swap.lock().unwrap().retired.is_none());
    }

    #[test]
    fn rows_keep_growth_headroom_and_never_lower_capacity() {
        assert_eq!(rows(0, 1), 0);
        assert_eq!(rows(15_000, 1), 20_000);
        assert_eq!(rows(15_000, 2), 40_000);
        let mut capacity = 30_000;
        raise(&mut capacity, rows(15_000, 1));
        assert_eq!(capacity, 30_000);
        raise(&mut capacity, rows(30_000, 1));
        assert_eq!(capacity, 40_000);
    }

    #[test]
    fn job_censuses_reserve_one_row_per_slot_with_headroom_only_once() {
        let mut census = Vec::<(usize, u32)>::with_capacity(512);
        assert!(reserve_rows(&mut census, rows(COUNTRIES + 1, 1)));
        assert!(census.capacity() > COUNTRIES);
        assert!(!reserve_rows(&mut census, rows(COUNTRIES + 1, 1)));
        assert!(!reserve_rows(&mut census, rows(10, 1)));
    }

    #[test]
    fn sizing_only_rises() {
        let small = Sizing::for_slots(0);
        let mut large = Sizing::for_slots(COUNTRIES + 1);
        assert!(large.core[0] > COUNTRIES && large.core[2] > COUNTRIES);
        assert!(large.country_cells >= COUNTRIES * 8);
        let before = large;
        large.raise_for_slots(10);
        assert!(large == before);
        assert!(small.core[0] <= large.core[0]);
        assert_eq!(small.country_cells, MIN_COUNTRY_CELLS);
    }
}
