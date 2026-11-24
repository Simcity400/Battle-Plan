use super::{BATCH, FULL_ENTRY, Memory, RecordField, RecordLayout, field};
pub use game_model::air_power::{Contribution, Menu};
use engine_allocation as allocation;
use engine_native_memory::{
    memory::gather_records,
    msvc_tree_node as node,
    read as native_read,
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

pub(super) struct BatchBuffers {
    pub(super) addresses: [usize; BATCH],
    pub(super) full: [u8; BATCH * FULL_ENTRY],
    pub(super) scales: [Option<i64>; BATCH],
    pub(super) menu_ok: [bool; BATCH],
    heads: [usize; BATCH * 2],
    gathered: [usize; BATCH * 2],
    sentinels: [u8; BATCH * 2 * 26],
    valid: [bool; BATCH * 2],
    pub(super) active: [usize; BATCH],
}

impl Default for BatchBuffers {
    fn default() -> Self {
        Self {
            addresses: [0; BATCH],
            full: [0; BATCH * FULL_ENTRY],
            scales: [None; BATCH],
            menu_ok: [false; BATCH],
            heads: [0; BATCH * 2],
            gathered: [usize::MAX; BATCH * 2],
            sentinels: [0; BATCH * 2 * 26],
            valid: [false; BATCH * 2],
            active: [0; BATCH],
        }
    }
}

#[derive(Default)]
pub struct Scratch {
    contributions: [Vec<Contribution>; 2],
    stack: Vec<Pending>,
    buffers: Option<Box<BatchBuffers>>,
    reads: Vec<Read>,
}

impl Scratch {
    pub(super) fn allocated_bytes(&self) -> usize {
        self.contributions
            .iter()
            .map(|rows| rows.capacity() * size_of::<Contribution>())
            .sum::<usize>()
            + self.stack.capacity() * size_of::<Pending>()
            + self
                .buffers
                .as_ref()
                .map_or(0, |_| size_of::<BatchBuffers>())
            + self.reads.capacity() * size_of::<Read>()
    }

    pub(super) fn buffers(&mut self) -> &mut BatchBuffers {
        self.buffers
            .get_or_insert_with(|| Box::new(BatchBuffers::default()))
    }

    pub(super) fn take_reads(&mut self) -> Vec<Read> {
        let mut reads = std::mem::take(&mut self.reads);
        reads.clear();
        reads
    }

    pub(super) fn return_reads(&mut self, mut reads: Vec<Read>) {
        reads.clear();
        self.reads = reads;
    }
}

pub(super) fn read_selected_entries(
    memory: &impl Memory,
    stride: usize,
    active_len: usize,
    all_menus: bool,
    held: &[Option<&Arc<Menu>>],
    scratch: &mut Scratch,
    out: &mut Vec<Read>,
) {
    out.clear();
    if active_len == 0 {
        return;
    }
    let Scratch {
        contributions,
        stack,
        buffers,
        ..
    } = scratch;
    let buffers = buffers.get_or_insert_with(|| Box::new(BatchBuffers::default()));
    let mut nonempty = 0;
    for pos in 0..active_len {
        let index = buffers.active[pos];
        buffers.gathered[pos * 2] = usize::MAX;
        buffers.gathered[pos * 2 + 1] = usize::MAX;
        if !all_menus && !buffers.menu_ok[index] {
            continue;
        }
        let base = index * stride + MAP_HEADERS;
        for side in 0..2 {
            let at = base + side * 16;
            if let Ok((head, map_count)) =
                node::Cursor::header(buffers.full[at..at + 16].try_into().unwrap())
                && map_count != 0
            {
                buffers.gathered[pos * 2 + side] = nonempty;
                buffers.heads[nonempty] = head;
                nonempty += 1;
            }
        }
    }
    if nonempty != 0 {
        buffers.valid[..nonempty].fill(false);
        gather_records(
            memory,
            &buffers.heads[..nonempty],
            &SENTINEL_LAYOUT,
            &mut buffers.sentinels[..nonempty * 26],
            &mut buffers.valid[..nonempty],
        );
    }
    out.reserve(active_len);
    for pos in 0..active_len {
        let index = buffers.active[pos];
        if !all_menus && !buffers.menu_ok[index] {
            out.push(Read::Unavailable);
            continue;
        }
        let sentinel = |map: usize| -> Result<Option<[u8; 26]>, ()> {
            match buffers.gathered[map] {
                usize::MAX => Ok(None),
                at if buffers.valid[at] => Ok(Some(
                    buffers.sentinels[at * 26..(at + 1) * 26].try_into().unwrap(),
                )),
                _ => Err(()),
            }
        };
        let (Ok(first), Ok(second)) = (sentinel(pos * 2), sentinel(pos * 2 + 1)) else {
            out.push(Read::Unavailable);
            continue;
        };
        let entry = &buffers.full[index * stride..(index + 1) * stride];
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
        let read = decode_parts(
            memory,
            coverage,
            reductions,
            headers,
            [first, second],
            held.get(pos).copied().flatten(),
            contributions,
            stack,
        )
        .unwrap_or(Read::Unavailable);
        out.push(read);
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "borrows split scratch buffers alongside decoded entry header parts"
)]
fn decode_parts(
    memory: &impl Memory,
    coverage: i64,
    reductions: [i64; 4],
    headers: [[u8; 16]; 2],
    sentinels: [Option<[u8; 26]>; 2],
    held: Option<&Arc<Menu>>,
    contributions: &mut [Vec<Contribution>; 2],
    stack: &mut Vec<Pending>,
) -> Result<Read, &'static str> {
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
