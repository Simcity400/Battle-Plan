//! Character-template identity: tag, content source file and line, and generated
//! flag. The character's optional template pointer selects these fields. Reading
//! a definition stores its file and line; generated templates set the flag.
//! Other writers do not alter these identity fields.

use crate::{Memory, ccharacter::Literal};

/// Character-template primary vtable.
pub const VTABLE_RVA: usize = 0x02a3_74d8;
/// Template lexer token, u32, copied to the built character and saved as its template tag.
pub const TAG: usize = 0x08;
/// Content source-file string, set before parsing; empty after construction.
pub const SOURCE_FILE: usize = 0x18;
/// Content source line, i32, set before parsing; never-read templates retain -1.
pub const SOURCE_LINE: usize = 0x38;
/// Generated-template byte; generation sets one and construction clears it.
pub const GENERATED: usize = 0x240;
/// One guarded span from the vtable through the source line.
pub const HEADER: usize = SOURCE_LINE + 4;

/// The fixed identity fields, exact as stored.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub tag: u32,
    pub line: i32,
    pub generated: u8,
}

/// A template identity read at an earlier hour for the same template
/// pointer, with its file bytes.
#[derive(Debug, Clone, Copy)]
pub struct Known<'a> {
    pub identity: Identity,
    pub file: &'a [u8],
}

/// The authenticated fixed fields and the file string's parsed header.
pub(crate) struct Header {
    pub(crate) identity: Identity,
    pub(crate) file: Literal,
}

/// Reads the vtable, tag, file header and line in one span and the generated
/// byte in a second, in one guarded scatter. The file's heap terminator and
/// bytes are the caller's (`Literal::validate`, `Literal::copy_all`).
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
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
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
