
use crate::{Memory, ccharacter::Literal};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a3_74d8);
pub const TAG: usize = 0x08;
pub const SOURCE_FILE: usize = 0x18;
pub const SOURCE_LINE: usize = 0x38;
pub const GENERATED: usize = 0x240;
pub const HEADER: usize = SOURCE_LINE + 4;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub tag: u32,
    pub line: i32,
    pub generated: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct Known<'a> {
    pub identity: Identity,
    pub file: &'a [u8],
}

pub(crate) struct Header {
    pub(crate) identity: Identity,
    pub(crate) file: Literal,
}

pub(crate) fn read(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Header, &'static str> {
    let generated_at = pointer
        .checked_add(GENERATED)
        .ok_or("character template address overflow")?;
    let mut header = [0_u8; HEADER];
    let mut generated = [0_u8; 1];
    if memory.copy_scattered([pointer, generated_at], &mut [&mut header, &mut generated]) != 2 {
        return Err("character template unreadable");
    }
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64 {
        return Err("character template vtable mismatch");
    }
    let file_at = pointer
        .checked_add(SOURCE_FILE)
        .ok_or("character template address overflow")?;
    let file = Literal::from_header(
        file_at,
        header[SOURCE_FILE..SOURCE_LINE].try_into().unwrap(),
    )?;
    Ok(Header {
        identity: Identity {
            tag: u32::from_le_bytes(header[TAG..TAG + 4].try_into().unwrap()),
            line: i32::from_le_bytes(header[SOURCE_LINE..HEADER].try_into().unwrap()),
            generated: generated[0],
        },
        file,
    })
}
