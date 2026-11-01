//! Invalidation before current-world retirement or publication.

pub struct Site {
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// First reset state write, before the reset byte, queue drain and
/// owned-object destruction. A load reaching reset invalidates the world
/// even if loading subsequently fails.
pub const RESET: Site = Site {
    rva: 0x001e_74c5,
    sentinel: [
        0x44, 0x89, 0xab, 0x68, 0x09, 0, 0, 0x48, 0x89, 0x9c, 0x24, 0xa8, 1, 0, 0, 0xc6,
    ],
};

/// World construction after base setup and before derived ownership
/// and singleton publication. No incoming branch or call return enters
/// the replaced seven-byte instruction.
pub const CONSTRUCT: Site = Site {
    rva: 0x001d_74c9,
    sentinel: [
        0x4c, 0x89, 0xb7, 0xe8, 0x09, 0, 0, 0x4c, 0x89, 0xb7, 0xf0, 0x09, 0, 0, 0x8b, 0x0d,
    ],
};

/// The state nudger's apply/save reload: clears and re-parses the state
/// template database, then rebuilds the live `CState` objects and their
/// template pointers, the only post-load writer of the state templates' base
/// resources. One argument (RCX, the editor); RBX is saved to RDX's home slot
/// on entry.
pub const STATE_RELOAD: Site = Site {
    rva: 0x01be_db80,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20,
        0x57,
    ],
};

/// The authored adjacency loader, which the straits reload runs again after
/// map load: it rewrites both provinces' neighbour records, crossing kind and
/// through province included. One argument (RCX, the map).
pub const STRAITS_RELOAD: Site = Site {
    rva: 0x00ab_35f0,
    sentinel: [
        0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d,
        0xac,
    ],
};

/// Deleting destructor. Its entry precedes quadtree clearing, runtime-state
/// reset and eventual deallocation.
pub const DELETE: Site = Site {
    rva: 0x001d_fbd0,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 8, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0xfa, 0x48, 0x8b, 0xd9, 0x48,
    ],
};
