//! Air superiority: each strategic region's friendly and enemy air scores, as moment windows and daily timelines.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! module keeps its dashboard path (`products::air_superiority`) and names what
//! it uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

pub mod products {
    pub mod air_superiority;
}
