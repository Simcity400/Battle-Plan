//! CCharacter's portrait-map entry; the concrete native template name is not established.
//! Identity bytes may be non-UTF-8. Empty heap strings are valid and do not imply inline storage.

pub const STRIDE: usize = 0x38;
/// Portrait role enum u32; extractor.rs:44,75-94 validates the compiled enum-to-token mapping.
pub const ROLE: usize = 0x00;
/// Portrait size enum u32; extractor.rs:45,75-94 validates the compiled enum-to-token mapping.
pub const SIZE: usize = 0x04;
/// Portrait entry-set byte; extractor.rs:46,78-80 distinguishes unused entries.
pub const IS_SET: usize = 0x08;
/// Portrait icon-kind enum u32; extractor.rs:47,85-88 distinguishes GFX names and file paths.
pub const KIND: usize = 0x0c;
/// Portrait identity MSVC string; extractor.rs:48 and attach_log.rs:90238-90350 handle both storage modes.
pub const IDENTITY: usize = 0x10;
/// Portrait GFX-name kind; extractor.rs:50,85-88 validates the engine discriminator.
pub const KIND_GFX: u32 = 0;
/// Portrait file-path kind; extractor.rs:51,85-88 validates the engine discriminator.
pub const KIND_FILE_PATH: u32 = 1;
/// Portrait role enum to lexer tokens; extractor.rs:58 and engine TokenToPortraitType inverse establish these IDs.
pub const ROLE_TOKEN_IDS: [u32; 6] = [0x4b26, 0x289d, 0x289e, 0x2e96, 0x3d13, 0x4005];
/// Portrait size enum to lexer tokens; extractor.rs:60 and the engine size parser inverse establish these IDs.
pub const SIZE_TOKEN_IDS: [u32; 2] = [0x30d7, 0x4c28];

/// Owned portrait descriptor, with no native pointer or texture-resource cache.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub category: u32,
    pub size: u32,
    pub is_set: u8,
    pub kind: u32,
    pub identity: crate::msvc_string::Text<256>,
}
pub fn read(memory: &impl crate::Memory, pointer: usize) -> Result<Row, &'static str> {
    use crate::cnaval_combatant::field;
    let header = field::<[u8; 16]>(memory, pointer, 0)?;
    let identity = crate::msvc_string::read_text(
        memory,
        pointer
            .checked_add(IDENTITY)
            .ok_or("portrait identity address overflow")?,
    )
    .ok_or("portrait identity invalid or exceeds256 bytes")?;
    if field::<[u8; 16]>(memory, pointer, 0)? != header {
        return Err("portrait descriptor changed");
    }
    Ok(Row {
        category: u32::from_le_bytes(header[..4].try_into().unwrap()),
        size: u32::from_le_bytes(header[4..8].try_into().unwrap()),
        is_set: header[8],
        kind: u32::from_le_bytes(header[12..16].try_into().unwrap()),
        identity,
    })
}
