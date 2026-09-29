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
    Some(RaidArenas {
        type_tokens: parents.checked_add(player)?,
        reservation_words,
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
    pub core: [usize; 11],
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
        };
        let mut sizing = Sizing::for_slots(440);
        let default_tokens = sizing.extended.projects.raid_type_tokens;
        assert!(default_tokens < demand.type_tokens);
        sizing.extended.raise_for_raid_arenas(demand);
        assert!(sizing.extended.projects.raid_type_tokens >= demand.type_tokens);
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

    /// Computes every byte on demand, so thousands of countries need no
    /// multi-megabyte fixture: slot `s` is object `OBJECTS + s * OBJECT` with
    /// native id `s`, every link id resolves to itself, and all other fields
    /// read as zero (with valid empty strings).
    struct World {
        slots: usize,
        /// Every country owns a dense intel vector over every slot with all
        /// four lanes nonzero: the worst case for the sparse table.
        intel: bool,
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
            if (DIPLOMACY..DIPLOMACY + 8).contains(&address) {
                return word(
                    DIPLOMACY,
                    (BASE + engine::cdiplomacy_status::VTABLE_RVA) as u64,
                );
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
                return word(TABLE + slot * 8, (OBJECTS + slot * OBJECT) as u64);
            }
            if (LINKS + 4..LINKS + self.slots * 4).contains(&address) {
                let id = (address - LINKS) / 4;
                return word(LINKS + id * 4, id as u64);
            }
            if (OBJECTS + OBJECT..OBJECTS + self.slots * OBJECT).contains(&address) {
                let slot = (address - OBJECTS) / OBJECT;
                let object = OBJECTS + slot * OBJECT;
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
