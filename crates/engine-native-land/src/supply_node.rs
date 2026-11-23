pub use engine_native_parts::supply_node_layout::{STRIDE, PROVINCE, MOTORIZATION_OVERRIDES, KIND_SUPPLY_HUB, KIND_NAVAL_BASE};

use engine_sites::Rva;
pub const INDEX: usize = 0;
pub const KIND: usize = 0x08;
pub const FLAG: usize = 0x0c;
pub const AVAILABLE: usize = 0x38;
pub const CONSUMED: usize = 0x40;
pub const ROUTE_VALUE: usize = 0x78;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x012a_6a60);

use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use engine_native::ccountry_supply_system::world as csupply_system;
use engine_native_parts::supply_companion;
use crate::{supply_parent_map, supply_route};

pub const VERTEX: u32 = 1 << 31;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub index: u32,
    pub province: u32,
    pub kind: u32,
    pub flag: u8,
    pub available: i64,
    pub consumed: i64,
    pub parent: i32,
    pub route: supply_route::Row,
}
pub struct Report {
    pub outcome: Outcome,
    pub paths: usize,
    pub required_paths: usize,
    pub null_provinces: u32,
    pub last_null_province: Option<ProvinceObservation>,
    pub refusal: Option<ProvinceObservation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvinceCheck {
    CensusUnavailable,
    OutsideCensus,
    SlotUnreadable,
    SlotEmpty,
    IdentityMismatch,
}

impl ProvinceCheck {
    pub const ALL: [Self; 5] = [
        Self::CensusUnavailable,
        Self::OutsideCensus,
        Self::SlotUnreadable,
        Self::SlotEmpty,
        Self::IdentityMismatch,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::CensusUnavailable => "census_unavailable",
            Self::OutsideCensus => "outside_census",
            Self::SlotUnreadable => "slot_unreadable",
            Self::SlotEmpty => "slot_empty",
            Self::IdentityMismatch => "identity_mismatch",
        }
    }

    pub const fn is_null_province(self) -> bool {
        matches!(self, Self::OutsideCensus | Self::SlotEmpty)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvinceRole {
    Node,
    Parent,
    Vertex,
}

impl ProvinceRole {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Parent => "parent",
            Self::Vertex => "vertex",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProvinceObservation {
    pub country: u32,
    pub index: u32,
    pub province: u32,
    pub role: ProvinceRole,
    pub check: ProvinceCheck,
}

const fn refusal_reason(role: ProvinceRole, check: ProvinceCheck) -> &'static str {
    use ProvinceCheck as C;
    use ProvinceRole as R;
    match (role, check) {
        (R::Node, C::CensusUnavailable) => "supply node province census unavailable",
        (R::Node, C::SlotUnreadable) => "supply node province slot unreadable",
        (R::Node, C::IdentityMismatch) => "supply node province identity mismatch",
        (R::Parent, C::CensusUnavailable) => "supply parent province census unavailable",
        (R::Parent, C::SlotUnreadable) => "supply parent province slot unreadable",
        (R::Parent, C::IdentityMismatch) => "supply parent province identity mismatch",
        (R::Vertex, C::CensusUnavailable) => "supply vertex province census unavailable",
        (R::Vertex, C::SlotUnreadable) => "supply vertex province slot unreadable",
        (R::Vertex, C::IdentityMismatch) => "supply vertex province identity mismatch",
        (_, C::OutsideCensus | C::SlotEmpty) => "supply province held as null",
    }
}

#[derive(Default)]
struct Observations {
    null_provinces: u32,
    last_null_province: Option<ProvinceObservation>,
    refusal: Option<ProvinceObservation>,
}

impl Observations {
    #[allow(clippy::too_many_arguments)]
    fn check(
        &mut self,
        memo: &mut ProvinceMemo,
        memory: &impl Memory,
        base: usize,
        role: ProvinceRole,
        country: u32,
        index: u32,
        province: u32,
    ) -> Result<(), &'static str> {
        let Err(check) = memo.province(memory, base, province)? else {
            return Ok(());
        };
        let observed = ProvinceObservation {
            country,
            index,
            province,
            role,
            check,
        };
        if check.is_null_province() {
            self.null_provinces = self.null_provinces.saturating_add(1);
            self.last_null_province = Some(observed);
            Ok(())
        } else {
            self.refusal = Some(observed);
            Err(refusal_reason(role, check))
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn parent(
        &mut self,
        memo: &mut ProvinceMemo,
        memory: &impl Memory,
        base: usize,
        country: u32,
        index: u32,
        parent: i32,
    ) -> Result<(), &'static str> {
        if parent < 1 {
            return Ok(());
        }
        self.check(
            memo,
            memory,
            base,
            ProvinceRole::Parent,
            country,
            index,
            parent as u32,
        )
    }
}

struct Edges<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    root: usize,
    outer: Option<crate::supply_parent_map::Map>,
    provinces: Array,
    paths: engine_native_memory::walk::Buffer<'a, u32>,
    used: &'a mut usize,
    required: &'a mut usize,
}
impl<M: Memory> Edges<'_, M> {
    fn read(
        &mut self,
        parent: i32,
        province: u32,
        memo: &mut supply_route::Memo,
    ) -> Result<Option<supply_route::Row>, &'static str> {
        if parent < 1 {
            return Ok(Some(supply_route::Row::default()));
        }
        let used = *self.used;
        let (cached, slot) = memo.find(parent as u32, province);
        if let Some(mut route) = cached {
            *self.required = self
                .required
                .checked_add(route.len)
                .ok_or("supply path demand overflow")?;
            self.paths.ensure(*self.required)?;
            if self.paths.len() - used < route.len {
                return Ok(None);
            }
            self.paths
                .copy_within(route.start..route.start + route.len, used);
            route.start = used;
            *self.used += route.len;
            return Ok(Some(route));
        }
        let outer = match self.outer {
            Some(outer) => outer,
            None => {
                let outer = supply_route::read_outer(self.memory, self.root)?;
                self.outer = Some(outer);
                outer
            }
        };
        let (mut route, need) = supply_route::read_edge_buffer(
            self.memory,
            self.base,
            &outer,
            &mut memo.inner,
            parent as u32,
            province,
            self.paths.suffix(used),
        )?;
        *self.required = self
            .required
            .checked_add(need)
            .ok_or("supply path demand overflow")?;
        if route.len != need {
            return Ok(None);
        }
        route.start = used;
        if self.paths[used..used + need]
            .iter()
            .any(|p| *p == 0 || *p >= self.provinces.count)
        {
            return Err("supply route province outside census");
        }
        *self.used += need;
        memo.insert(slot, parent as u32, province, route)?;
        Ok(Some(route))
    }
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<Row>,
    paths: &mut Vec<u32>,
) -> Report {
    let mut provinces = ProvinceMemo::default();
    provinces.begin(memory, base, state);
    walk_part(memory, base, state, 0, 1, output, paths, &mut provinces)
}

pub fn storage_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<usize, &'static str> {
    let systems = csupply_system::Reader::new(memory, base, state)?;
    let mut required = 0_usize;
    for slot in 1..systems.count() {
        let Some(system) = systems.at(slot)? else {
            continue;
        };
        let nodes = system
            .companion
            .checked_add(supply_companion::NODES)
            .and_then(|at| Array::read(memory, at, STRIDE))
            .ok_or("supply nodes array invalid")?;
        required = required
            .checked_add(nodes.count as usize)
            .ok_or("supply node demand overflow")?;
    }
    Ok(required)
}

#[derive(Default)]
pub struct ProvinceMemo {
    key: Option<(usize, usize, i64, [u8; 16])>,
    provinces: Option<Array>,
    pointers: Vec<usize>,
    vertices: Vec<u64>,
    worker: bool,
    routes: supply_route::Memo,
}

impl ProvinceMemo {
    pub fn with_scratch() -> std::io::Result<Self> {
        Ok(Self {
            worker: true,
            routes: supply_route::Memo::try_with_scratch()?,
            ..Self::default()
        })
    }

    pub fn begin(&mut self, memory: &impl Memory, base: usize, state: usize) {
        let root = state
            .checked_add(engine_native::cgame_state::PROVINCES)
            .and_then(|address| read::<[u8; 16]>(memory, address));
        let session = engine_sites::at(base, engine_native::cgame_state::PLAY_SESSION_STARTED_AT_RVA)
            .and_then(|address| read::<i64>(memory, address));
        let key = root
            .zip(session)
            .map(|(root, session)| (base, state, session, root));
        if key.is_none() || key != self.key {
            self.pointers.clear();
        }
        self.key = key;
        self.provinces = root.and_then(|root| Array::from_header(root, 8));
    }

    pub fn allocated_bytes(&self) -> usize {
        self.pointers.capacity() * std::mem::size_of::<usize>()
            + self.vertices.capacity() * std::mem::size_of::<u64>()
            + self.routes.allocated_bytes()
    }

    fn province(
        &mut self,
        memory: &impl Memory,
        base: usize,
        province: u32,
    ) -> Result<Result<usize, ProvinceCheck>, &'static str> {
        if let Some(&pointer) = self.pointers.get(province as usize)
            && pointer != 0
        {
            return Ok(Ok(pointer));
        }
        let Some(provinces) = self.provinces else {
            return Ok(Err(ProvinceCheck::CensusUnavailable));
        };
        let pointer = match validate_province(memory, base, provinces, province) {
            Ok(pointer) => pointer,
            Err(check) => return Ok(Err(check)),
        };
        if self.key.is_some() {
            if self.pointers.len() <= province as usize {
                engine_allocation::resize(&mut self.pointers, province as usize + 1, 0)
                    .map_err(|_| "supply province allocation failed")?;
            }
            self.pointers[province as usize] = pointer;
        }
        Ok(Ok(pointer))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn walk_part(
    memory: &impl Memory,
    base: usize,
    state: usize,
    part: usize,
    parts: usize,
    output: &mut Vec<Row>,
    paths: &mut Vec<u32>,
    provinces: &mut ProvinceMemo,
) -> Report {
    walk_part_buffer(
        memory,
        base,
        state,
        part,
        parts,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(paths),
        provinces,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn walk_part_selected(
    memory: &impl Memory,
    base: usize,
    state: usize,
    part: usize,
    parts: usize,
    output: &mut Vec<Row>,
    paths: &mut Vec<u32>,
    provinces: &mut ProvinceMemo,
    select: &dyn Fn(u32) -> bool,
) -> Report {
    walk_part_buffer(
        memory,
        base,
        state,
        part,
        parts,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(paths),
        provinces,
        Some(select),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn walk_part_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    part: usize,
    parts: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    paths: engine_native_memory::walk::Buffer<'_, u32>,
    provinces: &mut ProvinceMemo,
    select: Option<&dyn Fn(u32) -> bool>,
) -> Report {
    let mut used = 0;
    let mut required = 0;
    let mut observed = Observations::default();
    provinces.routes.begin();
    let result = (|| {
        use engine_native_memory::memory::{RecordField, RecordLayout, RecordSources};
        const FIELDS: [RecordField; 2] = [
            RecordField {
                source_offset: INDEX,
                destination_offset: 0,
                len: FLAG + 1,
            },
            RecordField {
                source_offset: AVAILABLE,
                destination_offset: 13,
                len: 16,
            },
        ];
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&FIELDS, 29).expect("valid supply node layout");
        let systems = csupply_system::Reader::new(memory, base, state)?;
        let root = csupply_system::read_root(memory, base, state)?;
        let census = Array::read(memory, state + engine_native::cgame_state::PROVINCES, 8)
            .ok_or("supply provinces invalid")?;
        if provinces.worker {
            engine_allocation::resize(
                &mut provinces.vertices,
                (census.count as usize).div_ceil(64),
                0,
            )
            .map_err(|_| "supply vertex memo allocation failed")?;
        }
        let mut edges = Edges {
            memory,
            base,
            root,
            outer: None,
            provinces: census,
            paths,
            used: &mut used,
            required: &mut required,
        };
        let mut len = 0_usize;
        let mut total = 0_usize;
        let mut cut = 0;
        let mut bytes = [0_u8; 64 * 29];
        for slot in (1 + part as u32..systems.count()).step_by(parts) {
            if let Some(select) = select
                && !systems
                    .country(slot)?
                    .is_some_and(|country| select(country.id))
            {
                continue;
            }
            let Some(system) = systems.at(slot)? else {
                continue;
            };
            let nodes_at = system.companion + supply_companion::NODES;
            let nodes =
                Array::read(memory, nodes_at, STRIDE).ok_or("supply nodes array invalid")?;
            total = total
                .checked_add(nodes.count as usize)
                .ok_or("supply node census overflows")?;
            output.ensure(
                len.checked_add(nodes.count as usize)
                    .ok_or("supply node census overflows")?,
            )?;
            if nodes.count == 0 {
                continue;
            }
            let parents = supply_parent_map::Map::read(
                memory,
                system.companion + supply_parent_map::HEADER,
                supply_parent_map::STRIDE,
                0,
            )?;
            let start = len;
            let mut first = 0;
            while first < nodes.count && len < output.len() {
                let count = (nodes.count - first).min(64) as usize;
                let count = count.min(output.len() - len);
                if memory.copy_records(
                    RecordSources::Strided {
                        base: nodes
                            .element(first, STRIDE)
                            .ok_or("supply node address overflow")?,
                        stride: STRIDE,
                        count,
                    },
                    &LAYOUT,
                    &mut bytes,
                ) != count * FIELDS.len()
                {
                    return Err("supply node fields unreadable");
                }
                let mut hinted = 0;
                for i in 0..count {
                    while hinted < (i + 4).min(count) {
                        let next = &bytes[hinted * 29..(hinted + 1) * 29];
                        let index = u32::from_le_bytes(next[..4].try_into().unwrap());
                        let province = u32::from_le_bytes(next[4..8].try_into().unwrap());
                        if index == first + hinted as u32
                            && province != 0
                            && let Some(slot) = census.element(province, 8)
                        {
                            engine_native_memory::prefetch::span(slot, 8);
                            parents.hint(province);
                        }
                        hinted += 1;
                    }
                    let b = &bytes[i * 29..(i + 1) * 29];
                    let word = |n| u32::from_le_bytes(b[n..n + 4].try_into().unwrap());
                    let index = word(INDEX);
                    if index != first + i as u32 {
                        return Err("supply node identity differs from array index");
                    }
                    let province = word(PROVINCE);
                    let country = system.country.id;
                    observed.check(
                        provinces,
                        memory,
                        base,
                        ProvinceRole::Node,
                        country,
                        index,
                        province,
                    )?;
                    let parent = parents.parent(memory, province)?;
                    observed.parent(provinces, memory, base, country, index, parent)?;
                    let Some(route) = edges.read(parent, province, &mut provinces.routes)? else {
                        continue;
                    };
                    output[len] = Row {
                        country: system.country.id,
                        index,
                        province,
                        kind: word(KIND),
                        flag: b[FLAG],
                        available: i64::from_le_bytes(b[13..21].try_into().unwrap()),
                        consumed: i64::from_le_bytes(b[21..29].try_into().unwrap()),
                        parent,
                        route,
                    };
                    len += 1;
                }
                first += count as u32;
            }
            let vertices = len;
            output[start..vertices].sort_unstable_by_key(|r| r.province);
            let complete = vertices - start == nodes.count as usize;
            provinces.vertices.fill(0);
            for row in &output[start..vertices] {
                if let Some(value) = provinces.vertices.get_mut(row.province as usize / 64) {
                    *value |= 1_u64 << (row.province % 64);
                }
            }
            'chains: for node in (start..vertices).filter(|_| complete) {
                let mut next = output[node].parent;
                while next > 0 {
                    let province = next as u32;
                    let word = province as usize / 64;
                    let bit = 1_u64 << (province % 64);
                    let seen = provinces.vertices.get(word).map_or_else(
                        || {
                            output[start..vertices]
                                .binary_search_by_key(&province, |r| r.province)
                                .is_ok()
                                || output[vertices..len]
                                    .iter()
                                    .any(|row| row.province == province)
                        },
                        |value| value & bit != 0,
                    );
                    if seen {
                        break;
                    }
                    output.ensure(len.checked_add(1).ok_or("supply vertex count overflow")?)?;
                    if len == output.len() {
                        cut += 1;
                        break 'chains;
                    }
                    let country = system.country.id;
                    observed.check(
                        provinces,
                        memory,
                        base,
                        ProvinceRole::Vertex,
                        country,
                        VERTEX | province,
                        province,
                    )?;
                    let parent = parents.parent(memory, province)?;
                    observed.parent(provinces, memory, base, country, VERTEX | province, parent)?;
                    let Some(route) = edges.read(parent, province, &mut provinces.routes)? else {
                        cut += 1;
                        break 'chains;
                    };
                    output[len] = Row {
                        country: system.country.id,
                        index: VERTEX | province,
                        province,
                        parent,
                        route,
                        ..Row::default()
                    };
                    if let Some(value) = provinces.vertices.get_mut(word) {
                        *value |= bit;
                    }
                    len += 1;
                    total += 1;
                    next = parent;
                }
            }
            output[vertices..len].sort_unstable_by_key(|r| r.province);
        }
        Ok(if len == total && cut == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: u32::try_from(total - len + cut)
                    .map_err(|_| "supply node unread count overflows")?,
            }
        })
    })();
    Report {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        paths: used,
        required_paths: required,
        null_provinces: observed.null_provinces,
        last_null_province: observed.last_null_province,
        refusal: observed.refusal,
    }
}

fn validate_province(
    memory: &impl Memory,
    base: usize,
    provinces: Array,
    province: u32,
) -> Result<usize, ProvinceCheck> {
    let slot = provinces
        .element(province, 8)
        .filter(|_| province != 0)
        .ok_or(ProvinceCheck::OutsideCensus)?;
    let p = read::<u64>(memory, slot).ok_or(ProvinceCheck::SlotUnreadable)?;
    if p == 0 {
        return Err(ProvinceCheck::SlotEmpty);
    }
    let p = p as usize;
    engine_native_memory::prefetch::span(p, 8);
    if let Some(id) = p.checked_add(engine_native::cprovince::ID) {
        engine_native_memory::prefetch::span(id, 4);
    }
    if read::<u64>(memory, p)
        != engine_sites::at(base, engine_native::cprovince::VTABLE_RVA).map(|v| v as u64)
        || read::<u32>(memory, p + engine_native::cprovince::ID) != Some(province)
    {
        return Err(ProvinceCheck::IdentityMismatch);
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const PROVINCES: Array = Array {
        data: 0x1000,
        count: 2,
    };
    const SLOT: usize = 0x1008;
    const POINTER: usize = 0x2000;

    struct MemoMemory;

    impl MemoMemory {
        const STATE: usize = 0x4000;
        const COUNT: u32 = 12;

        fn pointer(province: u32) -> usize {
            0x1_0000 + province as usize * 0x1000
        }
    }

    impl Memory for MemoMemory {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == Self::STATE + engine_native::cgame_state::PROVINCES && destination.len() == 16 {
                destination[..8].copy_from_slice(&(PROVINCES.data as u64).to_le_bytes());
                destination[8..12].copy_from_slice(&Self::COUNT.to_le_bytes());
                destination[12..].copy_from_slice(&Self::COUNT.to_le_bytes());
                return true;
            }
            if address == engine_native::cgame_state::PLAY_SESSION_STARTED_AT_RVA.0
                && destination.len() == 8
            {
                destination.copy_from_slice(&1_i64.to_le_bytes());
                return true;
            }
            for province in 1..Self::COUNT {
                let pointer = Self::pointer(province);
                if address == PROVINCES.data + province as usize * 8 && destination.len() == 8 {
                    destination.copy_from_slice(&(pointer as u64).to_le_bytes());
                    return true;
                }
                if address == pointer && destination.len() == 8 {
                    destination
                        .copy_from_slice(&(engine_native::cprovince::VTABLE_RVA.0 as u64).to_le_bytes());
                    return true;
                }
                if address == pointer + engine_native::cprovince::ID && destination.len() == 4 {
                    destination.copy_from_slice(&province.to_le_bytes());
                    return true;
                }
            }
            false
        }
    }

    #[test]
    fn allocation_growth_preserves_the_previous_lazy_province_capacity_trace() {
        let mut memo = ProvinceMemo::default();
        let mut previous = Vec::<usize>::new();
        memo.begin(&MemoMemory, 0, MemoMemory::STATE);
        assert_eq!(memo.allocated_bytes(), 0);
        assert_eq!(
            memo.province(&MemoMemory, 0, 0),
            Ok(Err(ProvinceCheck::OutsideCensus))
        );
        assert_eq!(memo.allocated_bytes(), 0);
        for province in [1, 5, 9, 2] {
            if previous.len() <= province as usize {
                previous.resize(province as usize + 1, 0);
            }
            let pointer = MemoMemory::pointer(province);
            previous[province as usize] = pointer;
            assert_eq!(memo.province(&MemoMemory, 0, province), Ok(Ok(pointer)));
            assert_eq!(memo.pointers, previous);
            assert_eq!(memo.pointers.capacity(), previous.capacity());
            assert_eq!(
                memo.allocated_bytes(),
                previous.capacity() * std::mem::size_of::<usize>()
            );
        }
    }

    struct ProvinceMemory {
        pointer: usize,
        vtable: u64,
        id: u32,
        fail: Option<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
    }

    impl ProvinceMemory {
        fn new() -> Self {
            Self {
                pointer: POINTER,
                vtable: engine_native::cprovince::VTABLE_RVA.0 as u64,
                id: 1,
                fail: None,
                reads: RefCell::new(Vec::new()),
            }
        }
    }

    impl Memory for ProvinceMemory {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            if self.fail == Some(address) {
                destination.fill(0xfe);
                return false;
            }
            if address == SLOT {
                destination.copy_from_slice(&(self.pointer as u64).to_le_bytes());
            } else if address == self.pointer {
                destination.copy_from_slice(&self.vtable.to_le_bytes());
            } else if Some(address) == self.pointer.checked_add(engine_native::cprovince::ID) {
                destination.copy_from_slice(&self.id.to_le_bytes());
            } else {
                panic!("unexpected province observation {address:#x}");
            }
            true
        }
    }

    #[test]
    fn province_hints_preserve_fault_prefixes_and_vtable_before_id() {
        let expected = [(SLOT, 8), (POINTER, 8), (POINTER + engine_native::cprovince::ID, 4)];
        for (failed, &(address, _)) in expected.iter().enumerate() {
            let mut memory = ProvinceMemory::new();
            memory.fail = Some(address);
            assert_eq!(
                validate_province(&memory, 0, PROVINCES, 1),
                Err(if failed == 0 {
                    ProvinceCheck::SlotUnreadable
                } else {
                    ProvinceCheck::IdentityMismatch
                })
            );
            assert_eq!(*memory.reads.borrow(), expected[..=failed]);
        }
        for (pointer, observed) in [(POINTER, 2), (usize::MAX - 8, 2), (usize::MAX - 7, 1)] {
            let mut memory = ProvinceMemory::new();
            memory.pointer = pointer;
            memory.vtable = 0;
            assert_eq!(
                validate_province(&memory, 0, PROVINCES, 1),
                Err(ProvinceCheck::IdentityMismatch)
            );
            assert_eq!(
                *memory.reads.borrow(),
                [(SLOT, 8), (pointer, 8)][..observed]
            );
        }
        let mut memory = ProvinceMemory::new();
        for province in [0, PROVINCES.count, u32::MAX] {
            assert_eq!(
                validate_province(&memory, 0, PROVINCES, province),
                Err(ProvinceCheck::OutsideCensus)
            );
        }
        assert!(memory.reads.borrow().is_empty());
        memory.pointer = 0;
        assert_eq!(
            validate_province(&memory, 0, PROVINCES, 1),
            Err(ProvinceCheck::SlotEmpty)
        );
        assert_eq!(*memory.reads.borrow(), [(SLOT, 8)]);
    }

    #[test]
    fn repeated_province_authentication_keeps_every_observation_fresh() {
        let mut memory = ProvinceMemory::new();
        for _ in 0..2 {
            assert_eq!(validate_province(&memory, 0, PROVINCES, 1), Ok(POINTER));
        }
        memory.id = 2;
        assert_eq!(
            validate_province(&memory, 0, PROVINCES, 1),
            Err(ProvinceCheck::IdentityMismatch)
        );
        assert_eq!(
            *memory.reads.borrow(),
            [(SLOT, 8), (POINTER, 8), (POINTER + engine_native::cprovince::ID, 4)].repeat(3)
        );
    }
}
