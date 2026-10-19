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
            rva: 0x19823b0,
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
            rva: 0x19817e0,
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
            rva: 0x19826f0,
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
            rva: 0x197fc30,
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
            rva: 0x1980120,
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
            rva: 0x197ff30,
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
            rva: 0xd74fc0,
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
            rva: 0xd79580,
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
            rva: 0xd7a0a0,
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
            rva: 0xd7a090,
            sentinel: [
                0x48, 0x81, 0xc1, 0xf0, 0x08, 0x00, 0x00, 0xe9, 0xd4, 0xe5, 0xed, 0xff, 0xcc, 0xcc,
                0xcc, 0xcc,
            ],
        },
        Family::Manpower,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "set_manpower_unlocked",
            rva: 0x4a9f9a,
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
            rva: 0x4aa10a,
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
            rva: 0x1178b80,
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
            rva: 0x117a0d0,
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
            rva: 0x1178b10,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x01, 0x51, 0x78, 0x48, 0x8b, 0xd9, 0xe8,
                0x7e, 0xcb,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "reset_account",
            rva: 0x1179f20,
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
            rva: 0x11789e0,
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
            rva: 0x11778b0,
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
            rva: 0xd7c1f0,
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
            rva: 0xd7f9f0,
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
            rva: 0xd7f980,
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
            rva: 0x25968d,
            sentinel: [
                0x49, 0x0f, 0x48, 0xd7, 0x48, 0x89, 0x50, 0x48, 0x48, 0x8b, 0x0d, 0x44, 0xdc, 0x0b,
                0x03, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_1",
            rva: 0x1a13ca5,
            sentinel: [
                0x4d, 0x0f, 0x48, 0xc7, 0x4c, 0x89, 0x40, 0x48, 0xeb, 0x22, 0x48, 0x8b, 0xcb, 0xe8,
                0x99, 0xf0,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_2",
            rva: 0x1b943e0,
            sentinel: [
                0x4d, 0x0f, 0x48, 0xc6, 0x4c, 0x89, 0x47, 0x48, 0x48, 0x8b, 0xce, 0xe8, 0x50, 0x0d,
                0x01, 0xff,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_3",
            rva: 0x1b947d2,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc8, 0x4c, 0x89, 0x4f, 0x48, 0x8b, 0x56, 0x28, 0xe8, 0x5e, 0x1d,
                0x1e, 0xff,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_4",
            rva: 0x1b948cc,
            sentinel: [
                0x48, 0x01, 0x88, 0x80, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xcf, 0xe8, 0xa5, 0x76, 0xec,
                0xfe, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_5",
            rva: 0x1b949ec,
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
            rva: 0x1b94eec,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc0, 0x4c, 0x89, 0x47, 0x48, 0x38, 0x05, 0xf8, 0x3c, 0xa3, 0x01,
                0x0f, 0x84,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_7",
            rva: 0x1b95247,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc6, 0x4c, 0x89, 0x43, 0x48, 0x48, 0x8b, 0x0d, 0x8a, 0x20, 0x78,
                0x01, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_8",
            rva: 0x1b9553a,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc1, 0x4c, 0x89, 0x40, 0x48, 0x38, 0x0d, 0xaa, 0x36, 0xa3, 0x01,
                0x0f, 0x84,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_9",
            rva: 0x1980094,
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
            rva: 0x1980279,
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
            rva: 0x10f527a,
            sentinel: [
                0x4d, 0x0f, 0x48, 0xc7, 0x4c, 0x89, 0x40, 0x48, 0x48, 0x8b, 0x1d, 0x57, 0x20, 0x22,
                0x02, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_12",
            rva: 0x1b94330,
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
            rva: 0x1172337,
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
            rva: 0x11712d3,
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
            rva: 0x4a8e43,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc0, 0x4c, 0x89, 0x43, 0x48, 0x48, 0x8b, 0x0d, 0x8e, 0xe4, 0xe6,
                0x02, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_16",
            rva: 0xd70858,
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
            rva: 0xd7315c,
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
            rva: 0xd7c066,
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
            rva: 0x13e6930,
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
            rva: 0x1bd1180,
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
            rva: 0xd701f6,
            sentinel: [
                0x48, 0x89, 0x97, 0x88, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xcf, 0xe8, 0x0b, 0x89, 0x40,
                0x00, 0xff,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_contribution_gain_parallel",
            rva: 0xd72606,
            sentinel: [
                0x48, 0x89, 0x96, 0x88, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xce, 0xe8, 0xfb, 0x64, 0x40,
                0x00, 0xff,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_contribution_gain_leaf",
            rva: 0xd7bcaa,
            sentinel: [
                0x48, 0x89, 0x97, 0x88, 0x00, 0x00, 0x00, 0xe8, 0x5a, 0xce, 0x3f, 0x00, 0x49, 0x8b,
                0x0e, 0xff,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_previous_score_cohort",
            rva: 0xd70678,
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
            rva: 0xd729c2,
            sentinel: [
                0x48, 0x89, 0x87, 0xb0, 0x00, 0x00, 0x00, 0x49, 0x8b, 0xd5, 0x48, 0x8d, 0x4d, 0x1f,
                0xe8, 0xfb,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_previous_score_leaf",
            rva: 0xd7bde4,
            sentinel: [
                0x48, 0x89, 0x87, 0xb0, 0x00, 0x00, 0x00, 0xe8, 0xe0, 0x4d, 0x3f, 0x00, 0xba, 0x02,
                0x00, 0x00,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_contribution_influence",
            rva: 0x1178b63,
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
            rva: 0x117a1f0,
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
            rva: 0xd7ae8c,
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
            rva: 0xd70a20,
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
            rva: 0xd734cf,
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
            rva: 0xd7c15c,
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
            rva: 0xd7b37d,
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
            rva: 0xd7e0e0,
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
            rva: 0xd7ac00,
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
            rva: 0xd755f7,
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
            rva: 0xd7560f,
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
            rva: 0xd78c28,
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
            rva: 0xd7ada0,
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
            rva: 0xd7b4ba,
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
            rva: 0xd74f99,
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
            rva: 0xd75789,
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
            rva: 0x13e4a00,
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
            rva: 0x13e4f90,
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
            rva: 0x13e6360,
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
            rva: 0x13e65d0,
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
            rva: 0x13e6c50,
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
            rva: 0x13e7410,
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
            rva: 0xd7af20,
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
            rva: 0xd7b0e3,
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
            rva: 0x1170d50,
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
            rva: 0x1170c70,
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
            rva: 0x116f71d,
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
            rva: 0x1170e3d,
            sentinel: [
                0xe8, 0x7e, 0x5c, 0x57, 0xff, 0x4c, 0x63, 0xc0, 0x48, 0xbe, 0x9, 0xe1, 0xd1, 0xc6,
                0x11, 0x6b,
            ],
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "projection_other_ship_operand",
            rva: 0x1170e82,
            sentinel: [
                0xe8, 0x39, 0x5c, 0x57, 0xff, 0x48, 0x8b, 0x5c, 0x24, 0x30, 0x48, 0x63, 0xc8, 0x48,
                0x8b, 0xc6,
            ],
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "projection_combat_air_operand",
            rva: 0x116f5fd,
            sentinel: [
                0xe8, 0x7e, 0x72, 0x57, 0xff, 0x4c, 0x63, 0xc0, 0x48, 0xbe, 0x9, 0xe1, 0xd1, 0xc6,
                0x11, 0x6b,
            ],
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "projection_other_air_operand",
            rva: 0x116f642,
            sentinel: [
                0xe8, 0x39, 0x72, 0x57, 0xff, 0x48, 0x8b, 0x5c, 0x24, 0x30, 0x48, 0x63, 0xc8, 0x48,
                0x8b, 0xc6,
            ],
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "rank_input_sum",
            rva: 0xd7b141,
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
            rva: 0xd7b410,
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
            rva: 0xd78c70,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x50, 0x48, 0x8b, 0xd9, 0xb9, 0x32, 0x0, 0x0, 0x0,
                0xe8, 0xbd,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "influence_share_boundary",
            rva: 0xd7adf0,
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
