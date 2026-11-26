use crate::{
    clock::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, default_vec, owned_vec, reserve_exact, vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::ccountry;
use engine_raid_inputs::raid_state_inputs as native;
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows, rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;

pub const N: usize = 5;
pub const TABLES: [TableId; N] = [
    TableId::StateResourceInputs,
    TableId::StateResources,
    TableId::StateDynamicModifiers,
    TableId::CountryBuildingResources,
    TableId::StateCores,
];
pub const STATE_TABLES: [usize; 4] = [0, 1, 2, 4];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    walk: native::Capacity,
    core_links: usize,
}

collector_primitives::capacity_max!(Capacities {
    walk { states, resources, resource_rows, modifier_rows, token_bytes, override_rows, origins, routes, provinces, entries, countries, definitions, name_bytes, batch_bytes },
    core_links,
});

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

    pub fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.walk.countries, rows(slots, 1));
    }
}

pub struct Stores<'a> {
    pub states: &'a mut SliceFingerprints,
    pub countries: &'a mut SliceFingerprints,
    pub changes: &'a dyn crate::raid_changes::Changes,
}

pub struct Captured {
    buffers: native::Buffers,
    cores: Vec<native::CoreRow>,
    core_links: Vec<u32>,
    core_required: usize,
    outcomes: [Outcome; N],
    token_bytes: usize,
    required: native::Capacity,
    built: native::Capacity,
    census: Vec<u32>,
    states: SliceSelection,
    countries: SliceSelection,
}

impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.outcomes = [super::failed_capture(TABLES[0], hour, force_full, reason); N];
        self.states.reset_hooked(hour, force_full);
        self.countries.reset_hooked(hour, force_full);
    }
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
            states: SliceSelection::new(),
            countries: SliceSelection::new(),
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
            states: SliceSelection::new(),
            countries: SliceSelection::new(),
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
    pub fn grow(&mut self, capacity: &mut Capacities) {
        capacity.walk.raise(&self.required);
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

    pub fn reset_selections(&mut self, hour: i32, force_full: bool) {
        self.states.reset_hooked(hour, force_full);
        self.countries.reset_hooked(hour, force_full);
    }

    pub fn state_selection(&self) -> &SliceSelection {
        &self.states
    }

    pub fn country_selection(&self) -> &SliceSelection {
        &self.countries
    }

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

    pub fn capture_sliced(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        stores: Option<Stores<'_>>,
    ) -> [u64; N] {
        self.capture_sliced_with(memory, base, game_state, hour, force_full, stores)
    }

    fn capture_sliced_with(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        mut stores: Option<Stores<'_>>,
    ) -> [u64; N] {
        use crate::raid_changes::Family;
        let _reads = crate::read_accounting::table(TABLES[0]);
        self.reset();
        self.buffers.recruitment_enabled =
            stores.is_none() && cfg!(any(test, feature = "test-support"));
        let full = force_full || stores.is_none();
        self.states.reset_hooked(hour, full);
        self.countries.reset_hooked(hour, full);
        if !should_walk(TABLES[0], hour, force_full) {
            self.outcomes = [Outcome::Unavailable {
                reason: SKIPPED_REASON,
            }; N];
            return [0; N];
        }
        let started = qpc();
        let scanned = stores.is_some();
        let mut scan_census = Ok(());
        let mut ids = [None; 2];
        if let Some(stores) = stores.as_mut() {
            let scan = qpc();
            scan_census = native::state_census(memory, game_state, &mut self.census)
                .map_err(crate::allocation::io);
            let states = scan_census.is_ok();
            for &state in &self.census {
                let origin = stores.states.held(state).unwrap_or(0);
                self.states.scan(stores.states, state, Some(origin));
                if origin != 0 {
                    self.states.select(state);
                }
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
            let changes = stores.changes;
            let states = &mut self.states;
            let epoch = changes.take(Family::States, &mut |id| states.mark(id as u32));
            self.buffers.recruitment_enabled = epoch.is_some();
            states.certify(stores.states, epoch);
            let mut resolved = true;
            let countries = &mut self.countries;
            let epoch = changes.take(Family::Countries, &mut |raw| match reader
                .as_ref()
                .map(|reader| reader.resolve(raw as u32))
            {
                Some(Ok(country)) => countries.mark(country.id),
                _ => resolved = false,
            });
            countries.certify(stores.countries, epoch.filter(|_| resolved));
        }
        let (states, countries) = (&self.states, &self.countries);
        let (any_state, any_country) = (states.reads_any(), countries.reads_any());
        let select = native::Selection {
            states: &|state| states.contains(state),
            countries: &|country| countries.contains(country),
            any_country,
        };
        let walk = if !any_state && !any_country {
            self.buffers.read_countries.clear();
            native::Walk {
                outcomes: [Outcome::Rows { len: 0 }; 4],
                token_bytes: 0,
                required: native::Capacity::zero(),
            }
        } else {
            native::walk(memory, base, game_state, &mut self.buffers, select)
        };
        let [inputs, resources, modifiers, overrides] = walk.outcomes;
        if let Some(stores) = stores.as_mut() {
            for row in self.buffers.inputs.get(..inputs.len()).unwrap_or_default() {
                stores.states.hold(row.state, u64::from(row.origin));
            }
        }
        self.token_bytes = walk.token_bytes;
        let walked = qpc().saturating_sub(started).max(0) as u64;
        let _cores_reads = crate::read_accounting::table(TABLES[4]);
        let cores_started = qpc();
        let census = if scanned {
            scan_census
        } else {
            native::state_census(memory, game_state, &mut self.census)
        };
        let cores = match census {
            Ok(()) if !any_state => Outcome::Rows { len: 0 },
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
        self.countries.track_reads();
        for &country in &self.buffers.read_countries {
            self.countries.select(country);
            self.countries.mark_read(country);
        }
        self.required = walk.required;
        if scanned {
            self.record_counts(ids);
        }
        [
            walked,
            0,
            0,
            0,
            qpc().saturating_sub(cores_started).max(0) as u64,
        ]
    }
    fn record_counts(&self, ids: [Option<usize>; 2]) {
        use crate::raid_changes::{Counts, Family, record};
        for (family, selection, ids) in [
            (Family::States, &self.states, ids[0]),
            (Family::Countries, &self.countries, ids[1]),
        ] {
            record(
                family,
                &Counts {
                    marked: u64::from(selection.marked_count),
                    read: selection.read_owners().count() as u64,
                    carried: selection.carried() as u64,
                    early: u64::from(selection.marked_early()),
                    full: selection.force_full,
                },
                ids,
            );
        }
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
                    local_manpower: rows::Recorded::Value(row.recruitment.map(|v| v[0])),
                    local_noncore_manpower: rows::Recorded::Value(row.recruitment.map(|v| v[1])),
                    recruitment_add: rows::Recorded::Value(row.recruitment.map(|v| v[2])),
                    recruitment_factor: rows::Recorded::Value(row.recruitment.map(|v| v[3])),
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

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    struct Traced<'a> {
        memory: engine_native_memory::memory::SliceMemory<'a>,
        copies: std::cell::RefCell<Vec<usize>>,
    }

    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.copies.borrow_mut().push(address);
            self.memory.copy(address, destination)
        }
    }

    fn world() -> (engine_native::ccountry::synthetic::World, [usize; 4]) {
        let mut world = engine_native::ccountry::synthetic::World::new(0, 4, 0x200);
        let slots = world.alloc(5 * 8);
        world.array(
            world.game_state() + engine_native::cgame_state::STATES,
            slots,
            5,
        );
        let objects = [1, 2, 3, 4].map(|slot| {
            let state = world.alloc(0x1000);
            world.u64(slots + slot * 8, state as u64);
            state
        });
        (world, objects)
    }

    struct Hour {
        full: bool,
        states: Vec<u32>,
        countries: Vec<u32>,
        touched: bool,
        outcomes: [Outcome; N],
    }

    fn hour(
        world: &engine_native::ccountry::synthetic::World,
        objects: &[usize; 4],
        captured: &mut Captured,
        states: &mut SliceFingerprints,
        countries: &mut SliceFingerprints,
        changes: &dyn crate::raid_changes::Changes,
        hour: i32,
    ) -> Hour {
        let memory = Traced {
            memory: world.memory(),
            copies: Default::default(),
        };
        captured.capture_sliced(
            &memory,
            0,
            world.game_state(),
            hour,
            false,
            Some(Stores {
                states,
                countries,
                changes,
            }),
        );
        let touched = memory
            .copies
            .borrow()
            .iter()
            .any(|&at| at < world.game_state() || at >= objects[0]);
        Hour {
            full: captured.state_selection().force_full,
            states: captured.state_selection().read_owners().collect(),
            countries: (1..=4)
                .filter(|&country| captured.country_selection().contains(country))
                .collect(),
            touched,
            outcomes: captured.outcomes(),
        }
    }

    #[test]
    fn quiet_hours_read_nothing_and_marks_read_only_their_owners() {
        use crate::raid_changes::{Family, Target, Watches};
        let (mut world, objects) = world();
        let watches = Watches::enabled(64, 64);
        let mut captured = Captured::with_capacities(&Capacities::default());
        let (mut states, mut countries) = (SliceFingerprints::new(), SliceFingerprints::new());
        macro_rules! read {
            ($changes:expr, $hour:expr) => {
                hour(
                    &world,
                    &objects,
                    &mut captured,
                    &mut states,
                    &mut countries,
                    $changes,
                    $hour,
                )
            };
        }
        let first = read!(&watches, 0);
        assert!(first.full && first.touched);
        assert_eq!(first.states, [1, 2, 3, 4]);
        for at in 1..=3 {
            world.u64(objects[at as usize - 1], 7);
            let quiet = read!(&watches, at);
            assert!(!quiet.full && !quiet.touched, "hour {at}");
            assert!(quiet.states.is_empty() && quiet.countries.is_empty());
            assert_eq!(quiet.outcomes, [Outcome::Rows { len: 0 }; N]);
        }
        drop(watches.notify(0, Target::State(Some(3))));
        let marked = read!(&watches, 4);
        assert!(!marked.full && marked.touched);
        assert_eq!((marked.states, marked.countries), (vec![3], vec![]));
        drop(watches.notify(10, Target::Country(Some(2))));
        let marked = read!(&watches, 5);
        assert_eq!((marked.states, marked.countries), (vec![], vec![2]));
        states.hold(1, 1);
        assert_eq!(read!(&watches, 25).states, [1]);
        assert_eq!(read!(&watches, 26).states, [1]);
        states.hold(1, 0);
        watches.boundary(true);
        let invalidated = read!(&watches, 27);
        assert!(invalidated.full);
        assert_eq!(invalidated.states, [1, 2, 3, 4]);
        assert!(!read!(&watches, 28).touched);
        let disabled = Watches::new();
        for at in 29..31 {
            let every = read!(&disabled, at);
            assert!(every.full && every.touched);
            assert_eq!(every.countries, [1, 2, 3, 4]);
        }
        assert!(!watches.poisoned(Family::States));
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

    #[test]
    fn an_unreadable_state_census_withholds_state_cores() {
        use crate::raid_changes::Watches;
        let base = 0x10_0000;
        let mut bytes = vec![0_u8; 0x1000];
        let header = engine_native::cgame_state::STATES;
        bytes[header..header + 8].copy_from_slice(&((base + 0x10_0000) as u64).to_le_bytes());
        bytes[header + 8..header + 12].copy_from_slice(&5_u32.to_le_bytes());
        bytes[header + 12..header + 16].copy_from_slice(&5_u32.to_le_bytes());
        let memory = engine_native_memory::memory::SliceMemory {
            base,
            bytes: &bytes,
        };
        let watches = Watches::enabled(64, 64);
        let mut captured = Captured::with_capacities(&Capacities::default());
        let (mut states, mut countries) = (SliceFingerprints::new(), SliceFingerprints::new());
        captured.capture_sliced(
            &memory,
            0,
            base,
            0,
            false,
            Some(Stores {
                states: &mut states,
                countries: &mut countries,
                changes: &watches,
            }),
        );
        assert!(matches!(
            captured.outcomes()[4],
            Outcome::Unavailable { .. }
        ));
    }
}
