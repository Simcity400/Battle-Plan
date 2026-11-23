
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029f_17a0);
pub const LISTENER: usize = 0x10;
pub const LISTENER_VTABLE_RVA: Rva = Rva(0x029f_17f0);
pub const LISTENERS: usize = 0x18;
pub const PENDING_LISTENERS: usize = 0x30;
pub const DEFINITION_INDEX: usize = 0x08;
pub const STATUS: usize = 0x158;
pub const TEMPLATE: usize = 0x160;
pub const CLASSIFIER: usize = 0x170;
pub const LEVEL: usize = 0x174;
pub const LIVE_PROGRESS: usize = 0x198;
pub const AHEAD_REDUCTION_YEARS: usize = 0x1c0;
pub const APPLIED_SPEED_BONUS: usize = 0x1c8;
pub const APPLIED_BONUS_IDS: usize = 0x1d0;
pub const FIXED_SCALE: i64 = 100_000;
pub const CLASSIFIER_COMPLETED: u32 = 4;
pub const CLASSIFIER_ACTIVE: u32 = 5;

pub const CONSTRUCTOR_RVA: Rva = Rva(0x00f4_f5d0);
pub const BASE_COST_DEBUG_FLAG_RVA: Rva = Rva(0x033d_dad1);
pub const BASE_COST_SCALE_RVA: Rva = Rva(0x033e_41e0);
pub const BASE_YEAR_AHEAD_PENALTY_FACTOR_RVA: Rva = Rva(0x033e_4138);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub technology_handle: u32,
    pub database_index: i32,
    pub applied_bonus_count: i32,
    pub classifier: u32,
    pub level: i32,
    pub researched: u8,
    pub progress_raw: i64,
    pub ahead_reduction_years_raw: i64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Instance {
    pub row: Row,
    pub template: usize,
}

pub const BATCH: usize = 64;
pub const INSTANCE_BYTES: usize = 56;
pub(crate) const BATCH_BYTES: usize = BATCH * INSTANCE_BYTES;
pub const FIELDS: [engine_native_memory::memory::RecordField; 6] = [
    engine_native_memory::memory::RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 12,
    },
    engine_native_memory::memory::RecordField {
        source_offset: STATUS,
        destination_offset: 12,
        len: 16,
    },
    engine_native_memory::memory::RecordField {
        source_offset: CLASSIFIER,
        destination_offset: 28,
        len: 8,
    },
    engine_native_memory::memory::RecordField {
        source_offset: LIVE_PROGRESS,
        destination_offset: 36,
        len: 8,
    },
    engine_native_memory::memory::RecordField {
        source_offset: AHEAD_REDUCTION_YEARS,
        destination_offset: 44,
        len: 8,
    },
    engine_native_memory::memory::RecordField {
        source_offset: APPLIED_BONUS_IDS + 12,
        destination_offset: 52,
        len: 4,
    },
];

const LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&FIELDS, INSTANCE_BYTES).expect("valid record layout");

pub(crate) const IDENTITY_BYTES: usize = 28;
pub(crate) const IDENTITY_FIELDS: [engine_native_memory::memory::RecordField; 2] = [FIELDS[0], FIELDS[1]];
const IDENTITY_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&IDENTITY_FIELDS, IDENTITY_BYTES)
        .expect("valid identity layout");

pub(crate) struct Identity {
    pub handle: u32,
    pub template: usize,
}

#[inline]
pub fn hint(pointer: usize) {
    for field in &FIELDS {
        engine_native_memory::prefetch::span(pointer.wrapping_add(field.source_offset), 1);
    }
}

pub fn copy_batch(
    memory: &impl engine_native_memory::Memory,
    pointers: &[usize],
    scratch: &mut [u8; BATCH_BYTES],
) -> usize {
    if pointers.len() > BATCH {
        return 0;
    }
    let bytes = &mut scratch[..pointers.len() * INSTANCE_BYTES];
    memory.copy_records(
        engine_native_memory::memory::RecordSources::Pointers(pointers),
        &LAYOUT,
        bytes,
    )
}

pub(crate) fn copy_identities(
    memory: &impl engine_native_memory::Memory,
    pointers: &[usize],
    scratch: &mut [u8; BATCH_BYTES],
) -> usize {
    if pointers.len() > BATCH {
        return 0;
    }
    memory.copy_records(
        engine_native_memory::memory::RecordSources::Pointers(pointers),
        &IDENTITY_LAYOUT,
        &mut scratch[..pointers.len() * IDENTITY_BYTES],
    )
}

pub(crate) fn copy_identity_one(
    memory: &impl engine_native_memory::Memory,
    pointer: usize,
    out: &mut [u8; IDENTITY_BYTES],
) -> usize {
    memory.copy_records(
        engine_native_memory::memory::RecordSources::Pointers(&[pointer]),
        &IDENTITY_LAYOUT,
        out,
    )
}

pub fn decode_batch(
    base: usize,
    status: usize,
    country: u32,
    bytes: &[u8],
    mut visit: impl FnMut(Instance) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
    for bytes in bytes.chunks_exact(INSTANCE_BYTES) {
        visit(decode(expected, status, country, bytes)?)?;
    }
    Ok(())
}

pub fn copy_one(
    memory: &impl engine_native_memory::Memory,
    pointer: usize,
    out: &mut [u8; INSTANCE_BYTES],
) -> usize {
    memory.copy_records(
        engine_native_memory::memory::RecordSources::Pointers(&[pointer]),
        &LAYOUT,
        out,
    )
}

pub fn decode_one(
    base: usize,
    status: usize,
    country: u32,
    bytes: &[u8; INSTANCE_BYTES],
) -> Result<Instance, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
    decode(expected, status, country, bytes)
}

#[inline]
pub(crate) fn decode(
    expected: u64,
    status: usize,
    country: u32,
    bytes: &[u8],
) -> Result<Instance, &'static str> {
    let identity = decode_identity(expected, status, bytes)?;
    Ok(Instance {
        row: Row {
            country,
            technology_handle: identity.handle,
            database_index: 0,
            applied_bonus_count: i32::from_le_bytes(bytes[52..56].try_into().unwrap()),
            classifier: u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
            level: i32::from_le_bytes(bytes[32..36].try_into().unwrap()),
            researched: 0,
            progress_raw: i64::from_le_bytes(bytes[36..44].try_into().unwrap()),
            ahead_reduction_years_raw: i64::from_le_bytes(bytes[44..52].try_into().unwrap()),
        },
        template: identity.template,
    })
}

pub(crate) fn decode_identity(
    expected: u64,
    status: usize,
    bytes: &[u8],
) -> Result<Identity, &'static str> {
    if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != expected {
        return Err("technology vtable mismatch");
    }
    if u64::from_le_bytes(bytes[12..20].try_into().unwrap()) != status as u64 {
        return Err("technology status backlink mismatch");
    }
    let template = usize::try_from(u64::from_le_bytes(bytes[20..28].try_into().unwrap()))
        .map_err(|_| "technology template address overflow")?;
    if template == 0 {
        return Err("technology template is null");
    }
    Ok(Identity {
        handle: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        template,
    })
}
