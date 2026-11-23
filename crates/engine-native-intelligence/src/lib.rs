#[cfg(test)]
#[global_allocator]
static ALLOCATOR: engine_allocation::test_support::DeniedAllocator =
    engine_allocation::test_support::DeniedAllocator;

pub mod agency_upgrade_branch;
pub mod agency_upgrade_branch_database;
pub mod agency_upgrade_levels;
pub mod cagency_upgrade;
pub mod ccaptured_operative_reference;
pub mod ccountry_decryption_state;
pub mod ccountry_intel;
pub mod ccryptology;
pub mod cintelligence_agency;
pub mod coperation;
pub mod coperation_instance;
pub mod coperative_leader;
pub mod coperative_recruitment;
pub mod country_intel_row;
pub mod country_memory;
pub mod operation_assigned_operative;
pub mod operative_action;
pub mod operative_action_implementation;
pub mod operative_missions;
