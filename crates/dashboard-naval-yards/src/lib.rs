//! Recorded repair queues and live refit work, prepared as keyed histories.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod naval_refits;
    pub mod naval_repairs;
    pub mod yards;

    dashboard_base::reply_warnings!(
        naval_repairs::NavalRepairsReply,
        naval_refits::NavalRefitsReply,
    );
}

#[cfg(test)]
mod tests;
