//! Building-content loader helpers; no shared receiver type is established.
//! Directory enumeration traverses a callback-populated tree. Its comparator
//! and callback order are unknown; platform enumeration order is not assumed.

/// VFS directory enumerator traversing a callback-populated tree and appending output entries.
pub const ENUMERATE_DIRECTORY_RVA: usize = 0x024c_5170;
/// Directory wrapper clearing output and prefixing each returned entry with the requested directory.
pub const ENUMERATE_DIRECTORY_WRAPPER_RVA: usize = 0x024c_73b0;
