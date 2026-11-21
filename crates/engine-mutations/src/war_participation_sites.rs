
use engine_sites::Rva;
pub struct Site {
    pub name: &'static str,
    pub rva: Rva,
    pub columns: u16,
}

pub const SITES: [Site; 15] = [
    Site {
        name: "bombing_decay",
        rva: Rva(0x12060c0),
        columns: 8,
    },
    Site {
        name: "air_damage",
        rva: Rva(0x1206170),
        columns: 4,
    },
    Site {
        name: "equipment_damage",
        rva: Rva(0x1206220),
        columns: 1,
    },
    Site {
        name: "destroyed_unit",
        rva: Rva(0x1206360),
        columns: 1,
    },
    Site {
        name: "province_capture",
        rva: Rva(0x1206590),
        columns: 2,
    },
    Site {
        name: "strategic_bombing",
        rva: Rva(0x1206660),
        columns: 8,
    },
    Site {
        name: "strength_damage",
        rva: Rva(0x1206690),
        columns: 1,
    },
    Site {
        name: "sunk_convoys",
        rva: Rva(0x1206760),
        columns: 32,
    },
    Site {
        name: "sunk_ships",
        rva: Rva(0x12067b0),
        columns: 16,
    },
    Site {
        name: "casualty_score",
        rva: Rva(0x1206960),
        columns: 192,
    },
    Site {
        name: "replace_ledger",
        rva: Rva(0xd8eea0),
        columns: 1023,
    },
    Site {
        name: "fuel_sent",
        rva: Rva(0xd8bb20),
        columns: 2,
    },
    Site {
        name: "industry_sent",
        rva: Rva(0xd8bb40),
        columns: 1,
    },
    Site {
        name: "fuel_received",
        rva: Rva(0xd8bb60),
        columns: 8,
    },
    Site {
        name: "industry_received",
        rva: Rva(0xd8bb80),
        columns: 4,
    },
];
