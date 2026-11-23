#[cfg(test)]
#[global_allocator]
static ALLOCATOR: engine_allocation::test_support::DeniedAllocator =
    engine_allocation::test_support::DeniedAllocator;

pub mod camera_root;
pub mod ccountry_characters;
pub mod ccountry_leader;
pub mod ccurrent_autonomy_status;
pub mod cdiplomacy_status;
pub mod cdoctrine_folder_template;
pub mod cdoctrine_sharing_status;
#[cfg(test)]
mod cfaction_companions_tests;
pub mod cfaction_program_status;
pub mod cfaction_rule_status;
pub mod cgame_gui;
pub mod cgui;
pub mod character_leader_node;
pub mod character_portrait_entry;
pub mod character_portraits;
pub mod chuman;
pub mod cpolitical_party;
pub mod cpolitical_status;
pub mod ctechnology_sharing_faction;
pub mod equipment_module_entry;
pub mod equipment_upgrade_entry;
pub mod modifier_breakdown;
pub mod portrait_cache;
