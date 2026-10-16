use super::{ATTACKER, DEFENDER, INDICATOR_FRAME_HIGH, Reader, Source};
use crate::{Memory, cland_combatant};

/// The four native ordered country-tag sets of one combat, as spans of the
/// caller's tag arena: attacker primary and secondary, then defender primary and
/// secondary. They are simulation state, unlike the side's display byte +0xBA.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CountrySetsRow {
    pub combat: u64,
    /// CLandCombat +0x98, the marker's frame and the window's indicator icon.
    pub indicator: u8,
    /// Attacker then defender CLandCombatant +0x08 flag words.
    pub flags: [u32; 2],
    pub start: usize,
    pub lens: [usize; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountrySetError {
    Unavailable(&'static str),
    /// The arena is too small; `required` tags are needed from its start.
    Truncated {
        required: usize,
    },
}

impl From<&'static str> for CountrySetError {
    fn from(reason: &'static str) -> Self {
        Self::Unavailable(reason)
    }
}

/// Read primary (+0x38) and secondary (+0x50) country sets in stored order,
/// keeping raw tags and aliases. Primary sets select shared vision and
/// secondary sets the viewer; defender markers check both for shared vision.
/// Insertion keeps at most one node per country plus tag zero, so `max_len`
/// is the live country count plus one. Check capacity before following nodes.
///
/// Read marker operands independently of other window context: combat +0x98
/// indicator byte and side +0x08 flag words. Each side flag and both set
/// headers at +0x08..+0x64 use one read.
pub fn read_country_sets(
    memory: &impl Memory,
    base: usize,
    source: Source,
    max_len: usize,
    tags: &mut [u32],
) -> Result<(CountrySetsRow, usize), CountrySetError> {
    read_country_sets_buffer(
        memory,
        base,
        source,
        max_len,
        crate::walk::Buffer::Fixed(tags),
    )
}

/// The four original list counts reserve the complete worker arena before nodes.
pub fn read_country_sets_buffer(
    memory: &impl Memory,
    base: usize,
    source: Source,
    max_len: usize,
    mut tags: crate::walk::Buffer<'_, u32>,
) -> Result<(CountrySetsRow, usize), CountrySetError> {
    let reader = Reader {
        memory,
        base,
        provinces: source.provinces,
    };
    let border = reader
        .class(source.pointer)?
        .ok_or("land country-set source is naval")?;
    if reader.header(source.pointer, border)? != (source.id, source.attackers, source.defenders) {
        return Err("land country-set source changed".into());
    }
    let sides = [
        reader.pointer(source.pointer, ATTACKER)?,
        reader.pointer(source.pointer, DEFENDER)?,
    ];
    const SPAN: usize = cland_combatant::SECONDARY_COUNTRIES + 20 - cland_combatant::FLAGS;
    let spans = [
        reader.field::<[u8; SPAN]>(sides[0], cland_combatant::FLAGS)?,
        reader.field::<[u8; SPAN]>(sides[1], cland_combatant::FLAGS)?,
    ];
    let mut row = CountrySetsRow {
        combat: source.id,
        indicator: reader.field(source.pointer, INDICATOR_FRAME_HIGH)?,
        flags: spans.map(|span| u32::from_le_bytes(span[..4].try_into().unwrap())),
        ..CountrySetsRow::default()
    };
    let mut headers = [(0_usize, 0_usize, 0_usize); 4];
    for (index, header) in headers.iter_mut().enumerate() {
        let offset = [
            cland_combatant::COUNTRIES,
            cland_combatant::SECONDARY_COUNTRIES,
        ][index % 2]
            - cland_combatant::FLAGS;
        // Head and tail node pointers, then the signed i32 count at +0x10.
        let bytes = &spans[index / 2][offset..offset + 20];
        let head = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
        let tail = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
        let count = i32::from_le_bytes(bytes[16..].try_into().unwrap());
        let count = usize::try_from(count)
            .ok()
            .filter(|&count| count <= max_len)
            .ok_or("land country-set count exceeds the country table")?;
        if (count == 0) != (head == 0) || (count == 0) != (tail == 0) {
            return Err("land country-set header links inconsistent".into());
        }
        *header = (head, tail, count);
    }
    let required: usize = headers.iter().map(|header| header.2).sum();
    if !tags.ensure(required)? {
        return Err(CountrySetError::Truncated { required });
    }
    let mut used = 0;
    for (index, (mut node, tail, count)) in headers.into_iter().enumerate() {
        let mut previous = 0;
        for ordinal in 0..count {
            // Node: tag u32, then previous and next node pointers at +8/+0x10.
            let bytes = reader.field::<[u8; 24]>(node, 0)?;
            let tag = u32::from_le_bytes(bytes[..4].try_into().unwrap());
            let prev = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
            let next = u64::from_le_bytes(bytes[16..].try_into().unwrap()) as usize;
            let last = ordinal + 1 == count;
            if prev != previous || (last && (node != tail || next != 0)) || (!last && next == 0) {
                return Err("land country-set node links inconsistent".into());
            }
            // Insertion keeps sets distinct and secondary tags positive. Reading
            // checks linked-list integrity without adding stricter value constraints.
            tags[used] = tag;
            used += 1;
            previous = node;
            node = next;
        }
        row.lens[index] = count;
    }
    Ok((row, used))
}
