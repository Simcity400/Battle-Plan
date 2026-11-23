
use engine_native_memory::cpdx_array::Array;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x29ce1e8);
pub const VALUES: usize = 0x08;
pub const VALUE_STRIDE: usize = 0x10;
pub const VALUE: usize = 0x00;

pub const TOKEN: usize = 0x08;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoredAmount {
    pub token: u32,
    pub raw: i64,
}

pub fn read_into(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    pool: usize,
    output: &mut Vec<StoredAmount>,
) -> Result<(), &'static str> {
    let header =
        engine_native_memory::read::<[u8; 24]>(memory, pool).ok_or("faction resource pool header unreadable")?;
    let count = u32::from_le_bytes(header[20..24].try_into().unwrap());
    let source = from_header(base, header, count)?;
    crate::allocation::resize_default(output, source.count as usize)
        .map_err(|_| "faction resource pool storage allocation failed")?;
    for (ordinal, value) in output.iter_mut().enumerate() {
        let address = source
            .element(ordinal as u32, VALUE_STRIDE)
            .ok_or("faction resource amount address overflow")?;
        let bytes =
            engine_native_memory::read::<[u8; 12]>(memory, address).ok_or("faction resource amount unreadable")?;
        *value = StoredAmount {
            raw: i64::from_le_bytes(bytes[..8].try_into().unwrap()),
            token: u32::from_le_bytes(bytes[8..].try_into().unwrap()),
        };
    }
    Ok(())
}

pub fn from_header(base: usize, header: [u8; 24], count: u32) -> Result<Array, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if u64::from_le_bytes(header[..8].try_into().expect("fixed vtable width")) != expected as u64 {
        return Err("resource pool vtable mismatch");
    }
    let data = usize::try_from(u64::from_le_bytes(
        header[8..16].try_into().expect("fixed pointer width"),
    ))
    .map_err(|_| "resource pool pointer overflow")?;
    let capacity = i32::from_le_bytes(header[16..20].try_into().expect("fixed capacity width"));
    let actual = i32::from_le_bytes(header[20..24].try_into().expect("fixed count width"));
    if actual < 0 || actual > capacity || actual as u32 != count || (count > 0 && data == 0) {
        return Err("resource pool count disagrees with database or capacity");
    }
    data.checked_add(
        (count as usize)
            .checked_mul(VALUE_STRIDE)
            .ok_or("resource pool extent overflow")?,
    )
    .ok_or("resource pool extent overflow")?;
    Ok(Array { data, count })
}

pub fn from_record(bytes: [u8; 12], token: u32) -> Result<i64, &'static str> {
    let actual_token = u32::from_le_bytes(bytes[8..12].try_into().expect("fixed token width"));
    if actual_token != token {
        return Err("resource value token disagrees with database");
    }
    Ok(i64::from_le_bytes(
        bytes[..8].try_into().expect("fixed value width"),
    ))
}

#[cfg(test)]
mod tests;
