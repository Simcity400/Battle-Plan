
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use crate::{ccountry_intel_network as network, csub_intel_network as sub};
use engine_allocation as allocation;
use engine_native::cstate;

const LOCAL_GRAPH: usize = 0x08;
const LOCAL_SUBNETWORK_COUNT: usize = 0x10;
const LOCAL_QUIET: usize = 0x18;
const LOCAL_PREVIOUS_MAX_DEPTH: usize = 0x30;
const LOCAL_PREVIOUS_SOURCES: usize = 0x38;
const GRAPH_EDGES: usize = 0x00;
const GRAPH_EDGE_COUNT: usize = 0x08;
const GRAPH_ROWS_BEGIN: usize = 0x10;
const GRAPH_ROWS_END: usize = 0x18;
const EDGE_NEXT: usize = 0x00;
const EDGE_FROM: usize = 0x10;
const EDGE_TO: usize = 0x18;
const ROW_DEPTH: usize = 0x20;
const NETWORK_OPERATIVES: usize = 0x60;
const NETWORK_RECORDS: usize = 0x90;
const SUB_COVERAGE: usize = 0x5c;
const SUB_COVERABLE: usize = 0x68;
const SUB_SUMS: usize = 0x28;
const SUB_DETECTION_FACTOR: usize = 0x10;
const SUB_RECORDS: usize = 0x78;
const SUB_STATES: usize = 0x90;
const SUB_CORE_STATES: usize = 0xa8;
const SUB_OPERATIVES: usize = 0xc0;
const RECORD_COUNTS: usize = 0x08;
const RECORD_GAIN: usize = 0x10;
const RECORD_DETECTION: usize = 0x18;
const RECORD_DETECTION_FACTOR: usize = 0x20;

pub const NATIONAL: u32 = u32::MAX;
pub mod kind {
    pub const NETWORK: u8 = 0;
    pub const VERTEX: u8 = 1;
    pub const EDGE: u8 = 2;
    pub const PREVIOUS_SOURCE: u8 = 3;
    pub const QUIET: u8 = 4;
    pub const OPERATIVE: u8 = 5;
    pub const RECORD: u8 = 6;
    pub const SUBNETWORK: u8 = 7;
    pub const COVERAGE: u8 = 8;
    pub const COVERABLE: u8 = 9;
    pub const AGGREGATES: u8 = 10;
    pub const MODIFIERS: u8 = 11;
    pub const STATE: u8 = 12;
    pub const CORE_STATE: u8 = 13;
    pub const UNSELECTED_STRENGTH: u8 = 14;
    pub const UNSELECTED_TARGET: u8 = 15;
    pub const UNSELECTED_QUIET: u8 = 16;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub kind: u8,
    pub subnetwork: u32,
    pub ordinal: u64,
    pub country: u32,
    pub count_a: i32,
    pub count_b: i32,
    pub count_c: i32,
    pub first: i64,
    pub second: i64,
    pub third: i64,
}
impl Entry {
    pub fn key(&self) -> (u8, u32, u64) {
        (self.kind, self.subnetwork, self.ordinal)
    }
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    at: usize,
    offset: usize,
) -> Result<T, &'static str> {
    at.checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("network auxiliary field unreadable")
}
fn array(
    memory: &impl Memory,
    at: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    at.checked_add(offset)
        .and_then(|at| Array::read(memory, at, stride))
        .ok_or("network auxiliary list header invalid")
}
fn push(out: &mut Vec<Entry>, row: Entry) -> Result<(), &'static str> {
    allocation::reserve(out, 1).map_err(|_| "network auxiliary occurrence allocation failed")?;
    out.push(row);
    Ok(())
}
fn list<M: Memory>(
    memory: &M,
    out: &mut Vec<Entry>,
    array: Array,
    stride: usize,
    kind: u8,
    scope: u32,
    mut item: impl FnMut(&M, usize, &mut Entry) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    for ordinal in 0..array.count {
        let at = array
            .element(ordinal, stride)
            .ok_or("network auxiliary occurrence address overflow")?;
        let mut row = Entry {
            kind,
            subnetwork: scope,
            ordinal: u64::from(ordinal),
            ..Entry::default()
        };
        item(memory, at, &mut row)?;
        push(out, row)?;
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, Default)]
pub struct State {
    pub ordinal: u32,
    pub state: i32,
    pub selector: i32,
}

pub fn acquire(
    memory: &impl Memory,
    base: usize,
    seed: &network::Summary,
    states: &[State],
    out: &mut Vec<Entry>,
) -> Result<(), &'static str> {
    let object = seed.native_pointer;
    if field::<u64>(memory, object, 0)? != auth(base, network::VTABLE_RVA, "network auxiliary holder authentication failed")?
        || field::<u32>(memory, object, network::TARGET)? != seed.target_country
    {
        return Err("network auxiliary holder authentication failed");
    }
    out.clear();
    let local = object
        .checked_add(network::LOCAL_NETWORK)
        .ok_or("network auxiliary address overflow")?;
    if field::<u64>(memory, local, 0)? != auth(
        base,
        crate::clocal_intel_network::VTABLE_RVA,
        "network auxiliary local holder authentication failed",
    )? {
        return Err("network auxiliary local holder authentication failed");
    }
    let graph = field::<u64>(memory, local, LOCAL_GRAPH)? as usize;
    if graph == 0 {
        return Err("network auxiliary graph absent");
    }
    let begin = field::<u64>(memory, graph, GRAPH_ROWS_BEGIN)? as usize;
    let end = field::<u64>(memory, graph, GRAPH_ROWS_END)? as usize;
    let bytes = end
        .checked_sub(begin)
        .filter(|bytes| bytes % crate::intel_network_graph_row::STRIDE == 0)
        .ok_or("network auxiliary row range invalid")?;
    let rows = u32::try_from(bytes / crate::intel_network_graph_row::STRIDE)
        .ok()
        .filter(|n| *n <= i32::MAX as u32)
        .ok_or("network auxiliary native row count invalid")?;
    if rows != 0 && begin == 0 {
        return Err("network auxiliary rows absent");
    }
    let subnetworks = array(memory, object, network::SUBNETWORKS_DATA, sub::STRIDE)?;
    push(
        out,
        Entry {
            kind: kind::NETWORK,
            subnetwork: NATIONAL,
            count_a: subnetworks.count as i32,
            count_b: if rows == 0 {
                0
            } else {
                field(memory, local, LOCAL_PREVIOUS_MAX_DEPTH)?
            },
            count_c: rows as i32,
            ..Entry::default()
        },
    )?;
    if rows != 0 {
        push(
            out,
            Entry {
                kind: kind::NETWORK,
                subnetwork: NATIONAL,
                ordinal: 1,
                count_a: field(memory, local, LOCAL_SUBNETWORK_COUNT)?,
                ..Entry::default()
            },
        )?;
        if states.len() != rows as usize {
            return Err("network auxiliary vertices lack matching acquired state census");
        }
        for state in states {
            if state.ordinal >= rows {
                return Err("network auxiliary cached vertex ordinal out of bounds");
            }
            let at = begin
                .checked_add(state.ordinal as usize * crate::intel_network_graph_row::STRIDE)
                .ok_or("network auxiliary vertex address overflow")?;
            push(
                out,
                Entry {
                    kind: kind::VERTEX,
                    subnetwork: NATIONAL,
                    ordinal: u64::from(state.ordinal),
                    count_a: state.state,
                    count_b: field(memory, at, ROW_DEPTH)?,
                    ..Entry::default()
                },
            )?;
        }
        let head = field::<u64>(memory, graph, GRAPH_EDGES)? as usize;
        let count = field::<u64>(memory, graph, GRAPH_EDGE_COUNT)?;
        allocation::reserve(
            out,
            usize::try_from(count).map_err(|_| "network auxiliary edge count invalid")?,
        )
        .map_err(|_| "network auxiliary edge allocation failed")?;
        let mut node = field::<u64>(memory, head, 0)? as usize;
        for ordinal in 0..count {
            if node == head || node == 0 {
                return Err("network auxiliary edge list shorter than count");
            }
            push(
                out,
                Entry {
                    kind: kind::EDGE,
                    subnetwork: NATIONAL,
                    ordinal,
                    first: field::<u64>(memory, node, EDGE_FROM)? as i64,
                    second: field::<u64>(memory, node, EDGE_TO)? as i64,
                    ..Entry::default()
                },
            )?;
            node = field::<u64>(memory, node, EDGE_NEXT)? as usize;
        }
        if node != head {
            return Err("network auxiliary edge list exceeds count");
        }
        list(
            memory,
            out,
            array(memory, local, LOCAL_PREVIOUS_SOURCES, 4)?,
            4,
            kind::PREVIOUS_SOURCE,
            NATIONAL,
            |r, at, row| {
                row.count_a = field(r, at, 0)?;
                Ok(())
            },
        )?;
        list(
            memory,
            out,
            array(memory, local, LOCAL_QUIET, 1)?,
            1,
            kind::QUIET,
            NATIONAL,
            |r, at, row| {
                row.count_a = i32::from(field::<u8>(r, at, 0)?);
                Ok(())
            },
        )?;
    }
    list(
        memory,
        out,
        array(memory, object, NETWORK_OPERATIVES, 12)?,
        12,
        kind::OPERATIVE,
        NATIONAL,
        |r, at, row| {
            row.first = field::<u64>(r, at, 0)? as i64;
            row.count_a = field(r, at, 8)?;
            Ok(())
        },
    )?;
    records(
        memory,
        out,
        array(memory, object, NETWORK_RECORDS, 40)?,
        40,
        NATIONAL,
    )?;
    for ordinal in 0..subnetworks.count {
        let at = subnetworks
            .element(ordinal, sub::STRIDE)
            .ok_or("network auxiliary subnetwork address overflow")?;
        if field::<u64>(memory, at, 0)? != auth(base, sub::VTABLE_RVA, "network auxiliary subnetwork authentication failed")? {
            return Err("network auxiliary subnetwork authentication failed");
        }
        push(
            out,
            Entry {
                kind: kind::SUBNETWORK,
                subnetwork: ordinal,
                ..Entry::default()
            },
        )?;
        for (offset, kind) in [
            (SUB_COVERAGE, kind::COVERAGE),
            (SUB_COVERABLE, kind::COVERABLE),
        ] {
            let bytes = field::<[u8; 12]>(memory, at, offset)?;
            push(
                out,
                Entry {
                    kind,
                    subnetwork: ordinal,
                    count_a: i32::from_le_bytes(bytes[0..4].try_into().unwrap()),
                    count_b: i32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                    count_c: i32::from_le_bytes(bytes[8..12].try_into().unwrap()),
                    ..Entry::default()
                },
            )?;
        }
        let sums = field::<[u8; 24]>(memory, at, SUB_SUMS)?;
        push(
            out,
            Entry {
                kind: kind::AGGREGATES,
                subnetwork: ordinal,
                first: i64::from_le_bytes(sums[0..8].try_into().unwrap()),
                second: i64::from_le_bytes(sums[8..16].try_into().unwrap()),
                third: i64::from_le_bytes(sums[16..24].try_into().unwrap()),
                ..Entry::default()
            },
        )?;
        let modifiers = field::<[u8; 16]>(memory, at, SUB_DETECTION_FACTOR)?;
        push(
            out,
            Entry {
                kind: kind::MODIFIERS,
                subnetwork: ordinal,
                first: i64::from_le_bytes(modifiers[8..16].try_into().unwrap()),
                second: i64::from_le_bytes(modifiers[0..8].try_into().unwrap()),
                ..Entry::default()
            },
        )?;
        records(
            memory,
            out,
            array(memory, at, SUB_RECORDS, 12)?,
            12,
            ordinal,
        )?;
        list(
            memory,
            out,
            array(memory, at, SUB_OPERATIVES, 8)?,
            8,
            kind::OPERATIVE,
            ordinal,
            |r, at, row| {
                row.first = field::<u64>(r, at, 0)? as i64;
                Ok(())
            },
        )?;
        for (offset, kind) in [
            (SUB_STATES, kind::STATE),
            (SUB_CORE_STATES, kind::CORE_STATE),
        ] {
            let stride = if kind == kind::STATE { 16 } else { 8 };
            list(
                memory,
                out,
                array(memory, at, offset, stride)?,
                stride,
                kind,
                ordinal,
                |r, at, row| {
                    let state = field::<u64>(r, at, 0)? as usize;
                    if state == 0 {
                        return Err("network auxiliary state reference null");
                    }
                    row.count_a = field(r, state, cstate::ID)?;
                    if stride == 16 {
                        row.first = field(r, at, 8)?;
                    }
                    Ok(())
                },
            )?;
        }
        if states
            .binary_search_by_key(&(ordinal as i32), |row| row.selector)
            .is_err()
        {
            push(
                out,
                Entry {
                    kind: kind::UNSELECTED_STRENGTH,
                    subnetwork: ordinal,
                    first: field(memory, at, sub::STRENGTH)?,
                    second: field(memory, at, sub::GAIN_FACTOR)?,
                    third: field(memory, at, sub::STRENGTH_TARGET)?,
                    ..Entry::default()
                },
            )?;
            push(
                out,
                Entry {
                    kind: kind::UNSELECTED_TARGET,
                    subnetwork: ordinal,
                    first: field(memory, at, sub::TARGET_FROM_OPERATIVES)?,
                    second: field(memory, at, sub::TARGET_FROM_COUNTERINTELLIGENCE)?,
                    ..Entry::default()
                },
            )?;
            push(
                out,
                Entry {
                    kind: kind::UNSELECTED_QUIET,
                    subnetwork: ordinal,
                    count_a: i32::from(field::<u8>(memory, at, sub::QUIET)?),
                    ..Entry::default()
                },
            )?;
        }
    }
    out.sort_unstable_by_key(Entry::key);
    Ok(())
}

fn records(
    memory: &impl Memory,
    out: &mut Vec<Entry>,
    array: Array,
    stride: usize,
    scope: u32,
) -> Result<(), &'static str> {
    list(
        memory,
        out,
        array,
        stride,
        kind::RECORD,
        scope,
        |r, at, row| {
            row.country = field(r, at, 0)?;
            let offset = if stride == 40 { RECORD_COUNTS } else { 4 };
            row.count_a = field(r, at, offset)?;
            row.count_b = field(r, at, offset + 4)?;
            if stride == 40 {
                row.first = field(r, at, RECORD_GAIN)?;
                row.second = field(r, at, RECORD_DETECTION)?;
                row.third = field(r, at, RECORD_DETECTION_FACTOR)?;
            }
            Ok(())
        },
    )
}

fn auth(base: usize, key: Rva, failed: &'static str) -> Result<u64, &'static str> {
    engine_sites::at(base, key).map(|address| address as u64).ok_or(failed)
}

pub struct Site {
    pub name: &'static str,
    pub rva: Rva,
}
pub const SITES: [Site; 7] = [
    Site {
        name: "national rebuild",
        rva: Rva(0x12638d0),
    },
    Site {
        name: "national token load",
        rva: Rva(0x1263400),
    },
    Site {
        name: "network construction",
        rva: Rva(0x125ccf0),
    },
    Site {
        name: "empty network construction",
        rva: Rva(0x125cda0),
    },
    Site {
        name: "network destruction",
        rva: Rva(0x125dff0),
    },
    Site {
        name: "local token load",
        rva: Rva(0x1251e10),
    },
    Site {
        name: "subnetwork preparation",
        rva: Rva(0x125e790),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    fn word(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn count(bytes: &mut [u8], at: usize, value: i32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn header(bytes: &mut [u8], at: usize, data: usize, len: i32) {
        word(bytes, at, data as u64);
        count(bytes, at + 8, len);
        count(bytes, at + 12, len);
    }
    #[test]
    fn network_auxiliary_keeps_independent_empty_subnetworks_and_occurrences() {
        const OBJECT: usize = 0x100;
        const GRAPH: usize = 0x800;
        const SUBS: usize = 0x1000;
        const OPS: usize = 0x2000;
        let mut bytes = vec![0; 0x3000];
        word(&mut bytes, OBJECT, network::VTABLE_RVA.0 as u64);
        count(&mut bytes, OBJECT + network::TARGET, 23);
        word(
            &mut bytes,
            OBJECT + network::LOCAL_NETWORK,
            crate::clocal_intel_network::VTABLE_RVA.0 as u64,
        );
        word(
            &mut bytes,
            OBJECT + network::LOCAL_NETWORK + LOCAL_GRAPH,
            GRAPH as u64,
        );
        header(&mut bytes, OBJECT + network::SUBNETWORKS_DATA, SUBS, 2);
        for index in 0..2 {
            let at = SUBS + index * sub::STRIDE;
            word(&mut bytes, at, sub::VTABLE_RVA.0 as u64);
            count(&mut bytes, at + SUB_COVERAGE, -7 - index as i32);
            word(&mut bytes, at + SUB_DETECTION_FACTOR, 100_000);
            word(&mut bytes, at + SUB_DETECTION_FACTOR + 8, i64::MIN as u64);
            word(&mut bytes, at + SUB_SUMS, 77 + index as u64);
        }
        header(&mut bytes, SUBS + SUB_OPERATIVES, OPS, 2);
        word(&mut bytes, OPS, u64::MAX);
        word(&mut bytes, OPS + 8, u64::MAX);
        let seed = network::Summary {
            native_pointer: OBJECT,
            owner_country: 17,
            target_country: 23,
            ..network::Summary::default()
        };
        let mut entries = Vec::new();
        acquire(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            &seed,
            &[],
            &mut entries,
        )
        .unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|row| row.kind == kind::SUBNETWORK)
                .count(),
            2
        );
        assert_eq!(
            entries
                .iter()
                .filter(|row| row.kind == kind::UNSELECTED_STRENGTH)
                .count(),
            2
        );
        assert!(
            entries
                .iter()
                .any(|row| row.kind == kind::COVERAGE && row.subnetwork == 1 && row.count_a == -8)
        );
        let modifiers: Vec<_> = entries
            .iter()
            .filter(|row| row.kind == kind::MODIFIERS)
            .map(|row| (row.subnetwork, row.first, row.second))
            .collect();
        assert_eq!(modifiers, [(0, i64::MIN, 100_000), (1, i64::MIN, 100_000)]);
        let operatives: Vec<_> = entries
            .iter()
            .filter(|row| row.kind == kind::OPERATIVE)
            .collect();
        assert_eq!(operatives.len(), 2);
        assert_eq!(operatives[0].first, -1);
        assert_ne!(operatives[0].ordinal, operatives[1].ordinal);
        word(&mut bytes, GRAPH + GRAPH_ROWS_BEGIN, 0x2500);
        word(&mut bytes, GRAPH + GRAPH_ROWS_END, 0x2550);
        word(&mut bytes, GRAPH + GRAPH_EDGES, 0x2700);
        word(&mut bytes, 0x2700, 0x2700);
        let selected = [State {
            ordinal: 0,
            state: 42,
            selector: 0,
        }];
        acquire(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            &seed,
            &selected,
            &mut entries,
        )
        .unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|row| row.kind == kind::UNSELECTED_STRENGTH)
                .map(|row| row.subnetwork)
                .collect::<Vec<_>>(),
            vec![1]
        );
        bytes[OBJECT] = 0;
        assert_eq!(
            acquire(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                &seed,
                &selected,
                &mut entries
            ),
            Err("network auxiliary holder authentication failed")
        );
    }
}
