use crate::{
    Memory, carmy, carmy_manpower_values,
    cpdx_array::Array,
    manpower_value_entry,
    walk::{Outcome, grow},
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
    pub naval_transfer: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Walk {
    pub outcome: Outcome,
    pub contributors: usize,
    pub required_contributors: usize,
}

pub struct Scratch<'a> {
    pub pointers: &'a mut [usize],
    pub ordinals: &'a mut [usize],
    pub first: &'a mut [[u8; 2 * manpower_value_entry::STRIDE]],
}

pub fn capture(
    memory: &impl Memory,
    base: usize,
    sources: &[carmy::DivisionSeed],
    source: Outcome,
    rows: &mut Vec<Row>,
    contributors: &mut Vec<Contributor>,
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
    let mut run = || -> Result<(usize, usize), &'static str> {
        if !matches!(source, Outcome::Rows { .. }) || source.len() > sources.len() {
            return Err("division land census incomplete");
        }
        grow(rows, source.len())?;
        let mut len = 0_usize;
        let mut used = 0_usize;
        use crate::memory::{RecordLayout, RecordSources, packed_fields};
        const ENTRY: usize = manpower_value_entry::STRIDE;
        const PAIR: usize = 2 * ENTRY;
        const ONE: [crate::memory::RecordField; 1] = packed_fields([(0, ENTRY)]);
        const TWO: [crate::memory::RecordField; 1] = packed_fields([(0, PAIR)]);
        const ONE_LAYOUT: RecordLayout<'static> = RecordLayout::new(&ONE, PAIR).unwrap();
        const TWO_LAYOUT: RecordLayout<'static> = RecordLayout::new(&TWO, PAIR).unwrap();
        let scratch_rows = source.len();
        if scratch.pointers.len() < scratch_rows
            || scratch.ordinals.len() < scratch_rows
            || scratch.first.len() < scratch_rows
        {
            return Err("division land gather scratch unavailable");
        }
        let pointers = &mut *scratch.pointers;
        let ordinals = &mut *scratch.ordinals;
        let first = scratch.first.as_flattened_mut();
        let army_vtable = engine_sites::at(base, carmy::VTABLE_RVA)
            .ok_or("division land primary identity unavailable")? as u64;
        let manpower_vtable = engine_sites::at(base, carmy_manpower_values::VTABLE_RVA)
            .ok_or("division manpower vtable mismatch")? as u64;
        for seed in &sources[..source.len()] {
            if seed.primary_vtable != Some(army_vtable) {
                return Err("division land primary identity unavailable");
            }
            if seed.handle != Some(seed.division) || seed.naval_transfer_membership.is_none() {
                return Err("division land shared operands unavailable");
            }
            let header = seed
                .manpower_values
                .ok_or("division manpower header unavailable")?;
            if u64::from_le_bytes(header[..carmy_manpower_values::DATA].try_into().unwrap())
                != manpower_vtable
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
            let next = used
                .checked_add(array.count as usize)
                .ok_or("division manpower demand overflow")?;
            grow(contributors, next)?;
            rows[len] = Row {
                division: seed.division,
                start: used,
                len: array.count as usize,
                naval_transfer: seed.naval_transfer_membership.unwrap() != 0,
            };
            used = next;
            len += 1;
        }
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
        Ok((len, used))
    };
    match run() {
        Ok((len, used)) => {
            result.contributors = used;
            result.required_contributors = used;
            result.outcome = Outcome::Rows { len };
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

#[cfg(test)]
mod sizing_tests {
    use super::*;
    use crate::memory::SliceMemory;

    fn seeds(count: usize, contributors: u32) -> Vec<carmy::DivisionSeed> {
        let mut header = [0; 24];
        header[..8].copy_from_slice(&(carmy_manpower_values::VTABLE_RVA.0 as u64).to_le_bytes());
        header[8..16].copy_from_slice(&0x1000u64.to_le_bytes());
        header[16..20].copy_from_slice(&contributors.to_le_bytes());
        header[20..24].copy_from_slice(&contributors.to_le_bytes());
        (0..count)
            .map(|ordinal| carmy::DivisionSeed {
                division: ((ordinal as u64 + 1) << 32) | 1,
                primary_vtable: Some(carmy::VTABLE_RVA.0 as u64),
                handle: Some(((ordinal as u64 + 1) << 32) | 1),
                counter_supply_status: None,
                supply_grace_maximum: None,
                supply_grace_remaining: None,
                naval_transfer_membership: Some(ordinal % 2),
                manpower_values: Some(header),
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn original_census_grows_land_rows_and_mod_sized_manpower_lists_before_publication() {
        let mut rows = Vec::new();
        let mut contributors = Vec::new();
        let mut pointers = vec![0; 5_000];
        let mut ordinals = vec![0; 5_000];
        let mut first = vec![[0; 2 * manpower_value_entry::STRIDE]; 5_000];
        for (count, entries) in [(1_500, 3), (3_000, 3), (5_000, 137)] {
            let sources = seeds(count, entries);
            let bytes: Vec<u8> = (0..entries)
                .flat_map(|ordinal| {
                    let mut entry = [0; 8];
                    entry[..4].copy_from_slice(&(ordinal + 1).to_le_bytes());
                    entry[4..].copy_from_slice(&(-(ordinal as i32)).to_le_bytes());
                    entry
                })
                .collect();
            let acquired = capture(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                0,
                &sources,
                Outcome::Rows { len: count },
                &mut rows,
                &mut contributors,
                Scratch {
                    pointers: &mut pointers,
                    ordinals: &mut ordinals,
                    first: &mut first,
                },
            );
            assert_eq!(acquired.outcome, Outcome::Rows { len: count });
            if count == 5_000 {
                assert!(rows.len() > count);
            }
            assert_eq!(acquired.contributors, count * entries as usize);
            for (ordinal, row) in rows[..count].iter().enumerate() {
                assert_eq!(row.division, sources[ordinal].division);
                assert_eq!(row.len, entries as usize);
                assert_eq!(row.naval_transfer, ordinal % 2 == 1);
                assert_eq!(
                    contributors[row.start + row.len - 1],
                    Contributor {
                        country: entries,
                        amount: -(entries as i32 - 1)
                    }
                );
            }
        }
    }

    #[test]
    fn original_land_fault_precedes_capacity_and_any_contributor_read() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("invalid original seed must stop before contributor reads")
            }
        }
        let mut sources = seeds(5_000, 137);
        sources[0].primary_vtable = None;
        let acquired = capture(
            &NoReads,
            0,
            &sources,
            Outcome::Rows { len: sources.len() },
            &mut Vec::new(),
            &mut Vec::new(),
            Scratch {
                pointers: &mut vec![0; sources.len()],
                ordinals: &mut vec![0; sources.len()],
                first: &mut vec![[0; 2 * manpower_value_entry::STRIDE]; sources.len()],
            },
        );
        assert_eq!(
            acquired.outcome,
            Outcome::Unavailable {
                reason: "division land primary identity unavailable"
            }
        );
    }
}
