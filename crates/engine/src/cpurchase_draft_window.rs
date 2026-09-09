//! NInternationalMarket::CPurchaseDraftWindow; FUN142034060 lambda RTTI identifies SetupDerived.

/// CPurchaseDraftWindow::SetupDerived RVA; old dynamic_collection_trace_live/src/a729_named_overlap_hook.rs:23 gates FUN142034060.
pub const SETUP_RVA: usize = 0x0203_4060;
/// CPurchaseDraftWindow setup entry bytes; old a729_named_overlap_hook.rs:73-75 verifies this sentinel before observation.
pub const SETUP_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
/// CPurchaseDraftWindow overlap widget pointer +0x1340; old a729_named_overlap_hook.rs:570 reads it after SetupDerived returns.
pub const OVERLAP: usize = 0x1340;
