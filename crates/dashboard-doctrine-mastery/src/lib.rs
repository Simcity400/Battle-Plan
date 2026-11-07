//! Prepared doctrine mastery windows, independent of content presentation failures.

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod products {
    pub mod doctrine_mastery;

    dashboard_base::reply_warnings!(doctrine_mastery::DoctrineMasteryReply,);
}
