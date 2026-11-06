//! Localization root and its first string pointer, used to obtain the active
//! language. No additional root fields are described here.

use engine_sites::Rva;
/// Localization root pointer.
pub const SINGLETON_RVA: Rva = Rva(0x0366_7f08);
/// Active-language MSVC string pointer, supporting inline or heap storage.
pub const LANGUAGE_STRING: usize = 0;
