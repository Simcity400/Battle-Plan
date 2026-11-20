//! Static modifier definition consumed by modifier composition.

use engine_sites::Rva;
/// Primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x0279_6628);
/// Inherited accumulator initialized on construction and visited by static-source composition.
pub const ACCUMULATOR: usize = 0x10;
/// Script name, an MSVC string; the database finds a definition by it.
pub const NAME: usize = 0x1a8;
