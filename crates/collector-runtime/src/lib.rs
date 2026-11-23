#![recursion_limit = "512"]

#[cfg(test)]
mod air_registration_contract_tests;
#[cfg(test)]
mod allocation_failure_tests;
#[cfg(test)]
mod allocation_tests_support;
mod division_panel_forwarder;
mod hook_failures;
mod hook_inventory;
#[cfg(test)]
mod hook_sites_tests;
#[cfg(test)]
mod land_detach_tests;
#[cfg(test)]
mod operation_retry_tests;
mod progress;
pub mod runtime;
#[cfg(test)]
mod snapshot_integration_tests;
#[cfg(test)]
mod state_flags_publication_tests;
mod status_file;
#[cfg(test)]
mod status_tests;
mod status_writer;
#[cfg(test)]
mod template_worker_tests;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;

#[cfg(test)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static TEST_THREAD_STACK: unsafe extern "C" fn() = collector_test_support::reserve_test_stack;
