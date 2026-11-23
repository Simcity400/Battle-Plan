use crate::{
    Memory,
    allocation,
    cequipment_type,
    cequipment_variant::pool as cequipment_variant_pool,
    corders_group,
    cpdx_array::Array,
    equipment_entry,
    read,
};
use engine_sites::Rva;

pub const HOLDER: usize = 0x188;
pub const TARGET: usize = 0x190;
const DEPLOYED: usize = 0x199;
const QUEUED: usize = 0x19a;
const GROUP_SPAN_BYTES: usize = QUEUED + 1 - HOLDER;
const SIZE: usize = 0x98;
const PERSISTENT: usize = 0x18;
const REQUIRED_ENTRIES: usize = 0x08;
pub const HOLDER_VTABLE_RVA: Rva = Rva(0x02a9_63c8);
pub const HOLDER_PERSISTENT_VTABLE_RVA: Rva = Rva(0x02a9_6400);
const REQUIRED_POOL_VTABLE_RVA: Rva = Rva(0x0280_7538);
const PRIORITY: usize = 0x08;
const OWNER: usize = 0x20;
const EQUIPMENT: usize = 0x28;
const REQUIRED: usize = 0x68;
const ASSEMBLED_MANPOWER: usize = 0x88;
const REQUESTED_MANPOWER: usize = 0x8c;
const ORDER: usize = 0x90;
pub const REQUISITIONED: usize = 0x94;
pub const TEMPLATE_REQUIREMENTS: usize = 0x120;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Deployment {
    pub country: u32,
    pub priority: i32,
    pub allow_zero: u8,
    pub assembled_manpower: i32,
    pub requested_manpower: i32,
    pub order: i32,
    pub requisitioned: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub holder: usize,
    pub target: u64,
    pub state: [u8; 2],
    pub deployment: Option<Deployment>,
    equipment: [u8; 16],
    required: [u8; 16],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Equipment {
    pub variant: u64,
    pub amount: i64,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Requirement {
    pub token: u32,
    pub amount: i64,
}
#[derive(Debug, Default)]
pub struct Pools {
    pub equipment: Vec<Equipment>,
    pub required: Vec<Requirement>,
}

pub fn header(
    memory: &impl Memory,
    base: usize,
    root: corders_group::Root,
) -> Result<Header, &'static str> {
    let bytes = root
        .pointer()
        .checked_add(HOLDER)
        .and_then(|at| read::<[u8; GROUP_SPAN_BYTES]>(memory, at))
        .ok_or("headquarters group context unreadable")?;
    let holder = usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
        .map_err(|_| "headquarters holder address overflow")?;
    let mut header = Header {
        holder,
        target: u64::from_le_bytes(
            bytes[TARGET - HOLDER..TARGET - HOLDER + 8]
                .try_into()
                .unwrap(),
        ),
        state: bytes[DEPLOYED - HOLDER..].try_into().unwrap(),
        ..Default::default()
    };
    if holder == 0 {
        return Ok(header);
    }
    let record =
        read::<[u8; SIZE]>(memory, holder).ok_or("headquarters deployment holder unreadable")?;
    let word = |offset| u64::from_le_bytes(record[offset..offset + 8].try_into().unwrap());
    let integer = |offset| i32::from_le_bytes(record[offset..offset + 4].try_into().unwrap());
    if engine_sites::at(base, HOLDER_VTABLE_RVA).map(|at| at as u64) != Some(word(0))
        || engine_sites::at(base, HOLDER_PERSISTENT_VTABLE_RVA).map(|at| at as u64)
            != Some(word(PERSISTENT))
    {
        return Err("headquarters deployment holder identity mismatch");
    }
    if integer(OWNER) as u32 != root.country {
        return Err("headquarters deployment owner mismatch");
    }
    if engine_sites::at(base, cequipment_variant_pool::VTABLE_RVA).map(|at| at as u64)
        != Some(word(EQUIPMENT))
        || engine_sites::at(base, REQUIRED_POOL_VTABLE_RVA).map(|at| at as u64)
            != Some(word(REQUIRED))
    {
        return Err("headquarters deployment pool identity mismatch");
    }
    header.deployment = Some(Deployment {
        country: integer(OWNER) as u32,
        priority: integer(PRIORITY),
        allow_zero: record[EQUIPMENT + cequipment_variant_pool::ALLOW_ZERO],
        assembled_manpower: integer(ASSEMBLED_MANPOWER),
        requested_manpower: integer(REQUESTED_MANPOWER),
        order: integer(ORDER),
        requisitioned: record[REQUISITIONED],
    });
    header.equipment.copy_from_slice(
        &record[EQUIPMENT + cequipment_variant_pool::ITEMS
            ..EQUIPMENT + cequipment_variant_pool::ITEMS + 16],
    );
    header
        .required
        .copy_from_slice(&record[REQUIRED + REQUIRED_ENTRIES..REQUIRED + REQUIRED_ENTRIES + 16]);
    Ok(header)
}

pub fn pools(
    memory: &impl Memory,
    base: usize,
    header: &Header,
    output: &mut Pools,
) -> Result<(), &'static str> {
    if header.deployment.is_none() {
        output.equipment.clear();
        output.required.clear();
        return Ok(());
    }
    let equipment = Array::from_header(header.equipment, equipment_entry::STRIDE)
        .ok_or("headquarters equipment array invalid")?;
    let required =
        Array::from_header(header.required, 16).ok_or("headquarters requirements array invalid")?;
    if equipment.count as usize > output.equipment.len() {
        let additional = equipment.count as usize - output.equipment.len();
        allocation::reserve(&mut output.equipment, additional)
            .map_err(|_| "headquarters equipment allocation failed")?;
    }
    output
        .equipment
        .resize(equipment.count as usize, Equipment::default());
    for (ordinal, row) in output.equipment.iter_mut().enumerate() {
        let at = equipment
            .element(ordinal as u32, equipment_entry::STRIDE)
            .ok_or("headquarters equipment address overflow")?;
        let entry = equipment_entry::read(memory, base, at, ordinal as u32)?;
        *row = Equipment {
            variant: entry.variant_handle,
            amount: entry.amount_raw,
        };
    }
    if required.count as usize > output.required.len() {
        let additional = required.count as usize - output.required.len();
        allocation::reserve(&mut output.required, additional)
            .map_err(|_| "headquarters requirement allocation failed")?;
    }
    output
        .required
        .resize(required.count as usize, Requirement::default());
    for (ordinal, row) in output.required.iter_mut().enumerate() {
        let at = required
            .element(ordinal as u32, 16)
            .ok_or("headquarters requirement address overflow")?;
        let entry = read::<[u8; 16]>(memory, at).ok_or("headquarters requirement unreadable")?;
        let definition = usize::try_from(u64::from_le_bytes(entry[..8].try_into().unwrap()))
            .map_err(|_| "headquarters requirement definition overflow")?;
        let identity = read::<[u8; 12]>(memory, definition)
            .ok_or("headquarters requirement definition unreadable")?;
        let vtable = u64::from_le_bytes(identity[..8].try_into().unwrap());
        if engine_sites::which(
            base,
            vtable as usize,
            &[
                cequipment_type::VTABLE_RVA,
                cequipment_type::NULL_VTABLE_RVA,
            ],
        )
        .is_none()
        {
            return Err("headquarters requirement definition identity mismatch");
        }
        *row = Requirement {
            token: u32::from_le_bytes(identity[8..].try_into().unwrap()),
            amount: i64::from_le_bytes(entry[8..].try_into().unwrap()),
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cequipment_variant, memory::SliceMemory};
    fn put(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture() -> (Vec<u8>, corders_group::Root) {
        let mut bytes = vec![0; 0x7000];
        let root = corders_group::Root {
            pointer: 0x1000,
            handle: (9 << 32) | 53,
            country: 7,
        };
        put(&mut bytes, 0x1000 + HOLDER, 0x2000);
        put(&mut bytes, 0x1000 + TARGET, (19 << 32) | 52);
        bytes[0x1199] = 2;
        bytes[0x119a] = 3;
        put(&mut bytes, 0x2000, HOLDER_VTABLE_RVA.0 as u64);
        put(&mut bytes, 0x2018, HOLDER_PERSISTENT_VTABLE_RVA.0 as u64);
        put(&mut bytes, 0x2000 + OWNER, 7);
        put(&mut bytes, 0x2000 + PRIORITY, 9);
        put(
            &mut bytes,
            0x2000 + EQUIPMENT,
            cequipment_variant_pool::VTABLE_RVA.0 as u64,
        );
        put(
            &mut bytes,
            0x2000 + REQUIRED,
            REQUIRED_POOL_VTABLE_RVA.0 as u64,
        );
        put(&mut bytes, 0x2048, 0x5000);
        put(&mut bytes, 0x2050, 2 | (2 << 32));
        put(&mut bytes, 0x2070, 0x5100);
        put(&mut bytes, 0x2078, 2 | (2 << 32));
        bytes[0x2060] = 4;
        put(
            &mut bytes,
            0x2088,
            (-7_i32 as u32 as u64) | ((19_u64) << 32),
        );
        put(&mut bytes, 0x2090, 31);
        bytes[0x2094] = 5;
        put(&mut bytes, 0x3000, cequipment_variant::VTABLE_RVA.0 as u64);
        put(&mut bytes, 0x3008, (u64::from(u32::MAX) << 32) | 70);
        put(&mut bytes, 0x4000, cequipment_type::VTABLE_RVA.0 as u64);
        put(&mut bytes, 0x4008, 41);
        for (at, amount) in [(0x5000, 0), (0x5010, i64::MIN)] {
            put(&mut bytes, at, 0x3000);
            put(&mut bytes, at + 8, amount as u64);
        }
        for (at, amount) in [(0x5100, i64::MAX), (0x5110, -19)] {
            put(&mut bytes, at, 0x4000);
            put(&mut bytes, at + 8, amount as u64);
        }
        (bytes, root)
    }
    #[test]
    fn template_hq_context_keeps_full_references_pool_order_native_flags_and_reuses_storage() {
        let (bytes, root) = fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let header = header(&memory, 0, root).unwrap();
        assert_eq!((header.target, header.state), ((19 << 32) | 52, [2, 3]));
        assert_eq!(
            header.deployment,
            Some(Deployment {
                country: 7,
                priority: 9,
                allow_zero: 4,
                assembled_manpower: -7,
                requested_manpower: 19,
                order: 31,
                requisitioned: 5
            })
        );
        let mut output = Pools::default();
        pools(&memory, 0, &header, &mut output).unwrap();
        assert_eq!(
            output.equipment,
            [
                Equipment {
                    variant: (u64::from(u32::MAX) << 32) | 70,
                    amount: 0
                },
                Equipment {
                    variant: (u64::from(u32::MAX) << 32) | 70,
                    amount: i64::MIN
                }
            ]
        );
        assert_eq!(
            output.required,
            [
                Requirement {
                    token: 41,
                    amount: i64::MAX
                },
                Requirement {
                    token: 41,
                    amount: -19
                }
            ]
        );
        let storage = (
            output.equipment.as_ptr(),
            output.equipment.capacity(),
            output.required.as_ptr(),
            output.required.capacity(),
        );
        pools(&memory, 0, &header, &mut output).unwrap();
        assert_eq!(
            storage,
            (
                output.equipment.as_ptr(),
                output.equipment.capacity(),
                output.required.as_ptr(),
                output.required.capacity()
            )
        );
    }
    #[test]
    fn template_hq_context_null_holder_keeps_target_and_invalid_pool_is_refused() {
        let (mut bytes, root) = fixture();
        put(&mut bytes, 0x2000 + EQUIPMENT, 0);
        assert_eq!(
            header(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                root
            ),
            Err("headquarters deployment pool identity mismatch")
        );
        put(&mut bytes, 0x1000 + HOLDER, 0);
        let read = header(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            root,
        )
        .unwrap();
        assert_eq!(read.target, (19 << 32) | 52);
        assert_eq!(read.deployment, None);
    }
}
