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
use engine_sites::Rva;

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
