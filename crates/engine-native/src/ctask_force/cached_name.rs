//! Retained task-force name returned without generation or mutation.
//! Serialization follows the secondary unit base to the same string. Capture
//! owns all bytes; save writing and localisation apply their own C-string rules.

use super::{SECONDARY_BASE, SECONDARY_VTABLE_RVA, VTABLE_RVA};
use crate::{
    Memory,
    cname_group_member::{Names, Span, StringErrors, copy_string},
    cunit, msvc_string, read,
};

pub(super) const NAME: usize = 0x258;
const ERRORS: StringErrors = [
    "task-force cached name header invalid",
    "task-force cached name pointer unreadable or null",
    "task-force cached name bytes unreadable",
    "task-force cached name terminator unreadable or invalid",
];


/// Copy the exact cached bytes after rechecking both vtables and the full
/// handle supplied by the authenticated fleet walk. No native getter runs.
/// The returned span starts at `names.len`; commit it only with its admitted
/// row. Healthy empty text is Some((start, 0)); insufficient arena is None and
/// retains the validated demand for checked growth outside acquisition.
pub fn cached_name(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    id: u64,
    names: &mut Names<'_>,
) -> Result<Option<Span>, &'static str> {
    let primary = engine_sites::at(base, VTABLE_RVA)
        .ok_or("task-force cached name module address overflow")?;
    let secondary = engine_sites::at(base, SECONDARY_VTABLE_RVA)
        .ok_or("task-force cached name module address overflow")?;
    let header = read::<[u8; 32]>(memory, pointer)
        .filter(|_| pointer != 0)
        .ok_or("task-force cached name identity unreadable")?;
    let word = |offset| u64::from_le_bytes(header[offset..offset + 8].try_into().unwrap());
    if word(0) != primary as u64
        || word(SECONDARY_BASE) != secondary as u64
        || id == 0
        || word(cunit::HANDLE) != id
    {
        return Err("task-force cached name identity changed or invalid");
    }
    let address = pointer
        .checked_add(NAME)
        .ok_or("task-force cached name address overflow")?;
    let header = read::<[u8; msvc_string::STRIDE]>(memory, address)
        .ok_or("task-force cached name header unreadable")?;
    copy_string(memory, &header, names, &ERRORS)
}

/// Authenticate the same header and boundary probes with an empty arena. No
/// heap text payload is copied; only proven byte demand sizes the live arena.
pub fn cached_name_demand(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    id: u64,
) -> Result<usize, &'static str> {
    let mut names = Names {
        bytes: crate::walk::Buffer::Count,
        len: 0,
        required: 0,
    };
    cached_name(memory, base, pointer, id, &mut names)?;
    Ok(names.required)
}

#[cfg(test)]
mod tests;
