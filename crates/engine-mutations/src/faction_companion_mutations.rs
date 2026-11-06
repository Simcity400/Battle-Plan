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
            rva: 0x1a28ee0,
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
            rva: 0x1a28310,
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
            rva: 0x1a29220,
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
            rva: 0x1a26710,
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
            rva: 0x1a26c50,
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
            rva: 0x1a26a60,
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
            rva: 0xdf3fc0,
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
            rva: 0xdf86f0,
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
            rva: 0xdf9210,
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
            rva: 0xdf9200,
            sentinel: [
                0x48, 0x81, 0xc1, 0xf0, 0x08, 0x00, 0x00, 0xe9, 0xe4, 0xb8, 0xec, 0xff, 0xcc, 0xcc,
                0xcc, 0xcc,
            ],
        },
        Family::Manpower,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "set_manpower_unlocked",
            rva: 0x4c99ba,
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
            rva: 0x4c9b2a,
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
            rva: 0x121a590,
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
            rva: 0x121bae0,
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
            rva: 0x121a520,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x01, 0x51, 0x78, 0x48, 0x8b, 0xd9, 0xe8,
                0xbe, 0xd6,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "reset_account",
            rva: 0x121b930,
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
            rva: 0x121a3f0,
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
            rva: 0x12192c0,
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
            rva: 0xdfb6a0,
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
            rva: 0xdff3f0,
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
            rva: 0xdff380,
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
            rva: 0x270fcd,
            sentinel: [
                0x49, 0x0f, 0x48, 0xd7, 0x48, 0x89, 0x50, 0x48, 0x48, 0x8b, 0x0d, 0x2c, 0xcb, 0x16,
                0x03, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_1",
            rva: 0x1ab9b95,
            sentinel: [
                0x4d, 0x0f, 0x48, 0xc7, 0x4c, 0x89, 0x40, 0x48, 0xeb, 0x22, 0x48, 0x8b, 0xcb, 0xe8,
                0x39, 0xd8,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_2",
            rva: 0x1c367d0,
            sentinel: [
                0x4d, 0x0f, 0x48, 0xc6, 0x4c, 0x89, 0x47, 0x48, 0x48, 0x8b, 0xce, 0xe8, 0xb0, 0x0e,
                0xfd, 0xfe,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_3",
            rva: 0x1c36bc2,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc8, 0x4c, 0x89, 0x4f, 0x48, 0x8b, 0x56, 0x28, 0xe8, 0xee, 0xe9,
                0x1b, 0xff,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_4",
            rva: 0x1c36cbf,
            sentinel: [
                0x48, 0x01, 0x88, 0x80, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xcf, 0xe8, 0xc2, 0x55, 0xe8,
                0xfe, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_5",
            rva: 0x1c36ddc,
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
            rva: 0x1c372dc,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc0, 0x4c, 0x89, 0x47, 0x48, 0x38, 0x05, 0x27, 0x83, 0xa5, 0x01,
                0x0f, 0x84,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_7",
            rva: 0x1c37637,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc6, 0x4c, 0x89, 0x43, 0x48, 0x48, 0x8b, 0x0d, 0xc2, 0x64, 0x7a,
                0x01, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_8",
            rva: 0x1c3792a,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc1, 0x4c, 0x89, 0x40, 0x48, 0x38, 0x0d, 0xd9, 0x7c, 0xa5, 0x01,
                0x0f, 0x84,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_9",
            rva: 0x1a26bc4,
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
            rva: 0x1a26da9,
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
            rva: 0x1190c3a,
            sentinel: [
                0x4d, 0x0f, 0x48, 0xc7, 0x4c, 0x89, 0x40, 0x48, 0x48, 0x8b, 0x1d, 0xbf, 0xce, 0x24,
                0x02, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_12",
            rva: 0x1c36722,
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
            rva: 0x1213d97,
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
            rva: 0x1212d33,
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
            rva: 0x4c8863,
            sentinel: [
                0x4c, 0x0f, 0x48, 0xc0, 0x4c, 0x89, 0x43, 0x48, 0x48, 0x8b, 0x0d, 0x96, 0x52, 0xf1,
                0x02, 0x48,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_16",
            rva: 0xdf3ae3,
            sentinel: [
                0x49, 0x3, 0x48, 0x48, 0x48, 0xf, 0x48, 0xcd, 0xff, 0xc3, 0x49, 0x89, 0x48, 0x48,
                0x3b, 0xdf,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_18",
            rva: 0xdfade7,
            sentinel: [
                0x49, 0x3, 0x48, 0x48, 0x49, 0xf, 0x48, 0xcc, 0xff, 0xc3, 0x49, 0x89, 0x48, 0x48,
                0x3b, 0xdd,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "set_minimum_upgrade",
            rva: 0x1497a50,
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
            rva: 0x1c7cf10,
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
            rva: 0xdf33aa,
            sentinel: [
                0x48, 0x89, 0x95, 0x88, 0x00, 0x00, 0x00, 0xe8, 0x6a, 0x71, 0x42, 0x00, 0x49, 0x8b,
                0x0f, 0x41,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_contribution_gain_leaf",
            rva: 0xdfb199,
            sentinel: [
                0x48, 0x89, 0x96, 0x88, 0x00, 0x00, 0x00, 0xe8, 0x7b, 0xf3, 0x41, 0x00, 0x49, 0x8b,
                0x0e, 0xff,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_previous_score_cohort",
            rva: 0xdf3608,
            sentinel: [
                0x48, 0x89, 0x85, 0xb0, 0x00, 0x00, 0x00, 0xe8, 0x1c, 0xf0, 0x41, 0x00, 0xba, 0x02,
                0x00, 0x00,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_previous_score_leaf",
            rva: 0xdfb327,
            sentinel: [
                0x48, 0x89, 0x87, 0xb0, 0x00, 0x00, 0x00, 0xe8, 0xfd, 0x72, 0x41, 0x00, 0xba, 0x02,
                0x00, 0x00,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_contribution_influence",
            rva: 0x121a573,
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
            rva: 0x121bc00,
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
            rva: 0xdfa0cc,
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
            rva: 0xdf3bbc,
            sentinel: [
                0x49, 0x8b, 0x4f, 0x10, 0x48, 0x8b, 0x9, 0x48, 0x85, 0xc9, 0x75, 0x4, 0x33, 0xed,
                0xeb, 0x1e,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_member_projection_share_leaf",
            rva: 0xdfaf0c,
            sentinel: [
                0x49, 0x8b, 0x4e, 0x38, 0x48, 0x8b, 0x9, 0x48, 0x85, 0xc9, 0x75, 0x4, 0x33, 0xf6,
                0xeb, 0x1e,
            ],
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_member_projection_rank",
            rva: 0xdfa5bd,
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
            rva: 0xdfda80,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
                0x20, 0x55,
            ],
        },
        Family::Pools,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "influence_refresh_boundary",
            rva: 0xdf9d70,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d,
                0xac, 0x24,
            ],
        },
        Family::Pools,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "store_projection_effect_accumulator",
            rva: 0xdf4677,
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
            rva: 0xdf468f,
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
            rva: 0xdf7d0e,
            sentinel: [
                0x48, 0x89, 0x9e, 0x20, 0x08, 0x00, 0x00, 0xe8, 0x16, 0x23, 0x00, 0x00, 0x48, 0x8b,
                0xce, 0xe8,
            ],
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_influence_accounting_total",
            rva: 0xdf9fc7,
            sentinel: [
                0x48, 0x89, 0x9e, 0x20, 0x08, 0x00, 0x00, 0x48, 0x8b, 0xce, 0xe8, 0x5a, 0x00, 0x00,
                0x00, 0x48,
            ],
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_projection_refresh_total",
            rva: 0xdfa7c4,
            sentinel: [
                0x48, 0x89, 0x8e, 0x18, 0x08, 0x00, 0x00, 0x48, 0x81, 0xc4, 0xd8, 0x00, 0x00, 0x00,
                0x41, 0x5f,
            ],
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "add_influence_award_total",
            rva: 0xdf3f99,
            sentinel: [
                0x48, 0x01, 0x9d, 0x20, 0x08, 0x00, 0x00, 0x48, 0x8b, 0xcd, 0xe8, 0x88, 0x60, 0x00,
                0x00, 0x48,
            ],
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "add_influence_effect_total",
            rva: 0xdf4809,
            sentinel: [
                0x48, 0x01, 0x9d, 0x20, 0x08, 0x00, 0x00, 0x48, 0x8b, 0xcd, 0xe8, 0x18, 0x58, 0x00,
                0x00, 0x48,
            ],
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "initialize_advisor_slots",
            rva: 0x1495b20,
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
            rva: 0x14960b0,
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
            rva: 0x1497480,
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
            rva: 0x14976f0,
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
            rva: 0x1497d70,
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
            rva: 0x1498530,
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
            rva: 0xdfa160,
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
            rva: 0xdfa323,
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
            rva: 0x12127b0,
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
            rva: 0x12126d0,
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
            rva: 0x121117d,
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
            rva: 0x121289d,
            sentinel: [
                0xe8, 0x8e, 0x89, 0x52, 0xff, 0x4c, 0x63, 0xc0, 0x48, 0xbe, 0x9, 0xe1, 0xd1, 0xc6,
                0x11, 0x6b,
            ],
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "projection_other_ship_operand",
            rva: 0x12128e2,
            sentinel: [
                0xe8, 0x49, 0x89, 0x52, 0xff, 0x48, 0x8b, 0x5c, 0x24, 0x30, 0x48, 0x63, 0xc8, 0x48,
                0x8b, 0xc6,
            ],
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "projection_combat_air_operand",
            rva: 0x121105d,
            sentinel: [
                0xe8, 0x8e, 0x9f, 0x52, 0xff, 0x4c, 0x63, 0xc0, 0x48, 0xbe, 0x9, 0xe1, 0xd1, 0xc6,
                0x11, 0x6b,
            ],
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "projection_other_air_operand",
            rva: 0x12110a2,
            sentinel: [
                0xe8, 0x49, 0x9f, 0x52, 0xff, 0x48, 0x8b, 0x5c, 0x24, 0x30, 0x48, 0x63, 0xc8, 0x48,
                0x8b, 0xc6,
            ],
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "rank_input_sum",
            rva: 0xdfa381,
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
            rva: 0xdfa650,
            sentinel: [
                0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0x6c, 0x24, 0xd1,
                0x48, 0x81,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "passive_initiative_boundary",
            rva: 0xdf7d50,
            sentinel: [
                0x40, 0x53, 0x48, 0x81, 0xec, 0xa0, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xd9, 0xb9, 0x32,
                0x00, 0x00,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "influence_share_boundary",
            rva: 0xdfa030,
            sentinel: [
                0x40, 0x56, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0x71, 0x58,
                0x4c, 0x8b,
            ],
        },
        Family::Accounts,
        Patch::Entry,
    ),
];

const SITES: usize = 79;
pub const ACCOUNT_STORE_FIRST: usize = 21;
