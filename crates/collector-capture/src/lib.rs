#![recursion_limit = "512"]

mod acquisition_countries;
mod acquisition_factions;
pub mod acquisition_jobs;
#[cfg(test)]
mod advisor_sizing_tests;
#[cfg(test)]
mod air_carrier_retirement_tests;
#[cfg(test)]
mod air_map_capture_tests;
pub mod capture_point;
pub mod command_batch;
pub mod country_scale;
pub mod decode;
mod division_detail;
pub mod extended;
pub mod frame_capacities;
pub mod frame_pool;
pub mod hourly_admission;
#[cfg(test)]
mod land_census_scale;
pub mod pack_conversion;
#[cfg(test)]
mod read_accounting_tests;
pub mod sampling;
#[cfg(test)]
mod states_worker_growth_tests;
#[cfg(test)]
mod template_worker_tests;
#[cfg(test)]
mod unit_scale_tests;
#[cfg(test)]
mod worker_job_tests;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
