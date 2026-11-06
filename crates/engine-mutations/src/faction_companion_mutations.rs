//! Sparse faction rules, programme references and manpower pool writers.
//! Native world replacement invalidates their certificates. Construction and
//! deletion are bounded by the authenticated faction handle census. Daily
//! programme awards do not mutate the programme reference list and get no hook.
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;
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
            rva: Rva(0x1a28ee0),
        },
        Family::Rules,
        Patch::Five,
    ),
    (
        MutationSite {
            name: "remove_rule",
            rva: Rva(0x1a28310),
        },
        Family::Rules,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "reset_rule_template",
            rva: Rva(0x1a29220),
        },
        Family::Rules,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "add_programme",
            rva: Rva(0x1a26710),
        },
        Family::Programmes,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "remove_programme",
            rva: Rva(0x1a26c50),
        },
        Family::Programmes,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "remove_programme_country",
            rva: Rva(0x1a26a60),
        },
        Family::Programmes,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "transfer_manpower",
            rva: Rva(0xdf3fc0),
        },
        Family::Manpower,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "refund_member_manpower",
            rva: Rva(0xdf86f0),
        },
        Family::Manpower,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "add_manpower_contribution",
            rva: Rva(0xdf9210),
        },
        Family::Manpower,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "clear_manpower_pool",
            rva: Rva(0xdf9200),
        },
        Family::Manpower,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "set_manpower_unlocked",
            rva: Rva(0x4c99ba),
        },
        Family::Unlocks,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "set_research_unlocked",
            rva: Rva(0x4c9b2a),
        },
        Family::Unlocks,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "add_account_influence",
            rva: Rva(0x121a590),
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "set_account_influence",
            rva: Rva(0x121bae0),
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "add_account_contribution",
            rva: Rva(0x121a520),
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "reset_account",
            rva: Rva(0x121b930),
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "append_account_goal",
            rva: Rva(0x121a3f0),
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "select_account_upgrade",
            rva: Rva(0x12192c0),
        },
        Family::Accounts,
        Patch::Five,
    ),
    (
        MutationSite {
            name: "append_account",
            rva: Rva(0xdfb6a0),
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "replace_account_storage",
            rva: Rva(0xdff3f0),
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "clear_accounts",
            rva: Rva(0xdff380),
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "account_balance_store_0",
            rva: Rva(0x270fcd),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_1",
            rva: Rva(0x1ab9b95),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_2",
            rva: Rva(0x1c367d0),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_3",
            rva: Rva(0x1c36bc2),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_4",
            rva: Rva(0x1c36cbf),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_5",
            rva: Rva(0x1c36ddc),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_6",
            rva: Rva(0x1c372dc),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_7",
            rva: Rva(0x1c37637),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_8",
            rva: Rva(0x1c3792a),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_9",
            rva: Rva(0x1a26bc4),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_10",
            rva: Rva(0x1a26da9),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_11",
            rva: Rva(0x1190c3a),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_12",
            rva: Rva(0x1c36722),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_13",
            rva: Rva(0x1213d97),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_14",
            rva: Rva(0x1212d33),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_15",
            rva: Rva(0x4c8863),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_16",
            rva: Rva(0xdf3ae3),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "account_balance_store_18",
            rva: Rva(0xdfade7),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "set_minimum_upgrade",
            rva: Rva(0x1497a50),
        },
        Family::Minimum,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "register_doctrine_folder",
            rva: Rva(0x1c7cf10),
        },
        Family::Doctrine,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "store_contribution_gain_cohort",
            rva: Rva(0xdf33aa),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_contribution_gain_leaf",
            rva: Rva(0xdfb199),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_previous_score_cohort",
            rva: Rva(0xdf3608),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_previous_score_leaf",
            rva: Rva(0xdfb327),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_contribution_influence",
            rva: Rva(0x121a573),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "set_member_projection",
            rva: Rva(0x121bc00),
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "store_member_influence_share",
            rva: Rva(0xdfa0cc),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_member_projection_share_cohort",
            rva: Rva(0xdf3bbc),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_member_projection_share_leaf",
            rva: Rva(0xdfaf0c),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_member_projection_rank",
            rva: Rva(0xdfa5bd),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "complete_system_extraction",
            rva: Rva(0xdfda80),
        },
        Family::Pools,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "influence_refresh_boundary",
            rva: Rva(0xdf9d70),
        },
        Family::Pools,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "store_projection_effect_accumulator",
            rva: Rva(0xdf4677),
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_projection_effect_total",
            rva: Rva(0xdf468f),
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_influence_refresh_total",
            rva: Rva(0xdf7d0e),
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_influence_accounting_total",
            rva: Rva(0xdf9fc7),
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "store_projection_refresh_total",
            rva: Rva(0xdfa7c4),
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "add_influence_award_total",
            rva: Rva(0xdf3f99),
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "add_influence_effect_total",
            rva: Rva(0xdf4809),
        },
        Family::Pools,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "initialize_advisor_slots",
            rva: Rva(0x1495b20),
        },
        Family::Advisors,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "initialize_advisor_modifier_names",
            rva: Rva(0x14960b0),
        },
        Family::Advisors,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "load_advisor_slots",
            rva: Rva(0x1497480),
        },
        Family::Advisors,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "clear_departing_advisor_slots",
            rva: Rva(0x14976f0),
        },
        Family::Advisors,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "select_advisor_slot",
            rva: Rva(0x1497d70),
        },
        Family::Advisors,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "resize_advisor_slots",
            rva: Rva(0x1498530),
        },
        Family::Advisors,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "rank_refresh_boundary",
            rva: Rva(0xdfa160),
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "rank_input_receiver",
            rva: Rva(0xdfa323),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "projection_resource_operand",
            rva: Rva(0x12127b0),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "projection_factory_operand",
            rva: Rva(0x12126d0),
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "projection_equipment_operand",
            rva: Rva(0x121117d),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "projection_heavy_ship_operand",
            rva: Rva(0x121289d),
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "projection_other_ship_operand",
            rva: Rva(0x12128e2),
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "projection_combat_air_operand",
            rva: Rva(0x121105d),
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "projection_other_air_operand",
            rva: Rva(0x12110a2),
        },
        Family::Accounts,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "rank_input_sum",
            rva: Rva(0xdfa381),
        },
        Family::Accounts,
        Patch::Tap,
    ),
    (
        MutationSite {
            name: "projection_refresh_boundary",
            rva: Rva(0xdfa650),
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "passive_initiative_boundary",
            rva: Rva(0xdf7d50),
        },
        Family::Accounts,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "influence_share_boundary",
            rva: Rva(0xdfa030),
        },
        Family::Accounts,
        Patch::Entry,
    ),
];

const SITES: usize = 79;
pub const ACCOUNT_STORE_FIRST: usize = 21;
