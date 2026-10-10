use super::*;

// ===========================================================================
// Scheduled raw-copy split (ProjectPrograms, Projects, ProjectHistory,
// ProjectBreakthroughs; independent tables use separate typed acquisition).
//
// The acquisition worker copies raw bytes: one shared country census and
// status read per country, program pointers/rows/strings, project pool
// records with their state machines and history entries, and breakthrough
// tree nodes. The single FIFO decode worker replays the existing typed walks
// against the recorded spans (`crate::raw::Recorded`), so typed rows,
// ordinals, skips, truncation counts and failure reasons are identical by
// construction (see `crate::raw` for the mirror rules).
//
// The three typed walks each run their own census and status reads; the
// replay serves every repeat of a staged key with its bytes, so the mirror
// stages each key once, including a slot's link, canonical pointer and status
// that the walks read again. Pool records, state machines and tree nodes are
// gathered in batches; a batch stages each field as its own span and stops at
// its first failure, which the replay then reaches at the same field.
// ===========================================================================

/// Fault-attribution bits, one per project table in `TABLES[0..4]` order.
pub(super) const M_PROGRAMS: u32 = 0x01;
const M_PROJECTS: u32 = 0x02;
const M_HISTORY: u32 = 0x04;
const M_BREAKTHROUGH: u32 = 0x08;
const M_ALL: u32 = 0x0f;
const M_PROJECT_TABLES: u32 = M_PROJECTS | M_HISTORY;

/// Project identity header length (`HISTORY_COUNT + 4`); the engine keeps
/// this private, so the mirror derives it from the public count constant.
const PROJECT_HEADER_LEN: usize = nproject_cproject::HISTORY_COUNT + 4;
/// Project lifecycle flag length (`RECORD_SIZE - HEADER_LEN`).
const PROJECT_FLAG_LEN: usize = 2;
/// Pool records per gather. The walk's own batch sets error precedence; this
/// one only bounds a guarded call, and stages a superset of each walk batch.
const PROJECT_CHUNK: usize = 64;
const POOL_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: PROJECT_HEADER_LEN,
    },
    RecordField {
        source_offset: nproject_cproject::STARTED,
        destination_offset: PROJECT_HEADER_LEN,
        len: PROJECT_FLAG_LEN,
    },
];
const POOL_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&POOL_FIELDS, PROJECT_HEADER_LEN + PROJECT_FLAG_LEN)
        .expect("valid record layout");
/// Breakthrough node left, parent and right links.
const NODE_LINKS: usize = nproject_breakthrough_node::RIGHT + 8;
/// Staged breakthrough nodes one tree walk remembers.
const TREE_NODES: usize = 64;

/// One refresh of raw project payload. Acquisition workers retain the acquired
/// prefix while growing logs and census scratch from native headers.
#[derive(Debug)]
pub(crate) struct ProjectsJob {
    pub log: RawLog<4>,
    pub game_state: usize,
    pub active: bool,
    pub(super) census: Vec<CensusEntry>,
    /// Program status per canonical census entry, in census order.
    pub(super) statuses: Vec<usize>,
    pub(super) pointers: Vec<usize>,
    pub(super) worker_growth: bool,
    pub(super) capacity_charge: crate::raw_memory::Charge,
}

impl Default for ProjectsJob {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct CensusEntry {
    pub(super) pointer: usize,
    pub(super) id: u32,
    pub(super) slot: u32,
    /// The slot its link names; equal to `slot` for a canonical object.
    pub(super) canonical: u32,
}

/// Shared census borrowed by every scheduled project walk mirror.
#[derive(Clone, Copy)]
pub(super) struct ProjectCensus<'a> {
    pub(super) links: &'a Array,
    pub(super) countries: &'a Array,
    pub(super) vtable: u64,
    /// Slots below this were read; a failed census stops at it.
    pub(super) read_to: u32,
    /// Non-null slots in slot order.
    pub(super) census: &'a [CensusEntry],
}

impl ProjectsJob {
    pub fn new() -> Self {
        let mut job = Self {
            log: RawLog::new(0, 0),
            game_state: 0,
            active: false,
            census: Vec::new(),
            statuses: Vec::new(),
            pointers: Vec::new(),
            worker_growth: true,
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.active = false;
        self.census.clear();
        self.statuses.clear();
        self.pointers.clear();
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Prepare only observed shortage and live census demand off-thread.
    pub fn grow(&mut self) {
        engine::allocation::advisory(|| {
            self.log.grow();
            let countries = self
                .census
                .len()
                .saturating_add(1)
                .max(crate::country_scale::census_rows(0));
            let _ = crate::allocation::reserve_raw(&mut self.census, countries, "projects census");
            let _ =
                crate::allocation::reserve_raw(&mut self.statuses, countries, "project statuses");
            let _ = engine::allocation::reserve(&mut self.pointers, 1);
        });
        self.update_capacity_charge();
    }

    /// Off the game thread: hold one census row per observed country slot so
    /// the first hour after a large save loads does not overflow.
    pub fn presize_countries(&mut self) {
        let census = crate::country_scale::reserve_census(&mut self.census);
        let statuses = crate::country_scale::reserve_census(&mut self.statuses);
        if census || statuses {
            self.update_capacity_charge();
        }
    }

    pub(super) fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            auxiliary_vectors: vector_bytes(&self.census)
                + vector_bytes(&self.statuses)
                + vector_bytes(&self.pointers),
            ..Default::default()
        });
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProjectsCopyReport {
    /// False when staging overflowed. The accepted acquisition cannot be
    /// replayed and must fail without a second live walk.
    pub raw_ok: bool,
    pub copy_ticks: [u64; 4],
    pub raw_bytes: [u64; 4],
}

/// Acquisition-job parameters supplied at the sampling boundary.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProjectsCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub active: bool,
}

fn pool_u32(log: &RawLog<4>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

fn pool_u64(log: &RawLog<4>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

/// One mirrored span read. `None` addrs (address arithmetic overflow) record
/// nothing because the typed walk fails locally without reads; every other
/// failure is logged with the affected-table mask.
pub(super) fn span(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Staged spans of one validated pool record.
#[derive(Clone, Copy, Default)]
struct PoolRecord {
    header: usize,
    machine: usize,
}

/// Mirror of `walk_tables`' read set for every country: pool header, pool
/// records and flags, state machines, their current phases and history
/// entries. Pooled capacities are ignored (a superset is staged); validation,
/// precedence and error scope are local parses, so any failure aborts the
/// walk exactly like the typed error.
fn copy_project_tables(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    shared: ProjectCensus<'_>,
    statuses: &[usize],
) {
    let Some(expected) = base
        .checked_add(nproject_cproject::VTABLE_RVA)
        .map(|expected| expected as u64)
    else {
        return;
    };
    let Some(pool_vtable) = base.checked_add(nproject_cproject_pool::VTABLE_RVA) else {
        return;
    };
    let mut machines_dead = false;
    let mut hist_dead = false;
    for (country, &status) in shared
        .census
        .iter()
        .filter(|entry| entry.canonical == entry.slot)
        .zip(statuses)
    {
        let Some(pool_start) = span(
            log,
            memory,
            status.checked_add(nproject_cprogram_status::PROJECT_POOL),
            8,
            M_PROJECT_TABLES,
        ) else {
            return;
        };
        let Some(pool) = usize::try_from(pool_u64(log, pool_start))
            .ok()
            .filter(|&pool| pool != 0)
        else {
            return;
        };
        // The walk reads the vtable, then the array header.
        let Some(array_address) = pool.checked_add(nproject_cproject_pool::PROJECTS) else {
            return;
        };
        let Some((starts, 2)) = log.take_scattered(
            memory,
            [pool, array_address],
            [8, 16],
            [M_PROJECT_TABLES; 2],
        ) else {
            return;
        };
        if pool_u64(log, starts[0]) != pool_vtable as u64 {
            return;
        }
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[starts[1]..starts[1] + 16]);
        let Some(pool_array) = Array::from_header(raw, nproject_cproject::POOL_STRIDE) else {
            return;
        };
        let mut authenticated_owner = country.id;
        let mut index = 0u32;
        while index < pool_array.count {
            let rows = ((pool_array.count - index) as usize).min(PROJECT_CHUNK);
            // `from_header` proved every record address in range.
            let first = pool_array.data + index as usize * nproject_cproject::POOL_STRIDE;
            let Some((start, completed)) = log.take_records(
                memory,
                RecordSources::Strided {
                    base: first,
                    stride: nproject_cproject::POOL_STRIDE,
                    count: rows,
                },
                &POOL_LAYOUT,
                M_PROJECTS,
            ) else {
                return;
            };
            // A failed header ends the walk; a failed flag word ends only
            // project rows, and the walk goes on after that record.
            let header_failed = completed < rows * 2 && completed.is_multiple_of(2);
            let headers = completed.div_ceil(2);
            let mut records = [PoolRecord::default(); PROJECT_CHUNK];
            let mut valid = 0;
            let mut invalid = false;
            for (ordinal, record) in records.iter_mut().enumerate().take(headers) {
                let header = start + ordinal * POOL_LAYOUT.stride();
                if pool_u64(log, header) != expected
                    || pool_u32(log, header + nproject_cproject::TYPE)
                        != nproject_cproject::REFERENCE_TYPE
                    || pool_u32(log, header + nproject_cproject::ID) == 0
                {
                    invalid = true;
                    break;
                }
                let owner = pool_u32(log, header + nproject_cproject::OWNER);
                if owner != authenticated_owner {
                    match copy_resolve(log, memory, shared, owner, M_PROJECT_TABLES) {
                        Some(pointer) if pointer == country.pointer => {
                            authenticated_owner = owner;
                        }
                        _ => {
                            invalid = true;
                            break;
                        }
                    }
                }
                let Ok(machine) =
                    usize::try_from(pool_u64(log, header + nproject_cproject::STATE_MACHINE))
                else {
                    invalid = true;
                    break;
                };
                *record = PoolRecord { header, machine };
                valid += 1;
            }
            if log.overflowed {
                return;
            }
            if !machines_dead {
                machines_dead = !copy_machines(log, memory, &records[..valid]);
                if log.overflowed {
                    return;
                }
            }
            for record in &records[..valid] {
                if hist_dead {
                    break;
                }
                copy_project_history(log, memory, base, record.header, &mut hist_dead);
                if log.overflowed {
                    return;
                }
            }
            if header_failed || invalid || headers == 0 {
                return;
            }
            index += headers as u32;
        }
    }
}

/// Mirror of one batch's state-machine gathers: the recorded words of every
/// machine, then each one's current phase. False once a gather faults; the
/// walk then reads no further machine.
fn copy_machines(log: &mut RawLog<4>, memory: &impl Memory, records: &[PoolRecord]) -> bool {
    if records.is_empty() {
        return true;
    }
    let mut pointers = [0usize; PROJECT_CHUNK];
    // Machines are scattered heap objects: hint every line first, so the
    // gather's misses overlap instead of arriving one machine at a time.
    for (pointer, record) in pointers.iter_mut().zip(records) {
        *pointer = record.machine;
        engine::prefetch::fields(record.machine, &machine::FIELDS);
    }
    let rows = records.len();
    let Some((start, completed)) = log.take_records(
        memory,
        RecordSources::Pointers(&pointers[..rows]),
        &machine::LAYOUT,
        M_PROJECTS,
    ) else {
        return false;
    };
    let copied = completed / machine::FIELDS.len();
    let mut phases = [0usize; PROJECT_CHUNK];
    for (ordinal, phase) in phases.iter_mut().enumerate().take(copied) {
        let at = start + ordinal * machine::FIELDS_SIZE;
        *phase = machine::phase_source(
            pointers[ordinal],
            log.bytes[at..at + machine::FIELDS_SIZE].try_into().unwrap(),
        );
        engine::prefetch::fields(*phase, &machine::PHASE_FIELDS);
    }
    let Some((_, phased)) = log.take_records(
        memory,
        RecordSources::Pointers(&phases[..copied]),
        &machine::PHASE_LAYOUT,
        M_PROJECTS,
    ) else {
        return false;
    };
    copied == rows && phased == rows
}

/// Mirror of one record's history read set: array header plus every entry's
/// row and context fields in one gather, then present optional payloads. Sets
/// `hist_dead` on the first typed abort point; later history is skipped
/// exactly like the filtered walk.
fn copy_project_history(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    header: usize,
    hist_dead: &mut bool,
) {
    let data = match usize::try_from(pool_u64(log, header + nproject_cproject::HISTORY)) {
        Ok(data) => data,
        Err(_) => {
            *hist_dead = true;
            return;
        }
    };
    let capacity = pool_u32(log, header + nproject_cproject::HISTORY_CAPACITY);
    let count = pool_u32(log, header + nproject_cproject::HISTORY_COUNT);
    if capacity > i32::MAX as u32
        || count > capacity
        || (count != 0 && data == 0)
        || (count as usize)
            .checked_mul(nproject_sproject_history::STRIDE)
            .and_then(|len| data.checked_add(len))
            .is_none()
    {
        *hist_dead = true;
        return;
    }
    let (Some(expected), Some(context_expected)) = (
        base.checked_add(nproject_sproject_history::VTABLE_RVA),
        base.checked_add(nproject_sproject_context::VTABLE_RVA),
    ) else {
        *hist_dead = true;
        return;
    };
    for ordinal in 0..count as usize {
        // The array check above keeps every entry field in range.
        let entry = data + ordinal * nproject_sproject_history::STRIDE;
        let context = entry + nproject_sproject_history::CONTEXT;
        // Row vtable, handles and date, then the context's vtable, fixed
        // words and presence bytes: the walk's read order.
        let Some((starts, completed)) = log.take_scattered(
            memory,
            [
                entry,
                entry + nproject_sproject_history::PROTOTYPE_REWARD_HANDLE,
                entry + nproject_sproject_history::DATE,
                context,
                context + nproject_sproject_context::PROVINCE,
                context + nproject_sproject_context::COUNTRY_PRESENT,
                context + nproject_sproject_context::SCIENTIST_PRESENT,
                context + nproject_sproject_context::STATE_PRESENT,
            ],
            [8, 8, 4, 8, 16, 1, 1, 1],
            [M_HISTORY; 8],
        ) else {
            *hist_dead = true;
            return;
        };
        if completed < 1
            || pool_u64(log, starts[0]) != expected as u64
            || completed < 4
            || pool_u64(log, starts[3]) != context_expected as u64
            || completed < 8
        {
            *hist_dead = true;
            return;
        }
        for (present, offset) in [
            (starts[5], nproject_sproject_context::COUNTRY_REFERENCE),
            (starts[6], nproject_sproject_context::SCIENTIST_REFERENCE),
            (starts[7], nproject_sproject_context::STATE_REFERENCE),
        ] {
            if log.bytes[present] != 0 && log.take(memory, context + offset, 4, M_HISTORY).is_none()
            {
                *hist_dead = true;
                return;
            }
        }
    }
}

/// Parsed breakthrough tree links for the traversal mirror.
#[derive(Clone, Copy, Default)]
struct BreakthroughLinks {
    left: usize,
    parent: usize,
    right: usize,
    nil: bool,
}

fn parse_links(log: &RawLog<4>, pointers: usize, nil: usize) -> Option<BreakthroughLinks> {
    if log.bytes[nil] > 1 {
        return None;
    }
    let pointer = |offset: usize| usize::try_from(pool_u64(log, pointers + offset)).ok();
    Some(BreakthroughLinks {
        left: pointer(nproject_breakthrough_node::LEFT)?,
        parent: pointer(nproject_breakthrough_node::PARENT)?,
        right: pointer(nproject_breakthrough_node::RIGHT)?,
        nil: log.bytes[nil] != 0,
    })
}

/// Mirror of `Links::read` for the tree sentinel: links and nil byte.
fn copy_breakthrough_links(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    address: usize,
) -> Option<BreakthroughLinks> {
    if address == 0 {
        return None;
    }
    let nil = address.checked_add(nproject_breakthrough_node::IS_NIL)?;
    let (starts, 2) =
        log.take_scattered(memory, [address, nil], [NODE_LINKS, 1], [M_BREAKTHROUGH; 2])?
    else {
        return None;
    };
    parse_links(log, starts[0], starts[1])
}

/// One tree's staged nodes, so a successor walk that revisits a node reads
/// nothing again. Bounded; a node past the bound is staged again.
struct TreeNodes {
    nodes: [(usize, BreakthroughLinks, Option<usize>); TREE_NODES],
    len: usize,
}

impl TreeNodes {
    fn new() -> Self {
        Self {
            nodes: [(0, BreakthroughLinks::default(), None); TREE_NODES],
            len: 0,
        }
    }

    /// Mirror of `Links::read` plus the node's payload, which the walk reads
    /// when the node is current. Returns the links and the payload's span.
    fn read(
        &mut self,
        log: &mut RawLog<4>,
        memory: &impl Memory,
        address: usize,
    ) -> Option<(BreakthroughLinks, Option<usize>)> {
        if address == 0 {
            return None;
        }
        if let Some(&(_, links, value)) = self.nodes[..self.len]
            .iter()
            .find(|(node, ..)| *node == address)
        {
            return Some((links, value));
        }
        let nil = address.checked_add(nproject_breakthrough_node::IS_NIL)?;
        let value = address.checked_add(nproject_breakthrough_node::SPECIALIZATION_HANDLE)?;
        let (starts, completed) = log.take_scattered(
            memory,
            [address, nil, value],
            [NODE_LINKS, 1, 8],
            [M_BREAKTHROUGH; 3],
        )?;
        if completed < 2 {
            return None;
        }
        let links = parse_links(log, starts[0], starts[1])?;
        let value = (completed == 3).then_some(starts[2]);
        if self.len < TREE_NODES {
            self.nodes[self.len] = (address, links, value);
            self.len += 1;
        }
        Some((links, value))
    }
}

/// Brent-style cycle guard with no reads, mirroring the typed `Chain`.
struct BreakthroughChain {
    checkpoint: usize,
    power: u64,
    distance: u64,
}

impl BreakthroughChain {
    fn new(start: usize) -> Self {
        Self {
            checkpoint: start,
            power: 1,
            distance: 0,
        }
    }

    fn visit(&mut self, address: usize) -> bool {
        self.distance += 1;
        if address == self.checkpoint {
            return false;
        }
        if self.distance == self.power {
            self.checkpoint = address;
            self.power = self.power.saturating_mul(2);
            self.distance = 0;
        }
        true
    }
}

/// Mirror of `successor`: the identical link-following sequence with local
/// cycle detection. Any failure aborts the breakthroughs walk.
fn copy_breakthrough_successor(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    nodes: &mut TreeNodes,
    header: usize,
    mut current: usize,
    links: BreakthroughLinks,
) -> Option<usize> {
    if links.right != header {
        let mut candidate = links.right;
        let mut chain = BreakthroughChain::new(current);
        loop {
            if !chain.visit(candidate) {
                return None;
            }
            let (child, _) = nodes.read(log, memory, candidate)?;
            if child.nil || child.parent != current {
                return None;
            }
            if child.left == header {
                return Some(candidate);
            }
            current = candidate;
            candidate = child.left;
        }
    }
    let mut parent = links.parent;
    let mut chain = BreakthroughChain::new(current);
    loop {
        if parent == header {
            return Some(header);
        }
        if !chain.visit(parent) {
            return None;
        }
        let (ancestor, _) = nodes.read(log, memory, parent)?;
        if ancestor.nil {
            return None;
        }
        if ancestor.left == current {
            return Some(parent);
        }
        if ancestor.right != current {
            return None;
        }
        current = parent;
        parent = ancestor.parent;
    }
}

/// Mirror of `walk_progress`'s read set: vtable and tree header, sentinel,
/// root and the in-order node/payload sequence. Capacity truncation is
/// ignored (a superset is staged); order and coherence checks are local.
fn copy_breakthrough_tree(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    status: usize,
) -> bool {
    let Some(address) = status.checked_add(nproject_cprogram_status::BREAKTHROUGH) else {
        return false;
    };
    let Some(expected) = base.checked_add(nproject_cbreakthrough_progress::VTABLE_RVA) else {
        return false;
    };
    let Some(header_address) = address.checked_add(nproject_cbreakthrough_progress::TREE_HEADER)
    else {
        return false;
    };
    let Some((starts, 2)) = log.take_scattered(
        memory,
        [address, header_address],
        [8, 16],
        [M_BREAKTHROUGH; 2],
    ) else {
        return false;
    };
    if pool_u64(log, starts[0]) != expected as u64 {
        return false;
    }
    let Ok(header) = usize::try_from(pool_u64(log, starts[1])) else {
        return false;
    };
    let Ok(count) = u32::try_from(pool_u64(log, starts[1] + 8)) else {
        return false;
    };
    let Some(sentinel) = copy_breakthrough_links(log, memory, header) else {
        return false;
    };
    if !sentinel.nil {
        return false;
    }
    if count == 0 {
        return sentinel.left == header && sentinel.parent == header && sentinel.right == header;
    }
    let mut nodes = TreeNodes::new();
    let Some((root, _)) = nodes.read(log, memory, sentinel.parent) else {
        return false;
    };
    if root.nil || root.parent != header {
        return false;
    }
    let mut current = sentinel.left;
    let mut previous_key = None;
    for ordinal in 0..count {
        if current == header {
            return false;
        }
        let Some((links, Some(value))) = nodes.read(log, memory, current) else {
            return false;
        };
        if links.nil {
            return false;
        }
        let specialization = pool_u32(log, value);
        if previous_key.is_some_and(|key| key >= specialization as i32) {
            return false;
        }
        previous_key = Some(specialization as i32);
        let last = current;
        let Some(next) =
            copy_breakthrough_successor(log, memory, &mut nodes, header, current, links)
        else {
            return false;
        };
        if ordinal + 1 == count && (next != header || sentinel.right != last) {
            return false;
        }
        current = next;
    }
    true
}

/// Mirror of `walk`'s read set for breakthroughs across the shared census.
fn copy_breakthroughs(log: &mut RawLog<4>, memory: &impl Memory, base: usize, statuses: &[usize]) {
    for &status in statuses {
        if !copy_breakthrough_tree(log, memory, base, status) || log.overflowed {
            return;
        }
    }
}

/// One structural pass over the scheduled read set: the shared census and
/// statuses first, then the three walk mirrors in typed order. A census or
/// status failure ends every walk at that country, so each mirror still
/// stages the countries before it. Failures stop only the affected walk; the
/// replay derives every outcome from the same recorded spans.
fn copy_pass(memory: &impl Memory, base: usize, game_state: usize, job: &mut ProjectsJob) {
    let _reads = read_accounting::table(TABLES[0]);
    job.game_state = game_state;
    job.census.clear();
    job.statuses.clear();
    let Some((links, countries, vtable, read_to)) =
        copy_census(&mut job.log, memory, base, game_state, &mut job.census)
    else {
        return;
    };
    if job.log.overflowed {
        return;
    }
    if job.worker_growth
        && engine::allocation::reserve(&mut job.statuses, job.census.len()).is_err()
    {
        job.log.overflowed = true;
        return;
    }
    let mut complete = read_to == countries.count;
    let shared = ProjectCensus {
        links: &links,
        countries: &countries,
        vtable,
        read_to,
        census: &job.census,
    };
    for country in job
        .census
        .iter()
        .filter(|entry| entry.canonical == entry.slot)
    {
        let Some(status) = copy_status(&mut job.log, memory, base, shared, country) else {
            complete = false;
            break;
        };
        if job.statuses.len() == job.statuses.capacity() {
            job.log.overflowed = true;
            return;
        }
        job.statuses.push(status);
    }
    if job.log.overflowed {
        return;
    }
    copy_programs(
        &mut job.log,
        memory,
        base,
        shared,
        &job.statuses,
        complete,
        &mut job.pointers,
    );
    if job.log.overflowed {
        return;
    }
    copy_project_tables(&mut job.log, memory, base, shared, &job.statuses);
    if job.log.overflowed {
        return;
    }
    copy_breakthroughs(&mut job.log, memory, base, &job.statuses);
}

/// Country census in walk order. Returns the headers, the country vtable and
/// the first slot not read (the country count when every slot was read); a
/// failed slot keeps the census before it, where every walk stops.
fn copy_census(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &mut Vec<CensusEntry>,
) -> Option<(Array, Array, u64, u32)> {
    let countries_start = span(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        M_ALL,
    )?;
    let links_start = span(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        M_ALL,
    )?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = Array::from_header(raw, 8)?;
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = Array::from_header(raw, 4)?;
    if countries.count < 2 || links.count == 0 {
        return None;
    }
    let vtable = base.checked_add(ccountry::VTABLE_RVA)? as u64;
    if log.worker_growth_enabled()
        && engine::allocation::reserve(census, countries.count as usize).is_err()
    {
        log.overflowed = true;
        return None;
    }
    for slot in 1..countries.count {
        match copy_slot(log, memory, vtable, &countries, &links, slot) {
            Some(Some(entry)) => {
                if census.len() == census.capacity() {
                    log.overflowed = true;
                    return None;
                }
                census.push(entry);
            }
            Some(None) => {}
            None => return Some((links, countries, vtable, slot)),
        }
    }
    Some((links, countries, vtable, countries.count))
}

/// Mirror of `Reader::read_at`. A repeated link or pointer read of a key this
/// slot already staged is not staged again. `None` ends the census; an inner
/// `None` is a null slot, skipped without further reads.
fn copy_slot(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    vtable: u64,
    countries: &Array,
    links: &Array,
    slot: u32,
) -> Option<Option<CensusEntry>> {
    let pointer_at = |slot: u32| {
        (slot as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset))
    };
    let pointer_start = span(log, memory, pointer_at(slot), 8, M_ALL)?;
    let Some(pointer) = usize::try_from(pool_u64(log, pointer_start))
        .ok()
        .filter(|&pointer| pointer != 0)
    else {
        return Some(None);
    };
    let Some(id_address) = pointer.checked_add(ccountry::ID) else {
        span(log, memory, Some(pointer), 8, M_ALL)?;
        return None;
    };
    let (starts, 2) = log.take_scattered(memory, [pointer, id_address], [8, 4], [M_ALL; 2])? else {
        return None;
    };
    if pool_u64(log, starts[0]) != vtable {
        return None;
    }
    let id = pool_u32(log, starts[1]);
    let Some(Some(canonical)) = copy_link(log, memory, links, countries.count, id) else {
        return None;
    };
    if slot != id {
        let Some(Some(canonical_slot)) = copy_link(log, memory, links, countries.count, slot)
        else {
            return None;
        };
        if canonical_slot != canonical {
            return None;
        }
    }
    if canonical != id && canonical != slot {
        let Some(Some(check)) = copy_link(log, memory, links, countries.count, canonical) else {
            return None;
        };
        if check != canonical {
            return None;
        }
    }
    if canonical != slot {
        let canonical_pointer = span(log, memory, pointer_at(canonical), 8, M_ALL)?;
        if pool_u64(log, canonical_pointer) != pointer as u64 {
            return None;
        }
    }
    Some(Some(CensusEntry {
        pointer,
        id,
        slot,
        canonical,
    }))
}

/// Acquire one refresh into an owned job.
/// The caller applies the costs after acquisition has completed.
/// The caller schedules this only for the four daily project tables.
pub(crate) fn copy_projects_refresh(
    memory: &impl Memory,
    ctx: ProjectsCopyContext,
    job: &mut ProjectsJob,
) -> ProjectsCopyReport {
    job.clear();
    if job.worker_growth {
        job.log.enable_worker_growth();
    }
    job.active = ctx.active;
    if !ctx.active {
        return ProjectsCopyReport {
            raw_ok: true,
            ..Default::default()
        };
    }
    let started = qpc();
    copy_pass(memory, ctx.base, ctx.game_state, job);
    if job.worker_growth {
        job.update_capacity_charge();
    }
    ProjectsCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: [elapsed(started), 0, 0, 0],
        raw_bytes: job.log.raw_bytes,
    }
}

impl Captured {
    /// Mark an accepted acquisition failure without retrying live memory.
    /// Only the four project tables are affected; independent tables retain their results.
    pub(crate) fn fail_projects_capture(&mut self, active: bool, reason: &'static str) {
        let reason = if active {
            reason
        } else {
            replay::cadence::SKIPPED_REASON
        };
        self.programs.outcome = Outcome::Unavailable { reason };
        self.projects.outcome = Outcome::Unavailable { reason };
        self.history.outcome = Outcome::Unavailable { reason };
        self.breakthroughs.outcome = Outcome::Unavailable { reason };
        self.names.reset();
        self.required_candidates = 0;
        self.raw_mode = false;
    }

    /// Decode-worker interpretation of a raw refresh payload into the pooled
    /// typed rows. Outcomes, arena bytes and candidate counts match the typed
    /// walk exactly because the identical walks run against the recorded
    /// spans. Clears
    /// `raw_mode` so the next capture starts clean.
    pub(crate) fn decode_projects_payload(
        &mut self,
        base: usize,
        job: &ProjectsJob,
        decode_ticks: &mut [u64; 4],
    ) {
        if !job.active {
            self.fail_projects_capture(false, replay::cadence::SKIPPED_REASON);
            return;
        }
        let started = qpc();
        let recorded = crate::raw::Recorded::build(&job.log);
        let first = nproject_cprogram::walk_buffer(
            &recorded,
            base,
            job.game_state,
            engine::walk::Buffer::for_worker(&mut self.candidates, job.worker_growth),
            engine::walk::Buffer::for_worker(&mut self.programs.rows, job.worker_growth),
            engine::walk::Buffer::for_worker(&mut self.names.first, job.worker_growth),
        );
        self.programs.outcome = program_outcome(first, &self.programs.rows, &self.names.first);
        self.names.len = first.bytes_len;
        self.names.required = first.required_bytes;
        self.required_candidates = first.required_candidates;
        let (projects, histories) = nproject_cproject::walk_with_history_buffer(
            &recorded,
            base,
            job.game_state,
            engine::walk::Buffer::for_worker(&mut self.projects.rows, job.worker_growth),
            engine::walk::Buffer::for_worker(&mut self.history.rows, job.worker_growth),
        );
        self.projects.outcome = bounded(projects, self.projects.rows.len());
        self.history.outcome = bounded(histories, self.history.rows.len());
        self.breakthroughs.outcome = bounded(
            nproject_cbreakthrough_progress::walk_buffer(
                &recorded,
                base,
                job.game_state,
                engine::walk::Buffer::for_worker(&mut self.breakthroughs.rows, job.worker_growth),
            ),
            self.breakthroughs.rows.len(),
        );
        decode_ticks[0] += elapsed(started);
        self.raw_mode = false;
    }
}

/// Mirror of `Reader::link`: one link-table span with local bounds checks.
/// Outer `None` aborts the walk; inner `None` is a readable absence (which
/// the typed callers also turn into a walk error). Either way the replay
/// derives the identical reason from the same recorded span.
fn copy_link(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<Option<u32>> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = span(log, memory, links.data.checked_add(offset), 4, M_ALL)?;
    let slot = pool_u32(log, start);
    if slot == 0 {
        return Some(None);
    }
    if slot >= countries {
        return None;
    }
    Some(Some(slot))
}

impl ProjectCensus<'_> {
    fn entry(&self, slot: u32) -> Option<&CensusEntry> {
        self.census
            .binary_search_by_key(&slot, |entry| entry.slot)
            .ok()
            .map(|index| &self.census[index])
    }
}

/// Mirror of `Reader::resolve` without re-reading authentication: the link
/// span is recorded here and the `read_at` spans come from the shared census.
/// A slot a failed census never reached is staged here, as the walk reads it
/// before reaching that slot itself. Local checks match the walk exactly.
fn copy_resolve(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    census: ProjectCensus<'_>,
    raw: u32,
    mask: u32,
) -> Option<usize> {
    if raw == 0 || raw >= census.links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = span(log, memory, census.links.data.checked_add(offset), 4, mask)?;
    let slot = pool_u32(log, start);
    if slot == 0 || slot >= census.countries.count {
        return None;
    }
    let entry = match census.entry(slot) {
        Some(entry) => *entry,
        None if slot >= census.read_to => copy_slot(
            log,
            memory,
            census.vtable,
            census.countries,
            census.links,
            slot,
        )??,
        None => return None,
    };
    (entry.canonical == entry.slot).then_some(entry.pointer)
}

/// Mirror of `from_country`: status pointer, 20-byte header and owner
/// resolution, staged once for all three walks. `None` aborts them exactly
/// like the typed error.
fn copy_status(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    census: ProjectCensus<'_>,
    country: &CensusEntry,
) -> Option<usize> {
    let pointer_start = span(
        log,
        memory,
        country.pointer.checked_add(ccountry::PROGRAM_STATUS),
        8,
        M_ALL,
    )?;
    let status = usize::try_from(pool_u64(log, pointer_start))
        .ok()
        .filter(|&status| status != 0)?;
    let header_start = span(log, memory, Some(status), 20, M_ALL)?;
    let mut header = [0u8; 20];
    header.copy_from_slice(&log.bytes[header_start..header_start + 20]);
    let expected = base.checked_add(nproject_cprogram_status::VTABLE_RVA)? as u64;
    let secondary = base.checked_add(nproject_cprogram_status::SECONDARY_VTABLE_RVA)? as u64;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected
        || u64::from_le_bytes(header[8..16].try_into().unwrap()) != secondary
    {
        return None;
    }
    // The census resolved this country's own id to itself.
    let owner = u32::from_le_bytes(header[16..20].try_into().unwrap());
    if owner != country.id && copy_resolve(log, memory, census, owner, M_ALL)? != country.pointer {
        return None;
    }
    Some(status)
}
/// Mirror of one program row's read set: identity, field scatter, optional
/// active-project handle, building terminator and payload, owner link.
/// String parses are local; any failure aborts the programs walk.
pub(super) fn copy_program_row(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    census: ProjectCensus<'_>,
    pointer: usize,
) -> bool {
    let expected = match base.checked_add(nproject_cprogram::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let Some(identity) = span(log, memory, Some(pointer), 16, M_PROGRAMS) else {
        return false;
    };
    if pool_u64(log, identity) != expected
        || pool_u32(log, identity + nproject_cprogram::TYPE) != nproject_cprogram::REFERENCE_TYPE
    {
        return false;
    }
    let id = pool_u32(log, identity + nproject_cprogram::ID);
    if id == 0 {
        return false;
    }
    let at = |offset| pointer.checked_add(offset);
    let mut spans = [0usize; 5];
    // Whole-group admission is only a path selection. A tight arena or a
    // missing later address must retain the scalar prefix and its exact demand.
    if let [
        Some(owner),
        Some(owner_copy),
        Some(scientist),
        Some(active),
        Some(building),
    ] = [
        at(nproject_cprogram::OWNER_PRIMARY),
        at(nproject_cprogram::OWNER_COPY),
        at(nproject_cprogram::SCIENTIST_REFERENCE_TYPE),
        at(nproject_cprogram::ACTIVE_FLAG),
        at(nproject_cprogram::BUILDING),
    ] && log.spans.capacity() - log.spans.len() >= 5
        && log
            .bytes
            .len()
            .checked_add(61)
            .is_some_and(|end| end <= log.bytes.capacity() && end <= u32::MAX as usize)
    {
        let Some((starts, 5)) = log.take_scattered_prefetched(
            memory,
            [owner, owner_copy, scientist, active, building],
            [8, 12, 8, 1, 32],
            [M_PROGRAMS; 5],
        ) else {
            return false;
        };
        spans = starts;
    } else {
        for (slot, (offset, len)) in [
            (nproject_cprogram::OWNER_PRIMARY, 8),
            (nproject_cprogram::OWNER_COPY, 12),
            (nproject_cprogram::SCIENTIST_REFERENCE_TYPE, 8),
            (nproject_cprogram::ACTIVE_FLAG, 1),
            (nproject_cprogram::BUILDING, 32),
        ]
        .iter()
        .enumerate()
        {
            let Some(start) = span(log, memory, at(*offset), *len, M_PROGRAMS) else {
                return false;
            };
            spans[slot] = start;
        }
    }
    if log.bytes[spans[0]..spans[0] + 8] != log.bytes[spans[1]..spans[1] + 8] {
        return false;
    }
    let owner = pool_u32(log, spans[0]);
    if log.bytes[spans[3]] > 1 {
        return false;
    }
    if log.bytes[spans[3]] == 1
        && span(
            log,
            memory,
            at(nproject_cprogram::ACTIVE_PROJECT_HANDLE),
            4,
            M_PROGRAMS,
        )
        .is_none()
    {
        return false;
    }
    let mut building = [0u8; 32];
    building.copy_from_slice(&log.bytes[spans[4]..spans[4] + 32]);
    let name_len = u64::from_le_bytes(building[16..24].try_into().unwrap());
    let name_capacity = u64::from_le_bytes(building[24..32].try_into().unwrap());
    if name_capacity < msvc_string::INLINE_CAPACITY || name_len > name_capacity {
        return false;
    }
    let Ok(building_len) = usize::try_from(name_len) else {
        return false;
    };
    let source = if name_capacity == msvc_string::INLINE_CAPACITY {
        at(nproject_cprogram::BUILDING)
    } else {
        usize::try_from(u64::from_le_bytes(building[..8].try_into().unwrap()))
            .ok()
            .filter(|&source| source != 0)
    };
    let Some(source) = source else {
        return false;
    };
    if span(log, memory, source.checked_add(building_len), 1, M_PROGRAMS).is_none() {
        return false;
    }
    if building_len != 0 && span(log, memory, Some(source), building_len, M_PROGRAMS).is_none() {
        return false;
    }
    copy_resolve(log, memory, census, owner, M_PROGRAMS).is_some()
}

/// Mirror of `nproject_cprogram::walk`'s read set across the shared census.
/// The walk runs in two phases — a pointer census over every country, then
/// row reads — so a row failure must not skip later countries' census
/// spans: the replay censuses everything before decoding the first row. An
/// incomplete census or status list ends the walk before its rows.
fn copy_programs(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    shared: ProjectCensus<'_>,
    statuses: &[usize],
    complete: bool,
    pointers: &mut Vec<usize>,
) {
    // Phase 1: pointer census. Only census-span failures abort the walk;
    // row failures below never reach back into this phase.
    pointers.clear();
    for &status in statuses {
        let Some(array_start) = span(
            log,
            memory,
            status.checked_add(nproject_cprogram_status::PROGRAMS),
            16,
            M_PROGRAMS,
        ) else {
            return;
        };
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[array_start..array_start + 16]);
        let Some(array) = Array::from_header(raw, 8) else {
            return;
        };
        if log.worker_growth_enabled()
            && engine::allocation::reserve(pointers, array.count as usize).is_err()
        {
            log.overflowed = true;
            return;
        }
        let mut index = 0;
        while index < array.count as usize {
            let count = (array.count as usize - index).min(64);
            // `from_header` proved the whole array in range.
            let Some((start, copied)) =
                log.take_array_chunk(memory, array.data + index * 8, 8, count, M_PROGRAMS)
            else {
                return;
            };
            for ordinal in 0..copied {
                let Some(pointer) = usize::try_from(pool_u64(log, start + ordinal * 8))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                else {
                    return;
                };
                if pointers.len() == pointers.capacity() {
                    log.overflowed = true;
                    return;
                }
                pointers.push(pointer);
            }
            if copied < count {
                return;
            }
            index += count;
        }
    }
    if !complete {
        return;
    }
    // Phase 2: row reads in census order (a superset of the walk's sorted
    // deduplicated order; duplicates record identical spans harmlessly).
    for &pointer in pointers.iter() {
        if !copy_program_row(log, memory, base, shared, pointer) {
            return;
        }
        if log.overflowed {
            return;
        }
    }
}
