
use engine_sites::Rva;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Domain {
    Status,
    Folder,
    Track,
    Gains,
    Costs,
    Grants,
    Equipment,
    GrantExpiry,
    Bank,
    Progress,
    StatusFolders,
}
pub const DOMAINS: usize = 11;
pub struct Site {
    pub name: &'static str,
    pub rva: Rva,
    pub domain: Domain,
}
pub const SITES: [Site; 16] = [
    Site {
        name: "reset_status",
        rva: Rva(0x1468db0),
        domain: Domain::StatusFolders,
    },
    Site {
        name: "select_grand",
        rva: Rva(0x104b3d0),
        domain: Domain::Folder,
    },
    Site {
        name: "select_sub",
        rva: Rva(0x1518130),
        domain: Domain::Track,
    },
    Site {
        name: "reset_track",
        rva: Rva(0x1517ee0),
        domain: Domain::Track,
    },
    Site {
        name: "clear_track",
        rva: Rva(0x1517e90),
        domain: Domain::Track,
    },
    Site {
        name: "add_mastery",
        rva: Rva(0x15178a0),
        domain: Domain::Progress,
    },
    Site {
        name: "add_bank",
        rva: Rva(0x1517870),
        domain: Domain::Bank,
    },
    Site {
        name: "refresh_sources",
        rva: Rva(0x104d770),
        domain: Domain::Gains,
    },
    Site {
        name: "complete_sub",
        rva: Rva(0x104d160),
        domain: Domain::Folder,
    },
    Site {
        name: "add_cost",
        rva: Rva(0x1465270),
        domain: Domain::Costs,
    },
    Site {
        name: "consume_cost",
        rva: Rva(0x1469790),
        domain: Domain::Costs,
    },
    Site {
        name: "add_grant",
        rva: Rva(0x14653c0),
        domain: Domain::Grants,
    },
    Site {
        name: "daily_grants",
        rva: Rva(0x1467b70),
        domain: Domain::GrantExpiry,
    },
    Site {
        name: "apply_reward",
        rva: Rva(0x1467e20),
        domain: Domain::Equipment,
    },
    Site {
        name: "remove_reward",
        rva: Rva(0x14699e0),
        domain: Domain::Equipment,
    },
    Site {
        name: "post_read",
        rva: Rva(0x1468a30),
        domain: Domain::Status,
    },
];

pub const EFFECTIVE_GAIN_STORES: [Rva; 2] = [Rva(0x104c825), Rva(0x104c82c)];
