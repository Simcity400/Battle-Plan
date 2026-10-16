//! Localization root and its first string pointer, used to obtain the active
//! language. No additional root fields are described here.

/// Localization root pointer.
pub const SINGLETON_RVA: usize = 0x035a_10d8;
/// Active-language MSVC string pointer, supporting inline or heap storage.
pub const LANGUAGE_STRING: usize = 0;
