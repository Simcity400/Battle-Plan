//! The two C++ container layouts both provenance readers walk.
//!
//! `engine_localisation_provenance` and `engine_modset_provenance` read
//! different engine objects, but they reach them through the same MSVC
//! `std::string` and the same PDX vector, and each carried its own copy of both
//! layouts.

pub const PDX_VECTOR_CAPACITY: u64 = 0x08;
pub const PDX_VECTOR_SIZE: u64 = 0x0c;

pub const STD_STRING_STRIDE: u64 = 0x20;
pub const STD_STRING_SIZE_OFFSET: usize = 0x10;
pub const STD_STRING_CAPACITY_OFFSET: usize = 0x18;
/// Up to and including 15 bytes live in the object; longer strings are behind
/// the pointer at offset 0.
pub const STD_STRING_SSO_CAPACITY: u64 = 15;
