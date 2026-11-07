//! Recorded repair queues and live refit work, prepared as day events and read at an hour.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod products {
    pub mod naval_refits;
    pub mod naval_repairs;
    pub mod yards;

    // Both views reply with the view at an hour.
    dashboard_base::reply_warnings!(yards::at::YardsAtReply);
}

#[cfg(test)]
mod tests;
