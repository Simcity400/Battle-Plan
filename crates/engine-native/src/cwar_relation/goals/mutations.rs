use engine_sites::Rva;
#[derive(Clone, Copy)]
pub enum Receiver {
    Diplomacy,
    ConsumedGoal,
    Goal,
    Politics,
    ActivityInsert,
    ActivityPrune,
    Activity,
}
impl Receiver {
    pub const fn part(self) -> usize {
        match self {
            Self::Politics | Self::ActivityInsert | Self::ActivityPrune | Self::Activity => 1,
            _ => 0,
        }
    }
}
pub struct Site {
    pub name: &'static str,
    pub rva: Rva,
    pub receiver: Receiver,
}
pub const SITES: [Site; 16] = [
    Site {
        name: "grant_goal",
        rva: Rva(0xda3eb0),
        receiver: Receiver::Diplomacy,
    },
    Site {
        name: "remove_country_goals",
        rva: Rva(0xd9e770),
        receiver: Receiver::Diplomacy,
    },
    Site {
        name: "invalidate_goals",
        rva: Rva(0xda8ab0),
        receiver: Receiver::Diplomacy,
    },
    Site {
        name: "remove_goals",
        rva: Rva(0xda90b0),
        receiver: Receiver::Diplomacy,
    },
    Site {
        name: "consume_goal",
        rva: Rva(0xd7b330),
        receiver: Receiver::ConsumedGoal,
    },
    Site {
        name: "destroy_goal",
        rva: Rva(0x1679290),
        receiver: Receiver::Goal,
    },
    Site {
        name: "read_goal",
        rva: Rva(0x167b710),
        receiver: Receiver::Goal,
    },
    Site {
        name: "insert_activity",
        rva: Rva(0xbfa910),
        receiver: Receiver::ActivityInsert,
    },
    Site {
        name: "cancel_activity",
        rva: Rva(0xc018f0),
        receiver: Receiver::Politics,
    },
    Site {
        name: "advance_activity",
        rva: Rva(0xb2f8c0),
        receiver: Receiver::Activity,
    },
    Site {
        name: "read_activity",
        rva: Rva(0xb2ee20),
        receiver: Receiver::Activity,
    },
    Site {
        name: "read_justification",
        rva: Rva(0xb2f3c0),
        receiver: Receiver::Activity,
    },
    Site {
        name: "destroy_justification",
        rva: Rva(0xb2a380),
        receiver: Receiver::Activity,
    },
    Site {
        name: "delete_justification",
        rva: Rva(0xb2a630),
        receiver: Receiver::Activity,
    },
    Site {
        name: "delete_goal",
        rva: Rva(0x1679340),
        receiver: Receiver::Goal,
    },
    Site {
        name: "prune_activities",
        rva: Rva(0xc01840),
        receiver: Receiver::ActivityPrune,
    },
];

