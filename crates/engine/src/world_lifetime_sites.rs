//! Invalidation before current-world retirement or publication.

pub struct Site {
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// First reset state write, before the reset byte, queue drain and
/// owned-object destruction. A load reaching reset invalidates the world
/// even if loading subsequently fails.
pub const RESET: Site = Site {
    rva: 0x001d_09e5,
    sentinel: [
        0x44, 0x89, 0xab, 0x68, 0x09, 0, 0, 0x48, 0x89, 0x9c, 0x24, 0xa8, 1, 0, 0, 0xc6,
    ],
};

/// World construction after base setup and before derived ownership
/// and singleton publication. No incoming branch or call return enters
/// the replaced seven-byte instruction.
pub const CONSTRUCT: Site = Site {
    rva: 0x001b_f509,
    sentinel: [
        0x4c, 0x89, 0xb7, 0xe8, 0x09, 0, 0, 0x4c, 0x89, 0xb7, 0xf0, 0x09, 0, 0, 0x8b, 0x0d,
    ],
};

/// Deleting-destructor entry before quadtree clearing, runtime reset
/// and eventual deallocation.
pub const DELETE: Site = Site {
    rva: 0x001c_9080,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 8, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0xfa, 0x48, 0x8b, 0xd9, 0x48,
    ],
};
