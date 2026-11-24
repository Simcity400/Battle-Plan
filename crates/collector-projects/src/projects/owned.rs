use super::*;


pub(super) const M_PROGRAMS: u32 = 0x01;
const M_BREAKTHROUGH: u32 = 0x02;
const M_ALL: u32 = 0x03;

const NODE_LINKS: usize = nproject_breakthrough_node::RIGHT + 8;
const TREE_NODES: usize = 64;

#[derive(Debug)]
pub struct ProjectsJob {
    pub log: RawLog<2>,
    pub game_state: usize,
    pub active: bool,
    pub(super) census: Vec<CensusEntry>,
    pub(super) statuses: Vec<usize>,
    pub(super) pointers: Vec<usize>,
    pub(super) assignment_references: Vec<u64>,
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
    pub(super) canonical: u32,
}

#[derive(Clone, Copy)]
pub(super) struct ProjectCensus<'a> {
    pub(super) links: &'a Array,
    pub(super) game_state: usize,
    pub(super) countries: &'a Array,
    pub(super) vtable: u64,
    pub(super) read_to: u32,
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
            assignment_references: Vec::new(),
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.active = false;
        self.census.clear();
        self.statuses.clear();
        self.pointers.clear();
        self.assignment_references.clear();
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        engine_allocation::advisory(|| {
            self.log.grow();
            let countries = self
                .census
                .len()
                .saturating_add(1)
                .max(crate::country_scale::census_rows(0));
            let _ = crate::allocation::reserve_raw(&mut self.census, countries, "projects census");
            let _ =
                crate::allocation::reserve_raw(&mut self.statuses, countries, "project statuses");
            let _ = engine_allocation::reserve(&mut self.pointers, 1);
            let _ = engine_allocation::reserve(&mut self.assignment_references, 1);
        });
        self.update_capacity_charge();
    }

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
                + vector_bytes(&self.pointers)
                + vector_bytes(&self.assignment_references),
            ..Default::default()
        });
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProjectsCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; RAW.len()],
    pub raw_bytes: [u64; RAW.len()],
}

#[derive(Debug, Clone, Copy)]
pub struct ProjectsCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub active: bool,
}

fn pool_u32(log: &RawLog<2>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

fn pool_u64(log: &RawLog<2>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

pub(super) fn span(
    log: &mut RawLog<2>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

#[derive(Clone, Copy, Default)]
struct BreakthroughLinks {
    left: usize,
    parent: usize,
    right: usize,
    nil: bool,
}

fn parse_links(log: &RawLog<2>, pointers: usize, nil: usize) -> Option<BreakthroughLinks> {
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

fn copy_breakthrough_links(
    log: &mut RawLog<2>,
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

    fn read(
        &mut self,
        log: &mut RawLog<2>,
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

fn copy_breakthrough_successor(
    log: &mut RawLog<2>,
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

fn copy_breakthrough_tree(
    log: &mut RawLog<2>,
    memory: &impl Memory,
    base: usize,
    status: usize,
) -> bool {
    let Some(address) = status.checked_add(nproject_cprogram_status::BREAKTHROUGH) else {
        return false;
    };
    let Some(expected) = engine_sites::at(base, nproject_cbreakthrough_progress::VTABLE_RVA) else {
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

fn copy_breakthroughs(log: &mut RawLog<2>, memory: &impl Memory, base: usize, statuses: &[usize]) {
    for &status in statuses {
        if !copy_breakthrough_tree(log, memory, base, status) || log.overflowed {
            return;
        }
    }
}

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
    if engine_allocation::reserve(&mut job.statuses, job.census.len()).is_err() {
        job.log.overflowed = true;
        return;
    }
    let mut complete = read_to == countries.count;
    let shared = ProjectCensus {
        game_state,
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
        &mut job.assignment_references,
    );
    if job.log.overflowed {
        return;
    }
    copy_breakthroughs(&mut job.log, memory, base, &job.statuses);
}

fn copy_census(
    log: &mut RawLog<2>,
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
    let vtable = engine_sites::at(base, ccountry::VTABLE_RVA)? as u64;
    if engine_allocation::reserve(census, countries.count as usize).is_err() {
        log.overflowed = true;
        return None;
    }
    let mut reserved = None;
    for slot in 1..countries.count {
        match copy_slot(
            log,
            memory,
            vtable,
            &countries,
            &links,
            slot,
            Some(&mut reserved),
        ) {
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

fn copy_slot(
    log: &mut RawLog<2>,
    memory: &impl Memory,
    vtable: u64,
    countries: &Array,
    links: &Array,
    slot: u32,
    reserved: Option<&mut Option<usize>>,
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
    if reserved
        .as_deref()
        .is_some_and(|root| *root == Some(pointer))
    {
        return Some(None);
    }
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
    if id == 0
        && let Some(reserved) = reserved
        && reserved.is_none()
    {
        let root_start = span(log, memory, pointer_at(0), 8, M_ALL)?;
        let root = usize::try_from(pool_u64(log, root_start))
            .ok()
            .filter(|&root| root != 0)?;
        let root_vtable = span(log, memory, Some(root), 8, M_ALL)?;
        if pool_u64(log, root_vtable) != vtable {
            return None;
        }
        let root_id = span(log, memory, root.checked_add(ccountry::ID), 4, M_ALL)?;
        if pool_u32(log, root_id) != 0 || root != pointer {
            return None;
        }
        *reserved = Some(root);
        return Some(None);
    }
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

pub fn copy_projects_refresh(
    memory: &impl Memory,
    ctx: ProjectsCopyContext,
    job: &mut ProjectsJob,
) -> ProjectsCopyReport {
    job.clear();
    job.active = ctx.active;
    if !ctx.active {
        return ProjectsCopyReport {
            raw_ok: true,
            ..Default::default()
        };
    }
    let started = qpc();
    copy_pass(memory, ctx.base, ctx.game_state, job);
    job.update_capacity_charge();
    ProjectsCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: [elapsed(started), 0],
        raw_bytes: job.log.raw_bytes,
    }
}

impl Captured {
    pub fn fail_projects_capture(&mut self, active: bool, reason: &'static str) {
        let reason = if active {
            reason
        } else {
            replay_data::cadence::SKIPPED_REASON
        };
        self.programs.outcome = Outcome::Unavailable { reason };
        self.breakthroughs.outcome = Outcome::Unavailable { reason };
        self.names.reset();
        self.required_candidates = 0;
        self.raw_mode = false;
    }

    pub fn decode_projects_payload(
        &mut self,
        base: usize,
        job: &ProjectsJob,
        decode_ticks: &mut [u64; RAW.len()],
    ) {
        if !job.active {
            self.fail_projects_capture(false, replay_data::cadence::SKIPPED_REASON);
            return;
        }
        let started = qpc();
        let recorded = match crate::raw::Recorded::build(&job.log) {
            Ok(recorded) => recorded,
            Err(reason) => {
                self.fail_projects_capture(true, reason);
                return;
            }
        };
        let first = nproject_cprogram::walk_buffer(
            &recorded,
            base,
            job.game_state,
            engine_native_memory::walk::Buffer::Growing(&mut self.candidates),
            engine_native_memory::walk::Buffer::Growing(&mut self.programs.rows),
            engine_native_memory::walk::Buffer::Growing(&mut self.names.first),
        );
        self.programs.outcome = program_outcome(first, &self.programs.rows, &self.names.first);
        self.names.len = first.bytes_len;
        self.names.required = first.required_bytes;
        self.required_candidates = first.required_candidates;
        self.breakthroughs.outcome = bounded(
            nproject_cbreakthrough_progress::walk_buffer(
                &recorded,
                base,
                job.game_state,
                engine_native_memory::walk::Buffer::Growing(&mut self.breakthroughs.rows),
            ),
            self.breakthroughs.rows.len(),
        );
        decode_ticks[0] += elapsed(started);
        self.raw_mode = false;
    }
}

fn copy_link(
    log: &mut RawLog<2>,
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

fn copy_resolve(
    log: &mut RawLog<2>,
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
            None,
        )??,
        None => return None,
    };
    (entry.canonical == entry.slot).then_some(entry.pointer)
}

fn copy_status(
    log: &mut RawLog<2>,
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
    let expected = engine_sites::at(base, nproject_cprogram_status::VTABLE_RVA)? as u64;
    let secondary = engine_sites::at(base, nproject_cprogram_status::SECONDARY_VTABLE_RVA)? as u64;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected
        || u64::from_le_bytes(header[8..16].try_into().unwrap()) != secondary
    {
        return None;
    }
    let owner = u32::from_le_bytes(header[16..20].try_into().unwrap());
    if owner != country.id && copy_resolve(log, memory, census, owner, M_ALL)? != country.pointer {
        return None;
    }
    Some(status)
}
pub(super) fn copy_program_row(
    log: &mut RawLog<2>,
    memory: &impl Memory,
    base: usize,
    census: ProjectCensus<'_>,
    pointer: usize,
    references: &mut Vec<u64>,
) -> bool {
    let expected = match engine_sites::at(base, nproject_cprogram::VTABLE_RVA) {
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
    let Some(owner_pointer) = copy_resolve(log, memory, census, owner, M_PROGRAMS) else {
        return false;
    };
    struct AssignmentCopy<'a, M> {
        memory: &'a M,
        log: std::cell::RefCell<&'a mut RawLog<2>>,
    }
    impl<M: Memory> Memory for AssignmentCopy<'_, M> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let mut log = self.log.borrow_mut();
            let Some(start) = log.take(self.memory, address, destination.len(), M_PROGRAMS) else {
                return false;
            };
            destination.copy_from_slice(&log.bytes[start..start + destination.len()]);
            true
        }
    }
    let primary = pool_u64(log, spans[2]);
    let copy = AssignmentCopy {
        memory,
        log: std::cell::RefCell::new(log),
    };
    references.clear();
    let assignments = nproject_cprogram::assignments(&copy, pointer, primary, |assignment| {
        if references.len() == references.capacity() {
            engine_allocation::reserve(references, 1)
                .map_err(|_| "program reference scratch allocation failed")?;
        }
        references.push(assignment.scientist);
        Ok(())
    });
    if assignments.is_ok() {
        let log = copy.log.borrow();
        let active_project = if log.bytes[spans[3]] == 0 {
            None
        } else {
            let address = at(nproject_cprogram::ACTIVE_PROJECT_HANDLE).unwrap();
            log.spans
                .iter()
                .rev()
                .find(|span| span.addr == address && span.len == 4 && span.ok)
                .map(|span| pool_u32(&log, span.start as usize))
        };
        let row = nproject_cprogram::Row {
            id,
            owner,
            province: pool_u32(&log, spans[0] + 4),
            scientist: primary,
            active_project,
            ..Default::default()
        };
        drop(log);
        let _ = engine_projects::program_details::observe(
            &copy,
            base,
            census.game_state,
            pointer,
            &row,
            || {
                let log = copy.log.borrow();
                let address = owner_pointer
                    .checked_add(ccountry::PROGRAM_STATUS)
                    .ok_or("facility status address overflow")?;
                let span = log
                    .spans
                    .iter()
                    .rev()
                    .find(|span| span.addr == address && span.len == 8 && span.ok)
                    .ok_or("facility status missing from shared census")?;
                let status = usize::try_from(pool_u64(&log, span.start as usize))
                    .map_err(|_| "facility status pointer invalid")?;
                Ok((owner_pointer, status))
            },
            references.len(),
            |index| references[index],
            |_| Ok(()),
        );
    }
    true
}

#[expect(
    clippy::too_many_arguments,
    reason = "The two-phase native read order shares separate pointer and reference scratch without changing their ownership."
)]
fn copy_programs(
    log: &mut RawLog<2>,
    memory: &impl Memory,
    base: usize,
    shared: ProjectCensus<'_>,
    statuses: &[usize],
    complete: bool,
    pointers: &mut Vec<usize>,
    references: &mut Vec<u64>,
) {
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
        if engine_allocation::reserve(pointers, array.count as usize).is_err() {
            log.overflowed = true;
            return;
        }
        let mut index = 0;
        while index < array.count as usize {
            let count = (array.count as usize - index).min(64);
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
    for &pointer in pointers.iter() {
        if !copy_program_row(log, memory, base, shared, pointer, references) {
            return;
        }
        if log.overflowed {
            return;
        }
    }
}
