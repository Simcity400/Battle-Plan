//! Native writer/lifetime inventory for carried naval management state.
//! Method sites take at most four integer/pointer arguments and return through RAX.
//! Tap sites use complete stack-independent data spans with no interior branch/return target.
use engine_sites::Rva;
#[derive(Clone, Copy)]
pub enum Register {
    Rax,
    Rbx,
    Rcx,
    Rdx,
    Rbp,
    Rsi,
    Rdi,
    R11,
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
    pub rva: Rva,
    pub register: Register,
    pub adjustment: isize,
    pub base: bool,
    pub retire: bool,
    pub write: Option<Write>,
}
pub struct Method {
    pub name: &'static str,
    pub rva: Rva,
    pub argument: usize,
    pub adjustment: isize,
    pub base: bool,
    pub retire: bool,
}
pub const REPAIR_CHILD_INSERT: Site = Site {
    name: "repair child insert",
    rva: Rva(0x00dd8878),
    register: Register::Rdi,
    write: None,
    adjustment: 1176,
    base: false,
    retire: false,
};
pub const REPAIR_CHILD_REMOVE: Site = Site {
    name: "repair child remove",
    rva: Rva(0x00dd8e2e),
    register: Register::Rdi,
    write: None,
    adjustment: 1176,
    base: false,
    retire: false,
};
pub const REPAIR_CHILD_CLEAR: Site = Site {
    name: "repair child clear",
    rva: Rva(0x00dd8cb1),
    register: Register::R13,
    write: None,
    adjustment: 1176,
    base: false,
    retire: false,
};
pub const REJOIN_ASSIGN: Site = Site {
    name: "rejoin assign",
    rva: Rva(0x00ddf2f6),
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const REJOIN_COMPLETE: Site = Site {
    name: "rejoin complete",
    rva: Rva(0x00de09c2),
    register: Register::Rdi,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const TASK_ACTIVITY: Site = Site {
    name: "task activity",
    rva: Rva(0x00dd9788),
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const CLEAR_QUEUED_DESIGN: Site = Site {
    name: "clear queued design",
    rva: Rva(0x00dcdb30),
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const ATTACH_REINFORCEMENT: Site = Site {
    name: "attach reinforcement",
    rva: Rva(0x00dd8677),
    register: Register::Rsi,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const PROMOTE_REFIT_DESIGN: Site = Site {
    name: "promote refit design",
    rva: Rva(0x00de09fc),
    register: Register::Rdi,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const QUEUE_REFIT_DESIGN: Site = Site {
    name: "queue refit design",
    rva: Rva(0x00ddf254),
    register: Register::Rax,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const QUEUE_REPAIR_REFIT: Site = Site {
    name: "queue repair refit",
    rva: Rva(0x00ddf308),
    register: Register::R15,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const REINFORCEMENT_SETTING: Site = Site {
    name: "reinforcement setting",
    rva: Rva(0x00dde0b0),
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
};
pub const REPAIR_ORDER_BEGIN: Site = Site {
    name: "repair order begin",
    rva: Rva(0x013e588f),
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const REPAIR_ORDER_OTHER: Site = Site {
    name: "repair order other",
    rva: Rva(0x013e58b4),
    register: Register::Rax,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const MOVE_REPAIR_ORDER: Site = Site {
    name: "move repair order",
    rva: Rva(0x013e6509),
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const CANCEL_REPAIR_ORDER: Site = Site {
    name: "cancel repair order",
    rva: Rva(0x013e7de4),
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const REPAIR_POLICY_COMMAND: Site = Site {
    name: "repair policy command",
    rva: Rva(0x013e8611),
    register: Register::Rdi,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
// Split can be commanded independently of repair mode.
pub const REPAIR_SPLIT_COMMAND: Site = Site {
    name: "repair split command",
    rva: Rva(0x013e8621),
    register: Register::Rdi,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const COPY_TASK_POLICY: Site = Site {
    name: "copy task policy",
    rva: Rva(0x013fbaa7),
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const AI_COMPOSITION_CHOICE: Site = Site {
    name: "ai composition choice",
    rva: Rva(0x013e99d6),
    register: Register::Rdi,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const FLEET_HOMEBASE_POLICY: Site = Site {
    name: "fleet homebase policy",
    rva: Rva(0x00dc0528),
    register: Register::Rcx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const SHIP_REFIT_ASSIGN: Site = Site {
    name: "ship refit assign",
    rva: Rva(0x00c8a8c7),
    register: Register::Rcx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const SHIP_REFIT_COMPLETE: Site = Site {
    name: "ship refit complete",
    rva: Rva(0x00c94329),
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const SHIP_REFIT_RETURN: Site = Site {
    name: "ship refit return",
    rva: Rva(0x00c94336),
    register: Register::Rcx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const REPAIR_FACTORY_COMMAND: Site = Site {
    name: "repair factory command",
    rva: Rva(0x00ed6bad),
    register: Register::Rbx,
    write: Some(Write {
        value: Some(Register::R12),
        literal: 0,
        offset: 1224,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
};
pub const REPAIR_FACTORY_SETTING: Site = Site {
    name: "repair factory setting",
    rva: Rva(0x00ee16c2),
    register: Register::Rcx,
    write: Some(Write {
        value: Some(Register::Rdx),
        literal: 0,
        offset: 1224,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
};
pub const REPAIR_FACTORY_CLAMP: Site = Site {
    name: "repair factory clamp",
    rva: Rva(0x00edaa7f),
    register: Register::Rbp,
    write: Some(Write {
        value: Some(Register::Rcx),
        literal: 0,
        offset: 1224,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
};
// The setting applies its own inline clamp; the hourly clamp above is separate.
pub const REPAIR_FACTORY_SETTING_CLAMP: Site = Site {
    name: "repair factory setting clamp",
    rva: Rva(0x00ee1753),
    register: Register::R11,
    write: Some(Write {
        value: Some(Register::Rcx),
        literal: 0,
        offset: 1224,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
};
pub const REPAIR_DOCKYARD_SETTING: Site = Site {
    name: "repair dockyard setting",
    rva: Rva(0x00f23420),
    register: Register::Rcx,
    write: Some(Write {
        value: Some(Register::Rdx),
        literal: 0,
        offset: 416,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
};
pub const REPAIR_DOCKYARD_COMMAND: Site = Site {
    name: "repair dockyard command",
    rva: Rva(0x00f24bf0),
    register: Register::R14,
    write: Some(Write {
        value: Some(Register::Rax),
        literal: 0,
        offset: 416,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
};
pub const REPAIR_DOCKYARD_CLAMP: Site = Site {
    name: "repair dockyard clamp",
    rva: Rva(0x00f2508a),
    register: Register::R14,
    write: Some(Write {
        value: Some(Register::Rcx),
        literal: 0,
        offset: 416,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
};
pub const REPAIR_DOCKYARDS_USED: Site = Site {
    name: "repair dockyards used",
    rva: Rva(0x00f2525f),
    register: Register::R14,
    write: Some(Write {
        value: Some(Register::R12),
        literal: 0,
        offset: 0x1a4,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
};
pub const NAVY_RESET_USED: Site = Site {
    name: "navy reset used",
    rva: Rva(0x00f15db6),
    register: Register::Rbx,
    write: Some(Write {
        value: Some(Register::R14),
        literal: 0,
        offset: 420,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
};
pub const NAVY_RESET_BUDGET: Site = Site {
    name: "navy reset budget",
    rva: Rva(0x00f15dbd),
    register: Register::Rbx,
    write: Some(Write {
        value: None,
        literal: 4294967295,
        offset: 416,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
};
pub const NAVY_REBUILD_USED: Site = Site {
    name: "navy rebuild used",
    rva: Rva(0x00f1efa6),
    register: Register::Rbx,
    write: Some(Write {
        value: Some(Register::Rbp),
        literal: 0,
        offset: 420,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
};
pub const NAVY_REBUILD_BUDGET: Site = Site {
    name: "navy rebuild budget",
    rva: Rva(0x00f1efac),
    register: Register::Rbx,
    write: Some(Write {
        value: None,
        literal: 4294967295,
        offset: 416,
        bytes: 4,
    }),
    adjustment: 0,
    base: false,
    retire: false,
};
pub const BASE_REPAIR_PRIORITY: Site = Site {
    name: "base repair priority",
    rva: Rva(0x00f23430),
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
};
pub const BASE_REPAIR_ALLOCATION: Site = Site {
    name: "base repair allocation",
    rva: Rva(0x00f2529e),
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
};
pub const BASE_QUEUE_PRUNE: Site = Site {
    name: "base queue prune",
    rva: Rva(0x00f15537),
    register: Register::R15,
    write: None,
    adjustment: 0,
    base: true,
    retire: false,
};
pub const BASE_QUEUE_INSERT: Site = Site {
    name: "base queue insert",
    rva: Rva(0x00f130ea),
    register: Register::Rbp,
    write: None,
    adjustment: 0,
    base: true,
    retire: false,
};
pub const BASE_QUEUE_ARRIVAL: Site = Site {
    name: "base queue arrival",
    rva: Rva(0x00f1ea90),
    register: Register::Rbp,
    write: None,
    adjustment: 0,
    base: true,
    retire: false,
};
pub const BASE_QUEUE_BULK: Site = Site {
    name: "base queue bulk",
    rva: Rva(0x00f134fb),
    register: Register::Rbp,
    write: None,
    adjustment: 0,
    base: true,
    retire: false,
};
pub const COMPOSITION_ATTACH_CLEAR: Site = Site {
    name: "composition attach clear",
    rva: Rva(0x00dd867e),
    register: Register::Rsi,
    write: None,
    adjustment: 1336,
    base: false,
    retire: false,
};
pub const COMPOSITION_DETACH_CLEAR: Site = Site {
    name: "composition detach clear",
    rva: Rva(0x00dd9163),
    register: Register::Rdi,
    write: None,
    adjustment: 1336,
    base: false,
    retire: false,
};
pub const REFIT_PRODUCTION_ADD: Site = Site {
    name: "refit production add",
    rva: Rva(0x00ff6288),
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
};
pub const REFIT_PRODUCTION_COMPLETE: Site = Site {
    name: "refit production complete",
    rva: Rva(0x00ff629f),
    register: Register::Rbx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const REFIT_PROGRESS_DAMAGE: Site = Site {
    name: "refit progress damage",
    rva: Rva(0x00ff61ef),
    register: Register::Rcx,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const REFIT_PROGRESS_REFRESH: Site = Site {
    name: "refit progress refresh",
    rva: Rva(0x019c525b),
    register: Register::R14,
    write: None,
    adjustment: 0,
    base: false,
    retire: false,
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
    REPAIR_FACTORY_SETTING_CLAMP,
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
    rva: Rva(0x01a1e040),
    argument: 0,
    adjustment: -8,
    base: false,
    retire: false,
};
pub const COMPOSITION_MERGE: Method = Method {
    name: "composition merge",
    rva: Rva(0x01a1e140),
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const COMPOSITION_ASSIGN: Method = Method {
    name: "composition assign",
    rva: Rva(0x00dca6a0),
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const COMPOSITION_ERASE: Method = Method {
    name: "composition erase",
    rva: Rva(0x01a1efe0),
    argument: 0,
    adjustment: -8,
    base: false,
    retire: false,
};
pub const COMPOSITION_FULFILLMENT: Method = Method {
    name: "composition fulfillment",
    rva: Rva(0x01a1f0d0),
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const COMPOSITION_RANGE: Method = Method {
    name: "composition range",
    rva: Rva(0x01a1f130),
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const COMPOSITION_ROSTER_ADD: Method = Method {
    name: "composition roster add",
    rva: Rva(0x01a1e260),
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const COMPOSITION_SHIP_ADD: Method = Method {
    name: "composition ship add",
    rva: Rva(0x01a1e390),
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const FLEET_DESTROY: Method = Method {
    name: "fleet destroy",
    rva: Rva(0x00db7240),
    argument: 0,
    adjustment: 0,
    base: false,
    retire: true,
};
pub const TASK_DESTROY: Method = Method {
    name: "task destroy",
    rva: Rva(0x00dc99e0),
    argument: 0,
    adjustment: 0,
    base: false,
    retire: true,
};
pub const SHIP_DESTROY: Method = Method {
    name: "ship destroy",
    rva: Rva(0x00c86a90),
    argument: 0,
    adjustment: 0,
    base: false,
    retire: true,
};
pub const REFIT_DESTROY: Method = Method {
    name: "refit destroy",
    rva: Rva(0x00ff6100),
    argument: 0,
    adjustment: 0,
    base: false,
    retire: true,
};
pub const BASE_DESTROY: Method = Method {
    name: "base destroy",
    rva: Rva(0x00f12ab0),
    argument: 0,
    adjustment: 0,
    base: true,
    retire: true,
};
pub const PRODUCTION_DESTROY: Method = Method {
    name: "production destroy",
    rva: Rva(0x00ecd4a0),
    argument: 0,
    adjustment: 0,
    base: false,
    retire: true,
};
pub const NAVY_DESTROY: Method = Method {
    name: "navy destroy",
    rva: Rva(0x00f11e90),
    argument: 0,
    adjustment: 0,
    base: false,
    retire: true,
};
pub const BASE_COUNTRY_REORDER: Method = Method {
    name: "base country reorder",
    rva: Rva(0x00f22790),
    argument: 0,
    adjustment: 0,
    base: true,
    retire: false,
};
pub const REFIT_RECALCULATE: Method = Method {
    name: "refit recalculate",
    rva: Rva(0x00ff6460),
    argument: 0,
    adjustment: 0,
    base: false,
    retire: false,
};
pub const BASE_QUEUE_REORDER: Method = Method {
    name: "base queue reorder",
    rva: Rva(0x00f22620),
    argument: 0,
    adjustment: 0,
    base: true,
    retire: false,
};
pub const BASE_QUEUE_REMOVE: Method = Method {
    name: "base queue remove",
    rva: Rva(0x00f22580),
    argument: 0,
    adjustment: 0,
    base: true,
    retire: false,
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
