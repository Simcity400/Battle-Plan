
use engine_sites::Rva;
pub const GROUP: usize = 0x08;
pub const ROOT_ORDER: usize = 0x20;
pub const ORDER_CACHE_DIRTY: usize = 0x28;
pub const ROOT_ORDER_RVA: Rva = Rva(0x00c4_5310);
pub const HOST_OFFSET: usize = crate::cunit::SCHEDULED_MEMBER;
pub const UNIT_ACCESSOR_VTABLE_SLOT: usize = 0x20;

use engine_native_memory::{Memory, cpdx_array::Array};
use crate::{corder_instance as order, corders_group as group};

const CAPACITY_ERROR: &str = "membership packet capacity";
const NONE: usize = usize::MAX;

pub trait MembershipSignals {
    fn prepare(&self, _members: usize) {}
    fn available(&self) -> bool {
        false
    }
    fn group(&self, _receiver: usize, _slot: usize) -> Option<((u64, u64), usize)> {
        None
    }
}
impl MembershipSignals for () {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MembershipCapacity {
    pub queries: usize,
    pub groups: usize,
    pub roots: usize,
    pub chunks: usize,
}

impl MembershipCapacity {
    pub fn grown(self, required: Self) -> Self {
        fn dimension(current: usize, required: usize) -> usize {
            if required <= current {
                current
            } else {
                current.checked_mul(2).unwrap_or(required).max(required)
            }
        }
        Self {
            queries: dimension(self.queries, required.queries),
            groups: dimension(self.groups, required.groups),
            roots: dimension(self.roots, required.roots),
            chunks: dimension(self.chunks, required.chunks),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MembershipQuery {
    pub member: usize,
    pub own_group: usize,
}

struct QueryInput {
    query: MembershipQuery,
    parent: Option<usize>,
    own_slot: Option<usize>,
    result: Option<Result<bool, &'static str>>,
}

#[derive(Default)]
struct RootSet {
    result: Option<Result<Array, &'static str>>,
    first: Option<usize>,
    failure: Option<(u32, &'static str)>,
}

struct GroupInput {
    pointer: usize,
    result: Result<(), &'static str>,
    parent: Option<Result<usize, &'static str>>,
    roots: [RootSet; 2],
}

struct RootInput {
    order: Option<usize>,
    result: Result<(), &'static str>,
    next: Option<usize>,
}

struct MembershipOrderInput {
    pointer: usize,
    owner: Option<u64>,
    members: Option<Result<Array, &'static str>>,
    identity: Result<(), &'static str>,
    first_chunk: Option<usize>,
    last_chunk: Option<usize>,
    consumed: u32,
    failure: Option<&'static str>,
}

struct MembershipChunkInput {
    bytes: [u8; 64 * 8],
    len: usize,
    next: Option<usize>,
}

pub struct MembershipPacket {
    prepared: MembershipCapacity,
    required: MembershipCapacity,
    queries: Vec<QueryInput>,
    groups: Vec<GroupInput>,
    roots: Vec<RootInput>,
    orders: Vec<MembershipOrderInput>,
    chunks: Vec<MembershipChunkInput>,
    group_index: Vec<usize>,
    order_index: Vec<usize>,
    work: Vec<(usize, usize)>,
    overflowed: bool,
}

impl MembershipPacket {
    pub fn vacant() -> Self {
        Self {
            prepared: MembershipCapacity::default(),
            required: MembershipCapacity::default(),
            queries: Vec::new(),
            groups: Vec::new(),
            roots: Vec::new(),
            orders: Vec::new(),
            chunks: Vec::new(),
            group_index: Vec::new(),
            order_index: Vec::new(),
            work: Vec::new(),
            overflowed: false,
        }
    }

    pub fn try_with_capacity(
        queries: usize,
        groups: usize,
        roots: usize,
        chunks: usize,
    ) -> std::io::Result<Self> {
        let group_index = groups.checked_mul(2).ok_or_else(engine_allocation::oom)?;
        let order_index = roots.checked_mul(2).ok_or_else(engine_allocation::oom)?;
        Ok(Self {
            prepared: MembershipCapacity {
                queries,
                groups,
                roots,
                chunks,
            },
            required: MembershipCapacity::default(),
            queries: engine_allocation::vec(queries)?,
            groups: engine_allocation::vec(groups)?,
            roots: engine_allocation::vec(roots)?,
            orders: engine_allocation::vec(roots)?,
            chunks: engine_allocation::vec(chunks)?,
            group_index: engine_allocation::filled(group_index, NONE)?,
            order_index: engine_allocation::filled(order_index, NONE)?,
            work: engine_allocation::vec(queries)?,
            overflowed: false,
        })
    }

    #[cfg(test)]
    pub fn new(queries: usize, groups: usize, roots: usize, chunks: usize) -> Self {
        Self::try_with_capacity(queries, groups, roots, chunks)
            .expect("membership packet test capacity")
    }

    pub fn capacity(&self) -> MembershipCapacity {
        MembershipCapacity {
            queries: self.prepared.queries.max(self.required.queries),
            groups: self.prepared.groups.max(self.required.groups),
            roots: self.prepared.roots.max(self.required.roots),
            chunks: self.prepared.chunks.max(self.required.chunks),
        }
    }

    pub fn required_capacity(&self) -> MembershipCapacity {
        self.required
    }

    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    pub fn allocated_bytes(&self) -> usize {
        self.queries.capacity() * size_of::<QueryInput>()
            + self.groups.capacity() * size_of::<GroupInput>()
            + self.roots.capacity() * size_of::<RootInput>()
            + self.orders.capacity() * size_of::<MembershipOrderInput>()
            + self.chunks.capacity() * size_of::<MembershipChunkInput>()
            + (self.group_index.capacity() + self.order_index.capacity()) * size_of::<usize>()
            + self.work.capacity() * size_of::<(usize, usize)>()
    }

    pub fn grow(&mut self, required: MembershipCapacity) -> std::io::Result<()> {
        let old = self.prepared;
        let next = old.grown(self.required).grown(required);
        let groups = engine_allocation::mul(next.groups, 2)?;
        let orders = engine_allocation::mul(next.roots, 2)?;
        fn retain<T>(rows: &mut Vec<T>, demand: usize) -> std::io::Result<()> {
            if demand > rows.capacity() {
                engine_allocation::reserve_exact(rows, demand - rows.len())?;
            }
            Ok(())
        }
        retain(&mut self.queries, next.queries)?;
        retain(&mut self.work, next.queries)?;
        retain(&mut self.groups, next.groups)?;
        retain(&mut self.roots, next.roots)?;
        retain(&mut self.orders, next.roots)?;
        retain(&mut self.chunks, next.chunks)?;
        retain(&mut self.group_index, groups)?;
        retain(&mut self.order_index, orders)?;
        self.group_index
            .resize(self.group_index.len().max(groups), NONE);
        self.order_index
            .resize(self.order_index.len().max(orders), NONE);
        self.prepared = next;
        self.clear();
        Ok(())
    }

    pub fn clear(&mut self) {
        self.queries.clear();
        self.groups.clear();
        self.roots.clear();
        self.orders.clear();
        self.chunks.clear();
        self.group_index.fill(NONE);
        self.order_index.fill(NONE);
        self.work.clear();
        self.required = MembershipCapacity::default();
        self.overflowed = false;
    }

    pub fn push_query(&mut self, query: MembershipQuery) -> Result<usize, &'static str> {
        self.required.queries = self.required.queries.saturating_add(1);
        reserve(
            &mut self.queries,
            self.required.queries,
            &mut self.overflowed,
        )?;
        reserve(&mut self.work, self.required.queries, &mut self.overflowed)?;
        let index = self.queries.len();
        self.queries.push(QueryInput {
            query,
            parent: None,
            own_slot: None,
            result: None,
        });
        Ok(index)
    }

    pub fn outcome(&self, index: usize) -> Result<bool, &'static str> {
        self.queries
            .get(index)
            .and_then(|q| q.result)
            .unwrap_or(Err("membership query not acquired"))
    }

    pub fn parent(&self, index: usize) -> Option<usize> {
        let query = self.queries.get(index)?;
        if query.query.own_group == 0 {
            Some(0)
        } else {
            query.parent
        }
    }

    pub fn acquire(&mut self, memory: &impl Memory, base: usize) {
        for index in 0..self.queries.len() {
            let query = self.queries[index].query;
            let result = (|| {
                let own = query.own_group;
                if own == 0 {
                    return Ok(Some(false));
                }
                let slot = self.group(memory, base, own)?;
                self.queries[index].own_slot = Some(slot);
                self.groups[slot].result?;
                let parent = *self.groups[slot]
                    .parent
                    .get_or_insert_with(|| pointer(memory, own, group::PARENT));
                self.queries[index].parent = Some(parent?);
                Ok(None)
            })();
            self.queries[index].result = match result {
                Ok(None) => None,
                Ok(Some(value)) => Some(Ok(value)),
                Err(reason) => Some(Err(reason)),
            };
        }
        for stage in 0..2 {
            self.work.clear();
            for index in 0..self.queries.len() {
                if self.queries[index].result.is_some() {
                    continue;
                }
                let slot = if stage == 0 {
                    Ok(self.queries[index]
                        .own_slot
                        .expect("authenticated own group"))
                } else {
                    let parent = self.queries[index].parent.expect("observed parent");
                    if parent == 0 {
                        self.queries[index].result = Some(Ok(false));
                        continue;
                    }
                    self.group(memory, base, parent)
                };
                match slot {
                    Ok(slot) => self.work.push((slot, index)),
                    Err(reason) => self.queries[index].result = Some(Err(reason)),
                }
            }
            let queries = &self.queries;
            self.work
                .sort_unstable_by_key(|&(slot, index)| (slot, queries[index].query.member));
            let mut start = 0;
            while start < self.work.len() {
                let slot = self.work[start].0;
                let mut end = start + 1;
                while end < self.work.len() && self.work[end].0 == slot {
                    end += 1;
                }
                self.search_group(memory, base, slot, start, end);
                if stage == 1 {
                    for work in start..end {
                        self.queries[self.work[work].1]
                            .result
                            .get_or_insert(Ok(false));
                    }
                }
                start = end;
            }
        }
    }

    fn group(
        &mut self,
        memory: &impl Memory,
        base: usize,
        at: usize,
    ) -> Result<usize, &'static str> {
        self.group_with(at, || group_identity(memory, base, at))
    }

    pub fn admit_group(&mut self, at: usize) -> Result<(), &'static str> {
        if at != 0 {
            self.group_with(at, || Ok(()))?;
        }
        Ok(())
    }

    fn group_with(
        &mut self,
        at: usize,
        identity: impl FnOnce() -> Result<(), &'static str>,
    ) -> Result<usize, &'static str> {
        if let Some(mut slot) = hash(at, self.group_index.len()) {
            while self.group_index[slot] != NONE {
                let existing = self.group_index[slot];
                if self.groups[existing].pointer == at {
                    return Ok(existing);
                }
                slot = (slot + 1) % self.group_index.len();
            }
        }
        self.required.groups = self
            .required
            .groups
            .max(self.groups.len().saturating_add(1));
        reserve(&mut self.groups, self.required.groups, &mut self.overflowed)?;
        if reserve_index(
            &mut self.group_index,
            self.required.groups,
            &mut self.overflowed,
        )? {
            for (slot, group) in self.groups.iter().enumerate() {
                insert_index(&mut self.group_index, group.pointer, slot);
            }
        }
        let mut input = GroupInput {
            pointer: at,
            result: Ok(()),
            parent: None,
            roots: Default::default(),
        };
        input.result = identity();
        let slot = self.groups.len();
        self.groups.push(input);
        insert_index(&mut self.group_index, at, slot);
        Ok(slot)
    }

    fn root_set(
        &mut self,
        memory: &impl Memory,
        slot: usize,
        set: usize,
    ) -> Result<Array, &'static str> {
        if let Some(result) = self.groups[slot].roots[set].result {
            return result;
        }
        let at = self.groups[slot].pointer;
        let result = (|| {
            let address = at
                .checked_add(root_offset(set))
                .ok_or("membership order roots overflow")?;
            let header = engine_native_memory::read(memory, address);
            header
                .and_then(|h| Array::from_header(h, 8))
                .ok_or("membership order roots invalid")
        })();
        self.groups[slot].roots[set].result = Some(result);
        result
    }

    fn order(
        &mut self,
        memory: &impl Memory,
        base: usize,
        at: usize,
    ) -> Result<usize, &'static str> {
        if let Some(mut slot) = hash(at, self.order_index.len()) {
            while self.order_index[slot] != NONE {
                let existing = self.order_index[slot];
                if self.orders[existing].pointer == at {
                    return Ok(existing);
                }
                slot = (slot + 1) % self.order_index.len();
            }
        }
        self.required.roots = self.required.roots.max(self.orders.len().saturating_add(1));
        let required = self.orders.len().saturating_add(1);
        reserve(&mut self.orders, required, &mut self.overflowed)?;
        if reserve_index(&mut self.order_index, required, &mut self.overflowed)? {
            for (slot, order) in self.orders.iter().enumerate() {
                insert_index(&mut self.order_index, order.pointer, slot);
            }
        }
        let mut input = MembershipOrderInput {
            pointer: at,
            owner: None,
            members: None,
            identity: Ok(()),
            first_chunk: None,
            last_chunk: None,
            consumed: 0,
            failure: None,
        };
        input.identity = (|| {
            let vtable = engine_sites::at(base, order::VTABLE_RVA)
                .ok_or("membership order identity mismatch")? as u64;
            if engine_native_memory::read::<u64>(memory, at) != Some(vtable) {
                return Err("membership order identity mismatch");
            }
            input.owner = read_at(memory, at, order::OWNER);
            input.owner.ok_or("orders membership pointer unreadable")?;
            Ok(())
        })();
        let slot = self.orders.len();
        self.orders.push(input);
        insert_index(&mut self.order_index, at, slot);
        Ok(slot)
    }

    fn members(&mut self, memory: &impl Memory, index: usize) -> Result<Array, &'static str> {
        if let Some(result) = self.orders[index].members {
            return result;
        }
        let result = (|| {
            let address = self.orders[index]
                .pointer
                .checked_add(order::ASSIGNED_MEMBER_DATA)
                .ok_or("assigned members overflow")?;
            let header = engine_native_memory::read(memory, address);
            header
                .and_then(|h| Array::from_header(h, 8))
                .ok_or("assigned members invalid")
        })();
        self.orders[index].members = Some(result);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn root(
        &mut self,
        memory: &impl Memory,
        base: usize,
        slot: usize,
        set: usize,
        roots: Array,
        ordinal: u32,
        previous: Option<usize>,
    ) -> Result<usize, &'static str> {
        let retained = match previous {
            Some(previous) => self.roots[previous].next,
            None => self.groups[slot].roots[set].first,
        };
        if let Some(root) = retained {
            return Ok(root);
        }
        if let Some((failed, reason)) = self.groups[slot].roots[set].failure
            && ordinal >= failed
        {
            return Err(reason);
        }
        self.required.roots = self.required.roots.max(self.roots.len().saturating_add(1));
        if let Err(reason) = reserve(&mut self.roots, self.required.roots, &mut self.overflowed) {
            self.groups[slot].roots[set].failure = Some((ordinal, reason));
            return Err(reason);
        }
        let mut root = RootInput {
            order: None,
            result: Ok(()),
            next: None,
        };
        root.result = (|| {
            let address = roots
                .element(ordinal, 8)
                .ok_or("membership root overflow")?;
            let at = engine_native_memory::read::<u64>(memory, address)
                .ok_or("orders membership pointer unreadable")? as usize;
            if at == 0 {
                return Err("membership order identity mismatch");
            }
            let order = self.order(memory, base, at)?;
            root.order = Some(order);
            self.orders[order].identity?;
            if self.orders[order].owner != Some(self.groups[slot].pointer as u64) {
                return Err("membership order identity mismatch");
            }
            self.members(memory, order)?;
            Ok(())
        })();
        let index = self.roots.len();
        self.roots.push(root);
        if let Some(previous) = previous {
            self.roots[previous].next = Some(index);
        } else {
            self.groups[slot].roots[set].first = Some(index);
        }
        Ok(index)
    }

    fn chunk(
        &mut self,
        memory: &impl Memory,
        order: usize,
        previous: Option<usize>,
    ) -> Result<Option<usize>, &'static str> {
        let retained = match previous {
            Some(previous) => self.chunks[previous].next,
            None => self.orders[order].first_chunk,
        };
        if retained.is_some() {
            return Ok(retained);
        }
        if let Some(reason) = self.orders[order].failure {
            return Err(reason);
        }
        let members = self.orders[order]
            .members
            .expect("admitted member header")?;
        let first = self.orders[order].consumed;
        if first == members.count {
            return Ok(None);
        }
        self.required.chunks = self
            .required
            .chunks
            .max(self.chunks.len().saturating_add(1));
        if let Err(reason) = reserve(&mut self.chunks, self.required.chunks, &mut self.overflowed) {
            self.orders[order].failure = Some(reason);
            return Err(reason);
        }
        let result = (|| {
            let count = (members.count - first).min(64) as usize;
            let address = members
                .element(first, 8)
                .ok_or("assigned members overflow")?;
            let mut chunk = MembershipChunkInput {
                bytes: [0; 64 * 8],
                len: count * 8,
                next: None,
            };
            if !engine_native_memory::read_slice(memory, address, &mut chunk.bytes[..chunk.len]) {
                return Err("assigned members unreadable");
            }
            let index = self.chunks.len();
            self.chunks.push(chunk);
            if let Some(previous) = self.orders[order].last_chunk {
                self.chunks[previous].next = Some(index);
            } else {
                self.orders[order].first_chunk = Some(index);
            }
            self.orders[order].last_chunk = Some(index);
            self.orders[order].consumed += count as u32;
            Ok(Some(index))
        })();
        if let Err(reason) = result {
            self.orders[order].failure = Some(reason);
        }
        result
    }

    fn unresolved(&self, start: usize, end: usize) -> bool {
        self.work[start..end]
            .iter()
            .any(|&(_, index)| self.queries[index].result.is_none())
    }

    fn fail_unresolved(&mut self, start: usize, end: usize, reason: &'static str) {
        for work in start..end {
            self.queries[self.work[work].1]
                .result
                .get_or_insert(Err(reason));
        }
    }

    fn search_group(
        &mut self,
        memory: &impl Memory,
        base: usize,
        slot: usize,
        start: usize,
        end: usize,
    ) {
        if let Err(reason) = self.groups[slot].result {
            self.fail_unresolved(start, end, reason);
            return;
        }
        for set in 0..2 {
            if !self.unresolved(start, end) {
                break;
            }
            let roots = match self.root_set(memory, slot, set) {
                Ok(roots) => roots,
                Err(reason) => {
                    self.fail_unresolved(start, end, reason);
                    break;
                }
            };
            let mut previous = None;
            for ordinal in 0..roots.count {
                if !self.unresolved(start, end) {
                    break;
                }
                let root = match self.root(memory, base, slot, set, roots, ordinal, previous) {
                    Ok(root) => root,
                    Err(reason) => {
                        self.fail_unresolved(start, end, reason);
                        break;
                    }
                };
                previous = Some(root);
                if let Err(reason) = self.roots[root].result {
                    self.fail_unresolved(start, end, reason);
                    break;
                }
                let order = self.roots[root].order.expect("authenticated root");
                let mut previous_chunk = None;
                let mut remaining = self.work[start..end]
                    .iter()
                    .filter(|&&(_, index)| self.queries[index].result.is_none())
                    .count();
                while remaining != 0 {
                    let chunk = match self.chunk(memory, order, previous_chunk) {
                        Ok(Some(chunk)) => chunk,
                        Ok(None) => break,
                        Err(reason) => {
                            self.fail_unresolved(start, end, reason);
                            break;
                        }
                    };
                    previous_chunk = Some(chunk);
                    for word in 0..self.chunks[chunk].len / 8 {
                        let bytes = &self.chunks[chunk].bytes[word * 8..word * 8 + 8];
                        let member = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
                        let mut matched = start
                            + self.work[start..end].partition_point(|&(_, index)| {
                                self.queries[index].query.member < member
                            });
                        while matched < end {
                            let query = &mut self.queries[self.work[matched].1];
                            if query.query.member != member {
                                break;
                            }
                            if query.result.is_none() {
                                query.result = Some(Ok(true));
                                remaining -= 1;
                            }
                            matched += 1;
                        }
                    }
                }
            }
        }
    }
}

fn reserve<T>(
    rows: &mut Vec<T>,
    required: usize,
    overflowed: &mut bool,
) -> Result<(), &'static str> {
    if required > rows.capacity() {
        engine_allocation::reserve(rows, required.saturating_sub(rows.len())).map_err(|_| {
            *overflowed = true;
            CAPACITY_ERROR
        })?;
    }
    Ok(())
}

fn reserve_index(
    index: &mut Vec<usize>,
    capacity: usize,
    overflowed: &mut bool,
) -> Result<bool, &'static str> {
    let required = capacity.checked_mul(2).ok_or_else(|| {
        let _ = engine_allocation::oom();
        *overflowed = true;
        CAPACITY_ERROR
    })?;
    if index.len() >= required {
        return Ok(false);
    }
    reserve(index, required, overflowed)?;
    index.resize(required, NONE);
    index.fill(NONE);
    Ok(true)
}

fn insert_index(index: &mut [usize], pointer: usize, value: usize) {
    let mut slot = hash(pointer, index.len()).expect("reserved membership index");
    while index[slot] != NONE {
        slot = (slot + 1) % index.len();
    }
    index[slot] = value;
}

fn hash(pointer: usize, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let mut value = pointer as u64;
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    Some((value ^ (value >> 27)) as usize % len)
}

pub(crate) fn group_identity(
    memory: &impl Memory,
    base: usize,
    at: usize,
) -> Result<(), &'static str> {
    let vtable = engine_native_memory::read::<u64>(memory, at).ok_or("orders group identity unreadable")?;
    if ![group::VTABLE_RVA, engine_native_parts::carmy_group::VTABLE_RVA]
        .iter()
        .any(|&key| engine_sites::at(base, key).map(|p| p as u64) == Some(vtable))
    {
        return Err("orders membership group identity mismatch");
    }
    if read_at::<u64>(memory, at, group::HANDLE).ok_or("orders membership pointer unreadable")?
        as u32
        != group::REFERENCE_TYPE
    {
        return Err("orders membership group identity mismatch");
    }
    Ok(())
}

fn root_offset(set: usize) -> usize {
    [group::PRIMARY_ORDER_DATA, group::SECONDARY_ORDER_DATA][set]
}

fn read_at<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Option<T> {
    pointer
        .checked_add(offset)
        .and_then(|address| engine_native_memory::read(memory, address))
}

fn pointer(memory: &impl Memory, at: usize, offset: usize) -> Result<usize, &'static str> {
    read_at::<u64>(memory, at, offset)
        .map(|word| word as usize)
        .ok_or("orders membership pointer unreadable")
}

#[cfg(test)]
fn contains_from(
    memory: &impl engine_native_memory::Memory,
    members: Array,
    member: usize,
    mut first: u32,
) -> Result<bool, &'static str> {
    let mut bytes = [0; 64 * 8];
    while first < members.count {
        let count = (members.count - first).min(64) as usize;
        let address = members
            .element(first, 8)
            .ok_or("assigned members overflow")?;
        if !engine_native_memory::read_slice(memory, address, &mut bytes[..count * 8]) {
            return Err("assigned members unreadable");
        }
        if bytes[..count * 8]
            .chunks_exact(8)
            .any(|word| u64::from_le_bytes(word.try_into().unwrap()) == member as u64)
        {
            return Ok(true);
        }
        first += count as u32;
    }
    Ok(false)
}

#[cfg(test)]
fn has_order(memory: &impl Memory, base: usize, member: usize) -> Result<bool, &'static str> {
    use crate::{corder_instance as order, corders_group as group};
    use engine_native_memory::{cpdx_array::Array, read};
    let pointer = |at: usize, offset: usize| {
        at.checked_add(offset)
            .and_then(|p| read::<u64>(memory, p))
            .map(|p| p as usize)
            .ok_or("orders membership pointer unreadable")
    };
    let own = pointer(member, GROUP)?;
    if own == 0 {
        return Ok(false);
    }
    let authenticate = |at| -> Result<(), &'static str> {
        let vptr = read::<u64>(memory, at).ok_or("orders group identity unreadable")?;
        if ![group::VTABLE_RVA, engine_native_parts::carmy_group::VTABLE_RVA]
            .iter()
            .any(|&key| engine_sites::at(base, key).map(|p| p as u64) == Some(vptr))
            || pointer(at, group::HANDLE)? as u32 != group::REFERENCE_TYPE
        {
            return Err("orders membership group identity mismatch");
        }
        Ok(())
    };
    authenticate(own)?;
    let vtable = engine_sites::at(base, order::VTABLE_RVA)
        .ok_or("membership order identity mismatch")? as u64;
    let parent = pointer(own, group::PARENT)?;
    for (stage, at) in [own, parent].into_iter().enumerate() {
        if at == 0 {
            continue;
        }
        if stage != 0 {
            authenticate(at)?;
        }
        for offset in [group::PRIMARY_ORDER_DATA, group::SECONDARY_ORDER_DATA] {
            let roots_address = at
                .checked_add(offset)
                .ok_or("membership order roots overflow")?;
            let roots = read(memory, roots_address)
                .and_then(|header| Array::from_header(header, 8))
                .ok_or("membership order roots invalid")?;
            let mut found = false;
            for index in 0..roots.count {
                let p = pointer(
                    roots.element(index, 8).ok_or("membership root overflow")?,
                    0,
                )?;
                if p == 0
                    || read::<u64>(memory, p) != Some(vtable)
                    || pointer(p, order::OWNER)? != at
                {
                    return Err("membership order identity mismatch");
                }
                let members_address = p
                    .checked_add(order::ASSIGNED_MEMBER_DATA)
                    .ok_or("assigned members overflow")?;
                let header =
                    read::<[u8; 16]>(memory, members_address).ok_or("assigned members invalid")?;
                let members = Array::from_header(header, 8).ok_or("assigned members invalid")?;
                found = contains_from(memory, members, member, 0)?;
                if found {
                    break;
                }
            }
            if found {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
#[cfg(test)]
mod tests;
