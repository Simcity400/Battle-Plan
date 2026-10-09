//! Invalidation before current-world retirement or publication, a729.

pub struct Site {
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// ResetRuntimeState@1401D09B0: first state write, before reset byte@1401D09F4,
/// queue drain@1401D0DEF and owned-object destruction. Load worker1401C85E0
/// reaches it through1401E9E30; even a subsequently failed load invalidates.
pub const RESET: Site = Site {
    rva: 0x001d_09e5,
    sentinel: [
        0x44, 0x89, 0xab, 0x68, 0x09, 0, 0, 0x48, 0x89, 0x9c, 0x24, 0xa8, 1, 0, 0, 0xc6,
    ],
};

/// Current-world ctor1401BF4D0, after base construction and before derived
/// ownership setup and singleton publication@1401D2883. No incoming branch
/// or CALL return enters the seven-byte MOV being replaced.
pub const CONSTRUCT: Site = Site {
    rva: 0x001b_f509,
    sentinel: [
        0x4c, 0x89, 0xb7, 0xe8, 0x09, 0, 0, 0x4c, 0x89, 0xb7, 0xf0, 0x09, 0, 0, 0x8b, 0x0d,
    ],
};

/// Deleting destructor1401C9080. Its entry precedes quadtree clear1401C90A4,
/// ResetRuntimeState1401C90E6 and eventual deallocation1401C911A.
pub const DELETE: Site = Site {
    rva: 0x001c_9080,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 8, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0xfa, 0x48, 0x8b, 0xd9, 0x48,
    ],
};
