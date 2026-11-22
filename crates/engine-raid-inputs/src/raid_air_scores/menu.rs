use super::{BATCH, Memory, RecordField, RecordLayout, field};
pub use game_model::air_power::{Contribution, Menu};
use engine_native::{
    allocation, memory::gather_records, msvc_tree_node as node, read as native_read,
};
use std::sync::Arc;

const COVERAGE: usize = 0x10;
const REDUCTIONS: usize = 0x30;
const MAP_HEADERS: usize = REDUCTIONS + 32;
const SENTINEL_FIELDS: [RecordField; 1] = [field(0, 0, 26)];
const SENTINEL_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SENTINEL_FIELDS, 26).expect("valid air power map sentinel layout");
const NODE_BYTES: usize = node::PAYLOAD + 16;

pub(super) enum Read {
    Unavailable,
    Zero,
    Same,
    Changed(Arc<Menu>),
}

#[derive(Clone, Copy)]
struct Pending {
    address: usize,
    right: usize,
    contribution: Contribution,
}

#[derive(Default)]
pub struct Scratch {
    contributions: [Vec<Contribution>; 2],
    stack: Vec<Pending>,
}

impl Scratch {
    pub(super) fn allocated_bytes(&self) -> usize {
        self.contributions
            .iter()
            .map(|rows| rows.capacity() * size_of::<Contribution>())
            .sum::<usize>()
            + self.stack.capacity() * size_of::<Pending>()
    }
}

pub(super) fn read_from_entries(
    memory: &impl Memory,
    entries: &[u8],
    stride: usize,
    complete: Option<&[bool]>,
    held: &[Option<&Arc<Menu>>],
    scratch: &mut Scratch,
) -> [Read; BATCH] {
    let count = entries.len() / stride;
    let is_complete = |index: usize| complete.is_none_or(|mask| mask[index]);
    let mut heads = [0_usize; BATCH * 2];
    let mut gathered = [usize::MAX; BATCH * 2];
    let mut nonempty = 0;
    for index in 0..count {
        if !is_complete(index) {
            continue;
        }
        let base = index * stride + MAP_HEADERS;
        for side in 0..2 {
            let at = base + side * 16;
            if let Ok((head, map_count)) =
                node::Cursor::header(entries[at..at + 16].try_into().unwrap())
                && map_count != 0
            {
                gathered[index * 2 + side] = nonempty;
                heads[nonempty] = head;
                nonempty += 1;
            }
        }
    }
    let mut sentinels = [0_u8; BATCH * 2 * 26];
    let mut valid = [false; BATCH * 2];
    if nonempty != 0 {
        gather_records(
            memory,
            &heads[..nonempty],
            &SENTINEL_LAYOUT,
            &mut sentinels[..nonempty * 26],
            &mut valid[..nonempty],
        );
    }
    let sentinel = |map: usize| -> Result<Option<[u8; 26]>, ()> {
        match gathered[map] {
            usize::MAX => Ok(None),
            at if valid[at] => Ok(Some(sentinels[at * 26..(at + 1) * 26].try_into().unwrap())),
            _ => Err(()),
        }
    };
    std::array::from_fn(|index| {
        if index >= count || !is_complete(index) {
            return Read::Unavailable;
        }
        let (Ok(first), Ok(second)) = (sentinel(index * 2), sentinel(index * 2 + 1)) else {
            return Read::Unavailable;
        };
        let entry = &entries[index * stride..(index + 1) * stride];
        let coverage = i64::from_le_bytes(entry[COVERAGE..COVERAGE + 8].try_into().unwrap());
        let reductions = std::array::from_fn(|i| {
            i64::from_le_bytes(
                entry[REDUCTIONS + i * 8..REDUCTIONS + (i + 1) * 8]
                    .try_into()
                    .unwrap(),
            )
        });
        let headers = [
            entry[MAP_HEADERS..MAP_HEADERS + 16].try_into().unwrap(),
            entry[MAP_HEADERS + 16..MAP_HEADERS + 32]
                .try_into()
                .unwrap(),
        ];
        decode_parts(
            memory,
            coverage,
            reductions,
            headers,
            [first, second],
            held.get(index).copied().flatten(),
            scratch,
        )
        .unwrap_or(Read::Unavailable)
    })
}

fn decode_parts(
    memory: &impl Memory,
    coverage: i64,
    reductions: [i64; 4],
    headers: [[u8; 16]; 2],
    sentinels: [Option<[u8; 26]>; 2],
    held: Option<&Arc<Menu>>,
    scratch: &mut Scratch,
) -> Result<Read, &'static str> {
    let Scratch {
        contributions,
        stack,
    } = scratch;
    for (side, rows) in contributions.iter_mut().enumerate() {
        read_map(memory, headers[side], sentinels[side], stack, rows)?;
    }
    if let Some(held) = held
        && held.coverage == coverage
        && held.reductions == reductions
        && held.contributions[0] == contributions[0]
        && held.contributions[1] == contributions[1]
    {
        return Ok(Read::Same);
    }
    if coverage == 0 && reductions == [0; 4] && contributions.iter().all(Vec::is_empty) {
        return Ok(Read::Zero);
    }
    let mut owned = [Vec::new(), Vec::new()];
    for (output, input) in owned.iter_mut().zip(contributions.iter()) {
        allocation::reserve_exact(output, input.len())
            .map_err(|_| "air power contribution allocation failed")?;
        output.extend_from_slice(input);
    }
    Ok(Read::Changed(Arc::new(Menu {
        coverage,
        reductions,
        contributions: owned,
    })))
}

fn read_map(
    memory: &impl Memory,
    header: [u8; 16],
    sentinel: Option<[u8; 26]>,
    stack: &mut Vec<Pending>,
    rows: &mut Vec<Contribution>,
) -> Result<(), &'static str> {
    rows.clear();
    stack.clear();
    let (head, count) = node::Cursor::header(header)?;
    if count == 0 {
        return Ok(());
    }
    let sentinel = sentinel.ok_or("air power contribution map sentinel unreadable")?;
    if sentinel[node::NIL] != 1 {
        return Err("air power contribution map sentinel invalid");
    }
    let link = |bytes: &[u8], offset: usize| {
        usize::try_from(u64::from_le_bytes(
            bytes[offset..offset + 8].try_into().unwrap(),
        ))
        .ok()
        .filter(|&pointer| pointer != 0)
        .ok_or("air power contribution map link unreadable or null")
    };
    let leftmost = link(&sentinel[..], node::LEFT)?;
    let root = link(&sentinel[..], node::PARENT)?;
    if leftmost == head || root == head {
        return Err("air power contribution map size disagrees with sentinel");
    }
    let (mut current, mut parent, mut pushed) = (root, head, 0_u32);
    loop {
        while current != head {
            if pushed == count {
                return Err("air power contribution map contains excess nodes");
            }
            pushed += 1;
            let record = native_read::<[u8; NODE_BYTES]>(memory, current)
                .ok_or("air power contribution unreadable")?;
            if record[node::NIL] != 0 {
                return Err("air power contribution map ended before declared count");
            }
            if link(&record[..], node::PARENT)? != parent {
                return Err("air power contribution map parent mismatch");
            }
            allocation::reserve_push_batch(stack, 1)
                .map_err(|_| "air power contribution allocation failed")?;
            stack.push(Pending {
                address: current,
                right: link(&record[..], node::RIGHT)?,
                contribution: Contribution {
                    country: u32::from_le_bytes(
                        record[node::PAYLOAD..node::PAYLOAD + 4].try_into().unwrap(),
                    ),
                    amount: i64::from_le_bytes(record[node::PAYLOAD + 8..].try_into().unwrap()),
                },
            });
            parent = current;
            current = link(&record[..], node::LEFT)?;
        }
        let Some(pending) = stack.pop() else {
            break;
        };
        if rows.is_empty() && pending.address != leftmost {
            return Err("air power contribution map leftmost link invalid");
        }
        allocation::reserve_push_batch(rows, 1)
            .map_err(|_| "air power contribution allocation failed")?;
        rows.push(pending.contribution);
        parent = pending.address;
        current = pending.right;
    }
    if rows.len() != count as usize {
        return Err("air power contribution map ended before declared count");
    }
    Ok(())
}
