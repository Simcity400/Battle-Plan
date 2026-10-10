//! The stored operands of state-scope raid target triggers: resource amounts,
//! state dynamic modifiers and state cores (`is_core_of`). One census walk
//! fills the first four tables and
//! a second pass over the same census reads the cores. They are two sliced
//! families: the
//! state tables (`raid_state`, owner: state ID) and CountryBuildingResources
//! (`raid_country_buildings`, owner: native country ID), read daily: each
//! hour reads that hour's owner slice, an owner new to its census and the
//! owners of a read that was not published. There is no change fingerprint;
//! the game re-checks raid offers in a rolling sweep too.
use crate::{
    sampling::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    spooling::{table, unavailable},
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
    ("state cores", 4, &[4]),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    walk: native::Capacity,
    /// StateCores' raw country links, every read state's list.
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
    pub(crate) fn presize(
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
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.walk.countries, rows(slots, 1));
    }
}

/// The two families' owner stores, loaned for one capture.
pub(crate) struct Stores<'a> {
    pub(crate) states: &'a mut SliceFingerprints,
    pub(crate) countries: &'a mut SliceFingerprints,
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
    /// The states whose StateResourceInputs, StateResources and
    /// StateDynamicModifiers rows this hour's read replaced.
    states: SliceSelection,
    /// The countries whose CountryBuildingResources rows it replaced.
    countries: SliceSelection,
}

impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.outcomes = [super::failed_capture(TABLES[0], hour, force_full, reason); N];
        self.states.reset(hour, force_full);
        self.countries.reset(hour, force_full);
    }
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
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
    pub(crate) fn reset_selections(&mut self, hour: i32, force_full: bool) {
        self.states.reset(hour, force_full);
        self.countries.reset(hour, force_full);
    }

    /// The states whose StateResourceInputs, StateResources,
    /// StateDynamicModifiers and StateCores rows this frame's read replaced.
    pub(crate) fn state_selection(&self) -> &SliceSelection {
        &self.states
    }

    /// The countries whose CountryBuildingResources rows it replaced.
    pub(crate) fn country_selection(&self) -> &SliceSelection {
        &self.countries
    }

    /// Fixture fixed-storage read without the families' owner stores.
    #[cfg(test)]
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
    #[cfg(test)]
    pub(crate) fn capture_sliced(
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
    pub(crate) fn capture_sliced_growing(
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
        stores: Option<Stores<'_>>,
        growing: bool,
    ) -> [u64; N] {
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
        if let Some(stores) = stores {
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
            self.states.scan_ticks = qpc().saturating_sub(scan).max(0) as u64;
            let scan = qpc();
            let countries = match ccountry::Reader::new(memory, base, game_state) {
                Err(_) => false,
                Ok(countries) => {
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
            self.countries.scan_ticks = qpc().saturating_sub(scan).max(0) as u64;
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
        // those the walk reports, not every selected census owner.
        self.countries.track_reads();
        for &country in &self.buffers.read_countries {
            self.countries.mark_read(country);
        }
        self.required = walk.required;
        [
            walked,
            0,
            0,
            0,
            qpc().saturating_sub(cores_started).max(0) as u64,
        ]
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
        let links = |row: &native::CoreRow| {
            row.start
                .checked_add(row.len)
                .and_then(|end| self.core_links.get(row.start..end))
        };
        let rows = self.cores.get(..outcome.len()).unwrap_or_default();
        if !rows.iter().all(|row| links(row).is_some()) {
            return unavailable("state core list outside captured links");
        }
        table::<tables::StateCores, _>(outcome, &self.cores, |row| rows::StateCoreRow {
            state: row.state,
            cores: owned_vec(copy_vec(links(row).unwrap())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

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
