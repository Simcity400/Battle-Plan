
use engine_sites::Rva;
use crate::{
    Memory, corders_group,
    cpdx_array::Array,
    memory::{
        Gathered, RecordField, RecordLayout, RecordSources, gather_records_once, packed_fields,
        packed_map, packed_stride,
    },
    read, read_slice,
    walk::Outcome,
};

#[cfg(test)]
use crate::memory::gather_records;

pub mod child_fronts;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanRow {
    pub owner_group: u64,
    pub instance: u32,
    pub front: u64,
    pub front_valid: u8,
    pub section: u32,
    pub segment_start: i32,
    pub segment_end: i32,
    pub blitz: u8,
    pub withdraw: u8,
    pub fallback: u8,
    pub virtual_order: u8,
    pub has_child: u8,
    pub child_fronts: Option<child_fronts::Inputs>,
    pub owner_override: u64,
    pub invasion_transports: Option<i32>,
    pub execution: Option<OrderExecutionInputs>,
    pub assigned_start: usize,
    pub assigned_len: usize,
    pub predecessors_start: usize,
    pub predecessors_len: usize,
    pub successors_start: usize,
    pub successors_len: usize,
    pub names: [(usize, usize); 6],
}
impl PlanRow {
    pub fn key(self) -> u128 {
        (u128::from(self.owner_group) << 32) | u128::from(self.instance)
    }
}

pub(crate) fn census_instance(
    visits: &[Visit],
    pointer: usize,
    owner: usize,
) -> Result<u32, &'static str> {
    visits
        .get(visit_index(visits, pointer))
        .filter(|visit| pointer != 0 && visit.pointer == pointer && visit.owner == owner)
        .map(|visit| visit.instance)
        .ok_or("root order absent from its group's census")
}

pub fn plan_field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|p| read(memory, p))
        .ok_or("plan field unreadable or address overflow")
}

#[inline(always)]
pub fn gathered_field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    record: Gathered<'_>,
) -> Result<T, &'static str> {
    match record.get(offset) {
        Some(value) => Ok(value),
        None => plan_field(memory, pointer, offset),
    }
}

fn invasion_transports(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    record: Gathered<'_>,
) -> Option<i32> {
    let expected = engine_sites::at(base, INVASION_SUBSCRIBER_VTABLE_RVA)? as u64;
    if gathered_field::<u64>(memory, pointer, 8, record).ok()? != expected {
        return None;
    }
    gathered_field(memory, pointer, INVASION_TRANSPORTS, record).ok()
}

#[derive(Clone, Copy, Debug)]
pub struct PlanWalk {
    pub outcome: Outcome,
    pub assigned: usize,
    pub required_assigned: usize,
    pub links: usize,
    pub names: usize,
}

#[derive(Debug, Default)]
pub struct AssignedSources {
    pointers: Vec<usize>,
    used: usize,
    required: usize,
    failure: Option<&'static str>,
}
impl AssignedSources {
    pub fn try_with_capacity(capacity: usize) -> std::io::Result<Self> {
        Ok(Self {
            pointers: crate::allocation::filled(capacity, 0)?,
            ..Self::default()
        })
    }
    pub fn reset(&mut self) {
        self.used = 0;
        self.required = 0;
        self.failure = None;
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.failure = Some(reason);
    }
    pub fn try_grow(&mut self, capacity: usize) -> std::io::Result<()> {
        let capacity = capacity.max(self.pointers.len());
        crate::allocation::resize_default(&mut self.pointers, capacity)?;
        self.reset();
        Ok(())
    }
    pub fn used(&self) -> usize {
        self.used
    }
    pub fn required(&self) -> usize {
        self.required
    }
    pub fn pointers(&self) -> &[usize] {
        &self.pointers[..self.used]
    }
    #[cfg(test)]
    pub(crate) fn fixture(pointers: Vec<usize>) -> Self {
        let used = pointers.len();
        Self {
            pointers,
            used,
            required: used,
            failure: None,
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        self.pointers.capacity() * size_of::<usize>()
    }
    pub fn outcome(&self) -> Outcome {
        match self.failure {
            Some(reason) => Outcome::Unavailable { reason },
            None => Outcome::Rows { len: self.used },
        }
    }
    fn retain(&mut self, ordinal: usize, pointer: usize) {
        if ordinal == self.used && self.failure.is_none() {
            if ordinal == self.pointers.len() {
                let result = crate::allocation::add(ordinal, 1)
                    .and_then(|len| crate::allocation::resize_default(&mut self.pointers, len));
                if result.is_err() {
                    self.failure = Some("plan assigned host allocation failed");
                    return;
                }
            }
            self.pointers[ordinal] = pointer;
            self.used += 1;
        }
    }
}

fn plan_order_ids(
    memory: &impl Memory,
    owner: usize,
    array: Array,
    visits: &[Visit],
    output: &mut Vec<u32>,
    used: &mut usize,
) -> Result<(usize, usize), &'static str> {
    let start = *used;
    let mut scratch = [std::mem::MaybeUninit::<u8>::uninit(); 64 * 8];
    for first in (0..array.count).step_by(64) {
        let count = (array.count - first).min(64) as usize;
        let address = array
            .element(first, 8)
            .ok_or("plan graph address overflow")?;
        let Some(bytes) =
            crate::memory::read_slice_uninit(memory, address, &mut scratch[..count * 8])
        else {
            return Err("plan graph pointers unreadable");
        };
        for bytes in bytes.chunks_exact(8) {
            let pointer = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
            let visit = visits
                .get(visit_index(visits, pointer))
                .filter(|visit| pointer != 0 && visit.pointer == pointer && visit.owner == owner)
                .ok_or("plan graph reference absent from owning census")?;
            grow_payload(output, *used + 1, "plan graph allocation failed")?;
            output[*used] = visit.instance;
            *used += 1;
        }
    }
    Ok((start, *used - start))
}

fn append_census_ids(
    ids: &[u32],
    output: &mut Vec<u32>,
    used: &mut usize,
) -> Result<(usize, usize), &'static str> {
    let start = *used;
    let end = start
        .checked_add(ids.len())
        .ok_or("plan graph allocation failed")?;
    grow_payload(output, end, "plan graph allocation failed")?;
    output[start..end].copy_from_slice(ids);
    *used = end;
    Ok((start, ids.len()))
}

pub(crate) const PLAN_BATCH: usize = 16;

#[inline(always)]
fn named_type(kind: i32) -> bool {
    matches!(kind, TYPE_MOVE | TYPE_INVASION | TYPE_PARADROP)
}

const PLAN_SPANS: [(usize, usize); 7] = [
    (0, OWNER + 8),
    (SEGMENT_START, 8),
    (MOVE_SPECIAL, WITHDRAW + 1 - MOVE_SPECIAL),
    (
        PREDECESSOR_DATA,
        ASSIGNED_MEMBER_DATA + 16 - PREDECESSOR_DATA,
    ),
    (ID, FRONT_SECTION + 4 - ID),
    (FALLBACK, 2),
    (HAS_CHILD, OWNER_OVERRIDE + 8 - HAS_CHILD),
];
const PLAN_FIELDS: [RecordField; 7] = packed_fields(PLAN_SPANS);
const PLAN_STRIDE: usize = packed_stride(PLAN_SPANS);
const PLAN_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PLAN_FIELDS, PLAN_STRIDE).expect("valid order plan layout");
const PLAN_MAP: [u16; OWNER_OVERRIDE + 8] = packed_map(PLAN_SPANS);

const NAMES: (usize, usize) = {
    let (mut first, mut end, mut index) = (usize::MAX, 0, 0);
    while index < NAME_STRINGS.len() {
        let at = NAME_STRINGS[index];
        if at < first {
            first = at;
        }
        if at + crate::msvc_string::STRIDE > end {
            end = at + crate::msvc_string::STRIDE;
        }
        index += 1;
    }
    assert!(
        end - first == NAME_STRINGS.len() * crate::msvc_string::STRIDE,
        "the name strings are one contiguous span"
    );
    assert!(
        first > WITHDRAW && end <= PREDECESSOR_DATA,
        "the name strings lie between the gathered plan spans"
    );
    (first, end)
};
const NAME_SPANS: [(usize, usize); 1] = [(NAMES.0, NAMES.1 - NAMES.0)];
const NAME_FIELDS: [RecordField; 1] = packed_fields(NAME_SPANS);
const NAME_STRIDE: usize = packed_stride(NAME_SPANS);
const NAME_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&NAME_FIELDS, NAME_STRIDE).expect("valid order name layout");
const NAME_MAP: [u16; NAMES.1] = packed_map(NAME_SPANS);

fn prefetch_plan(memory: &impl Memory, lists: &SuccessorLists, visit: &Visit) {
    if lists.plans.record(visit).is_some() {
        lists.plans.prefetch(visit);
    } else {
        for (offset, len) in PLAN_SPANS {
            if let Some(address) = visit.pointer.checked_add(offset) {
                memory.prefetch(address, len);
            }
        }
    }
}

fn prefetch_dependents(
    memory: &impl Memory,
    record: Gathered<'_>,
    visit: &Visit,
    lists: &SuccessorLists,
) {
    for offset in [PREDECESSOR_DATA, SUCCESSOR_DATA] {
        if let Some(array) = record
            .get::<[u8; 16]>(offset)
            .and_then(|header| Array::from_header(header, 8))
            .filter(|array| array.count != 0)
            && (offset == PREDECESSOR_DATA || lists.ids(visit, array).is_none())
        {
            memory.prefetch(array.data, array.count as usize * 8);
        }
    }
    if record.get::<i32>(TYPE).is_some_and(named_type)
        && lists.plans.names(visit).is_none()
        && let Some(address) = visit.pointer.checked_add(NAMES.0)
    {
        memory.prefetch(address, NAME_STRIDE);
    }
    if let Some(owner) = record
        .get::<u64>(OWNER_OVERRIDE)
        .filter(|owner| *owner != 0)
    {
        memory.prefetch(owner as usize, corders_group::HANDLE + 8);
    }
}

const NO_ROW: u8 = u8::MAX;

struct OrderBatch {
    visits: [Visit; PLAN_BATCH],
    size: usize,
    rows: [u8; PLAN_BATCH],
    records: [u8; PLAN_BATCH * PLAN_STRIDE],
    complete: [bool; PLAN_BATCH],
    name_rows: [u8; PLAN_BATCH],
    names: [u8; PLAN_BATCH * NAME_STRIDE],
    named: [bool; PLAN_BATCH],
}

impl OrderBatch {
    fn new() -> Self {
        Self {
            visits: [Visit::default(); PLAN_BATCH],
            size: 0,
            rows: [NO_ROW; PLAN_BATCH],
            records: [0; PLAN_BATCH * PLAN_STRIDE],
            complete: [false; PLAN_BATCH],
            name_rows: [NO_ROW; PLAN_BATCH],
            names: [0; PLAN_BATCH * NAME_STRIDE],
            named: [false; PLAN_BATCH],
        }
    }

    #[inline(always)]
    fn record<'a>(&'a self, lists: &'a SuccessorLists, index: usize) -> Gathered<'a> {
        match self.rows[index] {
            NO_ROW => lists
                .plans
                .record(&self.visits[index])
                .unwrap_or(Gathered::NONE),
            row => Gathered::row(
                &PLAN_LAYOUT,
                &PLAN_MAP,
                &self.records,
                &self.complete,
                usize::from(row),
            ),
        }
    }

    #[inline(always)]
    fn names<'a>(&'a self, lists: &'a SuccessorLists, index: usize) -> Gathered<'a> {
        match self.name_rows[index] {
            NO_ROW => lists
                .plans
                .names(&self.visits[index])
                .unwrap_or(Gathered::NONE),
            row => Gathered::row(
                &NAME_LAYOUT,
                &NAME_MAP,
                &self.names,
                &self.named,
                usize::from(row),
            ),
        }
    }

    fn stage<'a>(
        &mut self,
        memory: &impl Memory,
        lists: &SuccessorLists,
        census: &mut impl Iterator<Item = &'a Visit>,
    ) {
        self.size = 0;
        for (slot, visit) in self.visits.iter_mut().zip(census) {
            *slot = *visit;
            self.size += 1;
        }
        self.rows = [NO_ROW; PLAN_BATCH];
        self.name_rows = [NO_ROW; PLAN_BATCH];
        if self.size == 0 {
            return;
        }
        let mut pointers = [0_usize; PLAN_BATCH];
        let mut rows = 0;
        for (index, visit) in self.visits[..self.size].iter().enumerate() {
            if lists.plans.record(visit).is_none() {
                pointers[rows] = visit.pointer;
                self.rows[index] = rows as u8;
                rows += 1;
            }
        }
        if rows != 0 {
            gather_records_once(
                memory,
                &pointers[..rows],
                &PLAN_LAYOUT,
                &mut self.records,
                &mut self.complete,
                crate::batch_fallback::Batch::OrderPlans,
            );
        }
        for (index, visit) in self.visits[..self.size].iter().enumerate() {
            prefetch_dependents(memory, self.record(lists, index), visit, lists);
        }
    }

    fn stage_names(&mut self, memory: &impl Memory, lists: &SuccessorLists) {
        let mut pointers = [0_usize; PLAN_BATCH];
        let mut rows = 0;
        for index in 0..self.size {
            if self
                .record(lists, index)
                .get::<i32>(TYPE)
                .is_some_and(named_type)
                && lists.plans.names(&self.visits[index]).is_none()
            {
                pointers[rows] = self.visits[index].pointer;
                self.name_rows[index] = rows as u8;
                rows += 1;
            }
        }
        if rows != 0 {
            gather_records_once(
                memory,
                &pointers[..rows],
                &NAME_LAYOUT,
                &mut self.names,
                &mut self.named,
                crate::batch_fallback::Batch::OrderPlans,
            );
        }
        for index in 0..self.size {
            let names = self.names(lists, index);
            for offset in NAME_STRINGS {
                if let Some(header) = names.get::<[u8; crate::msvc_string::STRIDE]>(offset)
                    && let Some((address, len)) = crate::msvc_string::heap_span(&header)
                {
                    memory.prefetch(address, len);
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn walk_plans(
    memory: &impl Memory,
    base: usize,
    groups: &[corders_group::Root],
    visits: &[Visit],
    source: Outcome,
    output: &mut Vec<PlanRow>,
    assigned: &mut Vec<u64>,
    links: &mut Vec<u32>,
    names: &mut Vec<u8>,
) -> PlanWalk {
    walk_plans_ordered(
        memory,
        base,
        groups,
        visits,
        &SuccessorLists::default(),
        source,
        output,
        assigned,
        links,
        names,
        &mut PlanOrder::default(),
    )
}

#[derive(Debug, Default)]
pub struct PlanOrder {
    sorted: Vec<(u128, u32)>,
    rows: Vec<u32>,
    sequence: Vec<u32>,
}

impl PlanOrder {
    pub fn rows(&self) -> &[u32] {
        &self.rows
    }

    pub fn allocated_bytes(&self) -> usize {
        self.sorted.capacity() * size_of::<(u128, u32)>()
            + (self.rows.capacity() + self.sequence.capacity()) * size_of::<u32>()
    }

    fn place(
        &mut self,
        groups: &[corders_group::Root],
        census: CensusOrders<'_>,
        len: usize,
    ) -> Result<bool, ()> {
        self.sorted.clear();
        self.rows.clear();
        crate::allocation::reserve(&mut self.sorted, len).map_err(|_| ())?;
        crate::allocation::reserve(&mut self.rows, len).map_err(|_| ())?;
        self.sorted.extend(
            census
                .map(|(_, visit)| visit)
                .enumerate()
                .map(|(ordinal, visit)| {
                    let group = groups.get(visit.owner).map_or(0, |group| group.handle);
                    (
                        (u128::from(group) << 32) | u128::from(visit.instance),
                        ordinal as u32,
                    )
                }),
        );
        self.sorted.sort_unstable_by_key(|&(key, _)| key);
        let repeated = self.sorted.windows(2).any(|pair| pair[0].0 == pair[1].0);
        self.rows.resize(len, 0);
        for (row, &(_, ordinal)) in self.sorted.iter().enumerate() {
            self.rows[ordinal as usize] = row as u32;
        }
        Ok(repeated)
    }
}

#[derive(Clone, Copy)]
pub struct PlanPart<'a> {
    pub walk: PlanWalk,
    pub rows: &'a [PlanRow],
    pub assigned: &'a [u64],
    pub links: &'a [u32],
    pub names: &'a [u8],
    pub order: &'a PlanOrder,
    pub hosts: Option<&'a AssignedSources>,
    pub orders: &'a [Row],
}

#[allow(clippy::too_many_arguments)]
pub fn stitch_plans(
    parts: &[PlanPart<'_>],
    census: Outcome,
    output: &mut Vec<PlanRow>,
    assigned: &mut Vec<u64>,
    links: &mut Vec<u32>,
    names: &mut Vec<u8>,
    order: &mut PlanOrder,
    mut hosts: Option<&mut AssignedSources>,
) -> PlanWalk {
    if let Some(hosts) = hosts.as_deref_mut() {
        hosts.reset();
    }
    order.rows.clear();
    let mut report = PlanWalk {
        outcome: Outcome::Rows { len: 0 },
        assigned: 0,
        required_assigned: 0,
        links: 0,
        names: 0,
    };
    let result = (|| {
        if !matches!(census, Outcome::Rows { .. }) {
            return Err("plans require a complete army order census");
        }
        let reached = parts
            .iter()
            .position(|part| matches!(part.walk.outcome, Outcome::Unavailable { .. }))
            .map_or(parts.len(), |failed| failed + 1);
        for part in &parts[..reached] {
            report.assigned = add(report.assigned, part.walk.assigned)?;
            report.required_assigned = add(report.required_assigned, part.walk.required_assigned)?;
            report.links = add(report.links, part.walk.links)?;
            report.names = add(report.names, part.walk.names)?;
        }
        grow_payload(assigned, report.assigned, "plan assigned allocation failed")?;
        grow_payload(links, report.links, "plan graph allocation failed")?;
        grow_payload(names, report.names, "plan name allocation failed")?;
        let (mut members, mut ids, mut bytes) = (0, 0, 0);
        for part in &parts[..reached] {
            assigned[members..members + part.walk.assigned]
                .copy_from_slice(&part.assigned[..part.walk.assigned]);
            links[ids..ids + part.walk.links].copy_from_slice(&part.links[..part.walk.links]);
            names[bytes..bytes + part.walk.names].copy_from_slice(&part.names[..part.walk.names]);
            members += part.walk.assigned;
            ids += part.walk.links;
            bytes += part.walk.names;
        }
        if let Some(Outcome::Unavailable { reason }) =
            parts[..reached].last().map(|part| part.walk.outcome)
        {
            return Err(reason);
        }
        let rows = parts
            .iter()
            .try_fold(0_usize, |sum, part| add(sum, part.walk.outcome.len()))?;
        grow_payload(output, rows, "plan row allocation failed")?;
        crate::allocation::reserve(&mut order.rows, rows)
            .map_err(|_| "plan order allocation failed")?;
        let (mut row, mut members, mut ids, mut bytes) = (0, 0, 0, 0);
        for part in parts {
            let len = part.walk.outcome.len();
            for (at, source) in output[row..row + len].iter_mut().zip(&part.rows[..len]) {
                let mut moved = *source;
                moved.assigned_start += members;
                moved.predecessors_start += ids;
                moved.successors_start += ids;
                *at = moved;
            }
            let plans = &part.rows[..len];
            for census in part.orders.iter().filter(|row| named_type(row.order_type)) {
                let at = plans
                    .binary_search_by_key(&census.key(), |plan| plan.key())
                    .map_err(|_| "plan order absent from its census part")?;
                for span in &mut output[row + at].names {
                    span.0 += bytes;
                }
            }
            let base = u32::try_from(row).map_err(|_| "plan order allocation failed")?;
            for &placed in part.order.rows() {
                order.rows.push(
                    placed
                        .checked_add(base)
                        .ok_or("plan order allocation failed")?,
                );
            }
            row += len;
            members += part.walk.assigned;
            ids += part.walk.links;
            bytes += part.walk.names;
        }
        debug_assert!(
            output[..row]
                .windows(2)
                .all(|pair| pair[0].key() < pair[1].key())
        );
        Ok(Outcome::Rows { len: row })
    })();
    report.outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
    let failed = matches!(report.outcome, Outcome::Unavailable { .. });
    if let Some(hosts) = hosts {
        if matches!(census, Outcome::Rows { .. }) {
            for part in parts {
                let Some(lane) = part.hosts else {
                    hosts.failure = Some("plan assigned hosts unavailable");
                    break;
                };
                hosts.required = hosts.required.saturating_add(lane.required);
                if hosts.failure.is_none() {
                    for &pointer in lane.pointers() {
                        let ordinal = hosts.used;
                        hosts.retain(ordinal, pointer);
                    }
                    if hosts.failure.is_none()
                        && lane.failure.is_some()
                        && !matches!(part.walk.outcome, Outcome::Unavailable { .. })
                    {
                        hosts.failure = lane.failure;
                    }
                }
                if matches!(part.walk.outcome, Outcome::Unavailable { .. }) {
                    break;
                }
            }
        }
        if failed {
            hosts.failure = Some("plan assigned hosts require complete core plans");
        }
    }
    if failed {
        order.rows.clear();
        report.required_assigned = report.assigned;
    }
    report
}

fn add(a: usize, b: usize) -> Result<usize, &'static str> {
    a.checked_add(b).ok_or("plan arena length overflow")
}

#[inline]
pub fn keyed_row<T>(
    rows: &[T],
    guess: Option<u32>,
    key: u128,
    key_of: impl Fn(&T) -> u128,
) -> Result<usize, usize> {
    if let Some(guess) = guess.map(|guess| guess as usize)
        && rows.get(guess).is_some_and(|row| key_of(row) == key)
    {
        return Ok(guess);
    }
    rows.binary_search_by_key(&key, key_of)
}

#[allow(clippy::too_many_arguments)]
pub fn walk_plans_ordered(
    memory: &impl Memory,
    base: usize,
    groups: &[corders_group::Root],
    visits: &[Visit],
    lists: &SuccessorLists,
    source: Outcome,
    output: &mut Vec<PlanRow>,
    assigned: &mut Vec<u64>,
    links: &mut Vec<u32>,
    names: &mut Vec<u8>,
    order: &mut PlanOrder,
) -> PlanWalk {
    walk_plans_ordered_with_sources(
        memory, base, groups, visits, lists, source, output, assigned, links, names, order, None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn walk_plans_ordered_with_sources(
    memory: &impl Memory,
    base: usize,
    groups: &[corders_group::Root],
    visits: &[Visit],
    lists: &SuccessorLists,
    source: Outcome,
    output: &mut Vec<PlanRow>,
    assigned: &mut Vec<u64>,
    links: &mut Vec<u32>,
    names: &mut Vec<u8>,
    order: &mut PlanOrder,
    mut hosts: Option<&mut AssignedSources>,
) -> PlanWalk {
    if let Some(hosts) = hosts.as_deref_mut() {
        hosts.reset();
    }
    let mut report = PlanWalk {
        outcome: Outcome::Rows { len: 0 },
        assigned: 0,
        required_assigned: 0,
        links: 0,
        names: 0,
    };
    let mut sequence = std::mem::take(&mut order.sequence);
    let result = (|| {
        let Outcome::Rows { len: expected } = source else {
            return Err("plans require a complete army order census");
        };
        if visits.iter().filter(|v| v.pointer != 0).count() != expected {
            return Err("plan source census count mismatch");
        }
        let census_order = if lists.census().is_empty() {
            census_order_of(visits, expected, &mut sequence);
            &sequence[..]
        } else {
            lists.census()
        };
        let census = || census_orders(visits, census_order).map(|(_, visit)| visit);
        let invalid = plan_field::<u64>(
            memory,
            engine_sites::at(base, crate::creference_registry::INVALID_REFERENCE_RVA)
                .ok_or("plan field unreadable or address overflow")?,
            0,
        )?;
        grow_payload(output, expected, "plan row allocation failed")?;
        let placed = order.place(groups, census_orders(visits, census_order), expected);
        let mut len = 0;
        let mut ahead = census();
        for visit in ahead.by_ref().take(3 * PLAN_BATCH) {
            prefetch_plan(memory, lists, visit);
        }
        let mut census = census();
        let mut members = [0_u8; 64 * 8];
        let mut hosts_gathered = [0_u8; 64 * ASSIGNED_SPAN];
        let mut host_pointers = [0_usize; 64];
        let mut storage = [OrderBatch::new(), OrderBatch::new(), OrderBatch::new()];
        let [current, next, after] = &mut storage;
        let (mut current, mut next, mut after) = (current, next, after);
        current.stage(memory, lists, &mut census);
        next.stage(memory, lists, &mut census);
        current.stage_names(memory, lists);
        while current.size != 0 {
            after.stage(memory, lists, &mut census);
            next.stage_names(memory, lists);
            for (index, visit) in current.visits[..current.size].iter().enumerate() {
                if let Some(next) = ahead.next() {
                    prefetch_plan(memory, lists, next);
                }
                let record = current.record(lists, index);
                let group = groups
                    .get(visit.owner)
                    .ok_or("plan owner absent from census")?;
                let pointer = visit.pointer;
                let field = |offset| gathered_field::<u64>(memory, pointer, offset, record);
                let vtable = engine_sites::at(base, VTABLE_RVA)
                    .ok_or("plan order identity mismatch")? as u64;
                if record
                    .get::<u64>(0)
                    .or_else(|| read::<u64>(memory, pointer))
                    != Some(vtable)
                    || field(OWNER)? != group.pointer as u64
                {
                    return Err("plan order identity mismatch");
                }
                let front = field(FRONT_BINDING)?;
                let owner = field(OWNER_OVERRIDE)? as usize;
                let owner_override = if owner == 0 {
                    0
                } else {
                    corders_group::plan_pointer_handle(memory, base, owner)?
                };
                let mut row = PlanRow {
                    owner_group: group.handle,
                    instance: gathered_field(memory, pointer, ID, record)?,
                    front,
                    front_valid: u8::from(front != invalid),
                    section: gathered_field(memory, pointer, FRONT_SECTION, record)?,
                    segment_start: gathered_field(memory, pointer, SEGMENT_START, record)?,
                    segment_end: gathered_field(memory, pointer, SEGMENT_END, record)?,
                    blitz: gathered_field(memory, pointer, MOVE_SPECIAL, record)?,
                    withdraw: gathered_field(memory, pointer, WITHDRAW, record)?,
                    fallback: gathered_field(memory, pointer, FALLBACK, record)?,
                    virtual_order: gathered_field(memory, pointer, VIRTUAL, record)?,
                    has_child: gathered_field(memory, pointer, HAS_CHILD, record)?,
                    owner_override,
                    ..Default::default()
                };
                if row.instance != visit.instance {
                    return Err("plan order census identity changed");
                }
                for (offset, start, len) in [
                    (
                        PREDECESSOR_DATA,
                        &mut row.predecessors_start,
                        &mut row.predecessors_len,
                    ),
                    (
                        SUCCESSOR_DATA,
                        &mut row.successors_start,
                        &mut row.successors_len,
                    ),
                ] {
                    let address = pointer
                        .checked_add(offset)
                        .ok_or("plan graph address overflow")?;
                    let array = record
                        .get::<[u8; 16]>(offset)
                        .or_else(|| read::<[u8; 16]>(memory, address))
                        .and_then(|header| Array::from_header(header, 8))
                        .ok_or("plan graph header invalid")?;
                    let listed = if offset == SUCCESSOR_DATA {
                        lists.ids(visit, array)
                    } else {
                        None
                    };
                    (*start, *len) = match listed {
                        Some(ids) => append_census_ids(ids, links, &mut report.links)?,
                        None => plan_order_ids(
                            memory,
                            visit.owner,
                            array,
                            visits,
                            links,
                            &mut report.links,
                        )?,
                    };
                }
                let order_type = gathered_field::<i32>(memory, pointer, TYPE, record)?;
                if order_type == TYPE_INVASION {
                    row.invasion_transports = invasion_transports(memory, base, pointer, record);
                }
                if named_type(order_type) {
                    let names_record = current.names(lists, index);
                    for (span, offset) in row.names.iter_mut().zip(NAME_STRINGS) {
                        let address = pointer
                            .checked_add(offset)
                            .ok_or("plan name address overflow")?;
                        *span = match names_record.get::<[u8; crate::msvc_string::STRIDE]>(offset) {
                            Some(header) => crate::msvc_string::append_with_header(
                                memory,
                                &header,
                                names,
                                &mut report.names,
                            )?,
                            None => crate::msvc_string::append(
                                memory,
                                address,
                                names,
                                &mut report.names,
                            )?,
                        };
                    }
                }
                let at = pointer
                    .checked_add(ASSIGNED_MEMBER_DATA)
                    .ok_or("plan member address overflow")?;
                let array = record
                    .get::<[u8; 16]>(ASSIGNED_MEMBER_DATA)
                    .or_else(|| read::<[u8; 16]>(memory, at))
                    .and_then(|header| Array::from_header(header, 8))
                    .ok_or("plan assigned member header invalid")?;
                let count = array.count as usize;
                report.required_assigned = report
                    .required_assigned
                    .checked_add(count)
                    .ok_or("plan assigned length overflow")?;
                if let Some(hosts) = hosts.as_deref_mut() {
                    hosts.required = report.required_assigned;
                }
                row.assigned_start = report.assigned;
                row.assigned_len = count;
                for first in (0..array.count).step_by(64) {
                    let batched = (array.count - first).min(64);
                    let copied = array.element(first, 8).is_some_and(|address| {
                        read_slice(memory, address, &mut members[..batched as usize * 8])
                    });
                    let (mut attempted, mut gathered) = (0, 0);
                    if copied {
                        for slot in members[..batched as usize * 8].chunks_exact(8) {
                            match (u64::from_le_bytes(slot.try_into().unwrap()) as usize)
                                .checked_sub(crate::cunit::SCHEDULED_MEMBER)
                                .filter(|&unit| unit != 0)
                            {
                                Some(unit) => host_pointers[attempted] = unit,
                                None => break,
                            }
                            attempted += 1;
                        }
                        gathered = memory.copy_records(
                            RecordSources::Pointers(&host_pointers[..attempted]),
                            &ASSIGNED_LAYOUT,
                            &mut hosts_gathered[..attempted * ASSIGNED_SPAN],
                        );
                    }
                    if !copied || gathered < attempted {
                        crate::batch_fallback::record(crate::batch_fallback::Batch::OrderPlans);
                    }
                    for index in first..first + batched {
                        let member = (if copied {
                            let at = (index - first) as usize * 8;
                            Some(u64::from_le_bytes(members[at..at + 8].try_into().unwrap()))
                        } else {
                            read::<u64>(
                                memory,
                                array
                                    .element(index, 8)
                                    .ok_or("plan member address overflow")?,
                            )
                        })
                        .filter(|v| *v != 0)
                        .ok_or("plan member null or unreadable")?
                            as usize;
                        let unit = member
                            .checked_sub(crate::cunit::SCHEDULED_MEMBER)
                            .ok_or("plan member host underflow")?;
                        let slot = (index - first) as usize;
                        let header = (slot < attempted).then(|| {
                            (slot < gathered).then(|| {
                                hosts_gathered[slot * ASSIGNED_SPAN..][..ASSIGNED_SPAN]
                                    .try_into()
                                    .unwrap()
                            })
                        });
                        let handle = assigned_handle(memory, base, unit, header)?;
                        grow_payload(
                            assigned,
                            report.assigned + 1,
                            "plan assigned allocation failed",
                        )?;
                        assigned[report.assigned] = handle;
                        if let Some(hosts) = hosts.as_deref_mut() {
                            hosts.retain(report.assigned, unit);
                        }
                        report.assigned += 1;
                    }
                }
                let at = if placed.is_ok() {
                    order.rows[len] as usize
                } else {
                    len
                };
                output[at] = row;
                len += 1;
            }
            (current, next, after) = (next, after, current);
        }
        let repeated = placed.map_err(|()| "plan order allocation failed")?;
        if repeated {
            return Err("duplicate plan order key");
        }
        Ok(Outcome::Rows { len })
    })();
    order.sequence = sequence;
    report.outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
    if matches!(report.outcome, Outcome::Unavailable { .. }) {
        order.rows.clear();
        if let Some(hosts) = hosts {
            hosts.failure = Some("plan assigned hosts require complete core plans");
        }
        report.required_assigned = report.assigned;
    }
    report
}

const ASSIGNED_SPAN: usize = crate::cunit::HANDLE + 8;
const ASSIGNED_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: ASSIGNED_SPAN,
}];
const ASSIGNED_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ASSIGNED_FIELDS, ASSIGNED_SPAN).expect("valid assigned host layout");

fn assigned_handle(
    memory: &impl Memory,
    base: usize,
    unit: usize,
    header: Option<Option<[u8; ASSIGNED_SPAN]>>,
) -> Result<u64, &'static str> {
    let header = header.unwrap_or_else(|| read::<[u8; ASSIGNED_SPAN]>(memory, unit));
    let word =
        |at: usize| header.map(|header| u64::from_le_bytes(header[at..at + 8].try_into().unwrap()));
    let primary = engine_sites::at(base, crate::carmy::VTABLE_RVA)
        .ok_or("plan assigned division identity mismatch")? as u64;
    if word(0).or_else(|| read::<u64>(memory, unit)) != Some(primary)
        || match word(crate::carmy::SECONDARY_BASE) {
            Some(value) => value,
            None => plan_field::<u64>(memory, unit, crate::carmy::SECONDARY_BASE)?,
        } != engine_sites::at(base, crate::carmy::SECONDARY_VTABLE_RVA)
            .ok_or("module overflow")? as u64
    {
        return Err("plan assigned division identity mismatch");
    }
    let handle = match word(crate::cunit::HANDLE) {
        Some(value) => value,
        None => plan_field::<u64>(memory, unit, crate::cunit::HANDLE)?,
    };
    if handle == 0 {
        return Err("plan assigned division null handle");
    }
    Ok(handle)
}

pub const VTABLE_RVA: Rva = Rva(0x02a0_26e8);
const INVASION_SUBSCRIBER_VTABLE_RVA: Rva = Rva(0x029d_8b80);
const INVASION_TRANSPORTS: usize = 0x10;
pub const TYPE: usize = 0x30;
pub const OWNER: usize = 0x38;
pub const PATH_DATA: usize = 0x70;
#[cfg(test)]
pub const PATH_CAPACITY: usize = 0x78;
pub const PARADROP_TARGET: usize = 0xb8;
pub const ACCUMULATED_PREPARATION: usize = 0xd8;
pub const TARGET_STATE_DATA: usize = 0xe0;
pub const SEGMENT_START: usize = 0xa0;
pub const SEGMENT_END: usize = 0xa4;
pub const FRONT_SECTION: usize = 0x260;
pub const HAS_CHILD: usize = 0x370;
pub const MOVE_SPECIAL: usize = 0x118;
pub const WITHDRAW: usize = 0x119;
pub const CONDITION_CACHE: usize = 0x11a;
pub const PREDECESSOR_DATA: usize = 0x1e0;
pub const SUCCESSOR_DATA: usize = 0x1f8;
pub const ASSIGNED_MEMBER_DATA: usize = 0x210;
pub const ID: usize = 0x244;
pub const ACTIVE: usize = 0x248;
pub const FRONT_BINDING: usize = 0x258;
pub const FALLBACK: usize = 0x298;
pub const VIRTUAL: usize = 0x299;
pub const VIRTUAL_SUCCESSOR_DATA: usize = 0x2d0;
pub const OWNER_OVERRIDE: usize = 0x378;
pub const NAME_STRINGS: [usize; 6] = [0x1a0, 0x120, 0x140, 0x160, 0x180, 0x1c0];
pub const TYPE_MOVE: i32 = 1;
pub const TYPE_LINE: i32 = 2;
pub const TYPE_INVASION: i32 = 3;
pub const TYPE_PARADROP: i32 = 4;
pub const TYPE_AREA_DEFENSE: i32 = 5;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub owner_group: u64,
    pub instance: u32,
    pub country: u32,
    pub order_type: i32,
    pub active: u8,
    pub paradrop_target: u32,
    pub path_start: usize,
    pub path_len: usize,
    pub target_start: usize,
    pub target_len: usize,
}

impl Row {
    pub fn key(self) -> u128 {
        (u128::from(self.owner_group) << 32) | u128::from(self.instance)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Visit {
    pub(crate) pointer: usize,
    pub(crate) owner: usize,
    instance: u32,
    successor_data: usize,
    successor_count: u32,
    next: u32,
    parent: usize,
    successors: u32,
    record: u32,
    sequence: u32,
    completed_graphs: u8,
    active: bool,
}
impl Visit {
    pub(crate) fn instance(self) -> u32 {
        self.instance
    }
    #[cfg(test)]
    pub(crate) fn fixture(pointer: usize, owner: usize, instance: u32) -> Self {
        Self {
            pointer,
            owner,
            instance,
            ..Self::default()
        }
    }
}

const NO_NAME_ROW: u32 = u32::MAX;

#[derive(Default)]
struct PlanRecords {
    records: Vec<u8>,
    complete: Vec<bool>,
    name_rows: Vec<u32>,
    names: Vec<u8>,
    named: Vec<bool>,
    len: usize,
    names_len: usize,
    gathered: usize,
    names_gathered: usize,
    pointers: [usize; PLAN_BATCH],
    name_pointers: [usize; PLAN_BATCH],
    staging: bool,
    reads: crate::memory::ReadCounts,
    ticks: u64,
}

impl std::fmt::Debug for PlanRecords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanRecords")
            .field("records", &&self.records[..self.gathered * PLAN_STRIDE])
            .field("complete", &&self.complete[..self.gathered])
            .field("name_rows", &&self.name_rows[..self.gathered])
            .field("names", &&self.names[..self.names_gathered * NAME_STRIDE])
            .field("named", &&self.named[..self.names_gathered])
            .field("len", &self.len)
            .field("names_len", &self.names_len)
            .field("staging", &self.staging)
            .field("reads", &self.reads)
            .finish()
    }
}

impl PlanRecords {
    fn allocated_bytes(&self) -> usize {
        self.records.capacity()
            + self.complete.capacity()
            + self.name_rows.capacity() * size_of::<u32>()
            + self.names.capacity()
            + self.named.capacity()
    }

    fn begin(&mut self) {
        self.len = 0;
        self.names_len = 0;
        self.gathered = 0;
        self.names_gathered = 0;
        self.staging = true;
        self.reads = crate::memory::ReadCounts::ZERO;
        self.ticks = 0;
    }

    fn discard(&mut self) {
        self.len = 0;
        self.names_len = 0;
        self.gathered = 0;
        self.names_gathered = 0;
    }

    fn stage(&mut self, memory: &impl Memory, pointer: usize, order_type: i32) -> u32 {
        if !self.staging {
            return 0;
        }
        let index = self.len;
        let Ok(tag) = u32::try_from(index + 1) else {
            self.staging = false;
            return 0;
        };
        if grow_payload(&mut self.records, (index + 1) * PLAN_STRIDE, "").is_err()
            || grow_payload(&mut self.complete, index + 1, "").is_err()
            || grow_payload(&mut self.name_rows, index + 1, "").is_err()
        {
            self.staging = false;
            return 0;
        }
        for (offset, len) in &PLAN_SPANS[1..] {
            memory.prefetch(pointer.wrapping_add(*offset), *len);
        }
        self.name_rows[index] = NO_NAME_ROW;
        if named_type(order_type)
            && let Ok(row) = u32::try_from(self.names_len)
            && row != NO_NAME_ROW
            && grow_payload(&mut self.names, (self.names_len + 1) * NAME_STRIDE, "").is_ok()
            && grow_payload(&mut self.named, self.names_len + 1, "").is_ok()
        {
            memory.prefetch(pointer.wrapping_add(NAMES.0), NAME_STRIDE);
            self.name_rows[index] = row;
            self.name_pointers[self.names_len - self.names_gathered] = pointer;
            self.names_len += 1;
        }
        self.pointers[index - self.gathered] = pointer;
        self.len += 1;
        if self.len - self.gathered == PLAN_BATCH {
            self.gather(memory);
        }
        tag
    }

    fn gather(&mut self, memory: &impl Memory) {
        let (first, end) = (self.gathered, self.len);
        let (names_first, names_end) = (self.names_gathered, self.names_len);
        if first == end {
            return;
        }
        let begin = crate::clock::qpc();
        let census = crate::memory::ReadCounts::take_current();
        gather_records_once(
            memory,
            &self.pointers[..end - first],
            &PLAN_LAYOUT,
            &mut self.records[first * PLAN_STRIDE..end * PLAN_STRIDE],
            &mut self.complete[first..end],
            crate::batch_fallback::Batch::OrderPlans,
        );
        if names_first != names_end {
            gather_records_once(
                memory,
                &self.name_pointers[..names_end - names_first],
                &NAME_LAYOUT,
                &mut self.names[names_first * NAME_STRIDE..names_end * NAME_STRIDE],
                &mut self.named[names_first..names_end],
                crate::batch_fallback::Batch::OrderPlans,
            );
        }
        self.reads += memory.take_deferred_reads();
        census.add_current();
        self.gathered = end;
        self.names_gathered = names_end;
        self.ticks += crate::clock::since(begin);
    }

    #[inline(always)]
    fn record(&self, visit: &Visit) -> Option<Gathered<'_>> {
        let index = (visit.record as usize).checked_sub(1)?;
        (index < self.gathered).then(|| {
            Gathered::row(
                &PLAN_LAYOUT,
                &PLAN_MAP,
                &self.records,
                &self.complete,
                index,
            )
        })
    }

    #[inline(always)]
    fn names(&self, visit: &Visit) -> Option<Gathered<'_>> {
        let index = (visit.record as usize).checked_sub(1)?;
        if index >= self.gathered {
            return None;
        }
        let row = *self.name_rows.get(index)?;
        (row != NO_NAME_ROW && (row as usize) < self.names_gathered).then(|| {
            Gathered::row(
                &NAME_LAYOUT,
                &NAME_MAP,
                &self.names,
                &self.named,
                row as usize,
            )
        })
    }

    #[inline(always)]
    fn prefetch(&self, visit: &Visit) {
        let Some(index) = (visit.record as usize).checked_sub(1) else {
            return;
        };
        if let Some(record) = self
            .records
            .get(index * PLAN_STRIDE..(index + 1) * PLAN_STRIDE)
        {
            crate::prefetch::span(record.as_ptr() as usize, PLAN_STRIDE);
        }
        if let Some(&row) = self.name_rows.get(index)
            && row != NO_NAME_ROW
            && let Some(names) = self
                .names
                .get(row as usize * NAME_STRIDE..(row as usize + 1) * NAME_STRIDE)
        {
            crate::prefetch::span(names.as_ptr() as usize, NAME_STRIDE);
        }
    }
}

#[derive(Debug, Default)]
pub struct SuccessorLists {
    words: Vec<u32>,
    pending: Vec<u32>,
    listing: bool,
    plans: PlanRecords,
    census: Vec<u32>,
}

impl SuccessorLists {
    pub fn allocated_bytes(&self) -> usize {
        (self.words.capacity() + self.pending.capacity() + self.census.capacity())
            * size_of::<u32>()
            + self.plans.allocated_bytes()
    }

    pub fn census(&self) -> &[u32] {
        &self.census
    }

    pub fn plan_gathers(&self) -> (crate::memory::ReadCounts, u64) {
        (self.plans.reads, self.plans.ticks)
    }

    fn begin(&mut self) {
        self.words.clear();
        self.pending.clear();
        self.census.clear();
        self.listing = true;
        self.plans.begin();
    }

    fn finish_census(&mut self, memory: &impl Memory, outcome: Outcome, visits: &[Visit]) {
        if let Outcome::Rows { len } = outcome {
            self.plans.gather(memory);
            self.order(visits, len);
        } else {
            self.words.clear();
            self.plans.discard();
        }
        self.pending.clear();
    }

    fn order(&mut self, visits: &[Visit], len: usize) {
        census_order_of(visits, len, &mut self.census);
    }

    fn push(&mut self, instance: u32) {
        if self.listing {
            if self.pending.len() == self.pending.capacity()
                && crate::allocation::reserve(&mut self.pending, 1).is_err()
            {
                self.stop();
                return;
            }
            self.pending.push(instance);
        }
    }

    fn finish(&mut self, data: usize, count: u32) -> u32 {
        let count_usize = count as usize;
        if !self.listing {
            return 0;
        }
        if self.pending.len() < count_usize {
            self.stop();
            return 0;
        }
        let start = self.words.len();
        let Ok(tag) = u32::try_from(start + 1) else {
            self.stop();
            return 0;
        };
        if crate::allocation::reserve(&mut self.words, 3 + count_usize).is_err() {
            self.stop();
            return 0;
        }
        let ids = self.pending.len() - count_usize;
        self.words
            .extend_from_slice(&[data as u32, (data as u64 >> 32) as u32, count]);
        self.words.extend_from_slice(&self.pending[ids..]);
        self.pending.truncate(ids);
        tag
    }

    fn stop(&mut self) {
        self.listing = false;
        self.pending.clear();
    }

    fn ids(&self, visit: &Visit, array: Array) -> Option<&[u32]> {
        let start = (visit.successors as usize).checked_sub(1)?;
        let header = self.words.get(start..start.checked_add(3)?)?;
        let data = u64::from(header[0]) | u64::from(header[1]) << 32;
        if data != array.data as u64 || header[2] != array.count {
            return None;
        }
        self.words.get(start + 3..start + 3 + array.count as usize)
    }
}

#[derive(Clone)]
pub enum CensusOrders<'a> {
    Ordered {
        visits: &'a [Visit],
        order: std::slice::Iter<'a, u32>,
    },
    Table(std::iter::Enumerate<std::slice::Iter<'a, Visit>>),
}

impl<'a> Iterator for CensusOrders<'a> {
    type Item = (usize, &'a Visit);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Ordered { visits, order } => {
                let visits = *visits;
                order.next().map(|&slot| {
                    let slot = slot as usize;
                    (slot, &visits[slot])
                })
            }
            Self::Table(table) => table.find(|(_, visit)| visit.pointer != 0),
        }
    }
}

fn census_sequence(before: usize, ordinal: u32) -> Result<u32, &'static str> {
    u32::try_from(before)
        .ok()
        .and_then(|before| before.checked_add(ordinal))
        .ok_or("army order census sequence overflows")
}

fn census_order_of(visits: &[Visit], len: usize, census: &mut Vec<u32>) {
    const UNSET: u32 = u32::MAX;
    census.clear();
    if crate::allocation::reserve(census, len).is_err() {
        return;
    }
    census.resize(len, UNSET);
    for (slot, visit) in visits.iter().enumerate() {
        if visit.pointer == 0 {
            continue;
        }
        match census.get_mut(visit.sequence as usize) {
            Some(entry) if *entry == UNSET && slot < UNSET as usize => *entry = slot as u32,
            _ => {
                census.clear();
                return;
            }
        }
    }
    if census.contains(&UNSET) {
        census.clear();
    }
}

pub fn census_orders<'a>(visits: &'a [Visit], order: &'a [u32]) -> CensusOrders<'a> {
    if order.is_empty() {
        CensusOrders::Table(visits.iter().enumerate())
    } else {
        CensusOrders::Ordered {
            visits,
            order: order.iter(),
        }
    }
}

#[derive(Clone, Copy)]
enum Edge {
    Successor,
    VirtualSuccessor,
}

impl Edge {
    fn mask(self) -> u8 {
        match self {
            Self::Successor => 1,
            Self::VirtualSuccessor => 2,
        }
    }

    fn offset(self) -> usize {
        match self {
            Self::Successor => SUCCESSOR_DATA,
            Self::VirtualSuccessor => VIRTUAL_SUCCESSOR_DATA,
        }
    }

    fn cycle_reason(self) -> &'static str {
        match self {
            Self::Successor => "army order successor cycle",
            Self::VirtualSuccessor => "army order virtual successor cycle",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub root_outcome: Outcome,
    pub geometry_len: usize,
    pub required_geometry: usize,
    pub required_groups: usize,
    pub minimum_visits: usize,
    pub visited: usize,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<Row>,
    geometry: &mut Vec<u32>,
    groups: &mut Vec<corders_group::Root>,
    visits: &mut Vec<Visit>,
) -> Walk {
    walk_census(memory, base, output, geometry, groups, visits, None)
}

pub fn walk_listing_successors(
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<Row>,
    geometry: &mut Vec<u32>,
    groups: &mut Vec<corders_group::Root>,
    visits: &mut Vec<Visit>,
    lists: &mut SuccessorLists,
) -> Walk {
    lists.begin();
    let walk = walk_census(
        memory,
        base,
        output,
        geometry,
        groups,
        visits,
        Some(&mut *lists),
    );
    lists.finish_census(memory, walk.outcome, visits);
    walk
}

#[allow(clippy::too_many_arguments)]
pub fn walk_listing_successors_of(
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<Row>,
    geometry: &mut Vec<u32>,
    groups: &[corders_group::Root],
    root_outcome: Outcome,
    visits: &mut Vec<Visit>,
    lists: &mut SuccessorLists,
) -> Walk {
    lists.begin();
    let walk = walk_graph_census(
        memory,
        base,
        output,
        geometry,
        groups,
        root_outcome,
        visits,
        Some(&mut *lists),
    );
    lists.finish_census(memory, walk.outcome, visits);
    walk
}

#[allow(clippy::too_many_arguments)]
pub fn walk_listing_successors_part(
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<Row>,
    geometry: &mut Vec<u32>,
    groups: &[corders_group::Root],
    root_outcome: Outcome,
    range: std::ops::Range<usize>,
    visits: &mut Vec<Visit>,
    lists: &mut SuccessorLists,
) -> PartWalk {
    lists.begin();
    let part = walk_graph_census_part(
        memory,
        base,
        output,
        geometry,
        groups,
        root_outcome,
        Some(range),
        visits,
        Some(&mut *lists),
    );
    lists.finish_census(memory, part.walk.outcome, visits);
    part
}

#[derive(Clone, Copy)]
pub struct CensusPart<'a> {
    pub walk: PartWalk,
    pub rows: &'a [Row],
    pub geometry: &'a [u32],
    pub visits: &'a [Visit],
}

pub fn stitch_census(
    parts: &[CensusPart<'_>],
    output: &mut Vec<Row>,
    geometry: &mut Vec<u32>,
    visits: &mut Vec<Visit>,
    lists: &mut SuccessorLists,
) -> Walk {
    lists.begin();
    lists.listing = false;
    let Some(first) = parts.first() else {
        let none = Outcome::Unavailable {
            reason: "orders census has no parts",
        };
        return Walk {
            outcome: none,
            root_outcome: none,
            geometry_len: 0,
            required_geometry: 0,
            required_groups: 0,
            minimum_visits: 0,
            visited: 0,
        };
    };
    if let Some(part) = parts.iter().find(|part| part.walk.before_groups) {
        return part.walk.walk;
    }
    let mut report = Walk {
        outcome: Outcome::Rows { len: 0 },
        root_outcome: first.walk.walk.root_outcome,
        geometry_len: 0,
        required_geometry: 0,
        required_groups: first.walk.walk.required_groups,
        minimum_visits: 0,
        visited: 0,
    };
    let result = (|| {
        let entered = parts.iter().try_fold(0_usize, |sum, part| {
            sum.checked_add(part.walk.walk.visited)
                .ok_or("army order traversal length overflow")
        })?;
        let len = entered
            .checked_mul(4)
            .map(|four| (four / 3 + 1).next_power_of_two().max(4))
            .ok_or("army order traversal length overflow")?;
        for visit in visits.iter_mut() {
            *visit = Visit::default();
        }
        if visits.len() < len {
            let more = len - visits.len();
            crate::allocation::reserve(visits, more)
                .map_err(|_| "army order traversal allocation failed")?;
            visits.resize(len, Visit::default());
        }
        let mut stop = None;
        for (index, part) in parts.iter().enumerate() {
            let entered_before = |pointer: usize| {
                visits
                    .get(visit_index(visits, pointer))
                    .is_some_and(|visit| visit.pointer == pointer)
            };
            let shared = part
                .visits
                .iter()
                .filter(|visit| visit.pointer != 0 && entered_before(visit.pointer))
                .map(|visit| visit.sequence as usize)
                .min();
            let shared = match part.walk.failure {
                Some(PartFailure {
                    entered,
                    entering: Some(pointer),
                    ..
                }) if entered_before(pointer) => {
                    Some(shared.map_or(entered, |shared| shared.min(entered)))
                }
                _ => shared,
            };
            let own = part.walk.failure.map(|failure| {
                let Outcome::Unavailable { reason } = part.walk.walk.outcome else {
                    unreachable!("a failed census part is unavailable");
                };
                (failure, reason)
            });
            match (shared, own) {
                (Some(shared), Some((failure, reason)))
                    if failure.entered < shared
                        || (failure.entered == shared && failure.entering.is_none()) =>
                {
                    stop = Some((index, failure.entered, reason, true));
                }
                (Some(shared), _) => {
                    stop = Some((
                        index,
                        shared,
                        "army order shared across owning groups",
                        false,
                    ));
                }
                (None, Some((failure, reason))) => {
                    stop = Some((index, failure.entered, reason, true));
                }
                (None, None) => {}
            }
            let admitted = stop.map_or(usize::MAX, |(_, admitted, _, _)| admitted);
            for visit in part
                .visits
                .iter()
                .filter(|visit| visit.pointer != 0 && (visit.sequence as usize) < admitted)
            {
                let sequence = census_sequence(report.visited, visit.sequence)?;
                let at = visit_index(visits, visit.pointer);
                visits[at] = Visit {
                    pointer: visit.pointer,
                    owner: visit.owner,
                    instance: visit.instance,
                    sequence,
                    ..Visit::default()
                };
            }
            if let Some((_, admitted, reason, own)) = stop {
                let mut row = 0;
                let mut word = 0;
                for (at, part) in parts[..=index].iter().enumerate() {
                    let limit = if at == index { admitted } else { usize::MAX };
                    let entries = part_entries(part, limit);
                    let words = part_geometry(part, limit);
                    let rows = row + entries.len();
                    grow_payload(output, rows, "army order row allocation failed")?;
                    grow_payload(
                        geometry,
                        word + words,
                        "army order geometry allocation failed",
                    )?;
                    for (at, source) in output[row..rows].iter_mut().zip(entries) {
                        *at = Row {
                            path_start: source.path_start + word,
                            target_start: source.target_start + word,
                            ..*source
                        };
                    }
                    geometry[word..word + words].copy_from_slice(&part.geometry[..words]);
                    row = rows;
                    word += words;
                }
                report.visited += admitted.min(part.walk.walk.visited);
                report.geometry_len = word;
                report.required_geometry = word;
                if own {
                    report.minimum_visits = part.walk.walk.minimum_visits;
                }
                return Err(reason);
            }
            report.visited += part.walk.walk.visited;
        }
        let rows = parts.iter().try_fold(0_usize, |sum, part| {
            sum.checked_add(part.rows_len())
                .ok_or("army order row length overflow")
        })?;
        let words = parts.iter().try_fold(0_usize, |sum, part| {
            sum.checked_add(part.walk.walk.geometry_len)
                .ok_or("army order geometry length overflow")
        })?;
        grow_payload(output, rows, "army order row allocation failed")?;
        grow_payload(geometry, words, "army order geometry allocation failed")?;
        let (mut row, mut word) = (0, 0);
        for part in parts {
            let len = part.rows_len();
            let geometry_len = part.walk.walk.geometry_len;
            for (at, source) in output[row..row + len].iter_mut().zip(&part.rows[..len]) {
                *at = Row {
                    path_start: source.path_start + word,
                    target_start: source.target_start + word,
                    ..*source
                };
            }
            geometry[word..word + geometry_len].copy_from_slice(&part.geometry[..geometry_len]);
            row += len;
            word += geometry_len;
            report.required_geometry = report
                .required_geometry
                .checked_add(part.walk.walk.required_geometry)
                .ok_or("army order geometry length overflow")?;
        }
        report.geometry_len = word;
        output[..row].sort_unstable_by_key(|row| row.key());
        if output[..row]
            .windows(2)
            .any(|pair| pair[0].key() == pair[1].key())
        {
            return Err("duplicate native army order reference");
        }
        lists.order(visits, row);
        Ok(Outcome::Rows { len: row })
    })();
    match result {
        Ok(outcome) => report.outcome = outcome,
        Err(reason) => {
            report.outcome = Outcome::Unavailable { reason };
            lists.census.clear();
        }
    }
    report
}

fn part_entries<'a>(part: &CensusPart<'a>, limit: usize) -> &'a [Row] {
    &part.rows[..limit.min(part.rows_len())]
}

fn part_geometry(part: &CensusPart<'_>, limit: usize) -> usize {
    part.rows[..part.rows_len()]
        .get(limit)
        .map_or(part.walk.walk.geometry_len, |row| row.path_start)
}

fn walk_census(
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<Row>,
    geometry: &mut Vec<u32>,
    groups: &mut Vec<corders_group::Root>,
    visits: &mut Vec<Visit>,
    lists: Option<&mut SuccessorLists>,
) -> Walk {
    let root_outcome = corders_group::walk_roots(memory, base, groups);
    walk_graph_census(
        memory,
        base,
        output,
        geometry,
        groups,
        root_outcome,
        visits,
        lists,
    )
}

fn root_refusal(root_outcome: Outcome) -> Result<usize, Walk> {
    let mut report = Walk {
        outcome: Outcome::Rows { len: 0 },
        root_outcome,
        geometry_len: 0,
        required_geometry: 0,
        required_groups: 0,
        minimum_visits: 0,
        visited: 0,
    };
    match root_outcome {
        Outcome::Rows { len } => return Ok(len),
        Outcome::Truncated { len, unread_rows } => {
            report.required_groups = len + unread_rows as usize;
            report.outcome = Outcome::Unavailable {
                reason: "orders group scratch capacity exhausted",
            };
        }
        outcome @ Outcome::Unavailable { .. } => report.outcome = outcome,
    }
    Err(report)
}

pub fn refuse_census(root_outcome: Outcome, lists: &mut SuccessorLists) -> Option<Walk> {
    let walk = root_refusal(root_outcome).err()?;
    lists.begin();
    Some(walk)
}

#[allow(clippy::too_many_arguments)]
fn walk_graph_census(
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<Row>,
    geometry: &mut Vec<u32>,
    groups: &[corders_group::Root],
    root_outcome: Outcome,
    visits: &mut Vec<Visit>,
    lists: Option<&mut SuccessorLists>,
) -> Walk {
    walk_graph_census_part(
        memory,
        base,
        output,
        geometry,
        groups,
        root_outcome,
        None,
        visits,
        lists,
    )
    .walk
}

#[allow(clippy::too_many_arguments)]
fn walk_graph_census_part(
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<Row>,
    geometry: &mut Vec<u32>,
    groups: &[corders_group::Root],
    root_outcome: Outcome,
    range: Option<std::ops::Range<usize>>,
    visits: &mut Vec<Visit>,
    lists: Option<&mut SuccessorLists>,
) -> PartWalk {
    let group_count = match root_refusal(root_outcome) {
        Ok(len) => len,
        Err(report) => return PartWalk::refused(report),
    };
    let mut report = Walk {
        outcome: Outcome::Rows { len: 0 },
        root_outcome,
        geometry_len: 0,
        required_geometry: 0,
        required_groups: group_count,
        minimum_visits: 0,
        visited: 0,
    };
    let Some(groups) = groups.get(..group_count) else {
        report.outcome = Outcome::Unavailable {
            reason: "orders group census exceeds its roots",
        };
        return PartWalk::refused(report);
    };
    let parted = range.is_some();
    let range = range.unwrap_or(0..group_count);
    if range.start > range.end || range.end > group_count {
        report.outcome = Outcome::Unavailable {
            reason: "orders census part outside its roots",
        };
        return PartWalk::refused(report);
    }
    for visit in visits.iter_mut() {
        visit.pointer = 0;
    }
    let mut reader = Graph {
        memory,
        base,
        output,
        geometry,
        groups,
        visits,
        lists,
        report,
        len: 0,
        visited: 0,
        range,
        entering: None,
    };
    let mut failure = None;
    match reader.run() {
        Ok(()) if parted => {
            reader.report.outcome = Outcome::Rows { len: reader.len };
        }
        Ok(()) => {
            reader.output[..reader.len].sort_unstable_by_key(|row| row.key());
            if reader.output[..reader.len]
                .windows(2)
                .any(|pair| pair[0].key() == pair[1].key())
            {
                reader.report.outcome = Outcome::Unavailable {
                    reason: "duplicate native army order reference",
                };
            } else {
                reader.report.outcome = Outcome::Rows { len: reader.len };
            }
        }
        Err(reason) => {
            reader.report.outcome = Outcome::Unavailable { reason };
            reader.report.required_geometry = reader.report.geometry_len;
            failure = Some(PartFailure {
                entered: reader.visited,
                entering: reader.entering,
            });
        }
    }
    reader.report.visited = reader.visited;
    PartWalk {
        walk: reader.report,
        failure,
        before_groups: false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PartFailure {
    entered: usize,
    entering: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartWalk {
    pub walk: Walk,
    failure: Option<PartFailure>,
    before_groups: bool,
}

impl PartWalk {
    fn refused(walk: Walk) -> Self {
        Self {
            walk,
            failure: None,
            before_groups: true,
        }
    }
}

impl CensusPart<'_> {
    fn rows_len(&self) -> usize {
        self.walk.walk.visited.min(self.rows.len())
    }
}

struct Graph<'a, M> {
    memory: &'a M,
    base: usize,
    output: &'a mut Vec<Row>,
    geometry: &'a mut Vec<u32>,
    groups: &'a [corders_group::Root],
    visits: &'a mut Vec<Visit>,
    lists: Option<&'a mut SuccessorLists>,
    report: Walk,
    len: usize,
    visited: usize,
    range: std::ops::Range<usize>,
    entering: Option<usize>,
}

const ORDER_ROSTERS: usize =
    corders_group::TERTIARY_ORDER_DATA + 16 - corders_group::PRIMARY_ORDER_DATA;
const _: () = assert!(
    corders_group::SECONDARY_ORDER_DATA == corders_group::PRIMARY_ORDER_DATA + 0x18
        && corders_group::TERTIARY_ORDER_DATA == corders_group::SECONDARY_ORDER_DATA + 0x18
);

const CENSUS_SPANS: [(usize, usize); 8] = [
    (0, 8),
    (OWNER, 8),
    (ID, 4),
    (TYPE, 4),
    (ACTIVE, 1),
    (PARADROP_TARGET, 4),
    (PATH_DATA, 16),
    (TARGET_STATE_DATA, 16),
];
const CENSUS_FIELDS: [RecordField; 8] = packed_fields(CENSUS_SPANS);
const CENSUS_STRIDE: usize = packed_stride(CENSUS_SPANS);
const CENSUS_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&CENSUS_FIELDS, CENSUS_STRIDE).expect("valid census layout");

impl<M: Memory> Graph<'_, M> {
    fn run(&mut self) -> Result<(), &'static str> {
        for owner in self.range.clone() {
            let lists = self.groups[owner]
                .pointer
                .checked_add(corders_group::PRIMARY_ORDER_DATA)
                .and_then(|at| read::<[u8; ORDER_ROSTERS]>(self.memory, at));
            for offset in [
                corders_group::PRIMARY_ORDER_DATA,
                corders_group::SECONDARY_ORDER_DATA,
                corders_group::TERTIARY_ORDER_DATA,
            ] {
                let roots = match &lists {
                    Some(lists) => {
                        let at = offset - corders_group::PRIMARY_ORDER_DATA;
                        Array::from_header(lists[at..at + 16].try_into().unwrap(), 8)
                            .ok_or("army order vector unreadable or invalid")?
                    }
                    None => self.array(self.groups[owner].pointer, offset, 8)?,
                };
                for first in (0..roots.count).step_by(PLAN_BATCH) {
                    let count = (roots.count - first).min(PLAN_BATCH as u32) as usize;
                    let mut slots = [0_u8; PLAN_BATCH * 8];
                    let copied = roots
                        .element(first, 8)
                        .is_some_and(|at| read_slice(self.memory, at, &mut slots[..count * 8]));
                    let mut pointers = [0_usize; PLAN_BATCH];
                    let mut staged = 0;
                    if copied {
                        for raw in slots[..count * 8].chunks_exact(8) {
                            let Some(pointer) =
                                usize::try_from(u64::from_le_bytes(raw.try_into().unwrap()))
                                    .ok()
                                    .filter(|&pointer| pointer != 0)
                            else {
                                break;
                            };
                            pointers[staged] = pointer;
                            for header in [
                                PATH_DATA,
                                TARGET_STATE_DATA,
                                SUCCESSOR_DATA,
                                VIRTUAL_SUCCESSOR_DATA,
                            ] {
                                if let Some(at) = pointer.checked_add(header) {
                                    self.memory.prefetch(at, 16);
                                }
                            }
                            staged += 1;
                        }
                    } else {
                        crate::batch_fallback::record(crate::batch_fallback::Batch::OrderCensus);
                    }
                    let mut records = [0_u8; PLAN_BATCH * CENSUS_STRIDE];
                    let mut complete = [false; PLAN_BATCH];
                    gather_records_once(
                        self.memory,
                        &pointers[..staged],
                        &CENSUS_LAYOUT,
                        &mut records[..staged * CENSUS_STRIDE],
                        &mut complete,
                        crate::batch_fallback::Batch::OrderCensus,
                    );
                    for slot in 0..count {
                        let pointer = if copied {
                            usize::try_from(u64::from_le_bytes(
                                slots[slot * 8..][..8].try_into().unwrap(),
                            ))
                            .ok()
                            .filter(|&pointer| pointer != 0)
                            .ok_or("army order pointer unreadable or null")?
                        } else {
                            self.pointer(roots, first + slot as u32)?
                        };
                        let record = complete[slot]
                            .then(|| &records[slot * CENSUS_STRIDE..][..CENSUS_STRIDE]);
                        if offset == corders_group::PRIMARY_ORDER_DATA {
                            self.walk_graph(pointer, owner, Edge::Successor, record)?;
                            self.walk_graph(pointer, owner, Edge::VirtualSuccessor, record)?;
                        } else {
                            self.enter(pointer, owner, usize::MAX, None, record)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn walk_graph(
        &mut self,
        pointer: usize,
        owner: usize,
        edge: Edge,
        record: Option<&[u8]>,
    ) -> Result<(), &'static str> {
        let (_, Some(mut current)) = self.enter(pointer, owner, usize::MAX, Some(edge), record)?
        else {
            return Ok(());
        };
        let listing = matches!(edge, Edge::Successor);
        loop {
            let visit = self.visits[current];
            if visit.next == visit.successor_count {
                self.visits[current].completed_graphs |= edge.mask();
                self.visits[current].active = false;
                if listing && let Some(lists) = self.lists.as_deref_mut() {
                    self.visits[current].successors =
                        lists.finish(visit.successor_data, visit.successor_count);
                }
                if visit.parent == usize::MAX {
                    return Ok(());
                }
                current = visit.parent;
                continue;
            }
            self.visits[current].next += 1;
            let pointer = self.pointer(
                Array {
                    data: visit.successor_data,
                    count: visit.successor_count,
                },
                visit.next,
            )?;
            let (instance, child) = self.enter(pointer, owner, current, Some(edge), None)?;
            if listing && let Some(lists) = self.lists.as_deref_mut() {
                lists.push(instance);
            }
            if let Some(child) = child {
                current = child;
            }
        }
    }

    fn array(&self, pointer: usize, offset: usize, stride: usize) -> Result<Array, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|p| Array::read(self.memory, p, stride))
            .ok_or("army order vector unreadable or invalid")
    }

    fn pointer(&self, array: Array, index: u32) -> Result<usize, &'static str> {
        array
            .element(index, 8)
            .and_then(|p| read::<u64>(self.memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("army order pointer unreadable or null")
    }

    fn enter(
        &mut self,
        pointer: usize,
        owner: usize,
        parent: usize,
        edge: Option<Edge>,
        record: Option<&[u8]>,
    ) -> Result<(u32, Option<usize>), &'static str> {
        let mut index = visit_index(self.visits, pointer);
        if let Some(entry) = self
            .visits
            .get(index)
            .filter(|entry| entry.pointer == pointer)
        {
            if entry.owner != owner {
                return Err("army order shared across owning groups");
            }
            let instance = entry.instance;
            return Ok((instance, self.begin_graph(index, parent, edge)?));
        }
        self.entering = Some(pointer);
        let sequence = census_sequence(self.visited, 0)?;
        let parent = if self.visits.is_empty() || self.visited >= self.visits.len() * 3 / 4 {
            let parent = self.grow_visits(parent)?;
            index = visit_index(self.visits, pointer);
            parent
        } else {
            parent
        };
        let expected = engine_sites::at(self.base, VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let at = |offset| {
            pointer
                .checked_add(offset)
                .ok_or("army order address overflow")
        };
        let mut vtable = [0; 8];
        let mut raw_owner = [0; 8];
        let mut raw_instance = [0; 4];
        let mut raw_type = [0; 4];
        let mut raw_active = [0; 1];
        let mut raw_target = [0; 4];
        let headers = [
            Some(PATH_DATA),
            Some(TARGET_STATE_DATA),
            edge.map(Edge::offset),
            (parent == usize::MAX && matches!(edge, Some(Edge::Successor)))
                .then_some(VIRTUAL_SUCCESSOR_DATA),
        ];
        for offset in headers.into_iter().flatten() {
            if let Some(address) = pointer.checked_add(offset) {
                self.memory.prefetch(address, 16);
            }
        }
        let copied = if let Some(record) = record {
            vtable.copy_from_slice(&record[..8]);
            raw_owner.copy_from_slice(&record[8..16]);
            raw_instance.copy_from_slice(&record[16..20]);
            raw_type.copy_from_slice(&record[20..24]);
            raw_active.copy_from_slice(&record[24..25]);
            raw_target.copy_from_slice(&record[25..29]);
            6
        } else {
            self.memory.copy_scattered_prefetched(
                [
                    pointer,
                    at(OWNER)?,
                    at(ID)?,
                    at(TYPE)?,
                    at(ACTIVE)?,
                    at(PARADROP_TARGET)?,
                ],
                &mut [
                    &mut vtable,
                    &mut raw_owner,
                    &mut raw_instance,
                    &mut raw_type,
                    &mut raw_active,
                    &mut raw_target,
                ],
            )
        };
        if copied == 0 || u64::from_le_bytes(vtable) != expected {
            return Err("army order vtable mismatch");
        }
        if copied < 2 {
            return Err("army order owner unreadable");
        }
        let actual_owner = u64::from_le_bytes(raw_owner);
        if actual_owner != self.groups[owner].pointer as u64 {
            return Err("army order owner backlink mismatch");
        }
        if copied < 6 {
            return Err([
                "army order instance unreadable",
                "army order type unreadable",
                "army order active byte unreadable",
                "army order paradrop target unreadable",
            ][copied - 2]);
        }
        let instance = u32::from_le_bytes(raw_instance);
        let order_type = i32::from_le_bytes(raw_type);
        let active = raw_active[0];
        let paradrop_target = u32::from_le_bytes(raw_target);
        let header = |offset, start| match record {
            Some(record) => Array::from_header(record[start..start + 16].try_into().unwrap(), 4)
                .ok_or("army order vector unreadable or invalid"),
            None => self.array(pointer, offset, 4),
        };
        let path = header(PATH_DATA, 29)?;
        let targets = header(TARGET_STATE_DATA, 45)?;
        let words = (path.count as usize)
            .checked_add(targets.count as usize)
            .ok_or("army order geometry length overflow")?;
        self.report.required_geometry = self
            .report
            .required_geometry
            .checked_add(words)
            .ok_or("army order geometry length overflow")?;
        grow_payload(
            self.output,
            self.len + 1,
            "army order row allocation failed",
        )?;
        let path_start = self.report.geometry_len;
        let target_start = path_start + path.count as usize;
        copy_words(
            self.memory,
            path.data,
            path.count as usize,
            self.geometry,
            path_start,
        )?;
        copy_words(
            self.memory,
            targets.data,
            targets.count as usize,
            self.geometry,
            target_start,
        )?;
        self.output[self.len] = Row {
            owner_group: self.groups[owner].handle,
            instance,
            country: self.groups[owner].country,
            order_type,
            active,
            paradrop_target,
            path_start,
            path_len: path.count as usize,
            target_start,
            target_len: targets.count as usize,
        };
        self.len += 1;
        self.report.geometry_len += words;
        let record = match self.lists.as_deref_mut() {
            Some(lists) => lists.plans.stage(self.memory, pointer, order_type),
            None => 0,
        };
        self.visits[index] = Visit {
            pointer,
            owner,
            instance,
            record,
            sequence,
            ..Visit::default()
        };
        self.visited += 1;
        self.entering = None;
        Ok((instance, self.begin_graph(index, parent, edge)?))
    }

    fn grow_visits(&mut self, parent: usize) -> Result<usize, &'static str> {
        let required = self
            .visits
            .len()
            .checked_mul(2)
            .ok_or("army order traversal length overflow")?
            .max(4);
        self.report.minimum_visits = required;
        let mut expanded = Vec::new();
        crate::allocation::reserve_exact(&mut expanded, required)
            .map_err(|_| "army order traversal allocation failed")?;
        expanded.resize(required, Visit::default());
        for entry in self.visits.iter().filter(|entry| entry.pointer != 0) {
            let index = visit_index(&expanded, entry.pointer);
            expanded[index] = *entry;
        }
        for index in 0..expanded.len() {
            let entry = expanded[index];
            if entry.active && entry.parent != usize::MAX {
                expanded[index].parent = visit_index(&expanded, self.visits[entry.parent].pointer);
            }
        }
        let parent = if parent == usize::MAX {
            parent
        } else {
            visit_index(&expanded, self.visits[parent].pointer)
        };
        *self.visits = expanded;
        self.report.minimum_visits = 0;
        Ok(parent)
    }

    fn begin_graph(
        &mut self,
        index: usize,
        parent: usize,
        edge: Option<Edge>,
    ) -> Result<Option<usize>, &'static str> {
        let Some(edge) = edge else {
            return Ok(None);
        };
        let entry = self.visits[index];
        if entry.active {
            return Err(edge.cycle_reason());
        }
        if entry.completed_graphs & edge.mask() != 0 {
            return Ok(None);
        }
        let successors = self.array(entry.pointer, edge.offset(), 8)?;
        self.visits[index].successor_data = successors.data;
        self.visits[index].successor_count = successors.count;
        self.visits[index].next = 0;
        self.visits[index].parent = parent;
        self.visits[index].active = true;
        Ok(Some(index))
    }
}

#[inline]
fn visit_index(visits: &[Visit], pointer: usize) -> usize {
    let len = visits.len();
    if len == 0 {
        return 0;
    }
    let mut hash = pointer as u64;
    hash ^= hash >> 30;
    hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash ^= hash >> 27;
    if len.is_power_of_two() {
        let mask = len - 1;
        let mut index = hash as usize & mask;
        for _ in 0..len {
            if visits[index].pointer == 0 || visits[index].pointer == pointer {
                break;
            }
            index = (index + 1) & mask;
        }
        return index;
    }
    let mut index = hash as usize % len;
    for _ in 0..len {
        if visits[index].pointer == 0 || visits[index].pointer == pointer {
            break;
        }
        index = (index + 1) % len;
    }
    index
}

pub(crate) fn grow_payload<T: Default>(
    output: &mut Vec<T>,
    required: usize,
    reason: &'static str,
) -> Result<(), &'static str> {
    if required > output.len() {
        crate::allocation::reserve(output, required - output.len()).map_err(|_| reason)?;
        output.resize_with(required, T::default);
    }
    Ok(())
}

fn copy_words(
    memory: &impl Memory,
    source: usize,
    count: usize,
    output: &mut Vec<u32>,
    start: usize,
) -> Result<(), &'static str> {
    let mut raw = [0_u8; 256];
    for first in (0..count).step_by(64) {
        let len = (count - first).min(64);
        let address = source
            .checked_add(first * 4)
            .ok_or("army order geometry address overflow")?;
        if !read_slice(memory, address, &mut raw[..len * 4]) {
            return Err("army order geometry unreadable");
        }
        grow_payload(
            output,
            start + first + len,
            "army order geometry allocation failed",
        )?;
        let words = &mut output[start + first..start + first + len];
        for (word, bytes) in words.iter_mut().zip(raw.chunks_exact(4)) {
            *word = u32::from_le_bytes(bytes.try_into().unwrap());
        }
    }
    Ok(())
}

#[cfg(test)]
mod plan_batch_tests;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrderExecutionInputs {
    pub route_is_ok: Option<u8>,
    pub paradrop: Option<ParadropPreparationInputs>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParadropPreparationInputs {
    pub progress: i64,
    pub terms: Option<ParadropPreparationTerms>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParadropPreparationTerms {
    pub target: u32,
    pub country_term: i64,
    pub relation_term: Option<i64>,
    pub leader_term: Option<i64>,
    pub days_term: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DivisionOrderTransportInputs {
    pub naval_weight: Option<i64>,
    pub paradrop_eligible: Option<u8>,
    pub paradrop_modifiers: Option<ParadropTransportModifiers>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParadropTransportModifiers {
    pub country_term: i64,
    pub commander_term: Option<i64>,
}
