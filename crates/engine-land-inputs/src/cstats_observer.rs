
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029d_a0d0);
pub const LEADERS: usize = 0x20;
pub const LEADER_COUNT: usize = 0x2c;
pub const DAMAGE: usize = 0x38;
pub const MANPOWER_LOSS: usize = 0x60;
pub const EQUIPMENT_LOSSES: usize = 0x70;

use engine_native_memory::memory::{RecordField, RecordLayout, RecordSources};
use engine_native_memory::{Memory, cpdx_array::Array};
use engine_native::cequipment_variant::{self, pool as cequipment_variant_pool};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct LossEntry {
    pub variant: u64,
    pub amount_raw: i64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LossSpan {
    pub manpower: i32,
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct LossPlan {
    pub manpower: i32,
    pub equipment: Array,
}

const LOSS_LAYOUT: RecordLayout<'static> = RecordLayout::new(
    &[
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        },
        RecordField {
            source_offset: MANPOWER_LOSS,
            destination_offset: 8,
            len: 4,
        },
        RecordField {
            source_offset: EQUIPMENT_LOSSES,
            destination_offset: 12,
            len: 8,
        },
        RecordField {
            source_offset: EQUIPMENT_LOSSES + cequipment_variant_pool::ITEMS,
            destination_offset: 20,
            len: 16,
        },
    ],
    36,
)
.unwrap();

impl LossPlan {
    pub fn read(memory: &impl Memory, base: usize, side: usize) -> Result<Self, &'static str> {
        if side == 0 {
            return Err("land loss side null");
        }
        let observer = side
            .checked_add(engine_native::cland_combatant::STATS_OBSERVER)
            .ok_or("land loss observer address overflow")?;
        let mut header = [0; 36];
        if memory.copy_records(
            RecordSources::Pointers(&[observer]),
            &LOSS_LAYOUT,
            &mut header,
        ) != 4
        {
            return Err("land loss observer header unreadable");
        }
        Self::from_header(base, header)
    }

    fn from_header(base: usize, header: [u8; 36]) -> Result<Self, &'static str> {
        if u64::from_le_bytes(header[..8].try_into().unwrap())
            != engine_sites::at(base, VTABLE_RVA)
                .ok_or("land loss observer vtable address overflow")? as u64
        {
            return Err("land loss observer vtable mismatch");
        }
        if u64::from_le_bytes(header[12..20].try_into().unwrap())
            != engine_sites::at(base, cequipment_variant_pool::VTABLE_RVA)
                .ok_or("land loss pool vtable address overflow")? as u64
        {
            return Err("land loss pool vtable mismatch");
        }
        let manpower = i32::from_le_bytes(header[8..12].try_into().unwrap());
        let equipment = Array::from_header(
            header[20..].try_into().unwrap(),
            engine_native_parts::equipment_entry::STRIDE,
        )
        .ok_or("land loss pool array invalid")?;
        Ok(Self {
            manpower,
            equipment,
        })
    }

    pub fn read_sides(
        memory: &impl Memory,
        base: usize,
        sides: [usize; 2],
    ) -> Result<[Self; 2], &'static str> {
        let mut observers = [0; 2];
        for (at, side) in observers.iter_mut().zip(sides) {
            if side == 0 {
                return Err("land loss side null");
            }
            *at = side
                .checked_add(engine_native::cland_combatant::STATS_OBSERVER)
                .ok_or("land loss observer address overflow")?;
        }
        let mut headers = [[0; 36]; 2];
        if memory.copy_records(
            RecordSources::Pointers(&observers),
            &LOSS_LAYOUT,
            headers.as_flattened_mut(),
        ) != 8
        {
            return Err("land loss observer headers unreadable");
        }
        Ok([
            Self::from_header(base, headers[0])?,
            Self::from_header(base, headers[1])?,
        ])
    }

    pub fn copy_sides(
        plans: [Self; 2],
        memory: &impl Memory,
        base: usize,
        entries: &mut [LossEntry],
        pointers: &mut [usize],
        identities: &mut [[u8; 16]],
    ) -> Result<(), &'static str> {
        let first = plans[0].equipment.count as usize;
        let count = first
            .checked_add(plans[1].equipment.count as usize)
            .ok_or("land loss count overflow")?;
        if count > entries.len() || count > pointers.len() || count > identities.len() {
            return Err("land loss arena capacity exceeded");
        }
        if count == 0 {
            return Ok(());
        }
        let (attacker, defender) = entries[..count].split_at_mut(first);
        let mut destinations = [entry_bytes(attacker)?, entry_bytes(defender)?];
        if memory.copy_scattered(plans.map(|p| p.equipment.data), &mut destinations) != 2 {
            return Err("land loss equipment entries unreadable");
        }
        authenticate(memory, base, &mut entries[..count], pointers, identities)
    }

    pub fn words(self) -> Result<usize, &'static str> {
        (self.equipment.count as usize)
            .checked_mul(2)
            .and_then(|v| v.checked_add(2))
            .ok_or("land loss event words overflow")
    }
    pub fn stream(
        self,
        memory: &impl Memory,
        base: usize,
        put: &mut impl FnMut(u64),
    ) -> Result<(), &'static str> {
        put(self.manpower as u64);
        put(u64::from(self.equipment.count));
        for ordinal in 0..self.equipment.count {
            let at = self
                .equipment
                .element(ordinal, engine_native_parts::equipment_entry::STRIDE)
                .ok_or("land loss equipment address overflow")?;
            let entry = engine_native_parts::equipment_entry::read(memory, base, at, ordinal)?;
            put(entry.variant_handle);
            put(entry.amount_raw as u64);
        }
        Ok(())
    }
}

fn entry_bytes(entries: &mut [LossEntry]) -> Result<&mut [u8], &'static str> {
    const {
        assert!(std::mem::size_of::<LossEntry>() == engine_native_parts::equipment_entry::STRIDE);
    }
    let bytes = entries
        .len()
        .checked_mul(engine_native_parts::equipment_entry::STRIDE)
        .ok_or("land loss entry bytes overflow")?;
    Ok(unsafe { std::slice::from_raw_parts_mut(entries.as_mut_ptr().cast::<u8>(), bytes) })
}
fn authenticate(
    memory: &impl Memory,
    base: usize,
    entries: &mut [LossEntry],
    pointers: &mut [usize],
    identities: &mut [[u8; 16]],
) -> Result<(), &'static str> {
    let count = entries.len();
    for (pointer, entry) in pointers[..count].iter_mut().zip(&*entries) {
        *pointer =
            usize::try_from(entry.variant).map_err(|_| "land loss variant pointer overflow")?;
    }
    const IDENTITY: RecordLayout<'static> = RecordLayout::new(
        &[RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 16,
        }],
        16,
    )
    .unwrap();
    if memory.copy_records(
        RecordSources::Pointers(&pointers[..count]),
        &IDENTITY,
        identities[..count].as_flattened_mut(),
    ) != count
    {
        return Err("land loss variant identities unreadable");
    }
    let expected = engine_sites::at(base, engine_native_parts::equipment_variant_identity::VTABLE_RVA)
        .ok_or("land loss variant vtable address overflow")?;
    for (entry, identity) in entries.iter_mut().zip(&identities[..count]) {
        entry.variant = engine_native_parts::equipment_variant_identity::identity_from_header(expected, *identity)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    fn put(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    #[test]
    fn land_losses_preserve_signed_widths_complete_variants_and_refuse_short_copy() {
        const SIDE: usize = 0x100;
        let observer = SIDE + engine_native::cland_combatant::STATS_OBSERVER;
        let pool = observer + EQUIPMENT_LOSSES;
        let mut bytes = vec![0; 0x1000];
        put(&mut bytes, observer, VTABLE_RVA.0 as u64);
        bytes[observer + MANPOWER_LOSS..observer + MANPOWER_LOSS + 4]
            .copy_from_slice(&i32::MIN.to_le_bytes());
        put(
            &mut bytes,
            pool,
            cequipment_variant_pool::VTABLE_RVA.0 as u64,
        );
        put(&mut bytes, pool + cequipment_variant_pool::ITEMS, 0x800);
        put(
            &mut bytes,
            pool + cequipment_variant_pool::CAPACITY,
            2 | (2 << 32),
        );
        for (n, amount) in [i64::MIN, i64::MAX].into_iter().enumerate() {
            put(&mut bytes, 0x800 + n * 16, (0x900 + n * 16) as u64);
            put(&mut bytes, 0x808 + n * 16, amount as u64);
            put(
                &mut bytes,
                0x900 + n * 16,
                engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64,
            );
            put(&mut bytes, 0x908 + n * 16, (n as u64 + 1) << 32 | 70);
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let plan = LossPlan::read(&memory, 0, SIDE).unwrap();
        let plans = [
            plan,
            LossPlan {
                manpower: 0,
                equipment: Array::default(),
            },
        ];
        assert_eq!(plan.manpower, i32::MIN);
        let mut entries = [LossEntry::default(); 2];
        let mut pointers = [0; 2];
        let mut identities = [[0; 16]; 2];
        LossPlan::copy_sides(
            plans,
            &memory,
            0,
            &mut entries,
            &mut pointers,
            &mut identities,
        )
        .unwrap();
        assert_eq!(
            entries[0],
            LossEntry {
                variant: 1 << 32 | 70,
                amount_raw: i64::MIN
            }
        );
        assert_eq!(entries[1].amount_raw, i64::MAX);
        let mut streamed = Vec::new();
        plan.stream(&memory, 0, &mut |word| streamed.push(word))
            .unwrap();
        assert_eq!(
            streamed,
            [
                i32::MIN as u64,
                2,
                entries[0].variant,
                i64::MIN as u64,
                entries[1].variant,
                i64::MAX as u64
            ]
        );
        assert!(
            LossPlan::copy_sides(
                plans,
                &memory,
                0,
                &mut entries[..1],
                &mut pointers,
                &mut identities
            )
            .is_err()
        );
        let short = SliceMemory {
            base: 0,
            bytes: &bytes[..0x91f],
        };
        assert!(
            LossPlan::copy_sides(
                plans,
                &short,
                0,
                &mut entries,
                &mut pointers,
                &mut identities
            )
            .is_err()
        );
        assert!(LossPlan::read(&memory, 0, 0).is_err());
        put(
            &mut bytes,
            pool + cequipment_variant_pool::CAPACITY,
            u64::MAX,
        );
        assert!(
            LossPlan::read(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                SIDE
            )
            .is_err()
        );
    }
}
