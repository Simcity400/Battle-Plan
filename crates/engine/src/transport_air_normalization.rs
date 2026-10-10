//! F49710/F4B6E0 source graph borrowed by the original regional-air pass.
//! Native vectors are copied once per callback, including failed observations.
//! Direct combat scans preserve occurrences; effective lookups use the exact
//! 551820 key-search recurrence and metadata. No game getter is called.
//! F51060 rebuilds cached default/mission statistics from live equipment;
//! F4D5D0, creation and pool split can reach that writer. Capture borrows the
//! original F49280 selector and retains its stored values, including native
//! staleness. F044D0/F04640 read the held simulation clock; no astronomical
//! time or displayed weather substitutes. The owning callback must admit the
//! current world/hour and native writer boundary before lending this graph.

use crate::{
    Memory, cair_regional_mission::WingIdentity, ccountry, cpdx_array::Array,
    modifier_accumulator::Value, modifier_definition, read,
};
use std::mem::size_of;

pub type Fact<T> = Result<T, &'static str>;

/// Off-thread header demand for F49710/F97BF0 weather and sampled clocks.
/// Every physical clock is a possible next-hour sample; no scalar is acquired.
pub fn weather_entry_demand(memory: &impl Memory, base: usize, game: usize) -> Fact<usize> {
    let manager = field::<u64>(memory, game, crate::cgame_state::WEATHER_MANAGER)? as usize;
    authenticate(memory, base, manager, crate::cweather_manager::VTABLE_RVA)?;
    let mut total = 0usize;
    for (offset, stride, modifiers) in [
        (
            crate::cweather_manager::REGIONS,
            crate::sweather_per_region::STRIDE,
            crate::sweather_per_region::MODIFIERS,
        ),
        (crate::cweather_manager::PROVINCE_CLOCKS, 0x330, 0x280),
    ] {
        let array = Array::read(memory, address(manager, offset)?, stride)
            .ok_or("transport normalization sizing weather header invalid")?;
        for index in 0..array.count {
            let record = array
                .element(index, stride)
                .ok_or("transport normalization sizing weather index overflow")?;
            total = total
                .checked_add(entry_demand(memory, address(record, modifiers)?)?)
                .ok_or("transport normalization sizing demand overflow")?;
        }
    }
    Ok(total)
}

/// F62FE0's normalized producer can borrow wing, effective-country, live ace,
/// ace-type and ace-owner accumulators. Repeated census rows are an upper bound
/// for their unique journal; this hint records no values or admission.
pub fn wing_entry_demand(memory: &impl Memory, base: usize, game: usize) -> Fact<usize> {
    let countries = ccountry::Reader::new(memory, base, game)?;
    let mut total = Ok(0usize);
    let walked = crate::cair_wing::walk_shared(
        memory,
        base,
        game,
        &mut [],
        &mut [],
        usize::MAX,
        |wing, _, row, _| {
            total = total.and_then(|mut total| {
                let mut add = |at| -> Fact<()> {
                    total = total
                        .checked_add(entry_demand(memory, at)?)
                        .ok_or("transport normalization sizing demand overflow")?;
                    Ok(())
                };
                add(address(wing, 0x238)?)?; // F48230/F69BC0.
                let owner = if row.country_override > 0 {
                    row.country_override as u32
                } else {
                    row.owner
                };
                add(address(
                    countries.resolve(owner)?.pointer,
                    ccountry::MODIFIER_ACCUMULATOR,
                )?)?;
                if row.ace != 0
                    && let Some(ace) = crate::creference_registry::resolve(memory, base, row.ace)?
                {
                    authenticate(memory, base, ace, crate::cace::VTABLE_RVA)?;
                    if field::<u8>(memory, ace, crate::cace::ALIVE)? != 0 {
                        add(address(ace, crate::cace::MODIFIERS)?)?;
                        // GetTypeModifiers@14060D010 returns type+38, accumulator+10.
                        add(address(field::<u64>(memory, ace, 0x170)? as usize, 0x48)?)?;
                        let owner = countries.of_pointer(field::<u64>(
                            memory,
                            ace,
                            crate::cace::OWNER_COUNTRY,
                        )? as usize)?;
                        add(address(owner.pointer, ccountry::MODIFIER_ACCUMULATOR)?)?;
                    }
                }
                Ok(total)
            });
        },
    );
    match walked.naval {
        crate::walk::Outcome::Rows { .. } => total,
        // CountryEntry@1406ECA90 returns null when game+690 is null:
        // no wing accumulator can be reached for this sizing observation.
        crate::walk::Outcome::Unavailable {
            reason: "strategic air manager absent",
        } => Ok(0),
        crate::walk::Outcome::Unavailable { reason } => Err(reason),
        crate::walk::Outcome::Truncated { .. } => {
            Err("transport normalization sizing wing census truncated")
        }
    }
}

fn entry_demand(memory: &impl Memory, at: usize) -> Fact<usize> {
    Array::read(memory, at, 16)
        .map(|array| array.count as usize)
        .ok_or("transport normalization sizing modifier header invalid")
}
/// Native keys read by F48230/F69BC0/F47850/F46B00/F4B5E0. Their semantic
/// layout must be admitted independently before these observations are used.
pub const KEYS: [u32; 16] = [
    2, 0xb, 0xd, 0xf, 0x10, 0x6f, 0x70, 0x72, 0x74, 0x75, 0x76, 0x104, 0x105, 0x106, 0x10d, 0x25a,
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Modifiers {
    pub direct: [Option<i64>; 2],
    pub positive: [Option<u32>; 2],
    pub effective: [Option<i64>; 16],
    /// Only the ace's direct F48230 scan needs its ordered raw occurrences.
    pub combat: Option<(usize, usize)>,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Ace {
    pub assigned_aircraft: Option<Option<i32>>,
    pub modifiers: Option<Modifiers>,
    pub type_modifiers: Option<Modifiers>,
    pub owner_effectiveness: Option<i64>,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Environment {
    pub weather: [Option<i64>; 2],
    pub night_factor: Option<i64>,
    pub night: [Option<i64>; 2],
    pub target_region: Option<i32>,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    /// F4B7F0 indexes30/2F in the original current-mission stat block.
    pub attack: Option<i64>,
    pub defence: Option<i64>,
    pub speed: Option<i64>,
    pub enabled_missions: u32,
    pub wing: Option<Modifiers>,
    pub country: Option<Modifiers>,
    pub ace: Option<Option<Ace>>,
    pub environment: Option<Option<Environment>>,
    pub night_stat: Option<i64>,
    pub base_region: Option<i32>,
    pub carrier_mission: Option<Option<i32>>,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CombatModifier {
    pub lane: u8,
    pub raw: Option<i64>,
}
#[derive(Clone, Copy)]
struct Node {
    address: usize,
    value: Fact<Modifiers>,
    start: usize,
    len: usize,
    naval: Fact<[Value; 4]>,
}
#[derive(Clone, Copy)]
struct Entry {
    key: Option<u32>,
    raw: Option<i64>,
}
#[derive(Clone, Copy)]
struct Assignment {
    reference: u64,
    value: Fact<Option<i32>>,
}
#[derive(Clone, Copy)]
struct AceObservation {
    reference: u64,
    value: Fact<Option<Ace>>,
}

/// Off-thread prepared scratch. Native addresses are callback-local controls;
/// `retire` discards them before the original loan returns, retaining only
/// numerical combat entries for off-thread packet conversion.
pub struct Capture {
    nodes: Vec<Node>,
    index: Vec<usize>,
    entries: Vec<Entry>,
    combat: Vec<CombatModifier>,
    assignments: Vec<Assignment>,
    assignment_index: Vec<usize>,
    aces: Vec<AceObservation>,
    ace_index: Vec<usize>,
    node_capacity: usize,
    entry_capacity: usize,
    required: (usize, usize),
    metadata: [Option<Fact<modifier_definition::Operands>>; 16],
    naval_metadata: Option<Fact<modifier_definition::Operands>>,
    worker_growth: bool,
}
impl Capture {
    pub fn vacant() -> Self {
        Self {
            nodes: Vec::new(),
            index: Vec::new(),
            entries: Vec::new(),
            combat: Vec::new(),
            assignments: Vec::new(),
            assignment_index: Vec::new(),
            aces: Vec::new(),
            ace_index: Vec::new(),
            node_capacity: 0,
            entry_capacity: 0,
            required: (0, 0),
            metadata: [None; 16],
            naval_metadata: None,
            worker_growth: false,
        }
    }
    pub fn try_new(nodes: usize, combat_entries: usize) -> std::io::Result<Self> {
        let slots = if nodes == 0 {
            0
        } else {
            nodes
                .checked_mul(2)
                .and_then(usize::checked_next_power_of_two)
                .ok_or_else(crate::allocation::oom)?
        };
        Ok(Self {
            nodes: crate::allocation::vec(nodes)?,
            index: crate::allocation::filled(slots, 0)?,
            assignments: crate::allocation::vec(nodes)?,
            assignment_index: crate::allocation::filled(slots, 0)?,
            aces: crate::allocation::vec(nodes)?,
            ace_index: crate::allocation::filled(slots, 0)?,
            entries: crate::allocation::vec(combat_entries)?,
            combat: crate::allocation::vec(combat_entries)?,
            node_capacity: nodes,
            entry_capacity: combat_entries,
            required: (0, 0),
            metadata: [None; 16],
            naval_metadata: None,
            worker_growth: false,
        })
    }
    pub fn clear(&mut self) {
        self.retire();
        self.entries.clear();
        self.combat.clear();
        self.required = (0, 0);
        self.metadata.fill(None);
        self.naval_metadata = None;
        self.worker_growth = false;
    }
    pub fn retire(&mut self) {
        self.nodes.clear();
        self.index.fill(0);
        self.assignments.clear();
        self.assignment_index.fill(0);
        self.aces.clear();
        self.ace_index.fill(0);
    }
    pub fn capacity(&self) -> (usize, usize) {
        (self.node_capacity, self.entry_capacity)
    }
    pub fn required(&self) -> (usize, usize) {
        self.required
    }
    /// Allocating workers grow from original modifier headers and new memo
    /// identities. Every prior value, failure and numerical span survives.
    pub fn enable_worker_growth(&mut self) {
        self.worker_growth = true;
    }
    fn ensure_nodes(&mut self, required: usize) -> std::io::Result<()> {
        if !self.worker_growth || required <= self.node_capacity {
            return Ok(());
        }
        let capacity = required.max(self.node_capacity.saturating_mul(2));
        let slots = crate::allocation::mul(capacity, 2)?
            .checked_next_power_of_two()
            .ok_or_else(crate::allocation::oom)?;
        let additional = capacity.saturating_sub(self.nodes.len());
        crate::allocation::reserve_exact(&mut self.nodes, additional)?;
        let additional = capacity.saturating_sub(self.assignments.len());
        crate::allocation::reserve_exact(&mut self.assignments, additional)?;
        let additional = capacity.saturating_sub(self.aces.len());
        crate::allocation::reserve_exact(&mut self.aces, additional)?;
        crate::allocation::resize(&mut self.index, slots, 0)?;
        crate::allocation::resize(&mut self.assignment_index, slots, 0)?;
        crate::allocation::resize(&mut self.ace_index, slots, 0)?;
        self.index.fill(0);
        self.assignment_index.fill(0);
        self.ace_index.fill(0);
        for index in 0..self.nodes.len() {
            let slot = self.slot(self.nodes[index].address);
            self.index[slot] = index + 1;
        }
        for index in 0..self.assignments.len() {
            let slot = self.assignment_slot(self.assignments[index].reference);
            self.assignment_index[slot] = index + 1;
        }
        for index in 0..self.aces.len() {
            let slot = self.ace_slot(self.aces[index].reference);
            self.ace_index[slot] = index + 1;
        }
        self.node_capacity = capacity;
        Ok(())
    }
    fn ensure_entries(&mut self, required: usize) -> std::io::Result<()> {
        if !self.worker_growth || required <= self.entry_capacity {
            return Ok(());
        }
        let capacity = required.max(self.entry_capacity.saturating_mul(2));
        let additional = capacity.saturating_sub(self.entries.len());
        crate::allocation::reserve_exact(&mut self.entries, additional)?;
        let additional = capacity.saturating_sub(self.combat.len());
        crate::allocation::reserve_exact(&mut self.combat, additional)?;
        self.entry_capacity = capacity;
        Ok(())
    }
    pub fn try_grow(&mut self, nodes: usize, entries: usize) -> std::io::Result<()> {
        let desired = (
            nodes.max(self.node_capacity),
            entries.max(self.entry_capacity),
        );
        if desired == self.capacity() {
            self.clear();
        } else {
            *self = Self::try_new(desired.0, desired.1)?;
        }
        Ok(())
    }
    pub fn allocated_bytes(&self) -> usize {
        self.nodes.capacity() * size_of::<Node>()
            + self.index.capacity() * size_of::<usize>()
            + self.entries.capacity() * size_of::<Entry>()
            + self.combat.capacity() * size_of::<CombatModifier>()
            + self.assignments.capacity() * size_of::<Assignment>()
            + self.assignment_index.capacity() * size_of::<usize>()
            + self.aces.capacity() * size_of::<AceObservation>()
            + self.ace_index.capacity() * size_of::<usize>()
    }
    pub fn combat(&self, span: (usize, usize)) -> Fact<&[CombatModifier]> {
        let end = span
            .0
            .checked_add(span.1)
            .ok_or("transport ace combat span overflow")?;
        self.combat
            .get(span.0..end)
            .ok_or("transport ace combat span unavailable")
    }
    fn slot(&self, address: usize) -> usize {
        let mut slot = (address >> 4).wrapping_mul(0x9e37_79b9) & (self.index.len() - 1);
        while self.index[slot] != 0 && self.nodes[self.index[slot] - 1].address != address {
            slot = (slot + 1) & (self.index.len() - 1);
        }
        slot
    }
    fn modifiers(&mut self, memory: &impl Memory, base: usize, at: usize) -> Fact<Modifiers> {
        let mut slot = (!self.index.is_empty()).then(|| self.slot(at));
        if let Some(index) = slot.and_then(|slot| self.index[slot].checked_sub(1)) {
            return self.nodes[index].value;
        }
        self.required.0 = self.required.0.saturating_add(1);
        self.ensure_nodes(self.nodes.len().saturating_add(1))
            .map_err(|_| "transport air modifier node allocation failed")?;
        if self.worker_growth {
            slot = Some(self.slot(at));
        }
        if self.nodes.len() == self.node_capacity {
            return Err("transport air modifier node capacity exceeded");
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            address: at,
            value: Err("transport air modifier observation incomplete"),
            start: 0,
            len: 0,
            naval: Err("transport original naval weather observation unavailable"),
        });
        self.index[slot.expect("nonzero transport modifier capacity")] = index + 1;
        let first = self.entries.len();
        let result = self.copy_modifiers(memory, base, at);
        self.nodes[index].value = result;
        self.nodes[index].start = first;
        self.nodes[index].len = self.entries.len() - first;
        result
    }
    fn copy_modifiers(&mut self, memory: &impl Memory, base: usize, at: usize) -> Fact<Modifiers> {
        let array = Array::read(memory, at, 16).ok_or("transport air modifier header refused")?;
        self.required.1 = self.required.1.saturating_add(array.count as usize);
        self.ensure_entries(self.entries.len().saturating_add(array.count as usize))
            .map_err(|_| "transport air modifier entry allocation failed")?;
        if array.count as usize > self.entry_capacity.saturating_sub(self.entries.len()) {
            return Err("transport air modifier entry capacity exceeded");
        }
        let entry_first = self.entries.len();
        for index in 0..array.count {
            let at = array
                .element(index, 16)
                .ok_or("transport air modifier index overflow")?;
            self.entries.push(Entry {
                key: read(memory, at),
                raw: at.checked_add(8).and_then(|at| read(memory, at)),
            });
        }
        self.summarize_entries(memory, base, entry_first)
    }
    fn summarize_entries(
        &mut self,
        memory: &impl Memory,
        base: usize,
        entry_first: usize,
    ) -> Fact<Modifiers> {
        let first = self.combat.len();
        let mut output = Modifiers {
            direct: [Some(0); 2],
            positive: [Some(0); 2],
            effective: [Some(0); 16],
            ..Default::default()
        };
        let mut combat_complete = true;
        for entry in &self.entries[entry_first..] {
            let Some(key) = entry.key else {
                output.direct = [None; 2];
                output.positive = [None; 2];
                combat_complete = false;
                continue;
            };
            if key == 0x6f || key == 0x70 {
                let lane = usize::from(key == 0x70);
                output.direct[lane] = output.direct[lane]
                    .zip(entry.raw)
                    .map(|(sum, raw)| sum.wrapping_add(raw));
                output.positive[lane] = match output.positive[lane].zip(entry.raw) {
                    Some((count, raw)) => Some(
                        count
                            .checked_add(u32::from(raw > 0))
                            .ok_or("transport air modifier positive-count overflow")?,
                    ),
                    None => None,
                };
                if self.combat.len() < self.entry_capacity {
                    self.combat.push(CombatModifier {
                        lane: lane as u8,
                        raw: entry.raw,
                    });
                } else {
                    combat_complete = false;
                }
            }
        }
        for (position, key) in KEYS.into_iter().enumerate() {
            // 551820's exact key-search recurrence. Failed unvisited keys or
            // unrelated raw words do not suppress an independently valid lookup.
            let selected = lookup(&self.entries[entry_first..], key);
            output.effective[position] = match selected {
                Ok(None) => Some(0),
                Err(_) => None,
                Ok(Some(raw)) => {
                    let operands = *self.metadata[position]
                        .get_or_insert_with(|| modifier_definition::operands(memory, base, key));
                    operands.map(|operands| operands.effective(raw)).ok()
                }
            };
        }
        output.combat = if combat_complete {
            Some((first, self.combat.len() - first))
        } else {
            None
        };
        Ok(output)
    }

    /// Original lookup_many_at's full16 census also supplies the new native
    /// lookups. The old raw result has no extension entry-capacity dependency.
    fn weather_modifiers(
        &mut self,
        memory: &impl Memory,
        base: usize,
        at: usize,
    ) -> (Fact<Modifiers>, Fact<[Value; 4]>) {
        let mut slot = (!self.index.is_empty()).then(|| self.slot(at));
        if let Some(index) = slot.and_then(|slot| self.index[slot].checked_sub(1)) {
            let node = self.nodes[index];
            return (node.value, node.naval);
        }
        self.required.0 = self.required.0.saturating_add(1);
        if self
            .ensure_nodes(self.nodes.len().saturating_add(1))
            .is_err()
        {
            return (
                Err("transport air modifier node allocation failed"),
                Err("transport original naval weather node allocation failed"),
            );
        }
        if self.worker_growth {
            slot = Some(self.slot(at));
        }
        if self.nodes.len() == self.node_capacity {
            return (
                Err("transport air modifier node capacity exceeded"),
                Err("transport original naval weather node capacity exceeded"),
            );
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            address: at,
            value: Err("transport air modifier observation incomplete"),
            start: self.entries.len(),
            len: 0,
            naval: Err("transport original naval weather observation incomplete"),
        });
        self.index[slot.expect("nonzero weather modifier capacity")] = index + 1;
        let result = self.copy_weather_modifiers(memory, base, at, true);
        self.nodes[index].value = result.0;
        self.nodes[index].naval = result.1;
        self.nodes[index].len = self.entries.len() - self.nodes[index].start;
        result
    }
    fn copy_weather_modifiers(
        &mut self,
        memory: &impl Memory,
        base: usize,
        at: usize,
        materialize: bool,
    ) -> (Fact<Modifiers>, Fact<[Value; 4]>) {
        let Some(array) = Array::read(memory, at, 16) else {
            return (
                Err("transport air modifier header refused"),
                Err("modifier accumulator header unreadable or invalid"),
            );
        };
        self.required.1 = self.required.1.saturating_add(array.count as usize);
        if materialize
            && self
                .ensure_entries(self.entries.len().saturating_add(array.count as usize))
                .is_err()
        {
            return (
                Err("transport air modifier entry allocation failed"),
                Err("transport original naval weather entry allocation failed"),
            );
        }
        let fits = materialize
            && array.count as usize <= self.entry_capacity.saturating_sub(self.entries.len());
        let entry_first = self.entries.len();
        let mut naval = Ok([Value::default(); 4]);
        let mut previous = 0;
        let mut first = 0;
        // This is lookup_many_at's unchanged original record layout and batch.
        let fields = [crate::memory::RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 16,
        }];
        let layout = crate::memory::RecordLayout::new(&fields, 16).expect("weather entry layout");
        let mut bytes = [0; 64 * 16];
        while first < array.count as usize {
            let count = (array.count as usize - first).min(64);
            let Some(address) = array.element(first as u32, 16) else {
                return (
                    Err("transport air modifier index overflow"),
                    Err("modifier accumulator entry address overflow"),
                );
            };
            let completed = memory.copy_records(
                crate::memory::RecordSources::Strided {
                    base: address,
                    stride: 16,
                    count,
                },
                &layout,
                &mut bytes[..count * 16],
            );
            for entry in bytes[..completed * 16].chunks_exact(16) {
                let key = u32::from_le_bytes(entry[..4].try_into().unwrap());
                let raw = i64::from_le_bytes(entry[8..].try_into().unwrap());
                if let Ok(values) = &mut naval {
                    if key <= previous {
                        naval = Err("modifier accumulator keys not positive and increasing");
                    } else {
                        previous = key;
                        if let Ok(position) =
                            crate::naval_mission_inputs::TASK_FORCE_KEYS.binary_search(&key)
                        {
                            values[position] = Value { raw, present: true };
                        }
                    }
                }
                if fits {
                    self.entries.push(Entry {
                        key: Some(key),
                        raw: Some(raw),
                    });
                }
            }
            first += completed;
            if completed != count {
                if naval.is_ok() {
                    naval = Err("modifier accumulator entry unreadable");
                }
                // copy_records stops at this attempted failure. The new graph
                // may acquire the still-unattempted suffix, never this entry.
                if fits {
                    self.entries.push(Entry {
                        key: None,
                        raw: None,
                    });
                }
                first += 1;
            }
            if !fits && naval.is_err() {
                break;
            }
        }
        let modifiers = if fits {
            self.summarize_entries(memory, base, entry_first)
        } else {
            Err("transport air modifier entry capacity exceeded")
        };
        (modifiers, naval)
    }

    /// The old strict caller borrows any original graph observation. If its
    /// region was unattempted because extension node storage was unavailable,
    /// it still runs its original streaming census once. The caller guarantees
    /// one strict call per original, unique region before retiring this arena.
    pub fn strict_weather(
        &mut self,
        memory: &impl Memory,
        base: usize,
        record: Fact<usize>,
    ) -> Fact<[Value; 4]> {
        let record = record?;
        if record == 0 {
            return Ok([Value::default(); 4]);
        }
        let at = address(record, 0x58)?;
        if !self.index.is_empty()
            && let Some(index) = self.index[self.slot(at)].checked_sub(1)
        {
            return self.nodes[index].naval;
        }
        if self.worker_growth || self.nodes.len() < self.node_capacity {
            return self.weather_modifiers(memory, base, at).1;
        }
        // No native header/body was attempted at a node-capacity refusal.
        self.copy_weather_modifiers(memory, base, at, false).1
    }

    /// Original F97BF0 key2A and F49710's weather keys borrow the same node.
    /// A native null weather record skips F97BF0, while normalized combat
    /// retains its own reached null-record refusal.
    pub fn environment_and_detection(
        &mut self,
        memory: &impl Memory,
        base: usize,
        sampled: Fact<u32>,
        target_region: u32,
        weather_record: Fact<usize>,
        clocks: Fact<Array>,
    ) -> (Environment, Fact<Option<i64>>) {
        let (environment, detection, _) = self.environment_detection_and_naval(
            memory,
            base,
            sampled,
            target_region,
            weather_record,
            clocks,
        );
        (environment, detection)
    }

    pub fn environment_detection_and_naval(
        &mut self,
        memory: &impl Memory,
        base: usize,
        sampled: Fact<u32>,
        target_region: u32,
        weather_record: Fact<usize>,
        clocks: Fact<Array>,
    ) -> (Environment, Fact<Option<i64>>, Fact<[Value; 4]>) {
        let weather = weather_record.and_then(|record| {
            if record == 0 {
                return Ok((
                    Err("transport normalized combat weather record null"),
                    Ok([Value::default(); 4]),
                ));
            }
            Ok(self.weather_modifiers(memory, base, address(record, 0x58)?))
        });
        let environment = self.environment_from_weather(
            memory,
            base,
            sampled,
            target_region,
            weather.and_then(|weather| weather.0),
            clocks,
        );
        let detection = weather_record.and_then(|record| {
            if record == 0 {
                return Ok(None);
            }
            let at = address(record, 0x58)?;
            self.modifiers(memory, base, at)?;
            let node = self.index[self.slot(at)] - 1;
            let node = self.nodes[node];
            let raw = lookup(&self.entries[node.start..node.start + node.len], 0x2a)?;
            let Some(raw) = raw else {
                return Ok(Some(0));
            };
            let metadata = *self
                .naval_metadata
                .get_or_insert_with(|| modifier_definition::operands(memory, base, 0x2a));
            Ok(Some(metadata?.effective(raw)))
        });
        (
            environment,
            detection,
            weather.and_then(|weather| weather.1),
        )
    }

    /// F04E40's already attempted record and the original clocks header are
    /// borrowed. Neither selector can be healed by another header read.
    pub fn environment(
        &mut self,
        memory: &impl Memory,
        base: usize,
        sampled: Fact<u32>,
        target_region: u32,
        weather_record: Fact<usize>,
        clocks: Fact<Array>,
    ) -> Environment {
        let weather =
            weather_record.and_then(|record| self.modifiers(memory, base, address(record, 0x58)?));
        self.environment_from_weather(memory, base, sampled, target_region, weather, clocks)
    }
    fn environment_from_weather(
        &mut self,
        memory: &impl Memory,
        base: usize,
        sampled: Fact<u32>,
        target_region: u32,
        weather: Fact<Modifiers>,
        clocks: Fact<Array>,
    ) -> Environment {
        let clock = sampled.and_then(|id| {
            clocks.and_then(|array| {
                array
                    .element(id, 0x330)
                    .ok_or("transport sampled province outside original clock domain")
            })
        });
        let night_factor = clock.and_then(|clock| field(memory, clock, 0x240)).ok();
        let night = clock.and_then(|clock| self.modifiers(memory, base, address(clock, 0x280)?));
        Environment {
            weather: weather.map_or([None; 2], |m| [m.effective[5], m.effective[6]]),
            night_factor,
            night: night.map_or([None; 2], |m| [m.effective[5], m.effective[6]]),
            target_region: sampled.map(|_| target_region as i32).ok(),
        }
    }

    /// Called only for F62FE0's reached positive-disruption producer. `stats`
    /// is the original F49280 selector success/failure used by stat13/14.
    #[allow(clippy::too_many_arguments)]
    pub fn capture<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        game: usize,
        countries: &ccountry::Reader<'_, M>,
        original_wings: &[WingIdentity],
        original: &WingIdentity,
        wing: usize,
        _current_type: u32,
        stats: Fact<usize>,
        sampled: Fact<u32>,
        environment: Fact<Environment>,
        guard: Fact<()>,
    ) -> Source {
        let mut output = Source {
            attack: stats.and_then(|stats| field(memory, stats, 0x30 * 8)).ok(),
            defence: stats.and_then(|stats| field(memory, stats, 0x2f * 8)).ok(),
            speed: field(memory, wing, 0x580).ok(),
            enabled_missions: original.mission_mask,
            ..Default::default()
        };
        let country = countries.resolve(original.effective_country());
        output.wing = guard
            .and_then(|()| self.modifiers(memory, base, address(wing, 0x238)?))
            .ok();
        output.country = guard
            .and_then(|()| {
                country.and_then(|country| {
                    self.modifiers(
                        memory,
                        base,
                        address(country.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
                    )
                })
            })
            .ok();
        output.ace = guard
            .and_then(|()| self.ace(memory, base, countries, original_wings, original.ace))
            .ok();
        let reached = sampled.map(|id| id as i32 > 0 && original.region.is_some());
        output.environment = reached
            .and_then(|reached| {
                if reached {
                    environment.map(Some)
                } else {
                    Ok(None)
                }
            })
            .ok();
        if reached == Ok(true) {
            output.night_stat = stats.and_then(|stats| field(memory, stats, 0x4c * 8)).ok();
            let base_context = base_context(memory, base, game, original.air_base);
            output.base_region = base_context.0.ok();
            output.carrier_mission = base_context.1.ok();
        }
        output
    }
    fn ace<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        original_wings: &[WingIdentity],
        reference: u64,
    ) -> Fact<Option<Ace>> {
        if reference == 0 {
            return Ok(None);
        }
        let mut slot = (!self.ace_index.is_empty()).then(|| self.ace_slot(reference));
        if let Some(index) = slot.and_then(|slot| self.ace_index[slot].checked_sub(1)) {
            return self.aces[index].value;
        }
        self.required.0 = self.required.0.max(self.aces.len().saturating_add(1));
        self.ensure_nodes(self.aces.len().saturating_add(1))
            .map_err(|_| "transport air ace observation allocation failed")?;
        if self.worker_growth {
            slot = Some(self.ace_slot(reference));
        }
        if self.aces.len() == self.node_capacity {
            return Err("transport air ace observation capacity exceeded");
        }
        let index = self.aces.len();
        self.aces.push(AceObservation {
            reference,
            value: Err("transport air ace observation incomplete"),
        });
        self.ace_index[slot.expect("nonzero ace observation capacity")] = index + 1;
        let result = self.capture_ace(memory, base, countries, original_wings, reference);
        self.aces[index].value = result;
        result
    }
    fn capture_ace<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        original_wings: &[WingIdentity],
        reference: u64,
    ) -> Fact<Option<Ace>> {
        let Some(ace) = crate::creference_registry::resolve(memory, base, reference)? else {
            return Ok(None);
        };
        authenticate(memory, base, ace, crate::cace::VTABLE_RVA)?;
        if field::<u8>(memory, ace, crate::cace::ALIVE)? == 0 {
            return Ok(None);
        }
        let modifiers = self
            .modifiers(memory, base, address(ace, crate::cace::MODIFIERS)?)
            .ok();
        // 60D010 returns type+38; its accumulator starts another +10 later.
        let type_modifiers = field::<u64>(memory, ace, 0x170)
            .and_then(|kind| self.modifiers(memory, base, address(kind as usize, 0x48)?))
            .ok();
        let owner_effectiveness = field::<u64>(memory, ace, crate::cace::OWNER_COUNTRY)
            .and_then(|owner| countries.of_pointer(owner as usize))
            .and_then(|owner| {
                self.modifiers(
                    memory,
                    base,
                    address(owner.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
                )
            })
            .and_then(|modifiers| {
                modifiers.effective[15].ok_or("transport ace owner effectiveness refused")
            })
            .ok();
        let assigned_aircraft = field(memory, ace, crate::cace::ASSIGNED_WING_REFERENCE)
            .and_then(|reference| self.assigned_aircraft(memory, base, original_wings, reference))
            .ok();
        Ok(Some(Ace {
            assigned_aircraft,
            modifiers,
            type_modifiers,
            owner_effectiveness,
        }))
    }
    fn assignment_slot(&self, reference: u64) -> usize {
        let mask = self.assignment_index.len() - 1;
        let mut slot =
            (reference as usize ^ (reference >> 32) as usize).wrapping_mul(0x9e37_79b9) & mask;
        while self.assignment_index[slot] != 0
            && self.assignments[self.assignment_index[slot] - 1].reference != reference
        {
            slot = (slot + 1) & mask;
        }
        slot
    }
    fn ace_slot(&self, reference: u64) -> usize {
        let mask = self.ace_index.len() - 1;
        let mut slot =
            (reference as usize ^ (reference >> 32) as usize).wrapping_mul(0x9e37_79b9) & mask;
        while self.ace_index[slot] != 0
            && self.aces[self.ace_index[slot] - 1].reference != reference
        {
            slot = (slot + 1) & mask;
        }
        slot
    }
    fn assigned_aircraft(
        &mut self,
        memory: &impl Memory,
        base: usize,
        original_wings: &[WingIdentity],
        reference: u64,
    ) -> Fact<Option<i32>> {
        if reference == 0 {
            return Ok(None);
        }
        let mut slot = (!self.assignment_index.is_empty()).then(|| self.assignment_slot(reference));
        if let Some(index) = slot.and_then(|slot| self.assignment_index[slot].checked_sub(1)) {
            return self.assignments[index].value;
        }
        self.required.0 = self
            .required
            .0
            .max(self.assignments.len().saturating_add(1));
        self.ensure_nodes(self.assignments.len().saturating_add(1))
            .map_err(|_| "transport ace assignment allocation failed")?;
        if self.worker_growth {
            slot = Some(self.assignment_slot(reference));
        }
        if self.assignments.len() == self.node_capacity {
            return Err("transport ace assignment capacity exceeded");
        }
        let index = self.assignments.len();
        self.assignments.push(Assignment {
            reference,
            value: Err("transport ace assignment observation incomplete"),
        });
        self.assignment_index[slot.expect("nonzero assignment capacity")] = index + 1;
        let result = assigned_count(
            memory,
            base,
            original_wings,
            reference,
            crate::creference_registry::resolve(memory, base, reference),
        );
        self.assignments[index].value = result;
        result
    }
}

fn assigned_count(
    memory: &impl Memory,
    base: usize,
    original_wings: &[WingIdentity],
    reference: u64,
    resolved: Fact<Option<usize>>,
) -> Fact<Option<i32>> {
    let Some(pointer) = resolved? else {
        return Ok(None);
    };
    if pointer == 0x10 {
        return Ok(None);
    }
    let wing = pointer
        .checked_sub(0x10)
        .filter(|&wing| wing != 0)
        .ok_or("transport ace assigned wing pointer invalid")?;
    let mission = address(wing, crate::cair_wing::MISSION)?;
    if let Ok(index) = original_wings.binary_search_by_key(&mission, |wing| wing.mission_pointer) {
        let original = &original_wings[index];
        if original.wing != reference {
            return Err("transport ace assignment original wing handle mismatch");
        }
        return Ok(Some(original.planes));
    }
    // Genuine outside-census registry object, not a failed original census
    // row. The caller retains this independent observation, including failure.
    authenticate(memory, base, wing, crate::cair_wing::VTABLE_RVA)?;
    if field::<u64>(memory, wing, crate::cair_wing::REFERENCE_TYPE)? != reference {
        return Err("transport outside-census ace wing handle mismatch");
    }
    field(memory, wing, crate::cair_wing::PLANE_COUNT).map(Some)
}

fn lookup(entries: &[Entry], key: u32) -> Fact<Option<i64>> {
    let mut lower = 0;
    let mut count = entries.len();
    while count > 0 {
        let middle = count >> 1;
        let found = entries[lower + middle]
            .key
            .ok_or("transport air modifier search key unreadable")?;
        if found < key {
            lower += (count & 1) + middle;
        }
        count = middle;
    }
    let Some(entry) = entries.get(lower) else {
        return Ok(None);
    };
    let found = entry
        .key
        .ok_or("transport air modifier final key unreadable")?;
    if found > key {
        return Ok(None);
    }
    entry
        .raw
        .map(Some)
        .ok_or("transport air modifier selected value unreadable")
}

fn address(pointer: usize, offset: usize) -> Fact<usize> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("transport air normalization address invalid")
}
fn field<T: crate::memory::Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Fact<T> {
    read(memory, address(pointer, offset)?).ok_or("transport air normalization word unreadable")
}
fn authenticate(memory: &impl Memory, base: usize, pointer: usize, rva: usize) -> Fact<()> {
    if field::<u64>(memory, pointer, 0)?
        != base
            .checked_add(rva)
            .ok_or("transport air normalization vtable address overflow")? as u64
    {
        return Err("transport air normalization vtable mismatch");
    }
    Ok(())
}

/// F49710's base-region fallback AA is executable code, never a vanilla ID.
/// F46B00 separately reaches a carrier host even for a state-backed base.
/// Both directly resolve pool+18: F4A6B0's wing override is not their caller.
fn base_context(
    memory: &impl Memory,
    base: usize,
    game: usize,
    reference: u64,
) -> (Fact<i32>, Fact<Option<i32>>) {
    let airbase =
        crate::creference_registry::resolve(memory, base, reference).and_then(|resolved| {
            if let Some(airbase) = resolved {
                authenticate(memory, base, airbase, crate::cair_base::VTABLE_RVA)?;
            }
            Ok(resolved)
        });
    let state = airbase.and_then(|airbase| {
        airbase
            .map(|airbase| field::<u64>(memory, airbase, crate::cair_base::STATE))
            .transpose()
    });
    let host_task = airbase.and_then(|airbase| match airbase {
        None => Ok(None),
        Some(airbase) => {
            let host_ref = field(memory, airbase, crate::cair_base::HOST_REFERENCE)?;
            crate::creference_registry::resolve(memory, base, host_ref)?
                .map(|ship| {
                    authenticate(memory, base, ship, crate::cship::VTABLE_RVA)?;
                    let task = field::<u64>(memory, ship, 0x728)? as usize;
                    authenticate(memory, base, task, crate::ctask_force::VTABLE_RVA)?;
                    Ok(task)
                })
                .transpose()
        }
    });
    let carrier =
        host_task.and_then(|task| task.map(|task| field(memory, task, 0x374)).transpose());
    let region = (|| {
        let Some(airbase) = airbase? else {
            return Ok(0xaa);
        };
        let state = state?.ok_or("transport airbase state observation missing")?;
        let province = if state != 0 {
            let cached: u64 = field(memory, airbase, crate::cair_base::PROVINCE)?;
            if cached != 0 {
                cached as usize
            } else {
                authenticate(memory, base, state as usize, crate::cstate::VTABLE_RVA)?;
                let state_id = field::<u32>(memory, state as usize, crate::cstate::ID)?;
                let kind: i32 = field(memory, airbase, crate::cair_base::KIND)?;
                let manager =
                    field::<u64>(memory, game, crate::cgame_state::STRATEGIC_AIR_MANAGER)? as usize;
                authenticate(
                    memory,
                    base,
                    manager,
                    crate::cstrategic_air_manager::VTABLE_RVA,
                )?;
                let offset = match kind {
                    1 => 0x60,
                    2 => 0x78,
                    _ => 0x48,
                };
                let roster = Array::read(memory, address(manager, offset)?, 24)
                    .and_then(|array| array.element(state_id, 24))
                    .ok_or("transport airbase state roster refused")?;
                let ids = Array::read(memory, roster, 4)
                    .ok_or("transport airbase province roster refused")?;
                if ids.count == 0 {
                    return Err("transport native state airbase province unresolved");
                }
                province(
                    memory,
                    base,
                    game,
                    read(memory, ids.element(0, 4).unwrap())
                        .ok_or("transport airbase first province unreadable")?,
                )?
            }
        } else if let Some(task) = host_task? {
            let mut location = field::<u64>(memory, task, 0x1f0)? as usize;
            let definition = field::<u64>(memory, location, crate::cprovince::DEFINITION)? as usize;
            if field::<u8>(memory, definition, 0xd2)? & 1 != 0 {
                location = province(memory, base, game, field(memory, definition, 0xc8)?)?;
            }
            location
        } else {
            0
        };
        let region = if province == 0 {
            if state != 0 {
                return Err("transport native state airbase province unresolved");
            }
            0xaa
        } else {
            authenticate(memory, base, province, crate::cprovince::VTABLE_RVA)?;
            let region: u64 = field(memory, province, crate::cprovince::STRATEGIC_REGION)?;
            if region == 0 {
                0xaa
            } else {
                authenticate(
                    memory,
                    base,
                    region as usize,
                    crate::cstrategic_region::VTABLE_RVA,
                )?;
                field(memory, region as usize, crate::cstrategic_region::ID)?
            }
        };
        Ok(region)
    })();
    (region, carrier)
}
fn province(memory: &impl Memory, base: usize, game: usize, id: u32) -> Fact<usize> {
    if id == 0 {
        return Ok(0);
    }
    let array = Array::read(memory, address(game, crate::cgame_state::PROVINCES)?, 8)
        .ok_or("transport air province root refused")?;
    let at = array
        .element(id, 8)
        .ok_or("transport air province id outside current root")?;
    let province =
        read::<u64>(memory, at).ok_or("transport air province pointer unreadable")? as usize;
    authenticate(memory, base, province, crate::cprovince::VTABLE_RVA)?;
    if field::<u32>(memory, province, crate::cprovince::ID)? != id {
        return Err("transport air province identity mismatch");
    }
    Ok(province)
}

#[cfg(test)]
#[path = "transport_air_normalization_tests.rs"]
mod tests;
