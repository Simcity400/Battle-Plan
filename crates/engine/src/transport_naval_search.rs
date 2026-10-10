//! Original strategic-region graph of FA1CB0/FA1E90 -> F922B0/F91960.
//! E0DD70 owns the ordered +B0 neighbours and corresponding +C8 records;
//! E0F160 copies the signed integer centroid at +F0. F9E3A0 reads only
//! adjacency flags, +58 and ordered nullable rule references. Capture does
//! not call native search, reconstruct neighbours or retain native controls.
//!
//! Ownership: ctor140E0B040 copies template+E0 into region+F0 and constructs
//! both arrays; reset1401E0190 calls E0DD70 after constructing the original
//! region roster. E0DD70 clears/rebuilds both arrays and copies its temporary
//! records through E0B230. Serializer E0FF10 / loader E0F190 touch only the
//! name and naval-dominance object, not this search graph. Destructor E0B6E0
//! retires the +C8 records before +B0. Rule wrappers point into original
//! province-definition adjacencies; F9E3A0 dereferences wrapper then rule.
//!
//! The caller must supply the same quiescent world lease as the other native
//! readers. Production captures each hour in the original Ordinary mission
//! pass, and the callback joins that job before native HourlyUpdate resumes.
//! This reader re-observes the roster, width, arrays and rules every call; it
//! assumes no cross-hour or session-static pointer lifetime. Only numerical
//! output and owned name bytes escape. Synthetic retirement checks do not
//! independently certify the game's task/load quiescence at that boundary.
use crate::{
    Memory, cgame_state, cmap,
    cpdx_array::Array,
    cstrategic_region,
    msvc_string::Descriptor,
    read,
    transport_ledger_observation::{PointerIndex, PointerSlot},
};

type Fact<T> = Result<T, &'static str>;
const RULE_VTABLE: usize = 0x029b_f7f8; // CAdjacencyRule ctor141534660, RTTI.
const NEIGHBOURS: usize = 0xb0; // E0DD70/E0F170/E0F180.
const ADJACENCIES: usize = 0xc8; // E0DD70; E0EC70 returns parallel record.
const POSITION: usize = 0xf0; // E0F160: exact qword copy.
const ADJACENCY_STRIDE: usize = 0x60;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub present: bool,
    pub position: [i32; 2],
    pub first: usize,
    pub count: usize,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub to: u32,
    pub flags: u8,
    pub unrestricted: u8,
    pub first: usize,
    pub count: usize,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Name {
    pub first: usize,
    pub count: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub regions: usize,
    pub edges: usize,
    pub occurrences: usize,
    pub names: usize,
    pub bytes: usize,
}
#[derive(Debug, Clone, Copy)]
pub struct Capture {
    pub lengths: Fact<Demand>,
    pub required: Demand,
    pub width: Fact<i32>,
}

#[derive(Default)]
pub struct RegionControl {
    pointer: usize,
    neighbours: Option<Fact<Array>>,
    adjacencies: Option<Fact<Array>>,
}
#[derive(Default)]
pub struct NameControl {
    value: Option<Fact<Name>>,
}

pub struct Output<'a> {
    pub regions: &'a mut [Region],
    pub edges: &'a mut [Edge],
    /// None is the native null rule; Some indexes the unique name dictionary.
    pub occurrences: &'a mut [Option<u32>],
    pub names: &'a mut [Name],
    pub bytes: &'a mut [u8],
    pub region_controls: &'a mut [RegionControl],
    pub region_slots: &'a mut [PointerSlot],
    pub name_controls: &'a mut [NameControl],
    pub name_slots: &'a mut [PointerSlot],
}

pub struct GrowingOutput<'a> {
    pub regions: &'a mut Vec<Region>,
    pub edges: &'a mut Vec<Edge>,
    /// None is the native null rule; Some indexes the unique name dictionary.
    pub occurrences: &'a mut Vec<Option<u32>>,
    pub names: &'a mut Vec<Name>,
    pub bytes: &'a mut Vec<u8>,
    pub region_controls: &'a mut Vec<RegionControl>,
    pub region_slots: &'a mut Vec<PointerSlot>,
    pub name_controls: &'a mut Vec<NameControl>,
    pub name_slots: &'a mut Vec<PointerSlot>,
}

pub struct OutputBuffers<'a> {
    pub regions: crate::walk::Buffer<'a, Region>,
    pub edges: crate::walk::Buffer<'a, Edge>,
    /// None is the native null rule; Some indexes the unique name dictionary.
    pub occurrences: crate::walk::Buffer<'a, Option<u32>>,
    pub names: crate::walk::Buffer<'a, Name>,
    pub bytes: crate::walk::Buffer<'a, u8>,
    pub region_controls: crate::walk::Buffer<'a, RegionControl>,
    pub region_slots: crate::walk::Buffer<'a, PointerSlot>,
    pub name_controls: crate::walk::Buffer<'a, NameControl>,
    pub name_slots: crate::walk::Buffer<'a, PointerSlot>,
}

impl<'a> From<Output<'a>> for OutputBuffers<'a> {
    fn from(output: Output<'a>) -> Self {
        Self {
            regions: crate::walk::Buffer::Fixed(output.regions),
            edges: crate::walk::Buffer::Fixed(output.edges),
            occurrences: crate::walk::Buffer::Fixed(output.occurrences),
            names: crate::walk::Buffer::Fixed(output.names),
            bytes: crate::walk::Buffer::Fixed(output.bytes),
            region_controls: crate::walk::Buffer::Fixed(output.region_controls),
            region_slots: crate::walk::Buffer::Fixed(output.region_slots),
            name_controls: crate::walk::Buffer::Fixed(output.name_controls),
            name_slots: crate::walk::Buffer::Fixed(output.name_slots),
        }
    }
}
impl<'a> From<GrowingOutput<'a>> for OutputBuffers<'a> {
    fn from(output: GrowingOutput<'a>) -> Self {
        Self {
            regions: crate::walk::Buffer::Growing(output.regions),
            edges: crate::walk::Buffer::Growing(output.edges),
            occurrences: crate::walk::Buffer::Growing(output.occurrences),
            names: crate::walk::Buffer::Growing(output.names),
            bytes: crate::walk::Buffer::Growing(output.bytes),
            region_controls: crate::walk::Buffer::Growing(output.region_controls),
            region_slots: crate::walk::Buffer::Growing(output.region_slots),
            name_controls: crate::walk::Buffer::Growing(output.name_controls),
            name_slots: crate::walk::Buffer::Growing(output.name_slots),
        }
    }
}

fn at(object: usize, offset: usize) -> Fact<usize> {
    object
        .checked_add(offset)
        .ok_or("transport naval search address overflow")
}
fn field<T: crate::memory::Raw>(memory: &impl Memory, object: usize, offset: usize) -> Fact<T> {
    read(memory, at(object, offset)?).ok_or("transport naval search operand unreadable")
}
fn array(memory: &impl Memory, object: usize, offset: usize, stride: usize) -> Fact<Array> {
    Array::read(memory, at(object, offset)?, stride)
        .ok_or("transport naval search header invalid or unreadable")
}
fn pointer(memory: &impl Memory, address: usize) -> Fact<usize> {
    read::<u64>(memory, address)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("transport naval search pointer unreadable")
}
fn add(value: &mut usize, count: usize) -> Fact<()> {
    *value = value
        .checked_add(count)
        .ok_or("transport naval search demand overflow")?;
    Ok(())
}

/// Loan from this original authenticated roster; controls never escape.
pub trait SpottingObserver {
    fn begin(&mut self, original_count: Fact<usize>);
    fn region(
        &mut self,
        slot: u32,
        original: Fact<Option<crate::transport_spotting_context::RegionOriginal>>,
    );
}

/// Off-thread header/descriptor sizing only. Repeated rules contribute a safe
/// upper bound for the unique dictionary; no captured value or pointer escapes.
/// E0DD70 pairs the arrays; F9E3A0 reaches rules only through flags2/+58==0.
pub fn demand(memory: &impl Memory, base: usize, state: usize) -> Fact<Demand> {
    let roster = array(memory, state, cgame_state::STRATEGIC_REGIONS, 8)?;
    let mut demand = Demand {
        regions: roster.count as usize,
        ..Default::default()
    };
    for slot in 0..roster.count {
        let object = pointer(
            memory,
            roster
                .element(slot, 8)
                .ok_or("transport region roster overflow")?,
        )?;
        if object == 0 {
            continue;
        }
        if field::<u64>(memory, object, 0)? != at(base, cstrategic_region::VTABLE_RVA)? as u64
            || field::<u32>(memory, object, cstrategic_region::ID)? != slot
        {
            return Err("transport naval search region identity mismatch");
        }
        let neighbours = array(memory, object, NEIGHBOURS, 8)?;
        let adjacencies = array(memory, object, ADJACENCIES, ADJACENCY_STRIDE)?;
        if neighbours.count != adjacencies.count {
            return Err("transport region neighbour/adjacency counts differ");
        }
        add(&mut demand.edges, neighbours.count as usize)?;
        for ordinal in 0..adjacencies.count {
            let adjacency = adjacencies
                .element(ordinal, ADJACENCY_STRIDE)
                .ok_or("transport region adjacency overflow")?;
            if field::<u8>(memory, adjacency, 0)? & 2 == 0
                || field::<u8>(memory, adjacency, 0x58)? != 0
            {
                continue;
            }
            let rules = array(memory, adjacency, 0x10, 8)?;
            add(&mut demand.occurrences, rules.count as usize)?;
            add(&mut demand.names, rules.count as usize)?;
            for occurrence in 0..rules.count {
                let wrapper = pointer(
                    memory,
                    rules
                        .element(occurrence, 8)
                        .ok_or("transport rule occurrence overflow")?,
                )?;
                if wrapper == 0 {
                    return Err("transport naval rule wrapper is null");
                }
                let rule = pointer(memory, wrapper)?;
                if rule == 0 {
                    continue;
                }
                if field::<u64>(memory, rule, 0)? != at(base, RULE_VTABLE)? as u64 {
                    return Err("transport naval adjacency rule identity mismatch");
                }
                add(
                    &mut demand.bytes,
                    Descriptor::read(memory, at(rule, 0x38)?)?.len,
                )?;
            }
        }
    }
    Ok(demand)
}

/// Every original roster/header is read once. Known demand survives an arena
/// refusal; no prefix is published. Both pointer indexes and controls retire
/// before return, including identity/read/capacity failures.
pub fn capture(memory: &impl Memory, base: usize, state: usize, output: Output<'_>) -> Capture {
    capture_buffered(memory, base, state, output.into(), None)
}
pub fn capture_with_spotting(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: Output<'_>,
    observer: &mut dyn SpottingObserver,
) -> Capture {
    capture_buffered(memory, base, state, output.into(), Some(observer))
}
pub fn capture_growing(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: GrowingOutput<'_>,
) -> Capture {
    capture_buffered(memory, base, state, output.into(), None)
}
pub fn capture_growing_with_spotting(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: GrowingOutput<'_>,
    observer: &mut dyn SpottingObserver,
) -> Capture {
    capture_buffered(memory, base, state, output.into(), Some(observer))
}
pub fn capture_buffered(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: OutputBuffers<'_>,
    observer: Option<&mut dyn SpottingObserver>,
) -> Capture {
    let mut required = Demand::default();
    let width = (|| {
        let map = pointer(memory, at(base, cmap::SINGLETON_RVA)?)?;
        if map == 0 {
            return Err("transport naval search map is null");
        }
        let width = field(memory, map, cmap::WIDTH)?;
        if width <= 0 {
            return Err("transport naval search map width is not positive");
        }
        Ok(width)
    })();
    let lengths = read_graph(memory, base, state, &mut output, &mut required, observer);
    for control in output.region_controls.iter_mut() {
        *control = RegionControl::default();
    }
    for control in output.name_controls.iter_mut() {
        *control = NameControl::default();
    }
    output.region_slots.fill(PointerSlot::default());
    output.name_slots.fill(PointerSlot::default());
    Capture {
        lengths,
        required,
        width,
    }
}

fn read_graph(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut OutputBuffers<'_>,
    required: &mut Demand,
    mut observer: Option<&mut dyn SpottingObserver>,
) -> Fact<Demand> {
    let roster = array(memory, state, cgame_state::STRATEGIC_REGIONS, 8);
    if let Some(observer) = observer.as_deref_mut() {
        observer.begin(roster.map(|array| array.count as usize));
    }
    let roster = roster?;
    required.regions = roster.count as usize;
    output.regions.ensure(required.regions)?;
    output.region_controls.ensure(required.regions)?;
    let mut regions = PointerIndex::from_buffer(
        std::mem::replace(
            &mut output.region_slots,
            crate::walk::Buffer::Fixed(&mut []),
        ),
        output.region_controls.len(),
    )?;
    let mut first_failure = None;
    let expected = at(base, cstrategic_region::VTABLE_RVA)? as u64;
    // Preserve all known edge demand even when the region row/control arena
    // cannot admit the whole original physical roster.
    for slot in 0..roster.count {
        let mut spotting_original = Err("transport original spotting region identity refused");
        let original = (|| {
            let object = pointer(
                memory,
                roster
                    .element(slot, 8)
                    .ok_or("transport region roster overflow")?,
            )?;
            if object == 0 {
                spotting_original = Ok(None);
                return Ok(None);
            }
            if field::<u64>(memory, object, 0)? != expected
                || field::<u32>(memory, object, cstrategic_region::ID)? != slot
            {
                return Err("transport naval search region identity mismatch");
            }
            if observer.is_some() {
                spotting_original = Ok(Some(
                    crate::transport_spotting_context::RegionOriginal::capture_authenticated(
                        memory, object, slot,
                    ),
                ));
            }
            let neighbours = array(memory, object, NEIGHBOURS, 8);
            if let Ok(header) = neighbours {
                add(&mut required.edges, header.count as usize)?;
            }
            let adjacencies = array(memory, object, ADJACENCIES, ADJACENCY_STRIDE);
            Ok(Some((object, neighbours, adjacencies)))
        })();
        if let Some(observer) = observer.as_deref_mut() {
            observer.region(slot, spotting_original);
        }
        match original {
            Err(reason) => {
                first_failure.get_or_insert(reason);
            }
            Ok(value) => {
                let index = slot as usize;
                if index >= output.regions.len() || index >= output.region_controls.len() {
                    first_failure.get_or_insert("transport naval search region arena exhausted");
                    continue;
                }
                output.regions[index] = Region {
                    present: value.is_some(),
                    ..Default::default()
                };
                if let Some((object, neighbours, adjacencies)) = value {
                    if regions.find(object).is_some() {
                        first_failure.get_or_insert("transport region roster repeats a pointer");
                        continue;
                    }
                    regions.insert(object, index)?;
                    output.region_controls[index] = RegionControl {
                        pointer: object,
                        neighbours: Some(neighbours),
                        adjacencies: Some(adjacencies),
                    };
                    if let Err(reason) = neighbours.and(adjacencies) {
                        first_failure.get_or_insert(reason);
                    }
                }
            }
        }
    }
    if let Some(reason) = first_failure {
        return Err(reason);
    }
    output.edges.ensure(required.edges)?;
    let mut names = PointerIndex::from_buffer(
        std::mem::replace(&mut output.name_slots, crate::walk::Buffer::Fixed(&mut [])),
        output.name_controls.len(),
    )?;
    let mut lengths = Demand {
        regions: required.regions,
        ..Default::default()
    };
    for slot in 0..required.regions {
        let control = &output.region_controls[slot];
        if control.pointer == 0 {
            continue;
        }
        let neighbours = control
            .neighbours
            .expect("present original region header")?;
        let adjacencies = control
            .adjacencies
            .expect("present original adjacency header")?;
        if neighbours.count != adjacencies.count {
            return Err("transport region neighbour/adjacency counts differ");
        }
        let position = field::<[u8; 8]>(memory, control.pointer, POSITION)?;
        output.regions[slot].position = [
            i32::from_le_bytes(position[..4].try_into().unwrap()),
            i32::from_le_bytes(position[4..].try_into().unwrap()),
        ];
        output.regions[slot].first = lengths.edges;
        output.regions[slot].count = neighbours.count as usize;
        for ordinal in 0..neighbours.count {
            let result = (|| {
                let neighbour = pointer(
                    memory,
                    neighbours
                        .element(ordinal, 8)
                        .ok_or("transport region neighbour overflow")?,
                )?;
                let to = regions
                    .find(neighbour)
                    .ok_or("transport region neighbour outside original roster")?;
                let adjacency = adjacencies
                    .element(ordinal, ADJACENCY_STRIDE)
                    .ok_or("transport region adjacency overflow")?;
                let flags = field(memory, adjacency, 0)?;
                let unrestricted = field(memory, adjacency, 0x58)?;
                if flags & 2 == 0 || unrestricted != 0 {
                    *output
                        .edges
                        .get_mut(lengths.edges)
                        .ok_or("transport naval edge arena exhausted")? = Edge {
                        to: u32::try_from(to).map_err(|_| "transport region ID overflow")?,
                        flags,
                        unrestricted,
                        first: lengths.occurrences,
                        count: 0,
                    };
                    lengths.edges += 1;
                    return Ok(());
                }
                let rules = array(memory, adjacency, 0x10, 8)?;
                add(&mut required.occurrences, rules.count as usize)?;
                required.names = required.names.max(required.occurrences);
                output.occurrences.ensure(
                    lengths
                        .occurrences
                        .checked_add(rules.count as usize)
                        .ok_or("transport naval occurrence demand overflow")?,
                )?;
                let edge = Edge {
                    to: u32::try_from(to).map_err(|_| "transport region ID overflow")?,
                    flags,
                    unrestricted,
                    first: lengths.occurrences,
                    count: rules.count as usize,
                };
                for occurrence in 0..rules.count {
                    let wrapper = pointer(
                        memory,
                        rules
                            .element(occurrence, 8)
                            .ok_or("transport rule occurrence overflow")?,
                    )?;
                    if wrapper == 0 {
                        return Err("transport naval rule wrapper is null");
                    }
                    let rule = pointer(memory, wrapper)?;
                    let name = if rule == 0 {
                        None
                    } else {
                        let index = match names.find(rule) {
                            Some(index) => index,
                            None => {
                                let index = lengths.names;
                                output.name_controls.ensure(index + 1)?;
                                output.names.ensure(index + 1)?;
                                let entry = output
                                    .name_controls
                                    .get_mut(index)
                                    .ok_or("transport naval rule memo arena exhausted")?;
                                names.insert(rule, index)?;
                                lengths.names += 1;
                                let value = (|| {
                                    if field::<u64>(memory, rule, 0)?
                                        != at(base, RULE_VTABLE)? as u64
                                    {
                                        return Err(
                                            "transport naval adjacency rule identity mismatch",
                                        );
                                    }
                                    let text = Descriptor::read(memory, at(rule, 0x38)?)?;
                                    add(&mut required.bytes, text.len)?;
                                    let end = lengths
                                        .bytes
                                        .checked_add(text.len)
                                        .ok_or("transport naval rule byte overflow")?;
                                    output.bytes.ensure(end)?;
                                    let bytes = output
                                        .bytes
                                        .get_mut(lengths.bytes..end)
                                        .ok_or("transport naval rule byte arena exhausted")?;
                                    text.copy(memory, bytes)?;
                                    let value = Name {
                                        first: lengths.bytes,
                                        count: text.len,
                                    };
                                    lengths.bytes = end;
                                    *output
                                        .names
                                        .get_mut(index)
                                        .ok_or("transport naval rule name arena exhausted")? =
                                        value;
                                    Ok(value)
                                })();
                                *entry = NameControl { value: Some(value) };
                                index
                            }
                        };
                        output.name_controls[index]
                            .value
                            .expect("original rule name observation")?;
                        Some(
                            u32::try_from(index)
                                .map_err(|_| "transport naval rule index overflow")?,
                        )
                    };
                    *output
                        .occurrences
                        .get_mut(lengths.occurrences)
                        .ok_or("transport naval rule occurrence arena exhausted")? = name;
                    lengths.occurrences += 1;
                }
                *output
                    .edges
                    .get_mut(lengths.edges)
                    .ok_or("transport naval edge arena exhausted")? = edge;
                lengths.edges += 1;
                Ok(())
            })();
            if let Err(reason) = result {
                first_failure.get_or_insert(reason);
            }
        }
    }
    if let Some(reason) = first_failure {
        Err(reason)
    } else {
        Ok(lengths)
    }
}

#[cfg(test)]
mod tests;
