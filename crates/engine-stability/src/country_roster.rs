
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{ccountry, cgame_state};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
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

pub fn demand(memory: &impl Memory, _base: usize, state: usize) -> Result<usize, &'static str> {
    let (address, bytes, array) = header(memory, state)?;
    unchanged(memory, address, bytes)?;
    Ok(array.count as usize)
}

pub fn capture(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<u32>,
) -> Capture {
    capture_buffer(memory, base, state, engine_native_memory::walk::Buffer::Growing(output))
}

fn capture_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, u32>,
) -> Capture {
    let mut required = 0;
    let len = (|| {
        let (address, bytes, array) = header(memory, state)?;
        required = array.count as usize;
        output.ensure(required)?;
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
