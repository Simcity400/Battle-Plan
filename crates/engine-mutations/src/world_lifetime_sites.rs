//! Invalidation before current-world retirement or publication.

use engine_sites::Rva;
pub struct Site {
    pub rva: Rva,
}

/// First reset state write, before the reset byte, queue drain and
/// owned-object destruction. A load reaching reset invalidates the world
/// even if loading subsequently fails.
pub const RESET: Site = Site {
    rva: Rva(0x001e_74c5),
};

/// World construction after base setup and before derived ownership
/// and singleton publication. No incoming branch or call return enters
/// the replaced seven-byte instruction.
pub const CONSTRUCT: Site = Site {
    rva: Rva(0x001d_74c9),
};

/// The state nudger's apply/save reload: clears and re-parses the state
/// template database, then rebuilds the live `CState` objects and their
/// template pointers, the only post-load writer of the state templates' base
/// resources. One argument (RCX, the editor); RBX is saved to RDX's home slot
/// on entry.
pub const STATE_RELOAD: Site = Site {
    rva: Rva(0x01be_dbf0),
};

/// The authored adjacency loader, which the straits reload runs again after
/// map load: it rewrites both provinces' neighbour records, crossing kind and
/// through province included. One argument (RCX, the map).
pub const STRAITS_RELOAD: Site = Site {
    rva: Rva(0x00ab_3660),
};

/// Deleting destructor. Its entry precedes quadtree clearing, runtime-state
/// reset and eventual deallocation.
pub const DELETE: Site = Site {
    rva: Rva(0x001d_fbd0),
};
