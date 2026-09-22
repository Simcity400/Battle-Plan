//! Portrait-holder writer boundaries on hash-pinned a729. These observe normal
//! execution; the recorder never calls them to obtain a value.
//!
//! 141389960 handles insertion/replacement, including the parser 141389ac0 and
//! public setter 14138a0e0. 140f8a190 replaces a whole holder (definition build,
//! character copy, officer/UI copies). 1403a5130 bulk-inserts copied entries.
//! Constructors and destruction invalidate lifetimes, including address reuse.
//! Entry payload+30 is a renderer cache and is not part of our recorded descriptor.
//! Evidence: pinned hoi4re full decompiles, callers and instruction prefixes.

pub struct MutationSite {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

pub const MUTATIONS: [MutationSite; 6] = [
    MutationSite {
        name: "assign_entry",
        rva: 0x1389960,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
            0x20, 0x57,
        ],
    },
    MutationSite {
        name: "copy_holder",
        rva: 0xf8a190,
        sentinel: [
            0x40, 0x57, 0x41, 0x56, 0x48, 0x83, 0xec, 0x28, 0x48, 0x83, 0xc2, 0x08, 0x48, 0x8d,
            0x79, 0x08,
        ],
    },
    MutationSite {
        name: "construct",
        rva: 0x1388e30,
        sentinel: [
            0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0x9b, 0x65, 0x3c, 0x01, 0x48,
            0x8b, 0xd9,
        ],
    },
    MutationSite {
        name: "copy_construct",
        rva: 0x32c490,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24,
            0x08, 0x57,
        ],
    },
    MutationSite {
        name: "destroy",
        rva: 0x1388e60,
        sentinel: [
            0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0x6b, 0x65, 0x3c, 0x01, 0x48,
            0x8d, 0x59,
        ],
    },
    MutationSite {
        name: "insert_range",
        rva: 0x3a5130,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x41, 0x54, 0x41,
            0x55, 0x41,
        ],
    },
];
