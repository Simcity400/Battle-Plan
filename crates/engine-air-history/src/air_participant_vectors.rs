
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
};
use engine_native::ccountry;

pub const RECIPROCAL_DONORS: usize = 0x1410;

pub use game_model::air_participants::{DONOR, ALLY, ENEMY};

pub const BATCH: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub kind: u8,
    pub ordinal: u32,
    pub other: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Staged {
    pub slot: u32,
    pub country: usize,
    pub diplomacy: usize,
    pub counts: [u32; 3],
    pub sources: [usize; 3],
    pub offset: usize,
}

impl Staged {
    pub fn bytes(&self) -> usize {
        self.counts.iter().map(|&count| count as usize * 4).sum()
    }
}

const fn field(source_offset: usize, destination_offset: usize, len: usize) -> RecordField {
    RecordField {
        source_offset,
        destination_offset,
        len,
    }
}

const POINTER: [RecordField; 1] = [field(ccountry::DIPLOMACY_STATUS, 0, 8)];
const POINTER_LAYOUT: RecordLayout<'static> = RecordLayout::new(&POINTER, 8).expect("valid layout");
const DONORS: [RecordField; 1] = [field(RECIPROCAL_DONORS, 0, 16)];
const DONORS_LAYOUT: RecordLayout<'static> = RecordLayout::new(&DONORS, 16).expect("valid layout");
const STATUS: [RecordField; 3] = [
    field(0, 0, 8),
    field(engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES, 8, 16),
    field(engine_native_parts::cdiplomacy_status_layout::WARTIME_ALLIES, 24, 16),
];
const STATUS_LAYOUT: RecordLayout<'static> = RecordLayout::new(&STATUS, 40).expect("valid layout");

fn gather(
    memory: &impl Memory,
    pointers: &[usize],
    layout: &RecordLayout<'_>,
    fields: usize,
    output: &mut [u8],
) -> Result<(), &'static str> {
    if memory.copy_records(RecordSources::Pointers(pointers), layout, output)
        != pointers.len() * fields
    {
        return Err("air participant country or diplomacy unreadable");
    }
    Ok(())
}

pub fn diplomacy_pointers(
    memory: &impl Memory,
    countries: &[usize],
    output: &mut [usize],
) -> Result<(), &'static str> {
    let mut bytes = [0_u8; BATCH * 8];
    let bytes = &mut bytes[..countries.len() * 8];
    gather(memory, countries, &POINTER_LAYOUT, 1, bytes)?;
    for (pointer, bytes) in output.iter_mut().zip(bytes.chunks_exact(8)) {
        *pointer = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
            .ok()
            .filter(|&pointer| pointer != 0)
            .ok_or("air participant diplomacy pointer null")?;
    }
    Ok(())
}

pub fn headers(
    memory: &impl Memory,
    base: usize,
    links: u32,
    batch: &[(u32, usize, usize)],
    mut emit: impl FnMut(Staged),
) -> Result<(), &'static str> {
    let vtable = engine_sites::at(base, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA)
        .ok_or("air participant module address overflow")? as u64;
    let len = batch.len();
    let mut pointers = [0_usize; BATCH];
    let mut donors = [0_u8; BATCH * 16];
    let mut status = [0_u8; BATCH * 40];
    for (pointer, &(_, country, _)) in pointers.iter_mut().zip(batch) {
        *pointer = country;
    }
    gather(
        memory,
        &pointers[..len],
        &DONORS_LAYOUT,
        1,
        &mut donors[..len * 16],
    )?;
    for (pointer, &(_, _, diplomacy)) in pointers.iter_mut().zip(batch) {
        *pointer = diplomacy;
    }
    gather(
        memory,
        &pointers[..len],
        &STATUS_LAYOUT,
        3,
        &mut status[..len * 40],
    )?;
    let header = |bytes: &[u8]| {
        let vector = Array::from_header(bytes.try_into().expect("16-byte header"), 4)
            .ok_or("air participant vector header invalid")?;
        if vector.count > links {
            return Err("air participant vector longer than the country links");
        }
        let first = if vector.count == 0 {
            0
        } else {
            vector
                .element(0, 4)
                .ok_or("air participant vector address overflow")?
        };
        Ok::<_, &'static str>((first, vector.count))
    };
    for (index, &(slot, country, diplomacy)) in batch.iter().enumerate() {
        let status = &status[index * 40..index * 40 + 40];
        if u64::from_le_bytes(status[..8].try_into().unwrap()) != vtable {
            return Err("air participant diplomacy status mismatched");
        }
        let vectors = [
            header(&donors[index * 16..index * 16 + 16])?,
            header(&status[24..40])?,
            header(&status[8..24])?,
        ];
        emit(Staged {
            slot,
            country,
            diplomacy,
            counts: vectors.map(|(_, count)| count),
            sources: vectors.map(|(first, _)| first),
            offset: 0,
        });
    }
    Ok(())
}

pub fn copy_entries(
    memory: &impl Memory,
    staged: &[Staged],
    bytes: &mut [u8],
) -> Result<(), &'static str> {
    let mut addresses = [0_usize; BATCH];
    let mut spans = [(0_usize, 0_usize); BATCH];
    let mut pending = 0;
    let mut flush = |addresses: &mut [usize; BATCH],
                     spans: &mut [(usize, usize); BATCH],
                     count: usize| {
        let mut rest = &mut *bytes;
        let mut consumed = 0;
        let mut destinations: [&mut [u8]; BATCH] = std::array::from_fn(|_| <&mut [u8]>::default());
        for (destination, &(offset, len)) in destinations.iter_mut().zip(spans.iter()).take(count) {
            let (_, tail) = std::mem::take(&mut rest).split_at_mut(offset - consumed);
            let (head, tail) = tail.split_at_mut(len);
            *destination = head;
            rest = tail;
            consumed = offset + len;
        }
        let copied = memory.copy_scattered(*addresses, &mut destinations);
        *addresses = [0; BATCH];
        *spans = [(0, 0); BATCH];
        if copied != BATCH {
            return Err("air participant entry unreadable");
        }
        Ok(())
    };
    for staged in staged {
        let mut offset = staged.offset;
        for (&count, &source) in staged.counts.iter().zip(&staged.sources) {
            if count == 0 {
                continue;
            }
            addresses[pending] = source;
            spans[pending] = (offset, count as usize * 4);
            offset += count as usize * 4;
            pending += 1;
            if pending == BATCH {
                flush(&mut addresses, &mut spans, pending)?;
                pending = 0;
            }
        }
    }
    if pending != 0 {
        flush(&mut addresses, &mut spans, pending)?;
    }
    Ok(())
}

pub fn rows(staged: &Staged, entries: &[u32], mut emit: impl FnMut(Row)) {
    let mut entries = entries.iter();
    for (kind, &count) in [DONOR, ALLY, ENEMY].into_iter().zip(&staged.counts) {
        for (ordinal, &other) in (0..count).zip(entries.by_ref()) {
            emit(Row {
                country: staged.slot,
                kind,
                ordinal,
                other,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const BASE: usize = 0x1_4000_0000;
    const A: usize = 0x4000;
    const B: usize = 0x7000;
    const A_DIP: usize = 0xd000;
    const B_DIP: usize = 0xe000;
    const DATA: usize = 0x18000;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }
    fn header(bytes: &mut [u8], at: usize, data: u64, count: u32) {
        put(bytes, at, &data.to_le_bytes());
        put(bytes, at + 8, &count.to_le_bytes());
        put(bytes, at + 12, &count.to_le_bytes());
    }
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x19000];
        for (country, diplomacy) in [(A, A_DIP), (B, B_DIP)] {
            put(
                &mut bytes,
                country + ccountry::DIPLOMACY_STATUS,
                &(diplomacy as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                diplomacy,
                &((BASE + engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
        }
        header(&mut bytes, A + RECIPROCAL_DONORS, DATA as u64, 2);
        header(
            &mut bytes,
            A_DIP + engine_native_parts::cdiplomacy_status_layout::WARTIME_ALLIES,
            DATA as u64 + 8,
            1,
        );
        header(
            &mut bytes,
            B_DIP + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES,
            DATA as u64 + 12,
            2,
        );
        for (at, value) in [(0, 2_u32), (4, 2), (8, 2), (12, 4), (16, 1)] {
            put(&mut bytes, DATA + at, &value.to_le_bytes());
        }
        bytes
    }

    fn staged(bytes: &[u8], out: &mut [u8]) -> Result<Vec<Staged>, &'static str> {
        let memory = SliceMemory { base: 0, bytes };
        let mut diplomacy = [0; 2];
        diplomacy_pointers(&memory, &[A, B], &mut diplomacy)?;
        let batch = [(1, A, diplomacy[0]), (2, B, diplomacy[1])];
        let mut staged = Vec::new();
        let mut used = 0;
        headers(&memory, BASE, 8, &batch, |mut s| {
            s.offset = used;
            used += s.bytes();
            staged.push(s);
        })?;
        copy_entries(&memory, &staged, out)?;
        Ok(staged)
    }

    #[test]
    fn vectors_keep_native_order_repeats_and_raw_aliases() {
        let mut out = [0_u8; 20];
        let staged = staged(&fixture(), &mut out).unwrap();
        assert_eq!(
            staged
                .iter()
                .map(|s| (s.diplomacy, s.counts, s.offset))
                .collect::<Vec<_>>(),
            [(A_DIP, [2, 1, 0], 0), (B_DIP, [0, 0, 2], 12)]
        );
        let entries: Vec<u32> = out
            .chunks_exact(4)
            .map(|entry| u32::from_le_bytes(entry.try_into().unwrap()))
            .collect();
        let mut out_rows = Vec::new();
        for s in &staged {
            rows(
                s,
                &entries[s.offset / 4..(s.offset + s.bytes()) / 4],
                |row| out_rows.push((row.country, row.kind, row.ordinal, row.other)),
            );
        }
        assert_eq!(
            out_rows,
            [
                (1, DONOR, 0, 2),
                (1, DONOR, 1, 2),
                (1, ALLY, 0, 2),
                (2, ENEMY, 0, 4),
                (2, ENEMY, 1, 1),
            ]
        );
    }

    #[test]
    fn a_vector_longer_than_the_country_links_is_refused() {
        let mut bytes = fixture();
        header(
            &mut bytes,
            B_DIP + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES,
            DATA as u64,
            9,
        );
        assert_eq!(
            staged(&bytes, &mut [0; 20]).unwrap_err(),
            "air participant vector longer than the country links"
        );
    }

    #[test]
    fn a_foreign_diplomacy_status_is_refused() {
        let mut bytes = fixture();
        put(&mut bytes, B_DIP, &0_u64.to_le_bytes());
        assert_eq!(
            staged(&bytes, &mut [0; 20]).unwrap_err(),
            "air participant diplomacy status mismatched"
        );
    }
}
