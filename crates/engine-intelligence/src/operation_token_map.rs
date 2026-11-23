
use engine_sites::Rva;
pub const TABLE: usize = 0x08;
pub const COUNT: usize = 0x10;
pub const MASK: usize = 0x14;
pub const OVERFLOW: usize = 0x18;
pub const EMPTY_TARGET_STORAGE_RVA: Rva = Rva(0x0312_b8c0);

use engine_native_memory::{Memory, read};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Header {
    pub data: usize,
    pub count: u32,
    pub slots: usize,
    pub mask: u32,
    pub overflow: u8,
}

impl Header {
    pub(crate) fn read(
        memory: &impl Memory,
        address: usize,
        stride: usize,
        empty_storage: usize,
    ) -> Result<Self, &'static str> {
        let bytes =
            read::<[u8; 25]>(memory, address).ok_or("operation token map header unreadable")?;
        let data = usize::try_from(u64::from_le_bytes(bytes[TABLE..COUNT].try_into().unwrap()))
            .map_err(|_| "operation token map pointer overflow")?;
        let count = i32::from_le_bytes(bytes[COUNT..MASK].try_into().unwrap());
        let mask = u32::from_le_bytes(bytes[MASK..OVERFLOW].try_into().unwrap());
        let overflow = bytes[OVERFLOW];
        if count < 0 || stride == 0 {
            return Err("operation token map count or stride invalid");
        }
        if mask == 0 && overflow == 0 {
            return if count == 0 && data == empty_storage {
                Ok(Self {
                    data,
                    count: 0,
                    slots: 0,
                    mask,
                    overflow,
                })
            } else {
                Err("operation token static map has live entries")
            };
        }
        let buckets = mask
            .checked_add(1)
            .ok_or("operation token map mask overflow")?;
        if !buckets.is_power_of_two()
            || !(2..=1 << 30).contains(&buckets)
            || u32::from(overflow) != buckets.trailing_zeros() + 2
            || count as u32 > buckets
        {
            return Err("operation token map geometry invalid");
        }
        let slots = (buckets as usize)
            .checked_add(usize::from(overflow))
            .ok_or("operation token map extent overflow")?;
        if data == 0 {
            return Err("operation token map storage null");
        }
        let sentinel = data
            .checked_add(
                slots
                    .checked_mul(stride)
                    .ok_or("operation token map extent overflow")?,
            )
            .ok_or("operation token map address overflow")?;
        if sentinel.checked_add(4).and_then(|p| read::<u8>(memory, p)) != Some(u8::MAX) {
            return Err("operation token map end sentinel invalid or unreadable");
        }
        Ok(Self {
            data,
            count: count as u32,
            slots,
            mask,
            overflow,
        })
    }
}
