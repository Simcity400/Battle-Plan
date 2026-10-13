//! Active research-slot operands. The speed multiplier is separate from
//! technology progress.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0295_e328;
/// Technology definition handle, u32, matched against its token identity.
pub const TECHNOLOGY_HANDLE: usize = 0x08;
/// Owning technology-status pointer authenticated by backlink.
pub const STATUS: usize = 0x10;
/// Technology-definition pointer.
pub const TECHNOLOGY: usize = 0x18;
/// Banked idle days, signed i64 fixed5, increased by one fixed5 day per idle day.
pub const BANKED_DAYS: usize = 0x20;
/// Applied saved days, signed i64 fixed5, moved from the bank on assignment.
pub const APPLIED_DAYS: usize = 0x28;
/// Research-speed multiplier, signed i64 fixed5, initialized to one.
pub const RESEARCH_SPEED: usize = 0x38;
/// Signed i32 ordinal, validated against the positive array count.
pub const INDEX: usize = 0x40;
/// Empty lexer token stored when no technology is assigned; `fixed_layout`
/// checks its spelling and static range.
pub const IDLE_HANDLE: u32 = 0x2a4e;
/// Multiplier scale used when resetting and dividing the saved-days product.
pub const RESEARCH_SPEED_SCALE: i64 = 100_000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    /// Nonnegative native i32 ordinal; all bits preserved and checked against the array count.
    pub index: u32,
    pub technology_handle: u32,
    /// Exact selected technology template occurrence; zero when idle.
    pub database_index: i32,
    /// Native technology pointer presence; the pointer itself is not portable identity.
    pub technology_present: u8,
    pub research_speed_raw: i64,
    pub banked_days_raw: i64,
    pub applied_days_raw: i64,
}

pub(crate) fn row(
    memory: &impl crate::Memory,
    base: usize,
    pointer: usize,
    status: usize,
    country: u32,
) -> Result<Row, &'static str> {
    let mut raw = Raw::default();
    let completed = copy_raw(memory, pointer, &mut raw)?;
    if completed != 4 {
        return Err("research slot fields unreadable");
    }
    let technology_u64 = u64::from_le_bytes(raw.owner[8..].try_into().unwrap());
    let linked = if technology_u64 == 0 {
        None
    } else {
        let technology =
            usize::try_from(technology_u64).map_err(|_| "research technology address overflow")?;
        let mut bytes = [0; crate::ctechnology::INSTANCE_BYTES];
        if crate::ctechnology::copy_one(memory, technology, &mut bytes)
            != crate::ctechnology::FIELDS.len()
        {
            return Err("technology fields unreadable");
        }
        Some(bytes)
    };
    let mut row = decode_raw(base, status, country, &raw, linked.as_ref())?;
    if let Some(bytes) = &linked {
        let instance = crate::ctechnology::decode_one(base, status, country, bytes)?;
        let identity = crate::read::<[u8; 8]>(
            memory,
            instance
                .template
                .checked_add(crate::ctechnology_template::DATABASE_INDEX)
                .ok_or("technology template address overflow")?,
        )
        .ok_or("research slot template identity unreadable")?;
        row.database_index = i32::from_le_bytes(identity[..4].try_into().unwrap());
        if row.database_index <= 0
            || u32::from_le_bytes(identity[4..].try_into().unwrap()) != row.technology_handle
        {
            return Err("research slot template identity mismatch");
        }
    }
    Ok(row)
}

// Research serialization, updating and template selection use the same four
// spans as copy_raw, one record per slot, with no padding or new field
// acquisition.
pub(crate) const FIELDS: [crate::memory::RecordField; 4] = crate::memory::packed_fields([
    (0, 12),
    (STATUS, 16),
    (RESEARCH_SPEED, 12),
    (BANKED_DAYS, 16),
]);
pub(crate) const LAYOUT: crate::memory::RecordLayout<'static> =
    crate::memory::RecordLayout::new(&FIELDS, 56).expect("valid slot record layout");

/// Game-thread raw copy with no validation beyond address bounds.
/// Returns completed scattered fields; failed field may be partial.
pub fn copy_raw(
    memory: &impl crate::Memory,
    pointer: usize,
    out: &mut Raw,
) -> Result<usize, &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("research slot address overflow")
    };
    let mut header: &mut [u8] = &mut out.header;
    let mut owner: &mut [u8] = &mut out.owner;
    let mut speed_index: &mut [u8] = &mut out.speed_index;
    let mut days: &mut [u8] = &mut out.days;
    Ok(memory.copy_scattered(
        [pointer, at(STATUS)?, at(RESEARCH_SPEED)?, at(BANKED_DAYS)?],
        &mut [&mut header, &mut owner, &mut speed_index, &mut days],
    ))
}

/// Decode-side interpretation of already-copied slot bytes with identical faults.
pub fn decode_raw(
    base: usize,
    status: usize,
    country: u32,
    raw: &Raw,
    linked: Option<&[u8; crate::ctechnology::INSTANCE_BYTES]>,
) -> Result<Row, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    if u64::from_le_bytes(raw.header[..8].try_into().unwrap()) != expected {
        return Err("research slot vtable mismatch");
    }
    if u64::from_le_bytes(raw.owner[..8].try_into().unwrap()) != status as u64 {
        return Err("research slot status backlink mismatch");
    }
    let technology_handle = u32::from_le_bytes(raw.header[8..].try_into().unwrap());
    let technology = usize::try_from(u64::from_le_bytes(raw.owner[8..].try_into().unwrap()))
        .map_err(|_| "research technology address overflow")?;
    if technology == 0 {
        if technology_handle != IDLE_HANDLE {
            return Err("idle research slot handle mismatch");
        }
    } else {
        let Some(bytes) = linked else {
            return Err("technology fields unreadable");
        };
        let instance = crate::ctechnology::decode_one(base, status, country, bytes)?;
        if instance.row.technology_handle != technology_handle {
            return Err("research slot technology handle mismatch");
        }
    }
    Ok(Row {
        country,
        index: u32::from_le_bytes(raw.speed_index[8..].try_into().unwrap()),
        technology_handle,
        database_index: 0,
        technology_present: u8::from(technology != 0),
        research_speed_raw: i64::from_le_bytes(raw.speed_index[..8].try_into().unwrap()),
        banked_days_raw: i64::from_le_bytes(raw.days[..8].try_into().unwrap()),
        applied_days_raw: i64::from_le_bytes(raw.days[8..].try_into().unwrap()),
    })
}

/// Flat raw spans for one slot plus its linked-technology presence.
/// Game thread fills header/owner/speed/days via guarded copies only.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Raw {
    pub header: [u8; 12],
    pub owner: [u8; 16],
    pub speed_index: [u8; 12],
    pub days: [u8; 16],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_slot_requires_the_shared_empty_handle() {
        let mut raw = Raw::default();
        raw.header[..8].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
        raw.header[8..].copy_from_slice(&IDLE_HANDLE.to_le_bytes());
        let idle = decode_raw(0, 0, 7, &raw, None).unwrap();
        assert_eq!(idle.technology_handle, IDLE_HANDLE);
        assert_eq!(idle.technology_present, 0);
        for handle in [0, IDLE_HANDLE - 1, IDLE_HANDLE + 1] {
            raw.header[8..].copy_from_slice(&handle.to_le_bytes());
            assert_eq!(
                decode_raw(0, 0, 7, &raw, None),
                Err("idle research slot handle mismatch")
            );
        }
    }
}
