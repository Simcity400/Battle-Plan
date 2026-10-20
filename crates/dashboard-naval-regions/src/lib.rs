//! Sparse minefield histories and country-viewpoint naval control.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod operands;
mod shared;
pub mod products {
    pub mod minefields;
    pub mod naval_supremacy;
    dashboard_base::reply_warnings!(
        minefields::HistoryReply,
        minefields::ReportsReply,
        naval_supremacy::Reply
    );
}
#[cfg(test)]
mod tests;
