//! Hourly land manpower and supply operands for every division in the census.
//! Fixed fields are owned DivisionSeed copies; only contributor entries remain
//! live here. Supply classification belongs to the future dashboard product.
use crate::{
    Memory, carmy, carmy_manpower_values, cpdx_array::Array, manpower_value_entry, walk::Outcome,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Contributor {
    pub country: u32,
    pub amount: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub division: u64,
    pub start: usize,
    pub len: usize,
    pub received_supply: i64,
    pub maximum_grace: i64,
    pub remaining_grace: i64,
    pub naval_transfer: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Walk {
    pub outcome: Outcome,
    pub contributors: usize,
    pub required_contributors: usize,
}

/// Caller-owned gather scratch, sized with the live division census off-thread.
pub struct Scratch<'a> {
    pub pointers: &'a mut [usize],
    pub ordinals: &'a mut [usize],
    pub first: &'a mut [[u8; 2 * manpower_value_entry::STRIDE]],
}

/// No allocation or reread after a fault. Whole contributor lists only; after
/// the first shortage publish a census prefix while counting exact demand.
pub fn capture(
    memory: &impl Memory,
    base: usize,
    sources: &[carmy::DivisionSeed],
    source: Outcome,
    rows: &mut [Row],
    contributors: &mut [Contributor],
    scratch: Scratch<'_>,
) -> Walk {
    let mut result = Walk {
        outcome: source,
        contributors: 0,
        required_contributors: 0,
    };
    if matches!(source, Outcome::Unavailable { .. }) {
        return result;
    }
    let mut run = || -> Result<(usize, usize, usize), &'static str> {
        if !matches!(source, Outcome::Rows { .. }) || source.len() > sources.len() {
            return Err("division land census incomplete");
        }
        let mut len = 0;
        let mut used = 0;
        let mut required = 0usize;
        let mut stopped = false;
        use crate::memory::{RecordLayout, RecordSources, packed_fields};
        const ENTRY: usize = manpower_value_entry::STRIDE;
        const PAIR: usize = 2 * ENTRY;
        // Typical arrays fit two entries. A single-entry array uses a shorter
        // layout, never reads past its count. Zero entries need no child read.
        const ONE: [crate::memory::RecordField; 1] = packed_fields([(0, ENTRY)]);
        const TWO: [crate::memory::RecordField; 1] = packed_fields([(0, PAIR)]);
        const ONE_LAYOUT: RecordLayout<'static> = RecordLayout::new(&ONE, PAIR).unwrap();
        const TWO_LAYOUT: RecordLayout<'static> = RecordLayout::new(&TWO, PAIR).unwrap();
        if scratch.pointers.len() < rows.len()
            || scratch.ordinals.len() < rows.len()
            || scratch.first.len() < rows.len()
        {
            return Err("division land gather scratch unavailable");
        }
        let pointers = &mut *scratch.pointers;
        let ordinals = &mut *scratch.ordinals;
        let first = scratch.first.as_flattened_mut();
        for seed in &sources[..source.len()] {
            if seed.primary_vtable != Some((base + carmy::VTABLE_RVA) as u64) {
                return Err("division land primary identity unavailable");
            }
            if seed.handle != Some(seed.division)
                || seed.counter_supply_status.is_none()
                || seed.naval_transfer_membership.is_none()
                || seed.supply_grace_maximum.is_none()
                || seed.supply_grace_remaining.is_none()
            {
                return Err("division land shared operands unavailable");
            }
            let header = seed
                .manpower_values
                .ok_or("division manpower header unavailable")?;
            if u64::from_le_bytes(header[..carmy_manpower_values::DATA].try_into().unwrap())
                != (base + carmy_manpower_values::VTABLE_RVA) as u64
            {
                return Err("division manpower vtable mismatch");
            }
            let array = Array::from_header(
                header
                    [carmy_manpower_values::DATA..carmy_manpower_values::COUNT + size_of::<i32>()]
                    .try_into()
                    .unwrap(),
                manpower_value_entry::STRIDE,
            )
            .ok_or("division manpower array invalid")?;
            required = required
                .checked_add(array.count as usize)
                .ok_or("division manpower demand overflow")?;
            if stopped || len == rows.len() || array.count as usize > contributors.len() - used {
                stopped = true;
                continue;
            }
            rows[len] = Row {
                division: seed.division,
                start: used,
                len: array.count as usize,
                received_supply: seed.counter_supply_status.unwrap(),
                maximum_grace: seed.supply_grace_maximum.unwrap(),
                remaining_grace: seed.supply_grace_remaining.unwrap(),
                naval_transfer: seed.naval_transfer_membership.unwrap() != 0,
            };
            used += array.count as usize;
            len += 1;
        }
        // One gather for all two-or-more-entry pointers, with a separate
        // short gather only when single-entry arrays exist. Above two,
        // only that division's remainder uses ordinary guarded copies.
        for (count, layout) in [(2, &TWO_LAYOUT), (1, &ONE_LAYOUT)] {
            let mut gathered = 0;
            for (n, row) in rows[..len].iter().enumerate() {
                if (count == 2 && row.len >= 2) || (count == 1 && row.len == 1) {
                    let header = sources[n].manpower_values.unwrap();
                    pointers[gathered] = u64::from_le_bytes(
                        header[carmy_manpower_values::DATA
                            ..carmy_manpower_values::DATA + size_of::<u64>()]
                            .try_into()
                            .unwrap(),
                    ) as usize;
                    ordinals[gathered] = n;
                    gathered += 1;
                }
            }
            if gathered == 0 {
                continue;
            }
            if memory.copy_records(
                RecordSources::Pointers(&pointers[..gathered]),
                layout,
                &mut first[..gathered * PAIR],
            ) != gathered
            {
                return Err("division manpower entries unreadable");
            }
            for (g, &n) in ordinals[..gathered].iter().enumerate() {
                for (i, bytes) in first[g * PAIR..g * PAIR + count * ENTRY]
                    .chunks_exact(ENTRY)
                    .enumerate()
                {
                    contributors[rows[n].start + i] = contributor(bytes);
                }
            }
        }
        for (seed, row) in sources.iter().zip(&rows[..len]) {
            if row.len > 2 {
                let header = seed.manpower_values.unwrap();
                let data = u64::from_le_bytes(
                    header[carmy_manpower_values::DATA
                        ..carmy_manpower_values::DATA + size_of::<u64>()]
                        .try_into()
                        .unwrap(),
                ) as usize;
                let address = data
                    .checked_add(PAIR)
                    .ok_or("division manpower address overflow")?;
                let entries = &mut contributors[row.start + 2..row.start + row.len];
                const _: () = {
                    assert!(size_of::<Contributor>() == ENTRY);
                    assert!(std::mem::offset_of!(Contributor, country) == 0);
                    assert!(
                        std::mem::offset_of!(Contributor, amount) == manpower_value_entry::COUNT
                    );
                };
                // C repr, no padding, all u32/i32 bits valid; native eight-byte
                // stride is proved at 140C5B350/140C5B610 beside ENTRY's source.
                let bytes = unsafe {
                    std::slice::from_raw_parts_mut(
                        entries.as_mut_ptr().cast::<u8>(),
                        std::mem::size_of_val(entries),
                    )
                };
                if !memory.copy(address, bytes) {
                    return Err("division manpower entries unreadable");
                }
            }
        }
        Ok((len, used, required))
    };
    match run() {
        Ok((len, used, required)) => {
            result.contributors = used;
            result.required_contributors = required;
            result.outcome = if len < source.len() {
                Outcome::Truncated {
                    len,
                    unread_rows: u32::try_from(source.len() - len).expect("native census fits u32"),
                }
            } else {
                Outcome::Rows { len }
            };
        }
        Err(reason) => result.outcome = Outcome::Unavailable { reason },
    }
    result
}
fn contributor(bytes: &[u8]) -> Contributor {
    Contributor {
        country: u32::from_le_bytes(
            bytes[manpower_value_entry::SOURCE_COUNTRY_LINK..manpower_value_entry::COUNT]
                .try_into()
                .unwrap(),
        ),
        amount: i32::from_le_bytes(bytes[manpower_value_entry::COUNT..].try_into().unwrap()),
    }
}
