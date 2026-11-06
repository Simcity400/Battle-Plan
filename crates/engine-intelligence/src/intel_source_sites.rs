//! Writer boundaries for independent source state and carried derivations.

use engine_sites::Rva;
pub const CHEAT_POOL: u32 = 5;
pub struct Site {
    pub name: &'static str,
    pub rva: Rva,
}
pub const SITES: [Site; 24] = [
    Site {
        name: "reset",
        rva: Rva(0xd648a0),
    },
    Site {
        name: "replace allied contributions",
        rva: Rva(0xd67d70),
    },
    Site {
        name: "replace cryptology contributions",
        rva: Rva(0xd67dd0),
    },
    Site {
        name: "replace source contributions",
        rva: Rva(0xd67fe0),
    },
    Site {
        name: "rotate prior day",
        rva: Rva(0xd64130),
    },
    Site {
        name: "create source context",
        rva: Rva(0xd64d10),
    },
    Site {
        name: "cheat source context",
        rva: Rva(0xd65470),
    },
    Site {
        name: "set source context",
        rva: Rva(0xd67e90),
    },
    Site {
        name: "retire source context",
        rva: Rva(0xd67500),
    },
    Site {
        name: "pending contribution context",
        rva: Rva(0xd63eb0),
    },
    Site {
        name: "dynamic aggregation context",
        rva: Rva(0xd68960),
    },
    Site {
        name: "dynamic decay context",
        rva: Rva(0xd64d80),
    },
    Site {
        name: "prune sources",
        rva: Rva(0xd64e00),
    },
    Site {
        name: "clear static pool",
        rva: Rva(0xd64b90),
    },
    Site {
        name: "clear dynamic pool",
        rva: Rva(0xd64ab0),
    },
    Site {
        name: "set source lanes",
        rva: Rva(0x123bf70),
    },
    Site {
        name: "mark source removed",
        rva: Rva(0x123b150),
    },
    Site {
        name: "create static source",
        rva: Rva(0x123b160),
    },
    Site {
        name: "add pending contribution",
        rva: Rva(0x1a10e80),
    },
    Site {
        name: "aggregate dynamic pool",
        rva: Rva(0x1a10fe0),
    },
    Site {
        name: "decay dynamic pool",
        rva: Rva(0x1a11300),
    },
    Site {
        name: "move source reference",
        rva: Rva(0x1a11910),
    },
    Site {
        name: "reset source reference",
        rva: Rva(0x1a11940),
    },
    Site {
        name: "clear allied contributions",
        rva: Rva(0xd64b80),
    },
];
