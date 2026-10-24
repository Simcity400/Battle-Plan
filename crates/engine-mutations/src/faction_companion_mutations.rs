//! Sparse faction rules, programme references and manpower pool writers.
//! Native world replacement invalidates their certificates. Construction and
//! deletion are bounded by the authenticated faction handle census. Daily
//! programme awards do not mutate the programme reference list and get no hook.
pub use crate::advisor_mutations::MutationSite;
#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum Family {
    Rules,
    Programmes,
    Manpower,
    Unlocks,
    Accounts,
    Minimum,
    Doctrine,
    Pools,
    Advisors,
}
#[derive(Clone, Copy, Debug)]
pub enum Patch {
    Entry,
    Five,
    Call,
    Tap,
}
/// Site, independent writer domain and native patch kind.
pub const MUTATIONS: [(MutationSite, Family, Patch); SITES] = [
    (
        MutationSite {
            name: "select_rule",
            rva: 0x1995a60,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x55, 0x57, 0x41, 0x56,
                0x48, 0x8d,
            ],
        },
        Family::Rules,
        Patch::Five,
    ),
    (
        MutationSite {
            name: "remove_rule",
            rva: 0x1994e90,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
                0x20, 0x48,
            ],
        },
        Family::Rules,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "reset_rule_template",
            rva: 0x1995da0,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x48, 0x89, 0x48, 0x08, 0x55, 0x53, 0x56, 0x57, 0x48, 0x8d, 0x68,
                0xa1, 0x48,
            ],
        },
        Family::Rules,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "add_programme",
            rva: 0x19932e0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec,
                0x30, 0x48,
            ],
        },
        Family::Programmes,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "remove_programme",
            rva: 0x19937d0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48,
                0x8b, 0xfa,
            ],
        },
        Family::Programmes,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "remove_programme_country",
            rva: 0x19935e0,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x55, 0x56, 0x41, 0x55, 0x48, 0x81, 0xec, 0x90, 0x00, 0x00, 0x00,
                0x45, 0x33,
            ],
        },
        Family::Programmes,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "transfer_manpower",
            rva: 0xd897e0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x56, 0x57, 0x41, 0x54,
                0x41, 0x56,
            ],
        },
        Family::Manpower,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "refund_member_manpower",
            rva: 0xd8dda0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x48,
            ],
        },
        Family::Manpower,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "add_manpower_contribution",
            rva: 0xd8e8c0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x8b, 0x00, 0x48,
                0x8d, 0xb9,
            ],
        },
        Family::Manpower,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "clear_manpower_pool",
            rva: 0xd8e8b0,
            sentinel: [
                0x48, 0x81, 0xc1, 0xf0, 0x08, 0x00, 0x00, 0xe9, 0xf4, 0x94, 0xed, 0xff, 0xcc, 0xcc,
                0xcc, 0xcc,
            ],
        },
        Family::Manpower,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "set_manpower_unlocked",
            rva: 0x4abcca,
            sentinel: [
                0x88, 0x88, 0x39, 0x0a, 0, 0, 0x48, 0x83, 0xc4, 0x20, 0x5b, 0xc3, 0xcc, 0xcc, 0xcc,
                0xcc,
            ],
        },
        Family::Unlocks,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "set_research_unlocked",
            rva: 0x4abe3a,
            sentinel: [
                0x88, 0x88, 0x38, 0x0a, 0, 0, 0x48, 0x83, 0xc4, 0x20, 0x5b, 0xc3, 0xcc, 0xcc, 0xcc,
                0xcc,
            ],
        },
        Family::Unlocks,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "add_account_influence",
            rva: 0x118d8e0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
                0x30, 0x80,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "set_account_influence",
            rva: 0x118ee30,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
                0x30, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "add_account_contribution",
            rva: 0x118d870,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x01, 0x51, 0x78, 0x48, 0x8b, 0xd9, 0xe8,
                0x6e, 0x70,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "reset_account",
            rva: 0x118ec80,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x48, 0x83, 0xec,
                0x20, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "append_account_goal",
            rva: 0x118d740,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x48, 0x83, 0xec, 0x20, 0x4c, 0x63, 0x89, 0xc4,
                0x00, 0x00,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "select_account_upgrade",
            rva: 0x118c610,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57, 0x41, 0x56,
                0x48, 0x8b,
            ],
        },
        Family::Accounts,
        Patch::Five,
    ),
    (
        MutationSite {
            name: "append_account",
            rva: 0xd909d0,
            sentinel: [
                0x40, 0x53, 0x57, 0x48, 0x83, 0xec, 0x28, 0x48, 0x63, 0x41, 0x0c, 0x48, 0x8b, 0xf9,
                0x8b, 0x49,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "replace_account_storage",
            rva: 0xd941d0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "clear_accounts",
            rva: 0xd94160,
            sentinel: [
                0x40, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0x41, 0x0c, 0x48, 0x8b, 0xf9, 0x85, 0xc0,
                0x7e, 0x41,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "account_balance_store_0",
            rva: 0x259aad,
            sentinel: [
                0x49, 0x0f, 0x48, 0xd7, 0x48, 0x89, 0x50, 0x48, 0x48, 0x8b, 0x0d, 0xdc, 0x5b, 0x0d,
                0x03, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_1",
            rva: 0x1a27465,
            sentinel: [
                0x4d, 0x0f, 0x48, 0xc7, 0x4c, 0x89, 0x40, 0x48, 0xeb, 0x22, 0x48, 0x8b, 0xcb, 0xe8,
                0x19, 0x84,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_2",
            rva: 0x1ba9150,
            sentinel: [
                0x4d, 0x0f, 0x48, 0xc6, 0x4c, 0x89, 0x47, 0x48, 0x48, 0x8b, 0xce, 0xe8, 0x30, 0xb2,
                0x00, 0xff,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_3",
            rva: 0x1ba9542,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc8, 0x4c, 0x89, 0x4f, 0x48, 0x8b, 0x56, 0x28, 0xe8, 0x0e, 0x18,
                0x1e, 0xff,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_4",
            rva: 0x1ba963c,
            sentinel: [
                0x48, 0x01, 0x88, 0x80, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xcf, 0xe8, 0x15, 0x19, 0xec,
                0xfe, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_5",
            rva: 0x1ba975c,
            sentinel: [
                0x48, 0x29, 0x88, 0x80, 0x00, 0x00, 0x00, 0x39, 0x7b, 0x28, 0x75, 0x05, 0x39, 0x7b,
                0x2c, 0x74,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_6",
            rva: 0x1ba9c5c,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc0, 0x4c, 0x89, 0x47, 0x48, 0x38, 0x05, 0xe8, 0x7e, 0xa3, 0x01,
                0x0f, 0x84,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_7",
            rva: 0x1ba9fb7,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc6, 0x4c, 0x89, 0x43, 0x48, 0x48, 0x8b, 0x0d, 0xd2, 0x56, 0x78,
                0x01, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_8",
            rva: 0x1baa2aa,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc1, 0x4c, 0x89, 0x40, 0x48, 0x38, 0x0d, 0x9a, 0x78, 0xa3, 0x01,
                0x0f, 0x84,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_9",
            rva: 0x1993744,
            sentinel: [
                0x48, 0x01, 0x8b, 0x80, 0x00, 0x00, 0x00, 0x45, 0x33, 0xc0, 0x48, 0x8b, 0xd6, 0x49,
                0x8b, 0xce,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_10",
            rva: 0x1993929,
            sentinel: [
                0x48, 0x01, 0x8b, 0x80, 0x00, 0x00, 0x00, 0x4c, 0x3b, 0xc6, 0x75, 0xab, 0x48, 0x8b,
                0xbc, 0x24,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_11",
            rva: 0x1109fda,
            sentinel: [
                0x4d, 0x0f, 0x48, 0xc7, 0x4c, 0x89, 0x40, 0x48, 0x48, 0x8b, 0x1d, 0xaf, 0x56, 0x22,
                0x02, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_12",
            rva: 0x1ba90a0,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc8, 0x4c, 0x89, 0x4b, 0x48, 0x48, 0x8b, 0x57, 0x30, 0x41, 0xb1,
                0x01, 0x45,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_13",
            rva: 0x1187097,
            sentinel: [
                0x48, 0x0f, 0x48, 0xd0, 0x48, 0x89, 0x56, 0x48, 0x0f, 0x57, 0xc0, 0x33, 0xc0, 0x0f,
                0x11, 0x45,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_14",
            rva: 0x1186033,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xce, 0x4c, 0x89, 0x4b, 0x48, 0x4d, 0x8b, 0xce, 0x4c, 0x8b, 0xc7,
                0x41, 0x8b,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_15",
            rva: 0x4aab73,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc0, 0x4c, 0x89, 0x43, 0x48, 0x48, 0x8b, 0x0d, 0x16, 0x4b, 0xe8,
                0x02, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_16",
            rva: 0xd85098,
            sentinel: [
                0x49, 0x3, 0x48, 0x48, 0x48, 0xf, 0x48, 0xcf, 0x49, 0x89, 0x48, 0x48, 0x48, 0x83,
                0xc6, 0x8,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_17",
            rva: 0xd8797c,
            sentinel: [
                0x48, 0xf, 0x48, 0xcf, 0x49, 0x89, 0x48, 0x48, 0x48, 0xff, 0xc6, 0x49, 0x3b, 0xf5,
                0x75, 0x94,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_18",
            rva: 0xd90846,
            sentinel: [
                0x49, 0x3, 0x48, 0x48, 0x49, 0xf, 0x48, 0xcf, 0x48, 0xff, 0xc7, 0x49, 0x89, 0x48,
                0x48, 0x49,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "set_minimum_upgrade",
            rva: 0x13fbd20,
            sentinel: [
                0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x41, 0x56, 0x48, 0x83, 0xec, 0x48, 0x48, 0x8b,
                0xd9, 0x4d,
            ],
        },
        Family::Minimum,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "register_doctrine_folder",
            rva: 0x1be5ef0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x48, 0x83, 0xec, 0x20, 0x4c, 0x63, 0x49, 0x14,
                0x45, 0x33,
            ],
        },
        Family::Doctrine,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "store_contribution_gain_cohort",
            rva: 0xd84a26,
            sentinel: [
                0x48, 0x89, 0x97, 0x88, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xcf, 0xe8, 0x3b, 0x8e, 0x40,
                0x00, 0xff,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_contribution_gain_parallel",
            rva: 0xd86e46,
            sentinel: [
                0x48, 0x89, 0x96, 0x88, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xce, 0xe8, 0x1b, 0x6a, 0x40,
                0x00, 0xff,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_contribution_gain_leaf",
            rva: 0xd904ca,
            sentinel: [
                0x48, 0x89, 0x97, 0x88, 0x00, 0x00, 0x00, 0xe8, 0x9a, 0xd3, 0x3f, 0x00, 0x49, 0x8b,
                0x0e, 0xff,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_previous_score_cohort",
            rva: 0xd84eb8,
            sentinel: [
                0x48, 0x89, 0x83, 0xb0, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xd5, 0x48, 0x8d, 0x4c, 0x24,
                0x28, 0xe8,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_previous_score_parallel",
            rva: 0xd87202,
            sentinel: [
                0x48, 0x89, 0x87, 0xb0, 0x00, 0x00, 0x00, 0x49, 0x8b, 0xd5, 0x48, 0x8d, 0x4d, 0x1f,
                0xe8, 0x1b,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_previous_score_leaf",
            rva: 0xd90604,
            sentinel: [
                0x48, 0x89, 0x87, 0xb0, 0x00, 0x00, 0x00, 0xe8, 0x20, 0x53, 0x3f, 0x00, 0xba, 0x02,
                0x00, 0x00,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_contribution_influence",
            rva: 0x118d8c3,
            sentinel: [
                0x48, 0x8b, 0x43, 0x08, 0x48, 0x89, 0x50, 0x18, 0x48, 0x83, 0xc4, 0x20, 0x5b, 0xc3,
                0xcc, 0xcc,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "set_member_projection",
            rva: 0x118ef50,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
                0x30, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "store_member_influence_share",
            rva: 0xd8f6ac,
            sentinel: [
                0x49, 0x8b, 0x8e, 0x20, 0x8, 0x0, 0x0, 0x33, 0xd2, 0x48, 0x85, 0xc0, 0x48, 0xf,
                0x4f, 0xd0,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_member_projection_share_cohort",
            rva: 0xd85260,
            sentinel: [
                0x49, 0x8b, 0x4e, 0x8, 0x48, 0x8b, 0x9, 0x48, 0x85, 0xc9, 0x75, 0x4, 0x33, 0xff,
                0xeb, 0x1e,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_member_projection_share_parallel",
            rva: 0xd87cef,
            sentinel: [
                0x48, 0x8b, 0x4e, 0x8, 0x48, 0x8b, 0x9, 0x48, 0x85, 0xc9, 0x75, 0x4, 0x33, 0xf6,
                0xeb, 0x27,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_member_projection_share_leaf",
            rva: 0xd9093c,
            sentinel: [
                0x49, 0x8b, 0x4e, 0x8, 0x48, 0x8b, 0x9, 0x48, 0x85, 0xc9, 0x75, 0x4, 0x33, 0xff,
                0xeb, 0x1e,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_member_projection_rank",
            rva: 0xd8fb9d,
            sentinel: [
                0x89, 0x71, 0x50, 0xff, 0xc7, 0x48, 0x83, 0xc3, 0x10, 0xff, 0xc6, 0x3b, 0x7c, 0x24,
                0x3c, 0x7c,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "complete_system_extraction",
            rva: 0xd928c0,
            sentinel: [
                0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0x6c,
                0x24, 0x80,
            ],
        },
        Family::Pools,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "influence_refresh_boundary",
            rva: 0xd8f420,
            sentinel: [
                0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x56, 0x48, 0x8d, 0xac, 0x24, 0x50, 0xf3, 0xff,
                0xff, 0x48,
            ],
        },
        Family::Pools,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "store_projection_effect_accumulator",
            rva: 0xd89e17,
            sentinel: [
                0x48, 0x89, 0x81, 0x28, 0x08, 0x00, 0x00, 0x4c, 0x8b, 0x81, 0x18, 0x08, 0x00, 0x00,
                0x4c, 0x03,
            ],
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_projection_effect_total",
            rva: 0xd89e2f,
            sentinel: [
                0x4c, 0x89, 0x89, 0x18, 0x08, 0x00, 0x00, 0xc3, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
                0xcc, 0xcc,
            ],
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_influence_refresh_total",
            rva: 0xd8d448,
            sentinel: [
                0x48, 0x89, 0x9f, 0x20, 0x08, 0x00, 0x00, 0xe8, 0xbc, 0x21, 0x00, 0x00, 0x48, 0x8b,
                0xcf, 0xe8,
            ],
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_influence_accounting_total",
            rva: 0xd8f5c0,
            sentinel: [
                0x48, 0x89, 0x9e, 0x20, 0x08, 0x00, 0x00, 0x48, 0x8b, 0xce, 0xe8, 0x41, 0x00, 0x00,
                0x00, 0x48,
            ],
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_projection_refresh_total",
            rva: 0xd8fcda,
            sentinel: [
                0x48, 0x89, 0x8d, 0x18, 0x08, 0x00, 0x00, 0x48, 0x83, 0xc4, 0x68, 0x5f, 0x5e, 0x5d,
                0x5b, 0xc3,
            ],
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "add_influence_award_total",
            rva: 0xd897b9,
            sentinel: [
                0x48, 0x01, 0x9d, 0x20, 0x08, 0x00, 0x00, 0x48, 0x8b, 0xcd, 0xe8, 0x48, 0x5e, 0x00,
                0x00, 0x48,
            ],
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "add_influence_effect_total",
            rva: 0xd89fa9,
            sentinel: [
                0x48, 0x01, 0x9d, 0x20, 0x08, 0x00, 0x00, 0x48, 0x8b, 0xcd, 0xe8, 0x58, 0x56, 0x00,
                0x00, 0x48,
            ],
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "initialize_advisor_slots",
            rva: 0x13f9df0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24,
                0x08, 0x56,
            ],
        },
        Family::Advisors,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "initialize_advisor_modifier_names",
            rva: 0x13fa380,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57, 0x41, 0x54,
                0x41, 0x56,
            ],
        },
        Family::Advisors,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "load_advisor_slots",
            rva: 0x13fb750,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x81, 0xec,
                0xf0, 0x01,
            ],
        },
        Family::Advisors,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "clear_departing_advisor_slots",
            rva: 0x13fb9c0,
            sentinel: [
                0x89, 0x54, 0x24, 0x10, 0x55, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8b, 0xec,
                0x48, 0x83,
            ],
        },
        Family::Advisors,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "select_advisor_slot",
            rva: 0x13fc040,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x44, 0x89, 0x40, 0x18, 0x56, 0x48, 0x83, 0xec, 0x60, 0x48, 0x63,
                0xf2, 0x83,
            ],
        },
        Family::Advisors,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "resize_advisor_slots",
            rva: 0x13fc800,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x48,
            ],
        },
        Family::Advisors,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "rank_refresh_boundary",
            rva: 0xd8f740,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
                0x20, 0x55,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "rank_input_receiver",
            rva: 0xd8f903,
            sentinel: [
                0x49, 0x8b, 0x9c, 0x24, 0x30, 0x8, 0x0, 0x0, 0x49, 0x8d, 0x4d, 0x8, 0x83, 0x39,
                0x0, 0x75,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "projection_resource_operand",
            rva: 0x1185ab0,
            sentinel: [
                0x49, 0xb9, 0x9, 0xe1, 0xd1, 0xc6, 0x11, 0x6b, 0xf1, 0x29, 0x49, 0x8b, 0xc1, 0x48,
                0xf7, 0xeb,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "projection_factory_operand",
            rva: 0x11859d0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "projection_equipment_operand",
            rva: 0x118447d,
            sentinel: [
                0x48, 0xb8, 0x9, 0xe1, 0xd1, 0xc6, 0x11, 0x6b, 0xf1, 0x29, 0x48, 0x8b, 0x7c, 0x24,
                0x40, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "projection_heavy_ship_operand",
            rva: 0x1185b9d,
            sentinel: [
                0xe8, 0x8e, 0xda, 0x56, 0xff, 0x4c, 0x63, 0xc0, 0x48, 0xbe, 0x9, 0xe1, 0xd1, 0xc6,
                0x11, 0x6b,
            ],
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "projection_other_ship_operand",
            rva: 0x1185be2,
            sentinel: [
                0xe8, 0x49, 0xda, 0x56, 0xff, 0x48, 0x8b, 0x5c, 0x24, 0x30, 0x48, 0x63, 0xc8, 0x48,
                0x8b, 0xc6,
            ],
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "projection_combat_air_operand",
            rva: 0x118435d,
            sentinel: [
                0xe8, 0x8e, 0xf0, 0x56, 0xff, 0x4c, 0x63, 0xc0, 0x48, 0xbe, 0x9, 0xe1, 0xd1, 0xc6,
                0x11, 0x6b,
            ],
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "projection_other_air_operand",
            rva: 0x11843a2,
            sentinel: [
                0xe8, 0x49, 0xf0, 0x56, 0xff, 0x48, 0x8b, 0x5c, 0x24, 0x30, 0x48, 0x63, 0xc8, 0x48,
                0x8b, 0xc6,
            ],
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "rank_input_sum",
            rva: 0xd8f961,
            sentinel: [
                0x48, 0xf, 0x4f, 0xd8, 0x41, 0x3b, 0xfe, 0xf, 0x85, 0xa2, 0x0, 0x0, 0x0, 0x8d,
                0x47, 0x1,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "projection_refresh_boundary",
            rva: 0xd8fc30,
            sentinel: [
                0x4c, 0x8b, 0xdc, 0x53, 0x55, 0x56, 0x57, 0x48, 0x83, 0xec, 0x68, 0x48, 0x8b, 0x91,
                0x28, 0x8,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "passive_initiative_boundary",
            rva: 0xd8d490,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x50, 0x48, 0x8b, 0xd9, 0xb9, 0x32, 0x0, 0x0, 0x0,
                0xe8, 0xad,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "influence_share_boundary",
            rva: 0xd8f610,
            sentinel: [
                0x40, 0x56, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0x71, 0x58,
                0x4c, 0x8b,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
];

const SITES: usize = 83;
pub const ACCOUNT_STORE_FIRST: usize = 21;
