
pub const LEFT: usize = 0x00;
pub const PARENT: usize = 0x08;
pub const RIGHT: usize = 0x10;
pub const IS_NIL: usize = 0x19;
pub const SPECIALIZATION_HANDLE: usize = 0x1c;
pub const POINTS: usize = 0x20;

use engine_native_memory::{Memory, read};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Links {
    pub left: usize,
    pub parent: usize,
    pub right: usize,
    pub nil: bool,
}

impl Links {
    pub(crate) fn read(memory: &impl Memory, address: usize) -> Result<Self, &'static str> {
        if address == 0 {
            return Err("breakthrough tree null link");
        }
        let mut pointers = [0; 24];
        let mut nil = [0];
        let nil_address = address
            .checked_add(IS_NIL)
            .ok_or("breakthrough node address overflow")?;
        if memory.copy_scattered([address, nil_address], &mut [&mut pointers, &mut nil]) != 2 {
            return Err("breakthrough tree links unreadable");
        }
        if nil[0] > 1 {
            return Err("breakthrough tree nil flag invalid");
        }
        let pointer = |offset| {
            usize::try_from(u64::from_le_bytes(
                pointers[offset..offset + 8].try_into().unwrap(),
            ))
            .map_err(|_| "breakthrough tree pointer overflow")
        };
        Ok(Self {
            left: pointer(LEFT)?,
            parent: pointer(PARENT)?,
            right: pointer(RIGHT)?,
            nil: nil[0] != 0,
        })
    }
}

pub(crate) fn value(memory: &impl Memory, address: usize) -> Result<(u32, i32), &'static str> {
    let field = address
        .checked_add(SPECIALIZATION_HANDLE)
        .ok_or("breakthrough node address overflow")?;
    let bytes = read::<[u8; 8]>(memory, field).ok_or("breakthrough node payload unreadable")?;
    Ok((
        u32::from_le_bytes(bytes[..4].try_into().unwrap()),
        i32::from_le_bytes(bytes[4..].try_into().unwrap()),
    ))
}
