//! Supply companion node, distinct from the country node-settings record.
//! No hourly consistency lifetime is established by this layout alone.

/// Node-array stride.
pub const STRIDE: usize = 0xe8;
/// Node index, u32, used for weight-vector selection and initialized from
/// constructor argument two. Identity is country plus index.
pub const INDEX: usize = 0;
/// Destination province of the node.
pub const PROVINCE: usize = 4;
/// Node kind, u32.
pub const KIND: usize = 0x08;
/// Node flag, u8.
pub const FLAG: usize = 0x0c;
/// Locator-arm byte, tested separately from the `FLAG` byte.
pub const LOCATOR_ARM: usize = 0x0d;
/// Available supply, i64.
pub const AVAILABLE: usize = 0x38;
/// Consumed supply, i64.
pub const CONSUMED: usize = 0x40;
/// Route value, i64; the state uses its maximum over provinces and grants
/// a supply-route resource bonus when that maximum is positive.
pub const ROUTE_VALUE: usize = 0x78;
/// Node constructor initializing the `KIND`.
pub const CONSTRUCTOR_RVA: usize = 0x0120_4300;
/// First locator-arm writer updating `LOCATOR_ARM`.
pub const UPDATE_LOCATOR_ARM_RVA: usize = 0x0120_cee0;
/// Second locator-arm writer updating `LOCATOR_ARM`.
pub const REFRESH_LOCATOR_ARM_RVA: usize = 0x0120_e7c0;

use crate::{
    Memory, cpdx_array::Array, csupply_system, read, supply_companion, supply_parent_map,
    supply_route, walk::Outcome,
};

/// Index of a node-less parent-map vertex row: `VERTEX | province`.
/// CPdxArray's native signed count keeps node indices below this marker.
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
    /// Native province-keyed parent lookup, -1 for a missing key.
    pub parent: i32,
    pub route: supply_route::Row,
}
pub struct Report {
    pub outcome: Outcome,
    pub paths: usize,
    pub required_paths: usize,
    /// Province ids this walk recorded as the game's null province
    /// ([`ProvinceCheck::is_null_province`]).
    pub null_provinces: u32,
    /// The last of them, for diagnostics.
    pub last_null_province: Option<ProvinceObservation>,
    /// The province check that refused the walk, when one did.
    pub refusal: Option<ProvinceObservation>,
}

/// What resolving a province id stored in the supply companion found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvinceCheck {
    /// The hour's province census header was unreadable.
    CensusUnavailable,
    /// Zero or not below the census count: the game's province lookup
    /// returns null for it.
    OutsideCensus,
    /// The census slot could not be read.
    SlotUnreadable,
    /// The census slot holds null, which province lookup returns as is.
    SlotEmpty,
    /// The slot's object is not the CProvince with this id.
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

    /// The game holds this id as a null province. The walk's native readers only key maps with a
    /// node or parent province: train departure routing looks it up in the parent and route maps,
    /// and a missing parent key ends the walk. Node creators store it unchecked, from the copied
    /// record or in the supply companion. The walk therefore records such an id as stored.
    pub const fn is_null_province(self) -> bool {
        matches!(self, Self::OutsideCensus | Self::SlotEmpty)
    }
}

/// Where the walk met a province id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvinceRole {
    /// A node's own province (`PROVINCE`).
    Node,
    /// A province-keyed parent from the companion's parent map.
    Parent,
    /// A node-less vertex on a parent chain.
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

/// One province id the walk could not resolve: its country, the node index
/// (`VERTEX | province` on a parent chain), the stored value, where it was
/// met and what the check found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProvinceObservation {
    pub country: u32,
    pub index: u32,
    pub province: u32,
    pub role: ProvinceRole,
    pub check: ProvinceCheck,
}

/// A distinct refusal reason for each check that refuses, by role. A null
/// province never refuses.
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

/// The walk's province checks and what they found.
#[derive(Default)]
struct Observations {
    null_provinces: u32,
    last_null_province: Option<ProvinceObservation>,
    refusal: Option<ProvinceObservation>,
}

impl Observations {
    /// Resolve `province` as `role`; a null province is recorded as stored,
    /// any other unresolved id refuses the walk with its own reason.
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

    /// A positive parent is a province id; zero and negative are roots.
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

/// Immediate parent edges, written into one shared path arena.
struct Edges<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    root: usize,
    outer: Option<crate::supply_parent_map::Map>,
    provinces: Array,
    paths: crate::walk::Buffer<'a, u32>,
    used: &'a mut usize,
    required: &'a mut usize,
}
impl<M: Memory> Edges<'_, M> {
    /// Immediate parent/province edge. `None` means the arena is too small;
    /// `required` retains its demand.
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
            // Preserve a separate arena occurrence and the original capacity
            // demand for every row, including duplicate path words.
            self.paths
                .copy_within(route.start..route.start + route.len, used);
            route.start = used;
            *self.used += route.len;
            return Ok(Some(route));
        }
        // The route map is a dependency only of a positive-parent edge. Empty
        // censuses, root-only nodes and rows omitted for output capacity do
        // not read it; earlier node/parent failures keep their precedence.
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

/// No game calls, full province scan or repeated capital-path walk. The
/// province pointer memo grows lazily. Exactly 29 node bytes are gathered
/// per node in 64-record batches. Parent chains follow the companion
/// map through every vertex visited by capital search. A vertex without
/// a node row is recorded once as a `VERTEX` row with parent and edge.
/// Rows are grouped per country rather than in stored order.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut [Row],
    paths: &mut [u32],
) -> Report {
    let mut provinces = ProvinceMemo::default();
    provinces.begin(memory, base, state);
    walk_part(memory, base, state, 0, 1, output, paths, &mut provinces)
}

/// Advisory original node-header floor. Parent vertices and paths grow
/// from the worker's original walk before publication.
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

/// Session memo of authenticated province pointers. The province census
/// header and the play session identify it; a change clears it. Each
/// province is authenticated once per session instead of once per node.
#[derive(Default)]
pub struct ProvinceMemo {
    key: Option<(usize, usize, i64, [u8; 16])>,
    provinces: Option<Array>,
    pointers: Vec<usize>,
    /// Per-country node/vertex membership, not native data. IDs outside this
    /// bounded bitset use the captured rows directly.
    vertices: Vec<u64>,
    worker: bool,
    /// Complete edge and endpoint observations within one walk only.
    routes: supply_route::Memo,
}

impl ProvinceMemo {
    /// Enable worker growth from original route dependencies and live census. A
    /// default/vacant memo uses the exact traversal fallback; both variants
    /// grow their province pointer storage lazily after authentication.
    pub fn with_scratch() -> std::io::Result<Self> {
        Ok(Self {
            worker: true,
            routes: supply_route::Memo::try_with_scratch()?,
            ..Self::default()
        })
    }

    /// Re-key for this hour; reads the census header and the session once.
    pub fn begin(&mut self, memory: &impl Memory, base: usize, state: usize) {
        let root = state
            .checked_add(crate::cgame_state::PROVINCES)
            .and_then(|address| read::<[u8; 16]>(memory, address));
        let session = base
            .checked_add(crate::cgame_state::PLAY_SESSION_STARTED_AT_RVA)
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

    /// The authenticated pointer, or what its check found. Only an
    /// authenticated pointer is memoized; the outer error is allocation.
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
                crate::allocation::resize(&mut self.pointers, province as usize + 1, 0)
                    .map_err(|_| "supply province allocation failed")?;
            }
            self.pointers[province as usize] = pointer;
        }
        Ok(Ok(pointer))
    }
}

/// Share `part` of `parts` of the country slots: every `parts`-th slot, so
/// neighbouring big networks spread across shares. The caller restores the
/// per-country order. `provinces` must have begun this hour.
#[allow(clippy::too_many_arguments)]
pub fn walk_part(
    memory: &impl Memory,
    base: usize,
    state: usize,
    part: usize,
    parts: usize,
    output: &mut [Row],
    paths: &mut [u32],
    provinces: &mut ProvinceMemo,
) -> Report {
    walk_part_buffer(
        memory,
        base,
        state,
        part,
        parts,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(paths),
        provinces,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn walk_part_growing(
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
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(paths),
        provinces,
        None,
    )
}

/// [`walk_part_growing`] for the countries `select` admits by native country
/// id: an unselected slot's supply objects, nodes and routes are not read,
/// and its rows are neither returned nor counted as unread. A selected
/// country's rows equal the whole walk's.
#[allow(clippy::too_many_arguments)]
pub fn walk_part_selected_growing(
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
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(paths),
        provinces,
        Some(select),
    )
}

/// `select`, when present, admits countries by native id before their supply
/// objects are read ([`walk_part_selected_growing`]); without it every slot
/// is read and checked.
#[allow(clippy::too_many_arguments)]
pub fn walk_part_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    part: usize,
    parts: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    paths: crate::walk::Buffer<'_, u32>,
    provinces: &mut ProvinceMemo,
    select: Option<&dyn Fn(u32) -> bool>,
) -> Report {
    let mut used = 0;
    let mut required = 0;
    let mut observed = Observations::default();
    provinces.routes.begin();
    let result = (|| {
        use crate::memory::{RecordField, RecordLayout, RecordSources};
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
        let census = Array::read(memory, state + crate::cgame_state::PROVINCES, 8)
            .ok_or("supply provinces invalid")?;
        if provinces.worker {
            // Province-indexed vertices are reached through the live census.
            crate::allocation::resize(
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
        // Vertex rows the output or path arena had no room for.
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
                    // Each admitted record enters the four-node hint window
                    // once. Pointer reads and all dependent checks stay serial.
                    while hinted < (i + 4).min(count) {
                        let next = &bytes[hinted * 29..(hinted + 1) * 29];
                        let index = u32::from_le_bytes(next[..4].try_into().unwrap());
                        let province = u32::from_le_bytes(next[4..8].try_into().unwrap());
                        if index == first + hinted as u32
                            && province != 0
                            && let Some(slot) = census.element(province, 8)
                        {
                            crate::prefetch::span(slot, 8);
                            parents.hint(province);
                        }
                        hinted += 1;
                    }
                    let b = &bytes[i * 29..(i + 1) * 29];
                    let word = |n| u32::from_le_bytes(b[n..n + 4].try_into().unwrap());
                    let index = word(INDEX);
                    // Native weight indexing followed by array[index] requires
                    // this invariant; do not substitute a loop ordinal for it.
                    if index != first + i as u32 {
                        return Err("supply node identity differs from array index");
                    }
                    let province = word(PROVINCE);
                    let country = system.country.id;
                    // A null province is recorded as stored
                    // (ProvinceCheck::is_null_province); a resolved one is
                    // authenticated.
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
            // Keep the original node traversal order. Vertex membership uses
            // a bounded bitset; append rows and sort once after the chains,
            // instead of shifting every larger vertex row at each insertion.
            // A country whose nodes did not all fit is already truncated; the
            // first vertex without room ends its walk with one unread row.
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

/// Province lookup returns the census slot when `0 < id < count` (signed) and
/// null otherwise; a resolved slot must hold the CProvince with this id.
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
    // A known pointer permits hints; the vtable still gates the actual ID read.
    crate::prefetch::span(p, 8);
    if let Some(id) = p.checked_add(crate::cprovince::ID) {
        crate::prefetch::span(id, 4);
    }
    if read::<u64>(memory, p)
        != base
            .checked_add(crate::cprovince::VTABLE_RVA)
            .map(|v| v as u64)
        || read::<u32>(memory, p + crate::cprovince::ID) != Some(province)
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
            if address == Self::STATE + crate::cgame_state::PROVINCES && destination.len() == 16 {
                destination[..8].copy_from_slice(&(PROVINCES.data as u64).to_le_bytes());
                destination[8..12].copy_from_slice(&Self::COUNT.to_le_bytes());
                destination[12..].copy_from_slice(&Self::COUNT.to_le_bytes());
                return true;
            }
            if address == crate::cgame_state::PLAY_SESSION_STARTED_AT_RVA && destination.len() == 8
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
                        .copy_from_slice(&(crate::cprovince::VTABLE_RVA as u64).to_le_bytes());
                    return true;
                }
                if address == pointer + crate::cprovince::ID && destination.len() == 4 {
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
                vtable: crate::cprovince::VTABLE_RVA as u64,
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
            } else if Some(address) == self.pointer.checked_add(crate::cprovince::ID) {
                destination.copy_from_slice(&self.id.to_le_bytes());
            } else {
                panic!("unexpected province observation {address:#x}");
            }
            true
        }
    }

    #[test]
    fn province_hints_preserve_fault_prefixes_and_vtable_before_id() {
        let expected = [(SLOT, 8), (POINTER, 8), (POINTER + crate::cprovince::ID, 4)];
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
            [(SLOT, 8), (POINTER, 8), (POINTER + crate::cprovince::ID, 4)].repeat(3)
        );
    }
}
