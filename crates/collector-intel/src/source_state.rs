
use engine_allocation as allocation;
use engine_intelligence::intel_sources as native;
use engine_native_memory::{Memory, read};
use native::{Dense, DynamicPool, Lanes, SourceReference, StaticPool, StaticSource};
use replay_data::rows::{IntelContributionRow, IntelSourceRow};

pub const RESET: u8 = 0;
pub const STATIC_SET: u8 = 1;
pub const STATIC_REMOVE: u8 = 2;
pub const STATIC_CREATE: u8 = 3;
pub const STATIC_PRUNE: u8 = 4;
pub const STATIC_CLEAR: u8 = 5;
pub const DYNAMIC_ADD: u8 = 6;
pub const DYNAMIC_CLEAR: u8 = 7;
pub const DYNAMIC_AGGREGATE: u8 = 8;
pub const DYNAMIC_DECAY: u8 = 9;
pub const ALLIED_REPLACE: u8 = 10;
pub const CRYPTOLOGY_REPLACE: u8 = 11;
pub const CONTRIBUTIONS_REPLACE: u8 = 12;
pub const PRIOR_ROTATE: u8 = 13;
pub const REFERENCE: u8 = 14;
pub const ALLIED_CLEAR: u8 = 15;

#[derive(Clone, Copy, Debug, Default)]
pub struct Event {
    pub order: u64,
    pub object: usize,
    pub kind: u8,
    pub pool: u32,
    pub source: i32,
    pub target: i32,
    pub values: [i64; 4],
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Seed {
    pub owner: u32,
    pub root: usize,
    pub country: usize,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reference {
    pub address: usize,
    pub owner: u32,
    pub kind: u32,
    pub target: u32,
    pub value: SourceReference,
}

#[derive(Default)]
pub struct Owner {
    pub seed: Seed,
    pub raw_owner: u32,
    pub lifetime: u32,
    pub allied: Dense,
    pub cryptology: Dense,
    pub contributions: [Vec<Vec<Lanes>>; 3],
    pub statics: Vec<StaticPool>,
    pub dynamics: Vec<DynamicPool>,
    pub references: [Reference; 3],
    dirty: [bool; 5],
    accumulators: Vec<bool>,
    image_base: usize,
    references_dirty: bool,
}

impl Owner {

    pub fn bootstrap(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: Seed,
    ) -> Result<(), &'static str> {
        self.seed = seed;
        self.image_base = base;
        seed.root
            .checked_add(engine_native_intelligence::ccountry_intel::SIZE)
            .ok_or("intel source root span overflow")?;
        seed.country
            .checked_add(engine_native::ccountry::OPERATION_TOKEN_MANAGER + 8)
            .ok_or("intel source country span overflow")?;
        let root_vtable = engine_sites::at(base, engine_native_intelligence::ccountry_intel::VTABLE_RVA)
            .ok_or("intel source bootstrap root identity mismatch")?;
        if read::<u64>(memory, seed.root) != Some(root_vtable as u64) {
            return Err("intel source bootstrap root identity mismatch");
        }
        self.raw_owner = read(memory, seed.root + engine_native_intelligence::ccountry_intel::OWNER)
            .ok_or("intel source owner unreadable")?;
        self.lifetime =
            read(memory, seed.root + native::LIFETIME).ok_or("intel source lifetime unreadable")?;
        native::dense(memory, seed.root, native::ALLIED, &mut self.allied)?;
        native::dense(memory, seed.root, native::CRYPTOLOGY, &mut self.cryptology)?;
        for (index, offset) in [
            native::STATIC_CONTRIBUTIONS,
            native::DYNAMIC_CONTRIBUTIONS,
            native::PRIOR_CONTRIBUTIONS,
        ]
        .into_iter()
        .enumerate()
        {
            native::contributions(memory, seed.root, offset, &mut self.contributions[index])?;
        }
        native::static_pools(memory, base, seed.root, &mut self.statics)?;
        native::dynamic_pools(memory, base, seed.root, &mut self.dynamics)?;
        allocation::resize_default(&mut self.accumulators, self.dynamics.len())
            .map_err(|_| "intel dynamic dirty storage allocation failed")?;
        self.accumulators.fill(false);
        self.dirty.fill(false);
        self.references_dirty = false;
        self.refresh_references(memory)?;
        Ok(())
    }

    fn refresh_references(&mut self, memory: &impl Memory) -> Result<(), &'static str> {
        let seed = self.seed;
        let base = self.image_base;
        if self.references_dirty
            && seed
                .country
                .checked_add(engine_native::ccountry::INTEL_TABLE)
                .and_then(|address| read::<u64>(memory, address))
                != Some(seed.root as u64)
        {
            return Err("intel source root changed outside existing census");
        }
        let radar = seed
            .country
            .checked_add(engine_native::ccountry::RADARS_POOL)
            .ok_or("intel radar owner address overflow")?;
        let radar_vtable = engine_sites::at(base, engine_native_parts::cradars_pool::VTABLE_RVA)
            .ok_or("intel radar source identity mismatch")?;
        if read::<u64>(memory, radar) != Some(radar_vtable as u64) {
            return Err("intel radar source identity mismatch");
        }
        let token = seed
            .country
            .checked_add(engine_native::ccountry::OPERATION_TOKEN_MANAGER)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|p| *p != 0)
            .ok_or("intel token source manager unreadable")?;
        token
            .checked_add(native::TOKEN_REFERENCE + 24)
            .ok_or("intel token reference span overflow")?;
        let token_vtable = engine_sites::at(
            base,
            engine_intelligence::ccountry_operation_token_manager::VTABLE_RVA,
        )
        .ok_or("intel token source owner identity mismatch")?;
        if read::<u64>(memory, token) != Some(token_vtable as u64)
            || read::<u64>(
                memory,
                token + engine_intelligence::ccountry_operation_token_manager::COUNTRY,
            ) != Some(seed.country as u64)
        {
            return Err("intel token source owner identity mismatch");
        }
        for (kind, address) in [
            seed.root + native::CHEAT_REFERENCE,
            radar + native::RADAR_REFERENCE,
            token + native::TOKEN_REFERENCE,
        ]
        .into_iter()
        .enumerate()
        {
            self.references[kind] = Reference {
                address,
                owner: seed.owner,
                kind: kind as u32,
                target: 0,
                value: native::reference(memory, base, address)?,
            };
        }
        self.references_dirty = false;
        Ok(())
    }

    pub fn apply(&mut self, event: Event) -> Result<bool, &'static str> {
        let pool = event.pool as usize;
        Ok(match event.kind {
            RESET => {
                self.references_dirty = true;
                self.lifetime = 0;
                self.allied.count = 0;
                self.allied.entries.clear();
                self.cryptology.count = 0;
                self.cryptology.entries.clear();
                for list in &mut self.contributions {
                    list.clear();
                }
                allocation::resize_default(&mut self.statics, event.values[0] as usize)
                    .map_err(|_| "intel reset static pool storage allocation failed")?;
                for pool in &mut self.statics {
                    pool.sources.clear();
                    pool.next_id = 0;
                }
                allocation::resize_default(&mut self.dynamics, event.values[1] as usize)
                    .map_err(|_| "intel reset dynamic pool storage allocation failed")?;
                for pool in &mut self.dynamics {
                    pool.values.clear();
                    pool.accumulators.clear();
                }
                allocation::resize_default(&mut self.accumulators, self.dynamics.len())
                    .map_err(|_| "intel reset dirty storage allocation failed")?;
                self.accumulators.fill(false);
                self.dirty.fill(false);
                true
            }
            STATIC_SET | STATIC_REMOVE => {
                let pool = self
                    .statics
                    .get_mut(pool)
                    .ok_or("observed intel static pool outside retained extent")?;
                let source = pool
                    .sources
                    .iter_mut()
                    .rfind(|source| source.id == event.source)
                    .ok_or("observed intel source ID absent from retained pool")?;
                if event.kind == STATIC_REMOVE {
                    let changed = source.removed != 1;
                    source.removed = 1;
                    changed
                } else {
                    native::set_source(source, event.target, event.values)?
                }
            }
            STATIC_CREATE => {
                let pool = self
                    .statics
                    .get_mut(pool)
                    .ok_or("created intel source pool outside retained extent")?;
                allocation::reserve(&mut pool.sources, 1)
                    .map_err(|_| "intel created source storage allocation failed")?;
                pool.sources.push(StaticSource {
                    id: event.source,
                    ..Default::default()
                });
                pool.next_id = event.source;
                true
            }
            #[expect(
                clippy::unnecessary_fold,
                reason = "Every static pool must be pruned; short-circuiting would skip later mutations."
            )]
            STATIC_PRUNE => self.statics.iter_mut().fold(false, |changed, pool| {
                native::prune_sources(pool) || changed
            }),
            STATIC_CLEAR => {
                if let Some(pool) = self.statics.get_mut(pool) {
                    let changed = !pool.sources.is_empty() || pool.next_id != 0;
                    pool.sources.clear();
                    pool.next_id = 0;
                    changed
                } else {
                    false
                }
            }
            DYNAMIC_ADD => {
                let pool = self
                    .dynamics
                    .get_mut(pool)
                    .ok_or("observed intel pending pool outside retained extent")?;
                native::add_pending(pool, event.target, event.values)?
            }
            DYNAMIC_CLEAR => {
                if let Some(state) = self.dynamics.get_mut(pool) {
                    let changed = !state.values.is_empty() || !state.accumulators.is_empty();
                    state.values.clear();
                    state.accumulators.clear();
                    self.accumulators[pool] = false;
                    changed
                } else {
                    false
                }
            }
            DYNAMIC_AGGREGATE => {
                let state = self
                    .dynamics
                    .get_mut(pool)
                    .ok_or("observed intel aggregation pool outside retained extent")?;
                if state.values.is_empty() {
                    false
                } else {
                    state.values.clear();
                    self.accumulators[pool] = true;
                    true
                }
            }
            DYNAMIC_DECAY => {
                let state = self
                    .dynamics
                    .get_mut(pool)
                    .ok_or("observed intel decay pool outside retained extent")?;
                if self.accumulators[pool] {
                    false
                } else {
                    native::decay_accumulators(state, event.values[0], event.values[1])
                }
            }
            ALLIED_CLEAR => {
                let changed = self.allied.count != 0 || !self.allied.entries.is_empty();
                self.allied.count = 0;
                self.allied.entries.clear();
                self.dirty[0] = false;
                changed
            }
            ALLIED_REPLACE => {
                self.dirty[0] = true;
                false
            }
            CRYPTOLOGY_REPLACE => {
                self.dirty[1] = true;
                false
            }
            CONTRIBUTIONS_REPLACE => {
                self.dirty[2] = true;
                self.dirty[3] = true;
                false
            }
            PRIOR_ROTATE => {
                if self.dirty[3] {
                    self.dirty[4] = true;
                    false
                } else {
                    let changed = self.contributions[2] != self.contributions[1];
                    let (current, prior) = self.contributions.split_at_mut(2);
                    prior[0].clone_from(&current[1]);
                    self.dirty[4] = false;
                    changed
                }
            }
            _ => return Err("unknown intel source mutation kind"),
        })
    }

    pub fn acquire(&mut self, memory: &impl Memory) -> Result<bool, &'static str> {
        let mut changed = false;
        if self.references_dirty {
            self.refresh_references(memory)?;
            changed = true;
        }
        if self.dirty[0] {
            native::dense(memory, self.seed.root, native::ALLIED, &mut self.allied)?;
            changed = true;
        }
        if self.dirty[1] {
            native::dense(
                memory,
                self.seed.root,
                native::CRYPTOLOGY,
                &mut self.cryptology,
            )?;
            changed = true;
        }
        for (index, offset) in [
            native::STATIC_CONTRIBUTIONS,
            native::DYNAMIC_CONTRIBUTIONS,
            native::PRIOR_CONTRIBUTIONS,
        ]
        .into_iter()
        .enumerate()
        {
            if self.dirty[index + 2] {
                native::contributions(
                    memory,
                    self.seed.root,
                    offset,
                    &mut self.contributions[index],
                )?;
                changed = true;
            }
        }
        if self.accumulators.iter().any(|dirty| *dirty) {
            let pools = native::array(
                memory,
                self.seed.root,
                native::DYNAMIC_POOLS,
                native::DYNAMIC_POOL_STRIDE,
            )?;
            if pools.count as usize != self.dynamics.len() {
                return Err("intel dynamic membership changed without reset");
            }
            for (index, dirty) in self.accumulators.iter_mut().enumerate() {
                if *dirty {
                    let object = pools
                        .element(index as u32, native::DYNAMIC_POOL_STRIDE)
                        .ok_or("intel selected accumulator address overflow")?;
                    native::keyed(
                        memory,
                        object,
                        native::DYNAMIC_ACCUMULATORS,
                        &mut self.dynamics[index].accumulators,
                    )?;
                    *dirty = false;
                    changed = true;
                }
            }
        }
        self.dirty.fill(false);
        Ok(changed)
    }

    pub fn rows(
        &self,
        contributions: &mut Vec<IntelContributionRow>,
        sources: &mut Vec<IntelSourceRow>,
    ) -> Result<(), &'static str> {
        let owner = self.seed.owner;
        let mut contribution = |kind: u8,
                                pool: u32,
                                ordinal: u32,
                                target: i32,
                                values: [i64; 4]|
         -> Result<(), &'static str> {
            allocation::reserve(contributions, 1)
                .map_err(|_| "intel contribution rows allocation failed")?;
            contributions.push(IntelContributionRow {
                owner,
                kind,
                pool,
                ordinal,
                target,
                civilian: values[0],
                army: values[1],
                navy: values[2],
                air: values[3],
            });
            Ok(())
        };
        for (kind, dense) in [(0, &self.allied), (1, &self.cryptology)] {
            contribution(kind, 0, u32::MAX, dense.count as i32, [0; 4])?;
            for entry in &dense.entries {
                contribution(kind, 0, entry.ordinal, entry.ordinal as i32, entry.values)?;
            }
        }
        for (index, pools) in self.contributions.iter().enumerate() {
            let kind = index as u8 + 2;
            contribution(kind, u32::MAX, u32::MAX, pools.len() as i32, [0; 4])?;
            for (pool, entries) in pools.iter().enumerate() {
                contribution(kind, pool as u32, u32::MAX, entries.len() as i32, [0; 4])?;
                for (ordinal, entry) in entries.iter().enumerate() {
                    contribution(
                        kind,
                        pool as u32,
                        ordinal as u32,
                        entry.target,
                        entry.values,
                    )?;
                }
            }
        }
        for (pool, state) in self.dynamics.iter().enumerate() {
            for (kind, entries) in [(5, &state.values), (6, &state.accumulators)] {
                contribution(kind, pool as u32, u32::MAX, entries.len() as i32, [0; 4])?;
                for (ordinal, entry) in entries.iter().enumerate() {
                    contribution(
                        kind,
                        pool as u32,
                        ordinal as u32,
                        entry.target,
                        entry.values,
                    )?;
                }
            }
        }
        let mut source_row = |mut row: IntelSourceRow| -> Result<(), &'static str> {
            row.owner = owner;
            allocation::reserve(sources, 1)
                .map_err(|_| "intel static source rows allocation failed")?;
            sources.push(row);
            Ok(())
        };
        source_row(IntelSourceRow {
            pool: u32::MAX,
            source: u32::MAX,
            ordinal: u32::MAX,
            count: self.statics.len() as u32,
            other_count: self.dynamics.len() as u32,
            discriminant: self.lifetime,
            reference_country: self.raw_owner,
            ..Default::default()
        })?;
        for (pool, state) in self.statics.iter().enumerate() {
            source_row(IntelSourceRow {
                pool: pool as u32,
                source: u32::MAX,
                ordinal: u32::MAX,
                count: state.sources.len() as u32,
                next_id: state.next_id,
                ..Default::default()
            })?;
            for (source, state) in state.sources.iter().enumerate() {
                source_row(IntelSourceRow {
                    pool: pool as u32,
                    source: source as u32,
                    ordinal: u32::MAX,
                    count: state.entries.len() as u32,
                    id: state.id,
                    removed: state.removed,
                    ..Default::default()
                })?;
                for (ordinal, entry) in state.entries.iter().enumerate() {
                    source_row(IntelSourceRow {
                        pool: pool as u32,
                        source: source as u32,
                        ordinal: ordinal as u32,
                        target: entry.target,
                        civilian: entry.values[0],
                        army: entry.values[1],
                        navy: entry.values[2],
                        air: entry.values[3],
                        ..Default::default()
                    })?;
                }
            }
        }
        for reference in &self.references {
            source_row(reference.row())?;
        }
        Ok(())
    }
}

impl Reference {
    pub fn row(&self) -> IntelSourceRow {
        IntelSourceRow {
            owner: self.owner,
            pool: u32::MAX,
            source: self.kind,
            ordinal: self.target,
            id: self.value.id,
            discriminant: self.value.lifetime,
            reference_country: self.value.country,
            reference_pool: self.value.pool,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoRead;
    impl Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("derived source state read native memory")
        }
    }
    #[test]
    fn allied_daily_clear_derives_empty_after_last_sharing_group_and_cancels_replacement() {
        let mut owner = Owner::default();
        owner.allied = Dense {
            count: 5000,
            entries: vec![native::DenseEntry {
                ordinal: 4999,
                values: [1, 2, 3, 4],
            }],
        };
        owner.cryptology = owner.allied.clone();
        owner
            .apply(Event {
                kind: ALLIED_REPLACE,
                ..Default::default()
            })
            .unwrap();
        crate::test_support::with_allocations_denied(|| {
            assert!(
                owner
                    .apply(Event {
                        kind: ALLIED_CLEAR,
                        ..Default::default()
                    })
                    .unwrap()
            );
            assert!(!owner.acquire(&NoRead).unwrap());
            assert!(
                !owner
                    .apply(Event {
                        kind: ALLIED_CLEAR,
                        ..Default::default()
                    })
                    .unwrap()
            );
        });
        assert_eq!(owner.allied, Dense::default());
        assert_eq!(owner.cryptology.count, 5000);
        let (mut contributions, mut sources) = (Vec::new(), Vec::new());
        owner.rows(&mut contributions, &mut sources).unwrap();
        let allied: Vec<_> = contributions.iter().filter(|row| row.kind == 0).collect();
        assert_eq!(allied.len(), 1);
        assert_eq!((allied[0].ordinal, allied[0].target), (u32::MAX, 0));
    }
    #[test]
    fn intel_sources_reset_reacquires_replaced_token_manager_and_final_held_references() {
        let mut world = engine_native::ccountry::synthetic::World::new(0, 0, 0x200);
        let country = world.alloc(
            (engine_native::ccountry::OPERATION_TOKEN_MANAGER + 8)
                .max(engine_native::ccountry::RADARS_POOL + native::RADAR_REFERENCE + 24)
                .max(engine_native::ccountry::INTEL_TABLE + 8),
        );
        let root = world.alloc(engine_native_intelligence::ccountry_intel::SIZE);
        world.u64(country + engine_native::ccountry::INTEL_TABLE, root as u64);
        let token = world.alloc(
            (native::TOKEN_REFERENCE + 24)
                .max(engine_intelligence::ccountry_operation_token_manager::COUNTRY + 8),
        );
        let radar = country + engine_native::ccountry::RADARS_POOL;
        world.u64(radar, engine_native_parts::cradars_pool::VTABLE_RVA.0 as u64);
        world.u64(
            country + engine_native::ccountry::OPERATION_TOKEN_MANAGER,
            token as u64,
        );
        world.u64(
            token,
            engine_intelligence::ccountry_operation_token_manager::VTABLE_RVA.0 as u64,
        );
        world.u64(
            token + engine_intelligence::ccountry_operation_token_manager::COUNTRY,
            country as u64,
        );
        for (index, reference) in [
            root + native::CHEAT_REFERENCE,
            radar + native::RADAR_REFERENCE,
            token + native::TOKEN_REFERENCE,
        ]
        .into_iter()
        .enumerate()
        {
            world.u64(reference, native::REFERENCE_VTABLE.0 as u64);
            world.u32(reference + 8, 17);
            world.u32(reference + 16, index as u32 + 19);
        }
        let mut owner = Owner::default();
        owner.seed = Seed {
            owner: 17,
            root,
            country,
        };
        owner.references[2].address = 1;
        owner
            .apply(Event {
                kind: RESET,
                ..Default::default()
            })
            .unwrap();
        assert!(owner.acquire(&world.memory()).unwrap());
        assert_eq!(owner.references[2].address, token + native::TOKEN_REFERENCE);
        assert_eq!(
            owner.references.map(|reference| reference.value.id),
            [19, 20, 21]
        );
        assert!(!owner.acquire(&NoRead).unwrap());
    }
    #[test]
    fn intel_sources_ordered_mutations_derive_and_dirty_merge_respects_later_clear() {
        let mut owner = Owner::default();
        owner.statics.push(StaticPool::default());
        owner.dynamics.push(DynamicPool::default());
        owner.accumulators.push(false);
        let event = |kind, source, target, values| Event {
            kind,
            source,
            target,
            values,
            ..Default::default()
        };
        owner.apply(event(STATIC_CREATE, 7, 0, [0; 4])).unwrap();
        owner.apply(event(STATIC_SET, 7, -9, [1, 2, 3, 4])).unwrap();
        assert!(!owner.apply(event(STATIC_SET, 7, -9, [1, 2, 3, 4])).unwrap());
        owner.apply(event(STATIC_CREATE, 8, 0, [0; 4])).unwrap();
        owner.apply(event(STATIC_REMOVE, 7, 0, [0; 4])).unwrap();
        owner.apply(event(STATIC_PRUNE, 0, 0, [0; 4])).unwrap();
        assert_eq!(owner.statics[0].sources.len(), 1);
        assert_eq!(owner.statics[0].sources[0].id, 8);
        owner
            .apply(event(DYNAMIC_ADD, 0, -3, [4, -9, 0, 1]))
            .unwrap();
        assert_eq!(owner.dynamics[0].values[0].values, [4, 0, 0, 1]);
        owner.apply(event(DYNAMIC_AGGREGATE, 0, 0, [0; 4])).unwrap();
        assert!(owner.accumulators[0]);
        owner
            .apply(event(DYNAMIC_DECAY, 0, 0, [1, 50_000, 0, 0]))
            .unwrap();
        owner.apply(event(DYNAMIC_CLEAR, 0, 0, [0; 4])).unwrap();
        assert!(!owner.acquire(&NoRead).unwrap());
        assert!(owner.dynamics[0].accumulators.is_empty());
        assert!(!owner.apply(event(DYNAMIC_AGGREGATE, 0, 0, [0; 4])).unwrap());
        assert!(!owner.acquire(&NoRead).unwrap());
        owner.contributions[1] = vec![vec![Lanes {
            target: -1,
            values: [0; 4],
        }]];
        assert!(owner.apply(event(PRIOR_ROTATE, 0, 0, [0; 4])).unwrap());
        assert_eq!(owner.contributions[2], owner.contributions[1]);
        assert!(!owner.acquire(&NoRead).unwrap());
    }
}
