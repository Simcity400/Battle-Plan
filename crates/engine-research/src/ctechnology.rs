//! Technology instance layout and fixed-point research operands.
//! The template is a separate object with its own definition id.
//! These layouts do not establish hourly coherence.

/// CTechnology vtable.
pub const VTABLE_RVA: usize = 0x029f_1660;
/// The technology's listener subobject, whose vtable [`LISTENER_VTABLE_RVA`] maps slot 2 to
/// observer propagation on the owning technology (the object enclosing it).
pub const LISTENER: usize = 0x10;
/// The technology listener vtable at [`LISTENER`], the technology's second
/// vtable.
pub const LISTENER_VTABLE_RVA: usize = 0x029f_16b0;
/// The listeners classifier propagation notifies after a classifier change (pointer array with its
/// count): dependency-list rebuilding registers each technology's [`LISTENER`] with the
/// technologies of its template's three dependency lists, so a listener need not be a dependent
/// (from the template's dependent list).
pub const LISTENERS: usize = 0x18;
/// The listener container dependency-list rebuilding appends to instead of
/// [`LISTENERS`] while it is set (pointer, null otherwise).
pub const PENDING_LISTENERS: usize = 0x30;
/// CTechnology u32 technology handle; Serialization copies the template's handle, distinct from its
/// database index.
pub const DEFINITION_INDEX: usize = 0x08;
/// CTechnology owner CTechnologyStatus*.
pub const STATUS: usize = 0x158;
/// CTechnology template pointer.
pub const TEMPLATE: usize = 0x160;
/// Technology classifier, u32.
pub const CLASSIFIER: usize = 0x170;
/// Signed i32 researched level, incremented on completion.
pub const LEVEL: usize = 0x174;
/// CTechnology live progress i64 fixed5.
pub const LIVE_PROGRESS: usize = 0x198;
/// Ahead-of-time reduction, i64 fixed5 years.
pub const AHEAD_REDUCTION_YEARS: usize = 0x1c0;
/// Applied limited-use speed bonus, fixed5; selected as a maximum.
pub const APPLIED_SPEED_BONUS: usize = 0x1c8;
/// Applied signed bonus IDs; switching research can refund them.
pub const APPLIED_BONUS_IDS: usize = 0x1d0;
/// CTechnology fixed5 progress/reduction scale.
pub const FIXED_SCALE: i64 = 100_000;
/// CTechnology completed classifier enum.
pub const CLASSIFIER_COMPLETED: u32 = 4;
/// CTechnology active classifier enum.
pub const CLASSIFIER_ACTIVE: u32 = 5;

/// CTechnology constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00f4_f560;
/// Technology base-cost debug byte.
pub const BASE_COST_DEBUG_FLAG_RVA: usize = 0x033d_dad1;
/// Technology base-cost scale i64 fixed5.
pub const BASE_COST_SCALE_RVA: usize = 0x033e_41e0;
/// Live ahead-of-time penalty factor, i64 fixed5.
pub const BASE_YEAR_AHEAD_PENALTY_FACTOR_RVA: usize = 0x033e_4138;

/// Portable current state; classifier values and fixed-point operands are not narrowed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub technology_handle: u32,
    /// Positive template database index, including native holes.
    pub database_index: i32,
    /// An instance is active when its stored active word is nonzero.
    pub applied_bonus_count: i32,
    pub classifier: u32,
    pub level: i32,
    /// Membership read from the technology status, independently of classifier.
    pub researched: u8,
    pub progress_raw: i64,
    pub ahead_reduction_years_raw: i64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Instance {
    pub row: Row,
    pub template: usize,
}

/// Fixed stack scratch; one guarded scatter acquires this many instances.
pub const BATCH: usize = 64;
pub const INSTANCE_BYTES: usize = 56;
pub(crate) const BATCH_BYTES: usize = BATCH * INSTANCE_BYTES;
pub const FIELDS: [crate::memory::RecordField; 6] = [
    crate::memory::RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 12,
    },
    crate::memory::RecordField {
        source_offset: STATUS,
        destination_offset: 12,
        len: 16,
    },
    crate::memory::RecordField {
        source_offset: CLASSIFIER,
        destination_offset: 28,
        len: 8,
    },
    crate::memory::RecordField {
        source_offset: LIVE_PROGRESS,
        destination_offset: 36,
        len: 8,
    },
    crate::memory::RecordField {
        source_offset: AHEAD_REDUCTION_YEARS,
        destination_offset: 44,
        len: 8,
    },
    crate::memory::RecordField {
        source_offset: APPLIED_BONUS_IDS + 12,
        destination_offset: 52,
        len: 4,
    },
];

const LAYOUT: crate::memory::RecordLayout<'static> =
    crate::memory::RecordLayout::new(&FIELDS, INSTANCE_BYTES).expect("valid record layout");

pub(crate) const IDENTITY_BYTES: usize = 28;
pub(crate) const IDENTITY_FIELDS: [crate::memory::RecordField; 2] = [FIELDS[0], FIELDS[1]];
const IDENTITY_LAYOUT: crate::memory::RecordLayout<'static> =
    crate::memory::RecordLayout::new(&IDENTITY_FIELDS, IDENTITY_BYTES)
        .expect("valid identity layout");

pub(crate) struct Identity {
    pub handle: u32,
    pub template: usize,
}

/// Hints each field's first cache line across the whole decoded batch before
/// its gather, retaining the batch's lookahead without hinting both endpoints
/// here. The native gather also hints any crossing end line as its rolling
/// lookahead advances. Hints observe no value and cannot fault, so a null or
/// corrupt roster pointer stays inert here and still fails in the guarded copy
/// exactly as before.
#[inline]
pub fn hint(pointer: usize) {
    for field in &FIELDS {
        crate::prefetch::span(pointer.wrapping_add(field.source_offset), 1);
    }
}

/// Game-thread raw copy only: guarded field gather with no decode or auth.
/// Returns completed fields in row-major order; failed field may be partial.
pub fn copy_batch(
    memory: &impl crate::Memory,
    pointers: &[usize],
    scratch: &mut [u8; BATCH_BYTES],
) -> usize {
    if pointers.len() > BATCH {
        return 0;
    }
    let bytes = &mut scratch[..pointers.len() * INSTANCE_BYTES];
    memory.copy_records(
        crate::memory::RecordSources::Pointers(pointers),
        &LAYOUT,
        bytes,
    )
}

/// Copy the identity fields needed to authenticate an unindexed researched
/// pointer. The caller owns the pointer order; no state payload is acquired.
pub(crate) fn copy_identities(
    memory: &impl crate::Memory,
    pointers: &[usize],
    scratch: &mut [u8; BATCH_BYTES],
) -> usize {
    if pointers.len() > BATCH {
        return 0;
    }
    memory.copy_records(
        crate::memory::RecordSources::Pointers(pointers),
        &IDENTITY_LAYOUT,
        &mut scratch[..pointers.len() * IDENTITY_BYTES],
    )
}

/// Decode-side only: interpret already-copied raw blobs with identical faults.
pub fn decode_batch(
    base: usize,
    status: usize,
    country: u32,
    bytes: &[u8],
    mut visit: impl FnMut(Instance) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    for bytes in bytes.chunks_exact(INSTANCE_BYTES) {
        // Decode directly into the caller's final row: no intermediate row
        // array, source padding copy, or per-record scatter metadata is needed.
        visit(decode(expected, status, country, bytes)?)?;
    }
    Ok(())
}

/// Game-thread raw copy of one instance's fixed spans with no interpretation.
pub fn copy_one(
    memory: &impl crate::Memory,
    pointer: usize,
    out: &mut [u8; INSTANCE_BYTES],
) -> usize {
    memory.copy_records(
        crate::memory::RecordSources::Pointers(&[pointer]),
        &LAYOUT,
        out,
    )
}

/// Decode-side interpretation of one already-copied instance blob.
pub fn decode_one(
    base: usize,
    status: usize,
    country: u32,
    bytes: &[u8; INSTANCE_BYTES],
) -> Result<Instance, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
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
