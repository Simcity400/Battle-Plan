
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Status,
    Progress,
    Cancelled,
    PendingRbp,
    PendingRsi,
    FinishedFactionRsi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Patch {
    Entry,
    Call,
    Tap,
}

pub const MUTATIONS: [(MutationSite, Receiver, Patch); 19] = [
    (
        MutationSite {
            name: "construct_goal_status",
            rva: Rva(0xa722d0),
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "destroy_goal_status",
            rva: Rva(0xa72ac0),
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "add_goal_slots",
            rva: Rva(0xa73040),
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "activate_goal",
            rva: Rva(0xa73060),
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "insert_goal",
            rva: Rva(0xa73290),
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "remove_goal",
            rva: Rva(0xa75340),
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "select_manifest",
            rva: Rva(0xa75920),
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "initialize_goal_template",
            rva: Rva(0xa75c30),
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "load_goal_status",
            rva: Rva(0xa75090),
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "rebuild_goal_status",
            rva: Rva(0xa74910),
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "refresh_goal_progress",
            rva: Rva(0xa7668e),
        },
        Receiver::Progress,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "finish_goal_completion",
            rva: Rva(0xa743e9),
        },
        Receiver::FinishedFactionRsi,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "append_completed_goal",
            rva: Rva(0xa74e00),
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "append_cancelled_goal",
            rva: Rva(0xa74269),
        },
        Receiver::Cancelled,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "pend_goal_completion",
            rva: Rva(0xa73971),
        },
        Receiver::PendingRbp,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "pend_goal_cancellation",
            rva: Rva(0xa73987),
        },
        Receiver::PendingRbp,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "pend_initial_goal_completion",
            rva: Rva(0xa73f91),
        },
        Receiver::PendingRsi,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "pend_initial_goal_cancellation",
            rva: Rva(0xa73fa7),
        },
        Receiver::PendingRsi,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "finish_goal_cancellation",
            rva: Rva(0xa74513),
        },
        Receiver::FinishedFactionRsi,
        Patch::Tap,
    ),
];
