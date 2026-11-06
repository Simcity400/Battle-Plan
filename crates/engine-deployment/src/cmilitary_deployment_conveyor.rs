//! Military deployment conveyor: one queued order and its lines.
//!
//! Its reference base holds the full handle. Construction initializes the template, the name, zero
//! amount and location, priority one, the line vector, the role and the closed flag.
//!
//! Serialization keeps the template handle, name, integer amount/location/
//! priority, lexer role token, boolean closed flag and every line. Role
//! token zero is retained as zero. Location meaning beyond raw u32 remains
//! unproven.

use crate::{
    Memory, cpdx_array::Array, creferenced_division_template, msvc_string, read, read_slice,
};

/// CMilitaryDeploymentConveyor primary vtable.
pub const VTABLE_RVA: usize = 0x29daa08;
/// CMilitaryDeploymentConveyor reference kind.
pub const REFERENCE_TYPE: u32 = 64;
/// CMilitaryDeploymentConveyor reference type word, u32.
pub const TYPE: usize = 0x08;
/// CMilitaryDeploymentConveyor reference id, u32; the high half of the same qword.
pub const ID: usize = 0x0c;
/// CMilitaryDeploymentConveyor CReferencedDivisionTemplate pointer.
pub const TEMPLATE: usize = 0x20;
/// CMilitaryDeploymentConveyor MSVC byte string.
pub const NAME: usize = 0x28;
/// CMilitaryDeploymentConveyor queued amount, i32.
pub const AMOUNT: usize = 0x48;
/// CMilitaryDeploymentConveyor raw location, u32.
pub const LOCATION: usize = 0x4c;
/// CMilitaryDeploymentConveyor raw priority, u32.
pub const PRIORITY: usize = 0x50;
/// Deployment-line pointer array with its count.
pub const LINES: usize = 0x78;
/// CMilitaryDeploymentConveyor line-pointer stride.
pub const LINE_STRIDE: usize = 0x08;
/// CMilitaryDeploymentConveyor CPdxLexer role token, u32.
pub const ROLE_TOKEN: usize = 0x90;
/// CMilitaryDeploymentConveyor closed flag, u8.
pub const CLOSED: usize = 0x94;
/// Gated raw conveyor country u32 reference, saved only when positive.
/// Capture retains the raw positive word and reports None otherwise.
pub const GATED_COUNTRY: usize = 0x98;
/// Deployment-conveyor constructor.
pub const CONSTRUCT_RVA: usize = 0x00d4_6e00;

/// One queue entry. `name` indexes the caller's arena; empty names are valid.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Full native handle: reference type in the low word, id in the high word.
    pub handle: u64,
    /// Raw country reference of the owning CDeploymentStatus.
    pub country: u32,
    /// Stored queue position; the engine compacts this vector, so order is state.
    pub ordinal: u32,
    /// Authenticated CReferencedDivisionTemplate handle, not a process pointer.
    pub template: u64,
    pub amount: i32,
    pub location: u32,
    pub priority: u32,
    /// Raw CPdxLexer token; zero is retained and never turned into text here.
    pub role_token: u32,
    /// Raw engine byte, retained without boolean normalization.
    pub closed: u8,
    pub name_start: usize,
    pub name_len: usize,
    /// Gated raw country reference; nonpositive words are absent and omitted from the save.
    pub exile_country: Option<u32>,
}

/// One conveyor read. `handle` and `lines` are available even when the row
/// itself is omitted, so a caller that cannot store this conveyor can still
/// traverse its lines and count exactly what it left out.
pub(crate) struct Read {
    /// Full native handle, needed by child rows whether or not the row is kept.
    pub handle: u64,
    /// `None` when the literal name does not fit the caller's arena; the row is
    /// never published with a shortened or borrowed name.
    pub row: Option<Row>,
    pub lines: Array,
    /// Arena bytes used after this conveyor, unchanged when the row is omitted.
    pub used: usize,
    /// The name's full length, reported whether or not it was copied.
    pub wanted: usize,
}

/// Authenticates one conveyor and copies its scalars and its literal name.
pub(crate) fn read_row_buffer(
    memory: &impl Memory,
    base: usize,
    country: u32,
    ordinal: u32,
    pointer: usize,
    names: &mut crate::walk::Buffer<'_, u8>,
    used: usize,
) -> Result<Read, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("deployment conveyor module address overflow")?;
    let head =
        read::<[u8; 0x9c]>(memory, pointer).ok_or("deployment conveyor fields unreadable")?;
    let quad = |offset: usize| u64::from_le_bytes(head[offset..offset + 8].try_into().unwrap());
    let word = |offset: usize| u32::from_le_bytes(head[offset..offset + 4].try_into().unwrap());
    if quad(0) != expected as u64 {
        return Err("deployment conveyor vtable mismatch");
    }
    let handle = quad(TYPE);
    if handle as u32 != REFERENCE_TYPE {
        return Err("deployment conveyor reference type mismatch");
    }
    let template = usize::try_from(quad(TEMPLATE))
        .ok()
        .filter(|pointer| *pointer != 0)
        .ok_or("deployment conveyor template pointer unreadable or null")?;
    let template = creferenced_division_template::identity(memory, base, template)?;
    let lines = pointer
        .checked_add(LINES)
        .and_then(|address| Array::read(memory, address, LINE_STRIDE))
        .ok_or("deployment conveyor line array invalid")?;
    let name = pointer
        .checked_add(NAME)
        .ok_or("deployment conveyor address overflow")?;
    let (source, name_len) = literal(memory, name)?;
    names.ensure(
        used.checked_add(name_len)
            .ok_or("deployment name byte count overflow")?,
    )?;
    if name_len > names.len().saturating_sub(used) {
        return Ok(Read {
            handle,
            row: None,
            lines,
            used,
            wanted: name_len,
        });
    }
    if !read_slice(memory, source, &mut names[used..used + name_len]) {
        return Err("deployment conveyor name bytes unreadable");
    }
    Ok(Read {
        handle,
        row: Some(Row {
            handle,
            country,
            ordinal,
            template,
            amount: word(AMOUNT) as i32,
            location: word(LOCATION),
            priority: word(PRIORITY),
            role_token: word(ROLE_TOKEN),
            closed: head[CLOSED],
            name_start: used,
            name_len,
            exile_country: {
                let raw = word(GATED_COUNTRY);
                if raw == 0 { None } else { Some(raw) }
            },
        }),
        lines,
        used: used + name_len,
        wanted: name_len,
    })
}

fn literal(memory: &impl Memory, string: usize) -> Result<(usize, usize), &'static str> {
    let at = |offset: usize| {
        string
            .checked_add(offset)
            .ok_or("deployment conveyor name address overflow")
    };
    let len = read::<u64>(memory, at(msvc_string::LENGTH)?)
        .ok_or("deployment conveyor name length unreadable")?;
    let capacity = read::<u64>(memory, at(msvc_string::CAPACITY)?)
        .ok_or("deployment conveyor name capacity unreadable")?;
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("deployment conveyor name header invalid");
    }
    let len = usize::try_from(len).map_err(|_| "deployment conveyor name length overflow")?;
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        string
    } else {
        read::<u64>(memory, string)
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("deployment conveyor name pointer unreadable or null")?
    };
    if read::<u8>(
        memory,
        source
            .checked_add(len)
            .ok_or("deployment conveyor name address overflow")?,
    ) != Some(0)
    {
        return Err("deployment conveyor name terminator unreadable or invalid");
    }
    Ok((source, len))
}
