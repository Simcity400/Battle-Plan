//! Physical CGameState country-array occurrences for alias `checks_all`.
//!
//! Resolve@1407141F0 walks +0x310 in physical order through count +0x31C,
//! considers a pointer's own positive raw ID (+8), and retains the first
//! passing candidate (or the first tied score). Canonical Countries rows do
//! not preserve this order or repeated pointer occurrences. Reserved slot
//! zero follows the same pointer/vtable/ID-zero identity as reserved naming.

use crate::{Memory, ccountry, cgame_state, cpdx_array::Array, read};

pub const CAPACITY: &str = "country-array roster arena capacity exceeded";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    /// Only Ok proves every physical occurrence was authenticated and copied.
    pub len: Result<usize, &'static str>,
    pub required: usize,
}

fn header(memory: &impl Memory, state: usize) -> Result<(usize, [u8; 16], Array), &'static str> {
    let address = state
        .checked_add(cgame_state::COUNTRIES)
        .filter(|_| state != 0)
        .ok_or("country-array game state null or overflowed")?;
    let bytes = read::<[u8; 16]>(memory, address).ok_or("country-array header unreadable")?;
    let array = Array::from_header(bytes, 8).ok_or("country-array header invalid")?;
    Ok((address, bytes, array))
}

fn unchanged(memory: &impl Memory, address: usize, expected: [u8; 16]) -> Result<(), &'static str> {
    if read::<[u8; 16]>(memory, address) == Some(expected) {
        Ok(())
    } else {
        Err("country-array header changed or unreadable")
    }
}

/// Header-only capacity hint outside the callback; no country payload is
/// acquired and a complete hint never implies a usable recorded roster.
pub fn demand(memory: &impl Memory, _base: usize, state: usize) -> Result<usize, &'static str> {
    let (address, bytes, array) = header(memory, state)?;
    unchanged(memory, address, bytes)?;
    Ok(array.count as usize)
}

/// No allocation or native calls. Short storage preserves full known demand
/// without following any pointer. A native failure withholds the whole field;
/// the caller must commit the scalar arena only on Ok. Aliases are not filtered.
pub fn capture(memory: &impl Memory, base: usize, state: usize, output: &mut [u32]) -> Capture {
    capture_buffer(memory, base, state, crate::walk::Buffer::Fixed(output))
}

pub fn capture_growing(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<u32>,
) -> Capture {
    capture_buffer(memory, base, state, crate::walk::Buffer::Growing(output))
}

fn capture_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: crate::walk::Buffer<'_, u32>,
) -> Capture {
    let mut required = 0;
    let len = (|| {
        let (address, bytes, array) = header(memory, state)?;
        required = array.count as usize;
        output.ensure(required)?;
        if required > output.len() {
            return Err(CAPACITY);
        }
        let mut reserved = None;
        if array.count != 0 {
            let pointer = array
                .element(0, 8)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|pointer| *pointer != 0)
                .ok_or("reserved country roster pointer unreadable or null")?;
            let root = memory
                .authenticated_reserved_country(base, state)
                .unwrap_or_else(|| ccountry::reserved_from_array(memory, base, array))?;
            if root.pointer != pointer {
                return Err("reserved country roster differs from authenticated root");
            }
            output[0] = 0;
            reserved = Some(pointer);
        }
        if array.count > 1 {
            let authenticated = memory
                .authenticated_countries(base, state)
                .filter(|roster| roster.arrays()[0] == array && roster.slots().len() == required);
            let mut reader = None;
            for slot in 1..array.count {
                let pointer = authenticated
                    .and_then(|roster| roster.slots().get(slot as usize))
                    .and_then(|country| *country)
                    .map(|country| country.pointer)
                    .or_else(|| {
                        array
                            .element(slot, 8)
                            .and_then(|address| read::<u64>(memory, address))
                            .and_then(|pointer| usize::try_from(pointer).ok())
                    })
                    .filter(|pointer| *pointer != 0)
                    .ok_or("country-array roster pointer unreadable or null")?;
                // A physical alias of Country0 is a proved occurrence, even
                // though positive-only country reference resolution rejects
                // its raw ID. A missing census entry alone proves no zero.
                if Some(pointer) == reserved {
                    output[slot as usize] = 0;
                    continue;
                }
                if reader.is_none() {
                    let current = ccountry::Reader::new(memory, base, state)?;
                    if current.arrays()[0] != array {
                        return Err("country-array roster differs from authenticated census");
                    }
                    reader = Some(current);
                }
                let country = reader
                    .as_ref()
                    .expect("positive country reader was initialized")
                    .read_at(slot)?
                    .ok_or("country-array roster slot is null")?;
                if country.pointer != pointer {
                    return Err("country-array roster pointer differs from authenticated country");
                }
                output[slot as usize] = country.id;
            }
        }
        unchanged(memory, address, bytes)?;
        Ok(required)
    })();
    Capture { len, required }
}

#[cfg(test)]
#[path = "country_roster/tests.rs"]
mod tests;
