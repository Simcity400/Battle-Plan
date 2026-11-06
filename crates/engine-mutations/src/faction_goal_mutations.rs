//! Writer boundaries for stored faction goal state. Sparse structural writers
//! notify their status; progress refresh compares its one held scalar; taps
//! notify only actual pending-status stores. The cancellation-history CALL is
//! observed at its owning occurrence, never through the shared vector helper.
//!
//! Insertion, removal, activation, manifest/template selection and load/rebuild
//! enclose every vector relocation or assignment of live goal data. Activation
//! encloses the term-date write after insertion. Transition completion includes
//! the native dates and status, while both historical append paths are distinct.
//! Constructor/destructor and world invalidation bound receiver lifetimes.

pub use crate::advisor_mutations::MutationSite;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Status,
    /// Goal-specific progress CALL: status, incoming cache and update flags;
    /// the caller also holds the faction in its nonvolatile owner register.
    Progress,
    /// First argument is the cancellation-history vector inside the status.
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
            rva: 0xa722d0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x48, 0x89, 0x4c, 0x24,
                0x08, 0x56,
            ],
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "destroy_goal_status",
            rva: 0xa72ac0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
                0x20, 0x48,
            ],
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "add_goal_slots",
            rva: 0xa73040,
            sentinel: [
                0x83, 0xfa, 0x03, 0x7d, 0x0b, 0x48, 0x8b, 0x41, 0x78, 0x48, 0x63, 0xd2, 0x44, 0x01,
                0x04, 0x90,
            ],
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "activate_goal",
            rva: 0xa73060,
            sentinel: [
                0x40, 0x53, 0x56, 0x48, 0x83, 0xec, 0x38, 0x4c, 0x89, 0x74, 0x24, 0x68, 0x48, 0x8b,
                0xf1, 0x4c,
            ],
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "insert_goal",
            rva: 0xa73290,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x44, 0x88, 0x4c, 0x24,
                0x20, 0x55,
            ],
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "remove_goal",
            rva: 0xa75340,
            sentinel: [
                0x40, 0x56, 0x57, 0x41, 0x56, 0x48, 0x83, 0xec, 0x40, 0x48, 0x63, 0x41, 0x24, 0x33,
                0xf6, 0x48,
            ],
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "select_manifest",
            rva: 0xa75920,
            sentinel: [
                0x40, 0x56, 0x57, 0x41, 0x56, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0xf9, 0x45, 0x0f,
                0xb6, 0xf0,
            ],
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "initialize_goal_template",
            rva: 0xa75c30,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "load_goal_status",
            rva: 0xa75090,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x81, 0xec,
                0x40, 0x01,
            ],
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "rebuild_goal_status",
            rva: 0xa74910,
            sentinel: [
                0x48, 0x89, 0x4c, 0x24, 0x08, 0x53, 0x55, 0x48, 0x83, 0xec, 0x38, 0x4c, 0x89, 0x6c,
                0x24, 0x30,
            ],
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "refresh_goal_progress",
            rva: 0xa7668e,
            sentinel: [
                0xe8, 0xad, 0x07, 0xb6, 0x00, 0x8b, 0xd8, 0x48, 0x8d, 0x4c, 0x24, 0x20, 0xe8, 0x51,
                0x5f, 0xb0,
            ],
        },
        Receiver::Progress,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "finish_goal_completion",
            rva: 0xa743e9,
            sentinel: [
                0xc7, 0x43, 0x08, 0x01, 0x00, 0x00, 0x00, 0x33, 0xc0, 0x48, 0x8b, 0x5c, 0x24, 0x48,
                0x48, 0x8b,
            ],
        },
        Receiver::FinishedFactionRsi,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "append_completed_goal",
            rva: 0xa74e00,
            sentinel: [
                0x48, 0x89, 0x6c, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0xea, 0x48,
                0x89, 0x54,
            ],
        },
        Receiver::Status,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "append_cancelled_goal",
            rva: 0xa74269,
            sentinel: [
                0xe8, 0xf2, 0xda, 0x6a, 0xff, 0x48, 0x81, 0xc6, 0x40, 0x05, 0x00, 0x00, 0x49, 0x3b,
                0xf7, 0x0f,
            ],
        },
        Receiver::Cancelled,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "pend_goal_completion",
            rva: 0xa73971,
            sentinel: [
                0xc7, 0x43, 0x08, 0x03, 0x00, 0x00, 0x00, 0x48, 0x8b, 0x0b, 0x48, 0x8b, 0xd7, 0xe8,
                0x2d, 0xa2,
            ],
        },
        Receiver::PendingRbp,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "pend_goal_cancellation",
            rva: 0xa73987,
            sentinel: [
                0xc7, 0x43, 0x08, 0x04, 0x00, 0x00, 0x00, 0x48, 0x81, 0xc3, 0x40, 0x05, 0x00, 0x00,
                0x48, 0x3b,
            ],
        },
        Receiver::PendingRbp,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "pend_initial_goal_completion",
            rva: 0xa73f91,
            sentinel: [
                0xc7, 0x43, 0x08, 0x03, 0x00, 0x00, 0x00, 0x48, 0x8b, 0x0b, 0x48, 0x8b, 0xd7, 0xe8,
                0x0d, 0x9c,
            ],
        },
        Receiver::PendingRsi,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "pend_initial_goal_cancellation",
            rva: 0xa73fa7,
            sentinel: [
                0xc7, 0x43, 0x08, 0x04, 0x00, 0x00, 0x00, 0x48, 0x81, 0xc3, 0x40, 0x05, 0x00, 0x00,
                0x48, 0x3b,
            ],
        },
        Receiver::PendingRsi,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "finish_goal_cancellation",
            rva: 0xa74513,
            sentinel: [
                0xc7, 0x43, 0x08, 0x02, 0x00, 0x00, 0x00, 0x48, 0x8b, 0x5c, 0x24, 0x48, 0x48, 0x8b,
                0x74, 0x24,
            ],
        },
        Receiver::FinishedFactionRsi,
        Patch::Tap,
    ),
];
