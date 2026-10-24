//! Native writer/lifetime inventory for carried naval management state.
//! Method sites take at most four integer/pointer arguments and return through RAX.
//! Tap sites use complete stack-independent data spans with no interior branch/return target.
#[derive(Clone, Copy)]
pub enum Register {
    Rax,
    Rbx,
    Rcx,
    Rdx,
    Rbp,
    Rsi,
    Rdi,
    R9,
    R12,
    R13,
    R14,
    R15,
}
pub struct Write {
    pub value: Option<Register>,
    pub literal: u64,
    pub offset: usize,
    pub bytes: u8,
}
pub struct Site {
    pub name: &'static str,
    pub rva: usize,
    pub register: Register,
    pub adjustment: isize,
    pub base: bool,
    pub retire: bool,
    pub sentinel: [u8; 16],
    pub write: Option<Write>,
}
pub struct Method {
    pub name: &'static str,
    pub rva: usize,
    pub argument: usize,
    pub adjustment: isize,
    pub base: bool,
    pub retire: bool,
    pub sentinel: [u8; 16],
}
pub const REPAIR_CHILD_INSERT: Site = Site {
    name: "repair child insert",
    rva: 0x00d72421,
    register: Register::Rbx,
    write: None,
    adjustment: 1176,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x8d, 0x8b, 0x98, 0x04, 0x00, 0x00, 0x48, 0x8d, 0x95, 0x98, 0x00, 0x00, 0x00, 0xe8,
        0x2c,
    ],
};
pub const REPAIR_CHILD_REMOVE: Site = Site {
    name: "repair child remove",
    rva: 0x00d729de,
    register: Register::Rdi,
    write: None,
    adjustment: 1176,
    base: false,
    retire: false,
    sentinel: [
        0x89, 0x97, 0xa4, 0x04, 0x00, 0x00, 0x48, 0x8b, 0x05, 0x3d, 0xab, 0x5c, 0x02, 0x48, 0x8d,
        0x93,
    ],
};
pub const REPAIR_CHILD_CLEAR: Site = Site {
    name: "repair child clear",
    rva: 0x00d72861,
    register: Register::R13,
    write: None,
    adjustment: 1176,
    base: false,
    retire: false,
    sentinel: [
        0x45, 0x89, 0xa5, 0xa4, 0x04, 0x00, 0x00, 0x48, 0x8b, 0x4c, 0x24, 0x30, 0x8b, 0x44, 0x24,
        0x38,
    ],
};
pub const REJOIN_ASSIGN: Site = Site {
    name: "rejoin assign",
    rva: 0x00d78e66,
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x83, 0xe0, 0x04, 0x00, 0x00, 0xb3, 0x01, 0xeb, 0x30, 0x49, 0x8b, 0x87, 0xc8,
        0x04,
    ],
};
pub const REJOIN_COMPLETE: Site = Site {
    name: "rejoin complete",
    rva: 0x00d7a562,
    register: Register::Rdi,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x87, 0xe0, 0x04, 0x00, 0x00, 0x48, 0x8b, 0x81, 0xf0, 0x01, 0x00, 0x00, 0x48,
        0x39,
    ],
};
pub const TASK_ACTIVITY: Site = Site {
    name: "task activity",
    rva: 0x00d73338,
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x83, 0xc8, 0x04, 0x00, 0x00, 0x44, 0x89, 0xab, 0xb8, 0x04, 0x00, 0x00, 0x4c,
        0x89,
    ],
};
pub const CLEAR_QUEUED_DESIGN: Site = Site {
    name: "clear queued design",
    rva: 0x00d677c0,
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0xc7, 0x83, 0xd0, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x45, 0x33, 0xc9, 0x48,
        0xc7,
    ],
};
pub const ATTACH_REINFORCEMENT: Site = Site {
    name: "attach reinforcement",
    rva: 0x00d72222,
    register: Register::Rsi,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x44, 0x88, 0xbe, 0xf7, 0x04, 0x00, 0x00, 0x48, 0x8d, 0x8e, 0x38, 0x05, 0x00, 0x00, 0xe8,
        0x9b,
    ],
};
pub const PROMOTE_REFIT_DESIGN: Site = Site {
    name: "promote refit design",
    rva: 0x00d7a59c,
    register: Register::Rdi,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x87, 0xc8, 0x04, 0x00, 0x00, 0x48, 0xc7, 0x87, 0xd0, 0x04, 0x00, 0x00, 0x00,
        0x00,
    ],
};
pub const QUEUE_REFIT_DESIGN: Site = Site {
    name: "queue refit design",
    rva: 0x00d78dc4,
    register: Register::Rax,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x88, 0xd0, 0x04, 0x00, 0x00, 0x49, 0x8b, 0x57, 0x18, 0x48, 0x89, 0x94, 0x24,
        0xc8,
    ],
};
pub const QUEUE_REPAIR_REFIT: Site = Site {
    name: "queue repair refit",
    rva: 0x00d78e78,
    register: Register::R15,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x49, 0x89, 0x87, 0xd0, 0x04, 0x00, 0x00, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x45, 0x33, 0xc9,
        0x4d,
    ],
};
pub const REINFORCEMENT_SETTING: Site = Site {
    name: "reinforcement setting",
    rva: 0x00d77b50,
    register: Register::Rcx,
    write: Some(Write {
        value: Some(Register::Rdx),
        literal: 0,
        offset: 1271,
        bytes: 1,
    }),
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x88, 0x91, 0xf7, 0x04, 0x00, 0x00, 0xc3, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
        0xcc,
    ],
};
pub const REPAIR_ORDER_BEGIN: Site = Site {
    name: "repair order begin",
    rva: 0x0135086f,
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0xc7, 0x83, 0x64, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x83, 0x39, 0x00, 0x75, 0x06,
        0x83,
    ],
};
pub const REPAIR_ORDER_OTHER: Site = Site {
    name: "repair order other",
    rva: 0x01350894,
    register: Register::Rax,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0xc7, 0x80, 0x64, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xd3, 0x48, 0x8b,
        0xcf,
    ],
};
pub const MOVE_REPAIR_ORDER: Site = Site {
    name: "move repair order",
    rva: 0x0135151c,
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x44, 0x89, 0xab, 0x64, 0x04, 0x00, 0x00, 0x48, 0x8b, 0xcb, 0xe8, 0x35, 0x55, 0xa2, 0xff,
        0x41,
    ],
};
pub const CANCEL_REPAIR_ORDER: Site = Site {
    name: "cancel repair order",
    rva: 0x01352e14,
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x44, 0x89, 0xb3, 0x64, 0x04, 0x00, 0x00, 0x44, 0x39, 0xb3, 0x0c, 0x02, 0x00, 0x00, 0x7e,
        0x0a,
    ],
};
pub const REPAIR_POLICY_COMMAND: Site = Site {
    name: "repair policy command",
    rva: 0x0135365c,
    register: Register::Rdi,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x89, 0x87, 0x64, 0x04, 0x00, 0x00, 0x80, 0x7e, 0x48, 0x00, 0x74, 0x0a, 0x0f, 0xb6, 0x46,
        0x49,
    ],
};
// Split can be commanded independently of repair mode.
pub const REPAIR_SPLIT_COMMAND: Site = Site {
    name: "repair split command",
    rva: 0x0135366c,
    register: Register::Rdi,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x88, 0x87, 0xf4, 0x04, 0x00, 0x00, 0x80, 0x7e, 0x4a, 0x00, 0x74, 0x11, 0x83, 0xbf, 0xb8,
        0x04,
    ],
};
pub const COPY_TASK_POLICY: Site = Site {
    name: "copy task policy",
    rva: 0x01365a87,
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x89, 0x83, 0x64, 0x04, 0x00, 0x00, 0xe8, 0x1e, 0xa7, 0xa0, 0xff, 0x0f, 0xb6, 0xd0, 0x48,
        0x8b,
    ],
};
pub const AI_COMPOSITION_CHOICE: Site = Site {
    name: "ai composition choice",
    rva: 0x01354cfa,
    register: Register::Rdi,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x89, 0x87, 0x08, 0x06, 0x00, 0x00, 0x48, 0x8b, 0x4c, 0x24, 0x30, 0x48, 0x8b, 0x79, 0x08,
        0x80,
    ],
};
pub const FLEET_HOMEBASE_POLICY: Site = Site {
    name: "fleet homebase policy",
    rva: 0x00d5a64a,
    register: Register::Rcx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x88, 0x81, 0x04, 0x01, 0x00, 0x00, 0xe9, 0xbb, 0xed, 0xff, 0xff, 0xcc, 0xcc, 0xcc, 0xcc,
        0xcc,
    ],
};
pub const SHIP_REFIT_ASSIGN: Site = Site {
    name: "ship refit assign",
    rva: 0x00c341c7,
    register: Register::Rcx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x81, 0x38, 0x07, 0x00, 0x00, 0xc3, 0xcc, 0x48, 0x89, 0x5c, 0x24, 0x10, 0x57,
        0x48,
    ],
};
pub const SHIP_REFIT_COMPLETE: Site = Site {
    name: "ship refit complete",
    rva: 0x00c3dbe9,
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x83, 0x38, 0x07, 0x00, 0x00, 0x48, 0x83, 0xc4, 0x30, 0x5b, 0xc3, 0x48, 0x89,
        0x81,
    ],
};
pub const SHIP_REFIT_RETURN: Site = Site {
    name: "ship refit return",
    rva: 0x00c3dbf6,
    register: Register::Rcx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x81, 0x38, 0x07, 0x00, 0x00, 0x48, 0x83, 0xc4, 0x30, 0x5b, 0xc3, 0xcc, 0xcc,
        0xcc,
    ],
};
pub const REPAIR_FACTORY_COMMAND: Site = Site {
    name: "repair factory command",
    rva: 0x00e65dbd,
    register: Register::Rbx,
    write: Some(Write {
        value: Some(Register::R12),
        literal: 0,
        offset: 1200,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x44, 0x89, 0xa3, 0xb0, 0x04, 0x00, 0x00, 0x48, 0x8b, 0x83, 0x90, 0x02, 0x00, 0x00, 0x48,
        0x8b,
    ],
};
pub const REPAIR_FACTORY_SETTING: Site = Site {
    name: "repair factory setting",
    rva: 0x00e6fd8e,
    register: Register::Rcx,
    write: Some(Write {
        value: Some(Register::Rdx),
        literal: 0,
        offset: 1200,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x89, 0x91, 0xb0, 0x04, 0x00, 0x00, 0x48, 0x8b, 0xf9, 0xe8, 0x04, 0x14, 0x00, 0x00, 0x84,
        0xdb,
    ],
};
pub const REPAIR_FACTORY_CLAMP: Site = Site {
    name: "repair factory clamp",
    rva: 0x00e71245,
    register: Register::R9,
    write: Some(Write {
        value: Some(Register::Rdx),
        literal: 0,
        offset: 1200,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x41, 0x89, 0x91, 0xb0, 0x04, 0x00, 0x00, 0xc3, 0xcc, 0xcc, 0xcc, 0x40, 0x53, 0x56, 0x57,
        0x48,
    ],
};
pub const REPAIR_DOCKYARD_SETTING: Site = Site {
    name: "repair dockyard setting",
    rva: 0x00eaf7c0,
    register: Register::Rcx,
    write: Some(Write {
        value: Some(Register::Rdx),
        literal: 0,
        offset: 440,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x89, 0x91, 0xb8, 0x01, 0x00, 0x00, 0xe9, 0xe5, 0x16, 0x00, 0x00, 0xcc, 0xcc, 0xcc, 0xcc,
        0xcc,
    ],
};
pub const REPAIR_DOCKYARD_COMMAND: Site = Site {
    name: "repair dockyard command",
    rva: 0x00eb0fa5,
    register: Register::R14,
    write: Some(Write {
        value: Some(Register::Rdx),
        literal: 0,
        offset: 440,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x41, 0x89, 0x96, 0xb8, 0x01, 0x00, 0x00, 0xc6, 0x85, 0x28, 0x10, 0x00, 0x00, 0x01, 0x44,
        0x8b,
    ],
};
pub const REPAIR_DOCKYARD_CLAMP: Site = Site {
    name: "repair dockyard clamp",
    rva: 0x00eb143a,
    register: Register::R14,
    write: Some(Write {
        value: Some(Register::Rcx),
        literal: 0,
        offset: 440,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x41, 0x89, 0x8e, 0xb8, 0x01, 0x00, 0x00, 0x45, 0x39, 0xa6, 0xb8, 0x01, 0x00, 0x00, 0x0f,
        0x8d,
    ],
};
pub const REPAIR_DOCKYARDS_USED: Site = Site {
    name: "repair dockyards used",
    rva: 0x00eb160f,
    register: Register::R14,
    write: Some(Write {
        value: Some(Register::R12),
        literal: 0,
        offset: 0x1bc,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x45, 0x89, 0xa6, 0xbc, 0x01, 0x00, 0x00, 0x48, 0x8b, 0x8d, 0x38, 0x10, 0x00, 0x00, 0xe8,
        0xee,
    ],
};
pub const NAVY_RESET_USED: Site = Site {
    name: "navy reset used",
    rva: 0x00ea2136,
    register: Register::Rbx,
    write: Some(Write {
        value: Some(Register::R14),
        literal: 0,
        offset: 444,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x44, 0x89, 0xb3, 0xbc, 0x01, 0x00, 0x00, 0xc7, 0x83, 0xb8, 0x01, 0x00, 0x00, 0xff, 0xff,
        0xff,
    ],
};
pub const NAVY_RESET_BUDGET: Site = Site {
    name: "navy reset budget",
    rva: 0x00ea213d,
    register: Register::Rbx,
    write: Some(Write {
        value: None,
        literal: 4294967295,
        offset: 440,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0xc7, 0x83, 0xb8, 0x01, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x48, 0x8b, 0xbb, 0x80, 0x01,
        0x00,
    ],
};
pub const NAVY_REBUILD_USED: Site = Site {
    name: "navy rebuild used",
    rva: 0x00eab816,
    register: Register::Rbx,
    write: Some(Write {
        value: Some(Register::Rbp),
        literal: 0,
        offset: 444,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x89, 0xab, 0xbc, 0x01, 0x00, 0x00, 0xc7, 0x83, 0xb8, 0x01, 0x00, 0x00, 0xff, 0xff, 0xff,
        0xff,
    ],
};
pub const NAVY_REBUILD_BUDGET: Site = Site {
    name: "navy rebuild budget",
    rva: 0x00eab81c,
    register: Register::Rbx,
    write: Some(Write {
        value: None,
        literal: 4294967295,
        offset: 440,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0xc7, 0x83, 0xb8, 0x01, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xe8, 0x55, 0x2e, 0x00, 0x00,
        0x48,
    ],
};
pub const BASE_REPAIR_PRIORITY: Site = Site {
    name: "base repair priority",
    rva: 0x00eaf7d0,
    register: Register::Rcx,
    write: Some(Write {
        value: Some(Register::Rdx),
        literal: 0,
        offset: 32,
        bytes: 4,
    }),
    adjustment: 0,
    base: true,
    retire: false,
    sentinel: [
        0x89, 0x51, 0x20, 0x48, 0x8b, 0x49, 0x08, 0xe9, 0xd4, 0x16, 0x00, 0x00, 0xcc, 0xcc, 0xcc,
        0xcc,
    ],
};
pub const BASE_REPAIR_ALLOCATION: Site = Site {
    name: "base repair allocation",
    rva: 0x00eb164e,
    register: Register::Rdx,
    write: Some(Write {
        value: Some(Register::Rax),
        literal: 0,
        offset: 0x1c,
        bytes: 4,
    }),
    adjustment: 0,
    base: true,
    retire: false,
    sentinel: [
        0x89, 0x42, 0x1c, 0x48, 0x83, 0xc3, 0x10, 0x49, 0x3b, 0xd8, 0x75, 0xe6, 0x48, 0x8b, 0x5c,
        0x24,
    ],
};
pub const BASE_QUEUE_PRUNE: Site = Site {
    name: "base queue prune",
    rva: 0x00ea18b7,
    register: Register::R15,
    write: None,
    adjustment: 0,
    base: true,
    retire: false,
    sentinel: [
        0x49, 0x8b, 0x57, 0x28, 0x48, 0x63, 0xc6, 0x48, 0x8d, 0x0c, 0xc2, 0x49, 0x63, 0x47, 0x34,
        0x4c,
    ],
};
pub const BASE_QUEUE_INSERT: Site = Site {
    name: "base queue insert",
    rva: 0x00e9f4ba,
    register: Register::Rbp,
    write: None,
    adjustment: 0,
    base: true,
    retire: false,
    sentinel: [
        0x8b, 0xd7, 0x48, 0x8d, 0x4d, 0x28, 0xe8, 0x7b, 0x21, 0x31, 0xff, 0x48, 0x8b, 0x7c, 0x24,
        0x58,
    ],
};
pub const BASE_QUEUE_ARRIVAL: Site = Site {
    name: "base queue arrival",
    rva: 0x00eab2b0,
    register: Register::Rbp,
    write: None,
    adjustment: 0,
    base: true,
    retire: false,
    sentinel: [
        0x8b, 0xd7, 0x48, 0x8d, 0x4d, 0x28, 0xe8, 0x85, 0x63, 0x30, 0xff, 0x4c, 0x8b, 0x74, 0x24,
        0x60,
    ],
};
pub const BASE_QUEUE_BULK: Site = Site {
    name: "base queue bulk",
    rva: 0x00e9f8cb,
    register: Register::Rbp,
    write: None,
    adjustment: 0,
    base: true,
    retire: false,
    sentinel: [
        0x48, 0x8d, 0x4d, 0x28, 0x44, 0x8b, 0xce, 0x4d, 0x8b, 0xc6, 0x8b, 0xd0, 0xe8, 0x74, 0xe7,
        0x41,
    ],
};
pub const COMPOSITION_ATTACH_CLEAR: Site = Site {
    name: "composition attach clear",
    rva: 0x00d72229,
    register: Register::Rsi,
    write: None,
    adjustment: 1336,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x8d, 0x8e, 0x38, 0x05, 0x00, 0x00, 0xe8, 0x9b, 0xfc, 0x8b, 0xff, 0x4c, 0x63, 0xa6,
        0x54,
    ],
};
pub const COMPOSITION_DETACH_CLEAR: Site = Site {
    name: "composition detach clear",
    rva: 0x00d72d13,
    register: Register::Rdi,
    write: None,
    adjustment: 1336,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x8d, 0x8f, 0x38, 0x05, 0x00, 0x00, 0xe8, 0xb1, 0xf1, 0x8b, 0xff, 0x48, 0x8d, 0x8e,
        0xb0,
    ],
};
pub const REFIT_PRODUCTION_ADD: Site = Site {
    name: "refit production add",
    rva: 0x00f71bd8,
    register: Register::Rbx,
    write: Some(Write {
        value: Some(Register::Rdi),
        literal: 0,
        offset: 56,
        bytes: 8,
    }),
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x7b, 0x38, 0x48, 0x8b, 0x73, 0x28, 0x48, 0x3b, 0xfe, 0x7c, 0x4e, 0x48, 0x8b,
        0xc7,
    ],
};
pub const REFIT_PRODUCTION_COMPLETE: Site = Site {
    name: "refit production complete",
    rva: 0x00f71bef,
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x2b, 0xc6, 0x48, 0x89, 0x43, 0x38, 0xe8, 0xc5, 0xd2, 0xd9, 0xff, 0x48, 0x8b, 0x4b,
        0x20,
    ],
};
pub const REFIT_PROGRESS_DAMAGE: Site = Site {
    name: "refit progress damage",
    rva: 0x00f71b3f,
    register: Register::Rcx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x03, 0xd0, 0x48, 0x29, 0x51, 0x38, 0x79, 0x08, 0x48, 0xc7, 0x41, 0x38, 0x00, 0x00,
        0x00,
    ],
};
pub const REFIT_PROGRESS_REFRESH: Site = Site {
    name: "refit progress refresh",
    rva: 0x0193a72b,
    register: Register::R14,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x49, 0xc7, 0x46, 0x38, 0x00, 0x00, 0x00, 0x00, 0x49, 0x8d, 0x8e, 0x90, 0x00, 0x00, 0x00,
        0x83,
    ],
};
pub const SITES: &[Site] = &[
    REPAIR_CHILD_INSERT,
    REPAIR_CHILD_REMOVE,
    REPAIR_CHILD_CLEAR,
    REJOIN_ASSIGN,
    REJOIN_COMPLETE,
    TASK_ACTIVITY,
    CLEAR_QUEUED_DESIGN,
    ATTACH_REINFORCEMENT,
    PROMOTE_REFIT_DESIGN,
    QUEUE_REFIT_DESIGN,
    QUEUE_REPAIR_REFIT,
    REINFORCEMENT_SETTING,
    REPAIR_ORDER_BEGIN,
    REPAIR_ORDER_OTHER,
    MOVE_REPAIR_ORDER,
    CANCEL_REPAIR_ORDER,
    REPAIR_POLICY_COMMAND,
    REPAIR_SPLIT_COMMAND,
    COPY_TASK_POLICY,
    AI_COMPOSITION_CHOICE,
    FLEET_HOMEBASE_POLICY,
    SHIP_REFIT_ASSIGN,
    SHIP_REFIT_COMPLETE,
    SHIP_REFIT_RETURN,
    REPAIR_FACTORY_COMMAND,
    REPAIR_FACTORY_SETTING,
    REPAIR_FACTORY_CLAMP,
    REPAIR_DOCKYARD_SETTING,
    REPAIR_DOCKYARD_COMMAND,
    REPAIR_DOCKYARD_CLAMP,
    REPAIR_DOCKYARDS_USED,
    NAVY_RESET_USED,
    NAVY_RESET_BUDGET,
    NAVY_REBUILD_USED,
    NAVY_REBUILD_BUDGET,
    BASE_REPAIR_PRIORITY,
    BASE_REPAIR_ALLOCATION,
    BASE_QUEUE_PRUNE,
    BASE_QUEUE_INSERT,
    BASE_QUEUE_ARRIVAL,
    BASE_QUEUE_BULK,
    COMPOSITION_ATTACH_CLEAR,
    COMPOSITION_DETACH_CLEAR,
    REFIT_PRODUCTION_ADD,
    REFIT_PRODUCTION_COMPLETE,
    REFIT_PROGRESS_DAMAGE,
    REFIT_PROGRESS_REFRESH,
];
pub const COMPOSITION_ENTRY: Method = Method {
    name: "composition entry",
    rva: 0x0198a8e0,
    argument: 0,
    adjustment: -8,
    base: false,
    retire: false,
    sentinel: [
        0x40, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x40, 0x4c, 0x63, 0x49, 0x0c, 0x4d,
        0x8b,
    ],
};
pub const COMPOSITION_MERGE: Method = Method {
    name: "composition merge",
    rva: 0x0198a9e0,
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x20, 0x56, 0x48, 0x83, 0xec, 0x40, 0x48, 0x8b, 0x5a, 0x08, 0x48,
        0x63,
    ],
};
pub const COMPOSITION_ASSIGN: Method = Method {
    name: "composition assign",
    rva: 0x00d642e0,
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x40, 0x53, 0x56, 0x57, 0x41, 0x55, 0x41, 0x56, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x42,
        0x08,
    ],
};
pub const COMPOSITION_ERASE: Method = Method {
    name: "composition erase",
    rva: 0x0198b6e0,
    argument: 0,
    adjustment: -8,
    base: false,
    retire: false,
    sentinel: [
        0x4d, 0x63, 0xd8, 0x4c, 0x8b, 0xd1, 0x41, 0x8b, 0xc3, 0x2b, 0xc2, 0x85, 0xc0, 0x7f, 0x03,
        0x33,
    ],
};
pub const COMPOSITION_FULFILLMENT: Method = Method {
    name: "composition fulfillment",
    rva: 0x0198b7d0,
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x18,
        0x41,
    ],
};
pub const COMPOSITION_RANGE: Method = Method {
    name: "composition range",
    rva: 0x0198b830,
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x18,
        0x41,
    ],
};
pub const COMPOSITION_ROSTER_ADD: Method = Method {
    name: "composition roster add",
    rva: 0x0198ab00,
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x40, 0x55, 0x56, 0x48, 0x83, 0xec, 0x38, 0x48, 0x8b, 0x32, 0x48, 0x63, 0x42, 0x0c, 0x48,
        0x8d,
    ],
};
pub const COMPOSITION_SHIP_ADD: Method = Method {
    name: "composition ship add",
    rva: 0x0198ac30,
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x30,
        0x8b,
    ],
};
pub const FLEET_DESTROY: Method = Method {
    name: "fleet destroy",
    rva: 0x00d50ea0,
    argument: 0,
    adjustment: 0,
    base: false,
    retire: true,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0xda, 0x48, 0x8b, 0xf9,
        0xe8,
    ],
};
pub const TASK_DESTROY: Method = Method {
    name: "task destroy",
    rva: 0x00d63620,
    argument: 0,
    adjustment: 0,
    base: false,
    retire: true,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20,
        0x55,
    ],
};
pub const SHIP_DESTROY: Method = Method {
    name: "ship destroy",
    rva: 0x00c30380,
    argument: 0,
    adjustment: 0,
    base: false,
    retire: true,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x41, 0x54, 0x41, 0x55,
        0x41,
    ],
};
pub const REFIT_DESTROY: Method = Method {
    name: "refit destroy",
    rva: 0x00f71a50,
    argument: 0,
    adjustment: 0,
    base: false,
    retire: true,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0x27, 0xaa,
        0xa0,
    ],
};
pub const BASE_DESTROY: Method = Method {
    name: "base destroy",
    rva: 0x00e9ef20,
    argument: 0,
    adjustment: 0,
    base: true,
    retire: true,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0xda, 0x48, 0x8b, 0xf9,
        0xe8,
    ],
};
pub const PRODUCTION_DESTROY: Method = Method {
    name: "production destroy",
    rva: 0x00e5c870,
    argument: 0,
    adjustment: 0,
    base: false,
    retire: true,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
        0x48,
    ],
};
pub const NAVY_DESTROY: Method = Method {
    name: "navy destroy",
    rva: 0x00e9e0c0,
    argument: 0,
    adjustment: 0,
    base: false,
    retire: true,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20,
        0x48,
    ],
};
pub const BASE_COUNTRY_REORDER: Method = Method {
    name: "base country reorder",
    rva: 0x00eaeb30,
    argument: 0,
    adjustment: 0,
    base: true,
    retire: false,
    sentinel: [
        0x44, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08,
        0x55,
    ],
};
pub const REFIT_RECALCULATE: Method = Method {
    name: "refit recalculate",
    rva: 0x00f71db0,
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20,
        0x57,
    ],
};
pub const BASE_QUEUE_REORDER: Method = Method {
    name: "base queue reorder",
    rva: 0x00eae9c0,
    argument: 0,
    adjustment: 0,
    base: true,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x55, 0x56, 0x48, 0x83, 0xec, 0x30, 0x33, 0xed, 0x41,
        0x8b,
    ],
};
pub const BASE_QUEUE_REMOVE: Method = Method {
    name: "base queue remove",
    rva: 0x00eae920,
    argument: 0,
    adjustment: 0,
    base: true,
    retire: false,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x41, 0x56, 0x48, 0x83, 0xec,
        0x20,
    ],
};
pub const METHODS: &[Method] = &[
    COMPOSITION_ENTRY,
    COMPOSITION_MERGE,
    COMPOSITION_ASSIGN,
    COMPOSITION_ERASE,
    COMPOSITION_FULFILLMENT,
    COMPOSITION_RANGE,
    COMPOSITION_ROSTER_ADD,
    COMPOSITION_SHIP_ADD,
    FLEET_DESTROY,
    TASK_DESTROY,
    SHIP_DESTROY,
    REFIT_DESTROY,
    BASE_DESTROY,
    PRODUCTION_DESTROY,
    NAVY_DESTROY,
    BASE_COUNTRY_REORDER,
    REFIT_RECALCULATE,
    BASE_QUEUE_REORDER,
    BASE_QUEUE_REMOVE,
];
