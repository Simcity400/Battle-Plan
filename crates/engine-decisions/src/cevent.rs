//! CEvent definition identity and presentation fields.
//!
//! Function sites describe operations consuming events. Common dispatch
//! receives a separate owner whose class remains unknown; its location here
//! does not establish that it is an event member function.

use engine_sites::Rva;
/// Packed native reference type and combined identifier words.
pub const REFERENCE: usize = 0x08;
/// Primary event-definition vtable.
pub const VTABLE_RVA: Rva = Rva(0x02a1_6008);
/// Authored event identity, MSVC string; an empty name saves the numeric ID.
pub const TOKEN: usize = 0x20;
/// CEvent option pointer vector.
pub const OPTIONS: usize = 0x368;
/// CEvent option count, u32.
pub const OPTION_COUNT: usize = 0x374;
/// CEvent operative presentation flag, u8.
pub const OPERATIVE_CHROME: usize = 0x41d;
/// CEvent news presentation flag, u8.
pub const NEWS_CHROME: usize = 0x424;
/// Engine-global debug-assert flag used by event dispatch.
pub const DISPATCH_DEBUG_ASSERTS_FLAG_RVA: Rva = Rva(0x0368_f611);
