
use engine_native_memory::{Memory, cpdx_array::Array, read};
use crate::{nproject_cprogram_status, nproject_cproject};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a4_0e38);
pub const PROJECTS: usize = 0x10;
pub const CAPACITY: usize = 0x18;
pub const COUNT: usize = 0x1c;
const INDEX: usize = 0x30;

pub fn active_definition(
    memory: &impl Memory,
    base: usize,
    status: usize,
    token: u32,
) -> Result<Option<(u32, u32)>, &'static str> {
    let Some(pointer) = active_pointer(memory, base, status, token)? else {
        return Ok(None);
    };
    let pair = pointer
        .checked_add(nproject_cproject::DEFINITION_HANDLE)
        .and_then(|at| read::<[u8; 8]>(memory, at))
        .ok_or("active project definition unreadable")?;
    Ok(Some((
        u32::from_le_bytes(pair[..4].try_into().unwrap()),
        u32::from_le_bytes(pair[4..].try_into().unwrap()),
    )))
}

pub fn active_pointer(
    memory: &impl Memory,
    base: usize,
    status: usize,
    token: u32,
) -> Result<Option<usize>, &'static str> {
    let projects = from_status(memory, base, status)?;
    let pool = status
        .checked_add(nproject_cprogram_status::PROJECT_POOL)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|at| usize::try_from(at).ok())
        .ok_or("project pool pointer unreadable")?;
    let header = pool
        .checked_add(INDEX)
        .and_then(|at| read::<[u8; 17]>(memory, at))
        .ok_or("project index header unreadable")?;
    let data = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        .map_err(|_| "project index address overflow")?;
    let mask = i32::from_le_bytes(header[12..16].try_into().unwrap());
    if mask < 0 || data == 0 {
        return Err("project index header invalid");
    }
    let sentinel = (mask as usize)
        .checked_add(usize::from(header[16]) + 1)
        .ok_or("project index span overflow")?;
    let mix = |n: u32| (n ^ (n >> 16)).wrapping_mul(0x45d9f3b);
    let hash = mix(mix(token));
    let mut slot = ((hash ^ (hash >> 16)) & mask as u32) as usize;
    let mut distance = 1u8;
    loop {
        if slot > sentinel {
            return Err("project index probe outside allocation");
        }
        let bytes = slot
            .checked_mul(12)
            .and_then(|n| data.checked_add(n))
            .and_then(|at| read::<[u8; 10]>(memory, at))
            .ok_or("project index bucket unreadable")?;
        if bytes[0] < distance {
            return Ok(None);
        }
        if u32::from_le_bytes(bytes[4..8].try_into().unwrap()) == token {
            let index = i16::from_le_bytes(bytes[8..10].try_into().unwrap());
            if slot == sentinel || index < 0 {
                return Ok(None);
            }
            let pointer = projects
                .element(index as u32, nproject_cproject::POOL_STRIDE)
                .ok_or("project index outside pool")?;
            let expected = engine_sites::at(base, nproject_cproject::VTABLE_RVA)
                .ok_or("active project vtable mismatch")?;
            if read::<u64>(memory, pointer) != Some(expected as u64) {
                return Err("active project vtable mismatch");
            }
            return Ok(Some(pointer));
        }
        slot += 1;
        distance = distance
            .checked_add(1)
            .ok_or("project index probe distance overflow")?;
    }
}

pub(crate) fn from_status(
    memory: &impl Memory,
    base: usize,
    status: usize,
) -> Result<Array, &'static str> {
    let pool = status
        .checked_add(nproject_cprogram_status::PROJECT_POOL)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("project pool pointer unreadable or null")?;
    let vtable = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if read::<u64>(memory, pool) != Some(vtable as u64) {
        return Err("project pool vtable mismatch");
    }
    pool.checked_add(PROJECTS)
        .and_then(|p| Array::read(memory, p, nproject_cproject::POOL_STRIDE))
        .ok_or("project pool array unreadable or invalid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    fn put<T: AsRef<[u8]>>(bytes: &mut [u8], at: usize, value: T) {
        let value = value.as_ref();
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    #[test]
    fn active_token_lookup_keeps_missing_negative_zero_and_faults_distinct() {
        let mut bytes = vec![0; 0x400];
        put(
            &mut bytes,
            nproject_cprogram_status::PROJECT_POOL,
            0x1100u64.to_le_bytes(),
        );
        put(&mut bytes, 0x100, (VTABLE_RVA.0 as u64).to_le_bytes());
        put(&mut bytes, 0x110, 0x1200u64.to_le_bytes());
        put(&mut bytes, 0x118, ((1u64 << 32) | 1).to_le_bytes());
        put(&mut bytes, 0x130, 0x1180u64.to_le_bytes());
        put(
            &mut bytes,
            0x200,
            (nproject_cproject::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x21c, 4u32.to_le_bytes());
        let read = |bytes: &[u8]| {
            active_definition(
                &SliceMemory {
                    base: 0x1000,
                    bytes,
                },
                0,
                0x1000,
                0,
            )
        };
        assert_eq!(read(&bytes), Ok(None));
        bytes[0x180] = 1;
        assert_eq!(read(&bytes), Ok(Some((0, 4))));
        put(&mut bytes, 0x188, (-2i16).to_le_bytes());
        assert_eq!(read(&bytes), Ok(None));
        put(&mut bytes, 0x188, 1i16.to_le_bytes());
        assert_eq!(read(&bytes), Err("project index outside pool"));
        put(&mut bytes, 0x188, 0i16.to_le_bytes());
        assert_eq!(
            read(&bytes[..0x185]),
            Err("project index bucket unreadable")
        );
        bytes[0x200] ^= 1;
        assert_eq!(read(&bytes), Err("active project vtable mismatch"));
    }
}
