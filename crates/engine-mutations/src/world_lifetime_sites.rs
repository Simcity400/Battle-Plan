
use engine_sites::Rva;
pub struct Site {
    pub rva: Rva,
}

pub const RESET: Site = Site {
    rva: Rva(0x001e_74c5),
};

pub const CONSTRUCT: Site = Site {
    rva: Rva(0x001d_74c9),
};

pub const STATE_RELOAD: Site = Site {
    rva: Rva(0x01be_dbf0),
};

pub const STRAITS_RELOAD: Site = Site {
    rva: Rva(0x00ab_3660),
};

pub const DELETE: Site = Site {
    rva: Rva(0x001d_fbd0),
};
