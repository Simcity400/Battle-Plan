//! Prepared air mission registrations and current annual-ring statistics.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod air_missions;
    pub mod air_statistics;

    dashboard_base::reply_warnings!(
        air_missions::HistoryReply,
        air_missions::WingsReply,
        air_statistics::Reply,
    );
}
mod shared;
pub mod theatres;
pub use shared::{Choice as AirOperationChoice, TheatreChoice as AirOperationTheatreChoice};
#[cfg(test)]
mod tests;
