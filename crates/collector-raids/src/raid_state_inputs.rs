//! The stored operands of state-scope raid target triggers: resource amounts,
//! state dynamic modifiers and state cores and claims (`is_core_of`,
//! `is_claimed_by`). One census walk fills the first four tables and
//! a second pass over the same census reads the core and claim lists. They are two sliced
//! families: the state tables (`raid_state`, owner: state ID) and
//! CountryBuildingResources (`raid_country_buildings`, owner: native country
//! ID). Owner decision 2026-10-11: they equal the game's at every hour, as
//! the game's offer cache re-checks every offer at least hourly
//! (raid offer cache update). Each hour reads its owner slice,
//! an owner new to its census, the owners of a read that was not published
//! and every owner a hooked native writer marked since the previous hour
//! ([`crate::raid_changes`]); a controller of a read state is read with it.
//! An hour whose take of the marks does not certify the previous hour's read
//! reads every owner. The supply-route flag, which the hourly supply rebuild
//! rewrites, is read with its state and no hook certifies it. Every owner
//! read and not marked is compared with its previous read
//! ([`SliceSelection::compare`]), the supply-route flag excepted; a
//! difference disables the family's marks for the attachment.
use crate::{
    clock::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    table_conversion::{table, unavailable},
};
use engine::allocation::{copy_vec, default_vec, owned_vec, reserve_exact, vec};
use engine::{Memory, ccountry, raid_state_inputs as native, walk::Outcome};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{self, TableId, tables},
};

pub const N: usize = 5;
pub const TABLES: [TableId; N] = [
    TableId::StateResourceInputs,
    TableId::StateResources,
    TableId::StateDynamicModifiers,
    TableId::CountryBuildingResources,
    TableId::StateCores,
];
/// The `raid_state` family's tables (owner: state) among [`TABLES`].
pub const STATE_TABLES: [usize; 4] = [0, 1, 2, 4];
/// The walk reads once for the first four tables; the inputs table carries
/// it. The core lists are their own read.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    (
        "state resource trigger operands and state modifiers",
        0,
        &[0, 1, 2, 3],
    ),
    ("state cores, claims and strategic locations", 4, &[4]),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    walk: native::Capacity,
    /// StateCores' raw country links, every read state's core and claim lists.
    core_links: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            walk: native::Capacity::zero(),
            core_links: 0,
        }
    }
}

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
    ) -> Result<(), &'static str> {
        let (walk, core_links) = native::demand(memory, base, game_state)?;
        self.walk.raise(&walk);
        self.core_links = self.core_links.max(core_links);
        Ok(())
    }

    /// Presize the per-country buffers from the country registry, so a large
    /// mod's first hour is not truncated. Never lowers a capacity.
    pub fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.walk.countries, rows(slots, 1));
    }
}

/// The two families' owner stores, loaned for one capture, and the hook
/// marks that join each hour's read.
pub struct Stores<'a> {
    pub states: &'a mut SliceFingerprints,
    pub countries: &'a mut SliceFingerprints,
    pub changes: &'a dyn crate::raid_changes::Changes,
    /// Read every owner every hour while still comparing each unmarked one
    /// (`raid_verify`, [`crate::raid_changes::VERIFY`] in production).
    pub verify: bool,
}

pub struct Captured {
    buffers: native::Buffers,
    /// StateCores rows (one per read state) and their links.
    cores: Vec<native::CoreRow>,
    core_links: Vec<u32>,
    /// The links the last read needed, fitted or not.
    core_required: usize,
    outcomes: [Outcome; N],
    token_bytes: usize,
    required: native::Capacity,
    /// The capacity `buffers` was built with. Every frame compares its own,
    /// so an idle frame grows after another frame raised the shared capacity.
    built: native::Capacity,
    /// This hour's state census, the state owners scanned.
    census: Vec<u32>,
    /// Each read owner's row hashes, then its content hash
    /// ([`SliceSelection::compare`]); the worker grows it.
    hashes: Vec<(u32, u64)>,
    /// The states whose StateResourceInputs, StateResources and
    /// StateDynamicModifiers rows this hour's read replaced.
    states: SliceSelection,
    /// The countries whose CountryBuildingResources rows it replaced.
    countries: SliceSelection,
}

impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.outcomes = [super::failed_capture(TABLES[0], hour, force_full, reason); N];
        self.states.reset(hour, force_full);
        self.countries.reset(hour, force_full);
    }
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub fn vacant() -> Self {
        Self {
            buffers: native::Buffers::empty(),
            cores: Vec::new(),
            core_links: Vec::new(),
            core_required: 0,
            outcomes: [super::MISSING; N],
            token_bytes: 0,
            required: native::Capacity::zero(),
            built: native::Capacity::zero(),
            census: Vec::new(),
            hashes: Vec::new(),
            states: SliceSelection::new(false),
            countries: SliceSelection::new(false),
        }
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            buffers: native::Buffers::new(&capacity.walk)
                .map_err(crate::allocation::io)
                .unwrap_or_else(|_| native::Buffers::empty()),
            cores: owned_vec(default_vec(capacity.walk.states)),
            core_links: owned_vec(default_vec(capacity.core_links)),
            core_required: 0,
            outcomes: [super::MISSING; N],
            token_bytes: 0,
            required: native::Capacity::zero(),
            built: capacity.walk,
            census: owned_vec(vec(capacity.walk.states)),
            hashes: Vec::new(),
            states: SliceSelection::new(true),
            countries: SliceSelection::new(true),
        }
    }
    pub fn reset(&mut self) {
        self.outcomes = [super::MISSING; N];
        self.token_bytes = 0;
        self.required = native::Capacity::zero();
        self.core_required = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.buffers.allocated_bytes()
            + self.census.capacity() * size_of::<u32>()
            + self.hashes.capacity() * size_of::<(u32, u64)>()
            + self.cores.capacity() * size_of::<native::CoreRow>()
            + self.core_links.capacity() * size_of::<u32>()
    }
    /// Off-thread: grow every buffer a truncated walk measured as too small.
    pub fn grow(&mut self, capacity: &mut Capacities) {
        capacity.walk.raise(&self.required);
        // Capacities only rise, so a difference is a buffer below capacity,
        // whichever frame raised it.
        if self.built != capacity.walk {
            let Ok(buffers) = native::Buffers::new(&capacity.walk).map_err(crate::allocation::io)
            else {
                return;
            };
            self.buffers = buffers;
            self.built = capacity.walk;
        }
        let states = capacity.walk.states;
        let additional = states.saturating_sub(self.census.len());
        if reserve_exact(&mut self.census, additional).is_err() {
            return;
        }
        if self.cores.len() < states {
            let additional = states - self.cores.len();
            if reserve_exact(&mut self.cores, additional).is_err() {
                return;
            }
            self.cores.resize(states, native::CoreRow::default());
        }
        crate::country_scale::raise(
            &mut capacity.core_links,
            crate::country_scale::rows(self.core_required, 1),
        );
        if self.core_links.len() < capacity.core_links {
            let additional = capacity.core_links - self.core_links.len();
            if reserve_exact(&mut self.core_links, additional).is_err() {
                return;
            }
            self.core_links.resize(capacity.core_links, 0);
        }
    }

    /// Start a frame's selections at its hour; the capture resets them again.
    pub fn reset_selections(&mut self, hour: i32, force_full: bool) {
        self.states.reset(hour, force_full);
        self.countries.reset(hour, force_full);
    }

    /// The states whose StateResourceInputs, StateResources,
    /// StateDynamicModifiers and StateCores rows this frame's read replaced.
    pub fn state_selection(&self) -> &SliceSelection {
        &self.states
    }

    /// The countries whose CountryBuildingResources rows it replaced.
    pub fn country_selection(&self) -> &SliceSelection {
        &self.countries
    }

    /// Fixture fixed-storage read without the families' owner stores.
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_sliced(memory, base, game_state, hour, force_full, None)
    }

    /// Scan the state and country censuses into `stores` with a constant
    /// print, then read this hour's owner slice and every new or unpublished
    /// owner.
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture_sliced(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        stores: Option<Stores<'_>>,
    ) -> [u64; N] {
        self.capture_sliced_with(memory, base, game_state, hour, force_full, stores, false)
    }

    /// Worker-only: continue each original walk in its owned growing storage.
    pub fn capture_sliced_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        stores: Option<Stores<'_>>,
    ) -> [u64; N] {
        self.capture_sliced_with(memory, base, game_state, hour, force_full, stores, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_sliced_with(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        mut stores: Option<Stores<'_>>,
        growing: bool,
    ) -> [u64; N] {
        use crate::raid_changes::Family;
        let _reads = crate::read_accounting::table(TABLES[0]);
        self.reset();
        let full = force_full || stores.is_none();
        self.states.reset(hour, full);
        self.countries.reset(hour, full);
        if !should_walk(TABLES[0], hour, force_full) {
            self.outcomes = [Outcome::Unavailable {
                reason: SKIPPED_REASON,
            }; N];
            return [0; N];
        }
        let started = qpc();
        let scanned = stores.is_some();
        // The ids a complete census saw, sizing each family's mark set.
        let mut ids = [None; 2];
        if let Some(stores) = stores.as_mut() {
            // Scan first, so a new owner joins this hour's read.
            let scan = qpc();
            let states = native::state_census(memory, game_state, &mut self.census)
                .map_err(crate::allocation::io)
                .is_ok();
            for &state in &self.census {
                self.states.scan(stores.states, state, Some(0));
            }
            stores.states.finish_scan();
            self.states.finish_pass(stores.states, states);
            if states {
                ids[0] = self.census.iter().max().map(|&id| id as usize + 1);
            }
            self.states.scan_ticks = qpc().saturating_sub(scan).max(0) as u64;
            let scan = qpc();
            let reader = ccountry::Reader::new(memory, base, game_state).ok();
            let countries = match reader.as_ref() {
                None => false,
                Some(countries) => {
                    let mut complete = true;
                    for slot in 1..countries.count() {
                        match countries.at(slot) {
                            Ok(Some(country)) => {
                                self.countries.scan(stores.countries, country.id, Some(0))
                            }
                            Ok(None) => {}
                            Err(_) => complete = false,
                        }
                    }
                    complete
                }
            };
            stores.countries.finish_scan();
            self.countries.finish_pass(stores.countries, countries);
            if countries {
                ids[1] = reader
                    .as_ref()
                    .map(|countries| countries.arrays()[1].count as usize);
            }
            self.countries.scan_ticks = qpc().saturating_sub(scan).max(0) as u64;
            // Every owner a hooked writer marked since the previous hour
            // joins this hour's read; a take that does not certify the
            // previous hour's read reads every owner.
            let changes = stores.changes;
            let states = &mut self.states;
            let epoch = changes.take(Family::States, &mut |id| states.mark(id as u32));
            states.certify(stores.states, epoch, stores.verify);
            // A country mark is its raw link; an unresolved one certifies
            // nothing.
            let mut resolved = true;
            let countries = &mut self.countries;
            let epoch = changes.take(Family::Countries, &mut |raw| match reader
                .as_ref()
                .map(|reader| reader.resolve(raw as u32))
            {
                Some(Ok(country)) => countries.mark(country.id),
                _ => resolved = false,
            });
            countries.certify(stores.countries, epoch.filter(|_| resolved), stores.verify);
        }
        let (states, countries) = (&self.states, &self.countries);
        let select = native::Selection {
            states: &|state| states.contains(state),
            countries: &|country| countries.contains(country),
        };
        let walk = if growing {
            native::walk_growing(memory, base, game_state, &mut self.buffers, select)
        } else {
            native::walk(memory, base, game_state, &mut self.buffers, select)
        };
        let [inputs, resources, modifiers, overrides] = walk.outcomes;
        self.token_bytes = walk.token_bytes;
        let walked = qpc().saturating_sub(started).max(0) as u64;
        // The cores of the same states, over the census the scan took (the
        // inline fallback takes its own: every state is read).
        let _cores_reads = crate::read_accounting::table(TABLES[4]);
        let cores_started = qpc();
        let census = if scanned {
            Ok(())
        } else {
            native::state_census(memory, game_state, &mut self.census)
        };
        let cores = match census {
            Err(error) => {
                crate::allocation::observe(&error);
                Outcome::Unavailable {
                    reason: if error.kind() == std::io::ErrorKind::OutOfMemory {
                        "collector allocation failed"
                    } else {
                        "state census unreadable or invalid"
                    },
                }
            }
            Ok(()) if growing => native::state_cores_growing(
                memory,
                base,
                game_state,
                &self.census,
                &|state| states.contains(state),
                &mut self.cores,
                &mut self.core_links,
                &mut self.core_required,
            ),
            Ok(()) => native::state_cores(
                memory,
                base,
                game_state,
                &self.census,
                &|state| states.contains(state),
                &mut self.cores,
                &mut self.core_links,
                &mut self.core_required,
            ),
        };
        self.outcomes = [inputs, resources, modifiers, overrides, cores];
        // Only controllers are walked for overrides: the countries read are
        // those the walk reports, not every selected census owner, and they
        // include the controller of every read state.
        self.countries.track_reads();
        for &country in &self.buffers.read_countries {
            self.countries.select(country);
            self.countries.mark_read(country);
        }
        self.required = walk.required;
        if let Some(stores) = stores {
            self.compare(stores, ids);
        }
        [
            walked,
            0,
            0,
            0,
            qpc().saturating_sub(cores_started).max(0) as u64,
        ]
    }
    /// Compare every owner this hour read and no hook marked with its
    /// previous read, disable a family's marks at a difference, and record
    /// the hour's counts.
    fn compare(&mut self, stores: Stores<'_>, ids: [Option<usize>; 2]) {
        use crate::raid_changes::{Counts, Family, record};
        let state_tables = STATE_TABLES.map(|index| self.outcomes[index]);
        let complete = state_tables
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Rows { .. }))
            && state_hashes(
                &self.buffers,
                &self.outcomes,
                self.token_bytes,
                &self.cores,
                &self.core_links,
                &mut self.hashes,
            );
        let (verified, mismatches) = self
            .states
            .compare(stores.states, complete.then_some(&self.hashes[..]));
        if mismatches != 0 {
            stores.changes.mismatch(Family::States);
        }
        record(
            Family::States,
            &Counts {
                marked: u64::from(self.states.marked_count),
                read: self.states.read_owners().count() as u64,
                early: u64::from(self.states.marked_early()),
                full: self.states.force_full,
                verified,
                mismatches,
            },
            ids[0],
        );
        let complete = matches!(self.outcomes[3], Outcome::Rows { .. })
            && country_hashes(&self.buffers, self.outcomes[3], &mut self.hashes);
        let (verified, mismatches) = self
            .countries
            .compare(stores.countries, complete.then_some(&self.hashes[..]));
        if mismatches != 0 {
            stores.changes.mismatch(Family::Countries);
        }
        record(
            Family::Countries,
            &Counts {
                marked: u64::from(self.countries.marked_count),
                read: self.countries.read_owners().count() as u64,
                early: u64::from(self.countries.marked_early()),
                full: self.countries.force_full,
                verified,
                mismatches,
            },
            ids[1],
        );
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        self.outcomes
    }
    pub fn tables(&self) -> [TableView; N] {
        let buffers = &self.buffers;
        let Some(names) = buffers.tokens.get(..self.token_bytes) else {
            return std::array::from_fn(|_| {
                unavailable("state modifier name arena length invalid")
            });
        };
        let modifiers = buffers
            .modifiers
            .get(..self.outcomes[2].len())
            .unwrap_or_default();
        let name = |row: &native::ModifierRow| {
            row.token_start
                .checked_add(row.token_len)
                .and_then(|end| names.get(row.token_start..end))
        };
        let names_valid = modifiers.iter().all(|row| name(row).is_some());
        [
            table::<tables::StateResourceInputs, _>(self.outcomes[0], &buffers.inputs, |row| {
                rows::StateResourceInputRow {
                    state: row.state,
                    gate: row.gate,
                    global_factor: row.global_factor,
                    origin: row.origin,
                    infrastructure_level: row.infrastructure_level,
                    infrastructure_modifier: row.infrastructure_modifier,
                    supply_route: row.supply_route,
                }
            }),
            table::<tables::StateResources, _>(self.outcomes[1], &buffers.resources, |row| {
                rows::StateResourceRow {
                    state: row.state,
                    resource: row.resource,
                    pool: row.pool,
                    additive: row.additive,
                    factor: row.factor,
                    cost: row.cost,
                }
            }),
            if names_valid {
                table::<tables::StateDynamicModifiers, _>(
                    self.outcomes[2],
                    &buffers.modifiers,
                    |row| rows::StateDynamicModifierRow {
                        state: row.state,
                        ordinal: row.ordinal,
                        token: owned_vec(copy_vec(name(row).unwrap())),
                        scope_country: row.scope_country,
                        scope_state: row.scope_state,
                    },
                )
            } else {
                unavailable("state modifier name outside captured arena")
            },
            table::<tables::CountryBuildingResources, _>(
                self.outcomes[3],
                &buffers.overrides,
                |row| rows::CountryBuildingResourceRow {
                    country: row.country,
                    ordinal: row.ordinal,
                    entry: row.entry,
                    building: row.building,
                    count: row.count,
                    resource: row.resource,
                    amount: row.amount,
                },
            ),
            self.cores_table(),
        ]
    }

    fn cores_table(&self) -> TableView {
        let outcome = self.outcomes[4];
        let links = |start: usize, len: usize| {
            start
                .checked_add(len)
                .and_then(|end| self.core_links.get(start..end))
        };
        let rows = self.cores.get(..outcome.len()).unwrap_or_default();
        if !rows.iter().all(|row| {
            links(row.start, row.len).is_some()
                && links(row.claims_start, row.claims_len).is_some()
                && links(row.locations_start, 2 * row.locations_len).is_some()
                && links(row.province_locations_start, 2 * row.province_locations_len).is_some()
        }) {
            return unavailable("state core list outside captured links");
        }
        // Two words each: (high, low) into one u64.
        let pairs = |words: &[u32]| {
            owned_vec(vec(words.len() / 2).map(|mut out: Vec<u64>| {
                out.extend(
                    words
                        .chunks_exact(2)
                        .map(|pair| u64::from(pair[0]) << 32 | u64::from(pair[1])),
                );
                out
            }))
        };
        table::<tables::StateCores, _>(outcome, &self.cores, |row| rows::StateCoreRow {
            state: row.state,
            cores: owned_vec(copy_vec(links(row.start, row.len).unwrap())),
            claims: rows::Recorded::Value(owned_vec(copy_vec(
                links(row.claims_start, row.claims_len).unwrap(),
            ))),
            strategic_locations: rows::Recorded::Value(pairs(
                links(row.locations_start, 2 * row.locations_len).unwrap(),
            )),
            province_strategic_locations: rows::Recorded::Value(pairs(
                links(row.province_locations_start, 2 * row.province_locations_len).unwrap(),
            )),
        })
    }
}

/// A content hash's starting value and one word folded into it
/// (SplitMix64's finalizer over the running value and the word).
const SEED: u64 = 0x9e37_79b9_7f4a_7c15;
fn mix(hash: u64, word: u64) -> u64 {
    let mut value = (hash ^ word).wrapping_add(SEED);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// One row's words, tagged with its table.
fn row(tag: u64, words: impl IntoIterator<Item = u64>) -> u64 {
    words.into_iter().fold(mix(SEED, tag), mix)
}

/// Bytes as their length and little-endian eight-byte words.
fn bytes(bytes: &[u8]) -> impl Iterator<Item = u64> + '_ {
    std::iter::once(bytes.len() as u64).chain(bytes.chunks(8).map(|chunk| {
        let mut word = [0; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        u64::from_le_bytes(word)
    }))
}

/// Fold `hashes` (owner, row hash), in table and row order, into one content
/// hash per owner, ascending. The sort is stable, so an owner's rows keep
/// their order.
fn fold(hashes: &mut Vec<(u32, u64)>) {
    hashes.sort_by_key(|&(owner, _)| owner);
    hashes.dedup_by(|next, kept| {
        next.0 == kept.0 && {
            kept.1 = mix(kept.1, next.1);
            true
        }
    });
}

/// Each read state's content: every field of its StateResourceInputs,
/// StateResources, StateDynamicModifiers and StateCores (cores, claims and
/// strategic locations)
/// rows that a hooked
/// writer certifies (the supply-route flag, which the hourly supply rebuild
/// rewrites, is left out). False when a row's span or the storage fails.
fn state_hashes(
    buffers: &native::Buffers,
    outcomes: &[Outcome; N],
    token_bytes: usize,
    cores: &[native::CoreRow],
    core_links: &[u32],
    hashes: &mut Vec<(u32, u64)>,
) -> bool {
    hashes.clear();
    let (Some(inputs), Some(resources), Some(modifiers), Some(cores), Some(names)) = (
        buffers.inputs.get(..outcomes[0].len()),
        buffers.resources.get(..outcomes[1].len()),
        buffers.modifiers.get(..outcomes[2].len()),
        cores.get(..outcomes[4].len()),
        buffers.tokens.get(..token_bytes),
    ) else {
        return false;
    };
    let rows = inputs.len() + resources.len() + modifiers.len() + cores.len();
    if engine::allocation::reserve(hashes, rows).is_err() {
        return false;
    }
    for input in inputs {
        let level = input
            .infrastructure_level
            .map_or(0, |level| (1 << 32) | u64::from(level as u32));
        hashes.push((
            input.state,
            row(
                1,
                [
                    u64::from(input.gate),
                    input.global_factor as u64,
                    u64::from(input.origin),
                    level,
                    input.infrastructure_modifier as u64,
                ],
            ),
        ));
    }
    for resource in resources {
        hashes.push((
            resource.state,
            row(
                2,
                [
                    u64::from(resource.resource),
                    resource.pool as u64,
                    resource.additive as u64,
                    resource.factor as u64,
                    resource.cost as u64,
                ],
            ),
        ));
    }
    for modifier in modifiers {
        let Some(name) = modifier
            .token_start
            .checked_add(modifier.token_len)
            .and_then(|end| names.get(modifier.token_start..end))
        else {
            return false;
        };
        let head = [
            u64::from(modifier.ordinal),
            u64::from(modifier.scope_country),
            u64::from(modifier.scope_state as u32),
        ];
        hashes.push((modifier.state, row(3, head.into_iter().chain(bytes(name)))));
    }
    let list = |start: usize, len: usize| {
        start
            .checked_add(len)
            .and_then(|end| core_links.get(start..end))
    };
    for core in cores {
        let (Some(links), Some(claims), Some(locations), Some(provinces)) = (
            list(core.start, core.len),
            list(core.claims_start, core.claims_len),
            list(core.locations_start, 2 * core.locations_len),
            list(core.province_locations_start, 2 * core.province_locations_len),
        ) else {
            return false;
        };
        // Each list's length precedes its words, so no two alias.
        let words = [links, claims, locations, provinces]
            .into_iter()
            .flat_map(|list| {
                std::iter::once(list.len() as u64).chain(list.iter().map(|&word| u64::from(word)))
            });
        hashes.push((core.state, row(4, words)));
    }
    fold(hashes);
    true
}

/// Each read country's content: its CountryBuildingResources rows, an empty
/// read included. False when a span or the storage fails.
fn country_hashes(
    buffers: &native::Buffers,
    outcome: Outcome,
    hashes: &mut Vec<(u32, u64)>,
) -> bool {
    hashes.clear();
    let Some(overrides) = buffers.overrides.get(..outcome.len()) else {
        return false;
    };
    let read = &buffers.read_countries;
    if engine::allocation::reserve(hashes, read.len() + overrides.len()).is_err() {
        return false;
    }
    for &country in read {
        hashes.push((country, row(5, [])));
    }
    for entry in overrides {
        hashes.push((
            entry.country,
            row(
                6,
                [
                    u64::from(entry.ordinal),
                    u64::from(entry.entry),
                    u64::from(entry.building),
                    u64::from(entry.count),
                    u64::from(entry.resource),
                    entry.amount as u64,
                ],
            ),
        ));
    }
    fold(hashes);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

    /// A state a hooked writer marked is read within the hour, outside its
    /// daily slice; an unmarked state outside the slice is not. An hour whose
    /// take does not return the previous hour's epoch reads every state.
    #[test]
    fn marked_states_are_read_within_the_hour_and_an_uncertified_hour_reads_all() {
        use crate::raid_changes::{Target, Watches};
        let base = 0x10_0000;
        let mut bytes = vec![0_u8; 0x1000];
        // The state array: slot 0 is null, states 1 to 4 follow.
        let (header, data) = (engine::cgame_state::STATES, 0x800);
        bytes[header..header + 8].copy_from_slice(&((base + data) as u64).to_le_bytes());
        bytes[header + 8..header + 12].copy_from_slice(&5_u32.to_le_bytes());
        bytes[header + 12..header + 16].copy_from_slice(&5_u32.to_le_bytes());
        for slot in 1..5 {
            bytes[data + slot * 8..][..8].copy_from_slice(&0x2000_u64.to_le_bytes());
        }
        let memory = engine::memory::SliceMemory {
            base,
            bytes: &bytes,
        };
        let watches = Watches::enabled(64, 64);
        let mut captured = Captured::with_capacities(&Capacities::default());
        let (mut states, mut countries) = (SliceFingerprints::new(), SliceFingerprints::new());
        let mut read = |hour: i32| {
            captured.capture_sliced(
                &memory,
                0,
                base,
                hour,
                false,
                Some(Stores {
                    states: &mut states,
                    countries: &mut countries,
                    changes: &watches,
                    // The marks and slice alone; burn-in builds' raid_verify
                    // reads every owner.
                    verify: false,
                }),
            );
            let selection = captured.state_selection();
            (
                selection.force_full,
                selection.read_owners().collect::<Vec<_>>(),
            )
        };
        // The first take certifies no earlier read.
        assert_eq!(read(0), (true, vec![1, 2, 3, 4]));
        // State 3's resources were refreshed: read beside hour 1's slice.
        drop(watches.notify(0, Target::State(Some(3))));
        assert_eq!(read(1), (false, vec![1, 3]));
        // A quiet hour reads its slice alone.
        assert_eq!(read(2), (false, vec![2]));
        // A writer whose state did not read moves the epoch: every state.
        drop(watches.notify(0, Target::State(None)));
        assert_eq!(read(4), (true, vec![1, 2, 3, 4]));
        assert_eq!(read(5), (false, vec![]));
    }

    #[test]
    fn a_name_outside_the_arena_withholds_only_the_modifier_table() {
        let mut captured = Captured::with_capacities(&Capacities {
            walk: native::Capacity {
                states: 1,
                modifier_rows: 1,
                token_bytes: 4,
                ..native::Capacity::zero()
            },
            core_links: 0,
        });
        captured.buffers.tokens.copy_from_slice(b"oils");
        captured.buffers.modifiers[0] = native::ModifierRow {
            state: 3,
            token_len: 3,
            ..Default::default()
        };
        captured.token_bytes = 3;
        captured.outcomes = [
            Outcome::Rows { len: 0 },
            Outcome::Rows { len: 0 },
            Outcome::Rows { len: 1 },
            Outcome::Rows { len: 0 },
            Outcome::Rows { len: 0 },
        ];
        let [
            _,
            _,
            TableView::Complete(TableRows::StateDynamicModifiers(rows)),
            _,
            _,
        ] = captured.tables()
        else {
            panic!("expected complete modifier rows");
        };
        assert_eq!((rows[0].state, rows[0].token.as_slice()), (3, &b"oil"[..]));
        captured.buffers.modifiers[0].token_len = 4;
        let views = captured.tables();
        assert!(matches!(views[2], TableView::Unavailable { .. }));
        assert!(matches!(views[0], TableView::Complete(_)));
    }
}
