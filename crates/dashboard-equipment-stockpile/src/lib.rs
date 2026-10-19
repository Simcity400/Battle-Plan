//! Equipment stockpile: one country's stockpile and production of each variant over a branch, with dated variant artwork.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! module keeps its dashboard path (`products::equipment_stockpile`) and names
//! what it uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod equipment_stockpile;

    dashboard_base::reply_warnings!(
        equipment_stockpile::EquipmentStockpileReply,
        equipment_stockpile::artwork::StockpileArtworkReply,
    );
}
