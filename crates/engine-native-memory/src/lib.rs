#[cfg(test)]
#[global_allocator]
static ALLOCATOR: engine_allocation::test_support::DeniedAllocator =
    engine_allocation::test_support::DeniedAllocator;

#[cfg(feature = "acquisition_profile")]
pub mod acquisition_profile;
pub mod batch_fallback;
pub mod capture_text;
pub mod cdate;
pub mod certified_read;
pub mod cgame_date;
pub mod clock;
pub mod cpdx_array;
pub mod cpdx_lexer;
pub mod fixed5;
pub mod memory;
pub mod msvc_string;
pub mod msvc_tree_node;
pub mod prefetch;
pub mod raw_country_observations;
#[cfg(any(test, feature = "test-support"))]
pub mod read_census;
pub mod read_counts;
pub mod read_plan;
pub mod recent;
pub mod save_allocation_counters;
#[doc(hidden)]
pub mod slot_set;
pub mod walk;
pub mod country_identity;
pub mod modifier_cache;

pub use memory::{Memory, read, read_slice};
