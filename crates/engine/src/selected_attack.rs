//! Integrated hourly selected-attack operands from native14129F040/141296820.
//! Native selectors, world association and callback lifetime remain acceptance gates.
//! No unit selection, target grid, path cache, UI clock or displayed text.

use crate::{
    Memory, carmy, ccountry, cpdx_array::Array, cprovince, creference_registry,
    creferenced_division_template as template, csub_unit_definition as stats, read, walk::Outcome,
};
use std::{collections::HashMap, mem::size_of};

pub type Field<T> = Result<T, &'static str>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainValue {
    pub terrain: i32,
    pub attack: Field<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainSource {
    pub pointer: usize,
    pub index: i32,
    pub static_attack: Field<i64>,
}

#[derive(Clone, Copy, Debug)]
pub struct StatObservation {
    pub pointer: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct TemplateObservation {
    /// Null wrapper has handle zero; nonnull requires type52 registry equality.
    pub wrapper: usize,
    pub handle: u64,
}
impl TemplateObservation {
    /// B8EBD0 consumes the contained composition, not wrapper+A8.
    pub fn composition(self) -> Field<usize> {
        at(self.wrapper, template::COMPOSITION)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct OriginalObservation {
    pub division: u64,
    pub pointer: usize,
    pub stat: Field<StatObservation>,
    pub template: Field<TemplateObservation>,
}

#[derive(Clone, Copy, Debug)]
pub struct DivisionRow {
    pub division: u64,
    pub sample_hour: i32,
    pub template: Field<u64>,
    /// river390/398; fort368/370; snow3E0; night340, exact fixed5 words.
    pub stats: [Field<i64>; 6],
    pub terrain: Field<Span>,
    pub acclimatization: Field<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Commander {
    pub leader: u64,
    /// fortFB8; riverFE0; snow1030; night270.
    pub scalars: [Field<i64>; 4],
    pub terrain: Field<Span>,
}

#[derive(Clone, Copy, Debug)]
pub struct GroupRow {
    pub group: u64,
    pub sample_hour: i32,
    pub commander: Field<Option<Commander>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitCapacity {
    pub rows: usize,
    pub terrains: usize,
    pub values: usize,
}

#[derive(Clone, Copy, Debug)]
struct StatCache {
    observation: Field<StatObservation>,
    scalars: [Field<i64>; 6],
    header: Field<Array>,
    terrain: Option<Field<Span>>,
}

#[derive(Clone, Copy, Debug)]
struct TemplateCache {
    pointer: usize,
    observation: Field<TemplateObservation>,
}

#[derive(Clone, Copy, Debug)]
struct LeaderCache {
    pointer: usize,
    observed: Field<Commander>,
}

pub struct UnitsPacket {
    rows: Vec<DivisionRow>,
    originals: Vec<OriginalObservation>,
    domain: Vec<TerrainSource>,
    values: Vec<TerrainValue>,
    stat_cache: Vec<StatCache>,
    template_cache: Vec<TemplateCache>,
    stat_indices: HashMap<usize, usize>,
    domain_hour: Option<i32>,
    outcome: Outcome,
    domain_result: Field<()>,
    required: UnitCapacity,
    null_terrains: usize,
    growing: bool,
}

const UNCAPTURED: Outcome = Outcome::Unavailable {
    reason: "selected attack inputs not acquired",
};
const UNSUPPLIED: &str = "selected attack selecting observation unavailable";

fn at(pointer: usize, offset: usize) -> Field<usize> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("selected attack pointer null or overflow")
}
fn field<T: crate::memory::Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Field<T> {
    read(memory, at(pointer, offset)?).ok_or("selected attack field unreadable")
}
fn expected(base: usize, rva: usize) -> Field<u64> {
    base.checked_add(rva)
        .map(|value| value as u64)
        .ok_or("selected attack module address overflow")
}
fn reserve<T>(values: &mut Vec<T>, count: usize) -> std::io::Result<()> {
    if values.capacity() < count {
        crate::allocation::reserve_exact(values, count.saturating_sub(values.len()))?;
    }
    Ok(())
}
fn admitted<T>(values: &mut Vec<T>, value: T) -> Field<()> {
    if values.len() == values.capacity() {
        return Err("selected attack arena capacity exhausted");
    }
    values.push(value);
    Ok(())
}

pub fn authenticate_original(seed: &carmy::DivisionSeed, base: usize) -> Field<()> {
    if seed.pointer == 0
        || seed.division == 0
        || seed.handle != Some(seed.division)
        || seed.primary_vtable != Some(expected(base, carmy::VTABLE_RVA)?)
        || seed.secondary_vtable != Some(expected(base, carmy::SECONDARY_VTABLE_RVA)?)
        || seed.class != Some(0)
    {
        return Err("selected attack original class0 seed identity unavailable");
    }
    Ok(())
}

impl UnitsPacket {
    pub fn vacant() -> Self {
        Self {
            rows: Vec::new(),
            originals: Vec::new(),
            domain: Vec::new(),
            values: Vec::new(),
            stat_cache: Vec::new(),
            template_cache: Vec::new(),
            stat_indices: HashMap::new(),
            domain_hour: None,
            outcome: UNCAPTURED,
            domain_result: Err(UNSUPPLIED),
            required: UnitCapacity::default(),
            null_terrains: 0,
            growing: false,
        }
    }
    /// Worker-only growth continues from original census and terrain controls.
    pub fn enable_growth(&mut self) {
        self.growing = true;
    }
    pub fn try_new(capacity: UnitCapacity) -> std::io::Result<Self> {
        let mut packet = Self::vacant();
        packet.reserve(capacity)?;
        Ok(packet)
    }
    pub fn reserve(&mut self, capacity: UnitCapacity) -> std::io::Result<()> {
        reserve(&mut self.rows, capacity.rows)?;
        reserve(&mut self.originals, capacity.rows)?;
        reserve(&mut self.stat_cache, capacity.rows)?;
        reserve(&mut self.template_cache, capacity.rows)?;
        let additional = capacity.rows.saturating_sub(self.stat_indices.len());
        crate::allocation::reserve_map(&mut self.stat_indices, additional)?;
        reserve(&mut self.domain, capacity.terrains)?;
        reserve(&mut self.values, capacity.values)
    }
    pub fn clear(&mut self) {
        self.rows.clear();
        self.originals.clear();
        self.domain.clear();
        self.values.clear();
        self.stat_cache.clear();
        self.template_cache.clear();
        self.stat_indices.clear();
        self.domain_hour = None;
        self.outcome = UNCAPTURED;
        self.domain_result = Err(UNSUPPLIED);
        self.required = UnitCapacity::default();
        self.null_terrains = 0;
    }
    /// End the native lifetime, after all borrowers (including transport) join.
    /// Scalar results/spans survive. This never re-authenticates or follows pointers.
    pub fn finish_native(&mut self) {
        self.originals.clear();
        self.domain.clear();
        self.stat_cache.clear();
        self.template_cache.clear();
        self.stat_indices.clear();
        self.domain_hour = None;
    }
    pub fn fail(&mut self, reason: &'static str) {
        // An interrupted unit read cannot authorize retry of an attempted
        // shared domain. Partial native selectors/results remain current
        // refusal controls until final native retirement.
        self.rows.clear();
        self.originals.clear();
        self.values.clear();
        self.stat_cache.clear();
        self.template_cache.clear();
        self.stat_indices.clear();
        self.outcome = Outcome::Unavailable { reason };
    }
    pub fn refuse_domain_at(&mut self, hour: i32, reason: &'static str) {
        self.domain_hour = Some(hour);
        self.domain_result = Err(reason);
    }
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    pub fn required(&self) -> UnitCapacity {
        self.required
    }
    pub fn null_terrain_selectors(&self) -> usize {
        self.null_terrains
    }
    pub fn rows(&self) -> &[DivisionRow] {
        &self.rows
    }
    pub fn originals(&self) -> &[OriginalObservation] {
        &self.originals
    }
    pub fn values(&self, span: Span) -> Option<&[TerrainValue]> {
        span.start
            .checked_add(span.len)
            .and_then(|end| self.values.get(span.start..end))
    }
    pub fn original(&self, division: u64, pointer: usize) -> Option<&OriginalObservation> {
        self.originals
            .binary_search_by_key(&division, |row| row.division)
            .ok()
            .and_then(|index| self.originals.get(index))
            .filter(|row| row.pointer == pointer)
    }
    pub fn acclimatization(&self, division: u64) -> Option<Field<i64>> {
        self.rows
            .binary_search_by_key(&division, |row| row.division)
            .ok()
            .and_then(|index| self.rows.get(index))
            .map(|row| row.acclimatization)
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<DivisionRow>()
            + self.originals.capacity() * size_of::<OriginalObservation>()
            + self.domain.capacity() * size_of::<TerrainSource>()
            + self.values.capacity() * size_of::<TerrainValue>()
            + self.stat_cache.capacity() * size_of::<StatCache>()
            + self.template_cache.capacity() * size_of::<TemplateCache>()
            + self.stat_indices.capacity() * (size_of::<(usize, usize)>() + 1)
    }
    fn capture_domain(
        &mut self,
        memory: &impl Memory,
        base: usize,
        geometry: &mut cprovince::GeometryPacket,
    ) -> Field<()> {
        geometry.ready()?;
        let count = geometry.province_count()?;
        // A full province-domain upper bound survives a short dedup arena.
        self.required.terrains = count;
        if self.growing {
            reserve(&mut self.domain, count).map_err(|_| "selected attack allocation failed")?;
        }
        if self.domain.capacity() < count {
            return Err("selected attack terrain domain arena exhausted");
        }
        for id in 1..count {
            let id = u32::try_from(id).map_err(|_| "selected attack province ID overflow")?;
            if geometry.acquire_route_pointer(memory, id)? == 0 {
                continue;
            }
            let pointer = geometry.acquire_terrain(memory, id)?;
            if pointer == 0 {
                self.null_terrains += 1;
                continue;
            } // Observed null definition selector.
            if self.domain.iter().any(|terrain| terrain.pointer == pointer) {
                continue;
            }
            let vtable = field::<u64>(memory, pointer, 0)?;
            let index = field::<i32>(memory, pointer, crate::cterrain_type::DATABASE_INDEX)?;
            if vtable == expected(base, crate::cterrain_type::NULL_VTABLE_RVA)? && index == 0 {
                // The geometry controls retain this known-null selector. The
                // replay's positive one-based domain cannot fabricate its -1
                // cache entry. Consumer terrain0 refuses the unsupported leaf.
                self.null_terrains += 1;
                continue;
            } else if vtable != expected(base, crate::cterrain_type::VTABLE_RVA)? || index <= 0 {
                return Err("selected attack terrain domain identity invalid");
            }
            if self.domain.iter().any(|terrain| terrain.index == index) {
                return Err("selected attack terrain index identifies multiple objects");
            }
            let static_attack = (|| {
                if field::<u64>(memory, pointer, 0x168)? != expected(base, 0x0277_3a70)? {
                    return Err("selected attack static terrain adjuster identity mismatch");
                }
                field(memory, pointer, 0x178)
            })();
            admitted(
                &mut self.domain,
                TerrainSource {
                    pointer,
                    index,
                    static_attack,
                },
            )?;
        }
        self.domain.sort_unstable_by_key(|row| row.index);
        Ok(())
    }
    fn template(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: Field<usize>,
        resolver: &mut creference_registry::ScopedResolver,
    ) -> Field<TemplateObservation> {
        let pointer = pointer?;
        if pointer == 0 {
            return Ok(TemplateObservation {
                wrapper: 0,
                handle: 0,
            });
        }
        if let Some(entry) = self
            .template_cache
            .iter()
            .find(|entry| entry.pointer == pointer)
        {
            return entry.observation;
        }
        let observed = (|| {
            if field::<u64>(memory, pointer, 0)? != expected(base, template::VTABLE_RVA)? {
                return Err("selected attack template vtable mismatch");
            }
            let handle = field::<u64>(memory, pointer, template::TYPE)?;
            if handle >> 32 == 0 || handle as u32 != template::REFERENCE_TYPE {
                return Err("selected attack template full handle invalid");
            }
            if resolver.resolve(memory, handle)? != Some(pointer) {
                return Err("selected attack template registry does not identify original wrapper");
            }
            Ok(TemplateObservation {
                wrapper: pointer,
                handle,
            })
        })();
        admitted(
            &mut self.template_cache,
            TemplateCache {
                pointer,
                observation: observed,
            },
        )?;
        observed
    }
    fn stat(&mut self, memory: &impl Memory, base: usize, pointer: Field<usize>) -> Field<usize> {
        let pointer = pointer?;
        if let Some(&index) = self.stat_indices.get(&pointer) {
            return Ok(index);
        }
        if self.stat_indices.len() == self.stat_indices.capacity() {
            return Err("selected attack stat index capacity exhausted");
        }
        let observation = (|| {
            if field::<u64>(memory, pointer, 0)? != expected(base, stats::VTABLE_RVA)? {
                return Err("selected attack original stat block identity mismatch");
            }
            Ok(StatObservation { pointer })
        })();
        let scalars = [0x390, 0x398, 0x368, 0x370, 0x3e0, 0x340]
            .map(|offset| observation.and_then(|_| field(memory, pointer, offset)));
        let header = observation.and_then(|_| {
            Array::read(
                memory,
                at(pointer, stats::MOVEMENT_TERRAIN_ADJUSTERS)?,
                0x28,
            )
            .ok_or("selected attack stat terrain header invalid")
        });
        let index = self.stat_cache.len();
        admitted(
            &mut self.stat_cache,
            StatCache {
                observation,
                scalars,
                header,
                terrain: None,
            },
        )?;
        self.stat_indices.insert(pointer, index);
        Ok(index)
    }
    fn stat_terrain(&mut self, memory: &impl Memory, base: usize, stat: usize) -> Field<Span> {
        if let Some(observed) = self.stat_cache[stat].terrain {
            return observed;
        }
        let result = (|| {
            self.domain_result?;
            self.stat_cache[stat].observation?;
            let array = self.stat_cache[stat].header?;
            let start = self.values.len();
            self.required.values = self
                .required
                .values
                .checked_add(self.domain.len())
                .ok_or("selected attack terrain value demand overflow")?;
            if self.growing {
                reserve(
                    &mut self.values,
                    start
                        .checked_add(self.domain.len())
                        .ok_or("selected attack terrain value demand overflow")?,
                )
                .map_err(|_| "selected attack allocation failed")?;
            }
            if self.domain.len() > self.values.capacity().saturating_sub(start) {
                return Err("selected attack stat terrain value arena exhausted");
            }
            for terrain in &self.domain {
                let value = attack_terrain_from_controls(memory, base, array, *terrain);
                self.values.push(TerrainValue {
                    terrain: terrain.index,
                    attack: value,
                });
            }
            Ok(Span {
                start,
                len: self.domain.len(),
            })
        })();
        self.stat_cache[stat].terrain = Some(result);
        result
    }
    /// The domain has one owner and one current-hour attempt, independent
    /// of either unit or commander availability. Failures are memoized too.
    fn ensure_domain(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        geometry: &mut cprovince::GeometryPacket,
        current_geometry: Field<()>,
    ) {
        if self.domain_hour == Some(hour) {
            return;
        }
        self.domain_hour = Some(hour);
        self.domain_result = Err("selected attack domain acquisition interrupted");
        self.domain_result =
            current_geometry.and_then(|_| self.capture_domain(memory, base, geometry));
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        seeds: &[carmy::DivisionSeed],
        source: Outcome,
        geometry: &mut cprovince::GeometryPacket,
    ) {
        self.clear();
        self.required.rows = source.len();
        let result = (|| {
            let count = match source {
                Outcome::Rows { len } => len,
                Outcome::Truncated { .. } => {
                    return Err("selected attack division census incomplete");
                }
                Outcome::Unavailable { reason } => return Err(reason),
            };
            let seeds = seeds
                .get(..count)
                .ok_or("selected attack division census length invalid")?;
            // No selected attack stat/leader consumes a terrain domain when
            // the complete original census is explicitly empty.
            if count == 0 {
                return Ok(Outcome::Rows { len: 0 });
            }
            // 14129F040 visits the complete owned class0 census; the terrain
            // domain comes from this pass's original province root.
            if self.growing {
                self.reserve(UnitCapacity {
                    rows: count,
                    terrains: self.domain.len(),
                    values: self.values.len(),
                })
                .map_err(|_| "selected attack allocation failed")?;
            }
            if count > self.rows.capacity()
                || count > self.originals.capacity()
                || count > self.stat_cache.capacity()
                || count > self.template_cache.capacity()
                || count > self.stat_indices.capacity()
            {
                return Err("selected attack original census arena exhausted");
            }
            self.ensure_domain(memory, base, hour, geometry, Ok(()));
            let mut resolver = creference_registry::ScopedResolver::new(base);
            for seed in seeds {
                authenticate_original(seed, base)?;
                let stat = self.stat(
                    memory,
                    base,
                    seed.stat_block
                        .ok_or("selected attack stat selector unobserved"),
                );
                let template = self.template(
                    memory,
                    base,
                    seed.standard_template
                        .ok_or("selected attack template selector unobserved"),
                    &mut resolver,
                );
                let observation = stat.and_then(|index| self.stat_cache[index].observation);
                self.originals.push(OriginalObservation {
                    division: seed.division,
                    pointer: seed.pointer,
                    stat: observation,
                    template,
                });
                let scalars = stat
                    .map(|index| self.stat_cache[index].scalars)
                    .unwrap_or_else(|reason| [Err(reason); 6]);
                let terrain = stat.and_then(|index| self.stat_terrain(memory, base, index));
                let acclimatization =
                    crate::carmy_attrition::fresh_acclimatization(memory, base, seed);
                self.rows.push(DivisionRow {
                    division: seed.division,
                    sample_hour: hour,
                    template: template.map(|value| value.handle),
                    stats: scalars,
                    terrain,
                    acclimatization,
                });
            }
            Ok(Outcome::Rows { len: count })
        })();
        // Sort every completed prefix, including failures, before attrition
        // borrows it; a missed prefix must not authorize a native reread.
        self.rows.sort_unstable_by_key(|row| row.division);
        self.originals.sort_unstable_by_key(|row| row.division);
        self.outcome = result
            .and_then(|outcome| {
                if self
                    .rows
                    .windows(2)
                    .any(|pair| pair[0].division == pair[1].division)
                {
                    Err("selected attack original identity duplicated")
                } else {
                    Ok(outcome)
                }
            })
            .unwrap_or_else(|reason| Outcome::Unavailable { reason });
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GroupCapacity {
    pub rows: usize,
    pub values: usize,
}

/// Independent current orders-group inputs, including groups with no units.
/// The current +88 observation/identity belongs solely to the counter owner.
pub struct GroupsPacket {
    rows: Vec<GroupRow>,
    leaders: Vec<LeaderCache>,
    values: Vec<TerrainValue>,
    outcome: Outcome,
    required: GroupCapacity,
    growing: bool,
}
impl GroupsPacket {
    pub fn fail(&mut self, reason: &'static str) {
        self.clear();
        self.outcome = Outcome::Unavailable { reason };
    }
    pub fn vacant() -> Self {
        Self {
            rows: Vec::new(),
            leaders: Vec::new(),
            values: Vec::new(),
            outcome: UNCAPTURED,
            required: GroupCapacity::default(),
            growing: false,
        }
    }
    pub fn enable_growth(&mut self) {
        self.growing = true;
    }
    pub fn try_new(capacity: GroupCapacity) -> std::io::Result<Self> {
        let mut packet = Self::vacant();
        packet.reserve(capacity)?;
        Ok(packet)
    }
    pub fn reserve(&mut self, capacity: GroupCapacity) -> std::io::Result<()> {
        reserve(&mut self.rows, capacity.rows)?;
        reserve(&mut self.leaders, capacity.rows)?;
        reserve(&mut self.values, capacity.values)
    }
    pub fn clear(&mut self) {
        self.rows.clear();
        self.leaders.clear();
        self.values.clear();
        self.outcome = UNCAPTURED;
        self.required = GroupCapacity::default();
    }
    pub fn finish_native(&mut self) {
        self.leaders.clear();
    }
    pub fn rows(&self) -> &[GroupRow] {
        &self.rows
    }
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    pub fn required(&self) -> GroupCapacity {
        self.required
    }
    pub fn values(&self, span: Span) -> Option<&[TerrainValue]> {
        span.start
            .checked_add(span.len)
            .and_then(|end| self.values.get(span.start..end))
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<GroupRow>()
            + self.leaders.capacity() * size_of::<LeaderCache>()
            + self.values.capacity() * size_of::<TerrainValue>()
    }
    fn leader(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        leader: u64,
        domain: &UnitsPacket,
    ) -> Field<Commander> {
        if let Some(entry) = self.leaders.iter().find(|entry| entry.pointer == pointer) {
            return entry.observed;
        }
        let observed = {
            let scalars =
                [0xfb8, 0xfe0, 0x1030, 0x270].map(|offset| field(memory, pointer, offset));
            let terrain = (|| {
                domain.domain_result?;
                let array = Array::read(
                    memory,
                    at(pointer, crate::carmy_leader::MOVEMENT_TERRAIN_DATA)?,
                    0x28,
                )
                .ok_or("selected attack commander terrain header invalid")?;
                if array.data == 0 {
                    return Err("selected attack commander terrain data null");
                }
                let start = self.values.len();
                self.required.values = self
                    .required
                    .values
                    .checked_add(domain.domain.len())
                    .ok_or("selected attack commander terrain demand overflow")?;
                if self.growing {
                    reserve(
                        &mut self.values,
                        start
                            .checked_add(domain.domain.len())
                            .ok_or("selected attack commander terrain demand overflow")?,
                    )
                    .map_err(|_| "selected attack allocation failed")?;
                }
                if domain.domain.len() > self.values.capacity().saturating_sub(start) {
                    return Err("selected attack commander terrain value arena exhausted");
                }
                let mut zero_entry = None;
                for terrain in &domain.domain {
                    let index = if terrain.index >= 0 && (terrain.index as u32) < array.count {
                        terrain.index as usize
                    } else {
                        0
                    };
                    let read = || {
                        index
                            .checked_mul(0x28)
                            .ok_or("selected attack commander terrain index overflow")
                            .and_then(|offset| at(array.data, offset))
                            .and_then(|entry| field(memory, entry, 0x10))
                    };
                    let value = if index == 0 {
                        *zero_entry.get_or_insert_with(read)
                    } else {
                        read()
                    };
                    self.values.push(TerrainValue {
                        terrain: terrain.index,
                        attack: value,
                    });
                }
                Ok(Span {
                    start,
                    len: domain.domain.len(),
                })
            })();
            Ok(Commander {
                leader,
                scalars,
                terrain,
            })
        };
        admitted(&mut self.leaders, LeaderCache { pointer, observed })?;
        observed
    }
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        controls: &[crate::corders_group::DirectCommander],
        source: Outcome,
        domain: &mut UnitsPacket,
        geometry: &mut cprovince::GeometryPacket,
        current_geometry: Field<()>,
    ) {
        self.clear();
        self.required.rows = source.len();
        let result = (|| {
            let count = match source {
                Outcome::Rows { len } => len,
                Outcome::Truncated { .. } => return Err("selected attack group census incomplete"),
                Outcome::Unavailable { reason } => return Err(reason),
            };
            let controls = controls
                .get(..count)
                .ok_or("selected attack group controls incomplete")?;
            if self.growing {
                self.reserve(GroupCapacity {
                    rows: count,
                    values: self.values.len(),
                })
                .map_err(|_| "selected attack allocation failed")?;
            }
            if count > self.rows.capacity() || count > self.leaders.capacity() {
                return Err("selected attack group arena exhausted");
            }
            for control in controls {
                if control.group >> 32 == 0
                    || control.group as u32 != crate::corders_group::REFERENCE_TYPE
                {
                    return Err("selected attack group control identity unavailable");
                }
                let commander = control.leader.and_then(|leader| match leader {
                    None => Ok(None),
                    Some(leader) => {
                        let pointer = control.pointer?;
                        domain.ensure_domain(memory, base, hour, geometry, current_geometry);
                        self.leader(memory, pointer, leader, domain).map(Some)
                    }
                });
                self.rows.push(GroupRow {
                    group: control.group,
                    sample_hour: hour,
                    commander,
                });
            }
            Ok(Outcome::Rows { len: count })
        })();
        self.rows.sort_unstable_by_key(|row| row.group);
        self.outcome = result
            .and_then(|outcome| {
                if self
                    .rows
                    .windows(2)
                    .any(|pair| pair[0].group == pair[1].group)
                {
                    Err("selected attack group identity duplicated")
                } else {
                    Ok(outcome)
                }
            })
            .unwrap_or_else(|reason| Outcome::Unavailable { reason });
    }
}

/// Numeric141414F90 +140FFF550 only. Cached token is NOT demanded.
fn attack_terrain_from_controls(
    memory: &impl Memory,
    base: usize,
    array: Array,
    terrain: TerrainSource,
) -> Field<i64> {
    let index = u32::try_from(terrain.index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .ok_or("selected attack null or negative terrain cache index unsupported")?;
    let entry = array
        .element(index, 0x28)
        .ok_or("selected attack terrain outside stat cache")?;
    let vtable = expected(base, 0x0277_3a70)?;
    if field::<u64>(memory, entry, 0)? != vtable {
        return Err("selected attack terrain adjuster identity mismatch");
    }
    let cached = field::<i64>(memory, entry, 0x10)?;
    let static_attack = terrain.static_attack?;
    Ok(cached.wrapping_add(static_attack))
}

mod map;
pub use map::{CountryRow, MapCapacity, MapPacket, ProvinceRow, ScalarMetadata, WeatherAttack};

#[cfg(test)]
mod tests;
