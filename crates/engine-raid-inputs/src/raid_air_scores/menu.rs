use super::{BATCH, Memory, RecordField, RecordLayout, field};
use engine_native::{
    allocation, memory::gather_records, msvc_tree_node::Cursor, read as native_read,
};
use std::sync::{Arc, OnceLock};

const COVERAGE: usize = 0x10;
const REDUCTIONS: usize = 0x30;
const FIELDS: [RecordField; 2] = [field(COVERAGE, 0, 8), field(REDUCTIONS, 8, 64)];
const LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&FIELDS, 72).expect("valid air power menu layout");
const SENTINEL_FIELDS: [RecordField; 1] = [field(0, 0, 26)];
const SENTINEL_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SENTINEL_FIELDS, 26).expect("valid air power map sentinel layout");

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contribution {
    pub country: u32,
    pub amount: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Menu {
    pub coverage: i64,
    /// Enemy AA, friendly AA, enemy missiles, friendly missiles.
    pub reductions: [i64; 4],
    pub contributions: [Vec<Contribution>; 2],
}

impl Menu {
    pub(super) fn is_zero(&self) -> bool {
        self.coverage == 0
            && self.reductions == [0; 4]
            && self.contributions.iter().all(Vec::is_empty)
    }

    pub fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self
                .contributions
                .iter()
                .map(|rows| rows.capacity() * size_of::<Contribution>())
                .sum::<usize>()
    }
}

pub(super) fn read(memory: &impl Memory, addresses: &[usize]) -> [Option<Arc<Menu>>; BATCH] {
    let mut bytes = [0_u8; BATCH * 72];
    let mut complete = [false; BATCH];
    gather_records(
        memory,
        addresses,
        &LAYOUT,
        &mut bytes[..addresses.len() * 72],
        &mut complete[..addresses.len()],
    );
    let mut heads = [0_usize; BATCH * 2];
    for index in 0..addresses.len() {
        if !complete[index] {
            continue;
        }
        for side in 0..2 {
            let at = index * 72 + 40 + side * 16;
            if let Ok((head, _)) = Cursor::header(bytes[at..at + 16].try_into().unwrap()) {
                heads[index * 2 + side] = head;
            }
        }
    }
    let mut sentinels = [0_u8; BATCH * 2 * 26];
    let mut valid = [false; BATCH * 2];
    gather_records(
        memory,
        &heads[..addresses.len() * 2],
        &SENTINEL_LAYOUT,
        &mut sentinels[..addresses.len() * 2 * 26],
        &mut valid[..addresses.len() * 2],
    );
    std::array::from_fn(|index| {
        if index >= addresses.len()
            || !complete[index]
            || !valid[index * 2]
            || !valid[index * 2 + 1]
        {
            return None;
        }
        decode(
            memory,
            &bytes[index * 72..(index + 1) * 72],
            &sentinels[index * 52..(index + 1) * 52],
        )
        .ok()
        .map(|menu| {
            if menu.is_zero() {
                static EMPTY: OnceLock<Arc<Menu>> = OnceLock::new();
                Arc::clone(EMPTY.get_or_init(|| Arc::new(Menu::default())))
            } else {
                Arc::new(menu)
            }
        })
    })
}

fn decode(memory: &impl Memory, bytes: &[u8], sentinels: &[u8]) -> Result<Menu, &'static str> {
    let value = |at| i64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    Ok(Menu {
        coverage: value(0),
        reductions: std::array::from_fn(|index| value(8 + index * 8)),
        contributions: [
            contributions(
                memory,
                bytes[40..56].try_into().unwrap(),
                sentinels[..26].try_into().unwrap(),
            )?,
            contributions(
                memory,
                bytes[56..72].try_into().unwrap(),
                sentinels[26..].try_into().unwrap(),
            )?,
        ],
    })
}

fn contributions(
    memory: &impl Memory,
    header: [u8; 16],
    sentinel: [u8; 26],
) -> Result<Vec<Contribution>, &'static str> {
    let mut cursor = Cursor::from_parts(header, sentinel)
        .map_err(|_| "air power contribution map sentinel invalid")?;
    let mut rows = Vec::new();
    while let Some(node) = cursor
        .next(memory)
        .map_err(|_| "air power contribution map traversal failed")?
    {
        let at = node
            .checked_add(0x20)
            .ok_or("air power contribution address overflow")?;
        let bytes =
            native_read::<[u8; 16]>(memory, at).ok_or("air power contribution unreadable")?;
        allocation::reserve_push_batch(&mut rows, 1)
            .map_err(|_| "air power contribution allocation failed")?;
        rows.push(Contribution {
            country: u32::from_le_bytes(bytes[..4].try_into().unwrap()),
            amount: i64::from_le_bytes(bytes[8..].try_into().unwrap()),
        });
    }
    Ok(rows)
}
