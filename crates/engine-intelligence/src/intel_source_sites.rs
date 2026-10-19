//! Writer boundaries for independent source state and carried derivations.

pub const CHEAT_POOL: u32 = 5;
pub struct Site {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}
pub const SITES: [Site; 24] = [
    Site {
        name: "reset",
        rva: 0xcee9b0,
        sentinel: [
            72, 137, 92, 36, 16, 72, 137, 108, 36, 24, 72, 137, 116, 36, 32, 87,
        ],
    },
    Site {
        name: "replace allied contributions",
        rva: 0xcf1e80,
        sentinel: [
            64, 86, 72, 131, 236, 32, 72, 141, 113, 40, 72, 59, 242, 116, 62, 72,
        ],
    },
    Site {
        name: "replace cryptology contributions",
        rva: 0xcf1ee0,
        sentinel: [
            64, 86, 72, 131, 236, 32, 72, 141, 177, 136, 0, 0, 0, 72, 59, 242,
        ],
    },
    Site {
        name: "replace source contributions",
        rva: 0xcf20f0,
        sentinel: [
            72, 137, 92, 36, 8, 72, 137, 116, 36, 16, 72, 137, 124, 36, 24, 65,
        ],
    },
    Site {
        name: "rotate prior day",
        rva: 0xcee240,
        sentinel: [
            72, 141, 81, 88, 72, 131, 193, 112, 233, 83, 250, 255, 255, 204, 204, 204,
        ],
    },
    Site {
        name: "create source context",
        rva: 0xceee20,
        sentinel: [
            72, 137, 92, 36, 8, 72, 137, 108, 36, 16, 72, 137, 116, 36, 24, 87,
        ],
    },
    Site {
        name: "cheat source context",
        rva: 0xcef580,
        sentinel: [
            72, 137, 92, 36, 16, 72, 137, 116, 36, 24, 87, 72, 131, 236, 80, 72,
        ],
    },
    Site {
        name: "set source context",
        rva: 0xcf1fa0,
        sentinel: [
            72, 137, 92, 36, 8, 72, 137, 116, 36, 16, 87, 72, 131, 236, 64, 73,
        ],
    },
    Site {
        name: "retire source context",
        rva: 0xcf1610,
        sentinel: [
            64, 83, 72, 131, 236, 32, 69, 51, 192, 72, 139, 218, 232, 191, 232, 255,
        ],
    },
    Site {
        name: "pending contribution context",
        rva: 0xcedfc0,
        sentinel: [
            133, 210, 116, 77, 87, 72, 131, 236, 32, 73, 139, 249, 131, 250, 6, 119,
        ],
    },
    Site {
        name: "dynamic aggregation context",
        rva: 0xcf2a70,
        sentinel: [
            72, 137, 108, 36, 24, 87, 72, 131, 236, 32, 139, 129, 196, 0, 0, 0,
        ],
    },
    Site {
        name: "dynamic decay context",
        rva: 0xceee90,
        sentinel: [
            72, 137, 108, 36, 16, 72, 137, 116, 36, 24, 72, 137, 124, 36, 32, 65,
        ],
    },
    Site {
        name: "prune sources",
        rva: 0xceef10,
        sentinel: [
            72, 137, 92, 36, 8, 87, 72, 131, 236, 32, 72, 139, 153, 160, 0, 0,
        ],
    },
    Site {
        name: "clear static pool",
        rva: 0xceeca0,
        sentinel: [
            133, 210, 15, 136, 105, 1, 0, 0, 72, 137, 92, 36, 8, 72, 137, 116,
        ],
    },
    Site {
        name: "clear dynamic pool",
        rva: 0xceebc0,
        sentinel: [
            133, 210, 15, 136, 197, 0, 0, 0, 72, 137, 92, 36, 8, 87, 72, 131,
        ],
    },
    Site {
        name: "set source lanes",
        rva: 0x11999e0,
        sentinel: [
            72, 137, 92, 36, 8, 137, 84, 36, 16, 87, 72, 131, 236, 64, 72, 141,
        ],
    },
    Site {
        name: "mark source removed",
        rva: 0x1198bc0,
        sentinel: [
            198, 65, 36, 1, 195, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204,
        ],
    },
    Site {
        name: "create static source",
        rva: 0x1198bd0,
        sentinel: [
            72, 131, 236, 104, 72, 137, 116, 36, 96, 76, 139, 193, 139, 113, 64, 76,
        ],
    },
    Site {
        name: "add pending contribution",
        rva: 0x196ad80,
        sentinel: [
            72, 137, 92, 36, 24, 72, 137, 108, 36, 32, 86, 87, 65, 84, 72, 131,
        ],
    },
    Site {
        name: "aggregate dynamic pool",
        rva: 0x196aee0,
        sentinel: [
            72, 137, 92, 36, 16, 72, 137, 116, 36, 24, 72, 137, 124, 36, 32, 85,
        ],
    },
    Site {
        name: "decay dynamic pool",
        rva: 0x196b200,
        sentinel: [
            64, 83, 76, 139, 81, 32, 76, 139, 202, 72, 99, 65, 44, 72, 141, 12,
        ],
    },
    Site {
        name: "move source reference",
        rva: 0x196b810,
        sentinel: [
            139, 66, 8, 137, 65, 8, 139, 66, 12, 137, 65, 12, 139, 66, 16, 137,
        ],
    },
    Site {
        name: "reset source reference",
        rva: 0x196b840,
        sentinel: [
            72, 137, 92, 36, 24, 86, 72, 131, 236, 48, 131, 121, 8, 0, 72, 139,
        ],
    },
    Site {
        name: "clear allied contributions",
        rva: 0xceec90,
        sentinel: [
            199, 65, 52, 0, 0, 0, 0, 195, 204, 204, 204, 204, 204, 204, 204, 204,
        ],
    },
];
