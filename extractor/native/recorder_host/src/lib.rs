pub mod auto_attach;
pub mod campaign_identity;
pub mod compact_domain_registry;
pub mod control_transport;
pub mod domain_data_authority;
pub mod duplex_session;
#[cfg(windows)]
pub mod early_render_host_channel;
pub mod early_render_launch;
pub mod emitted_token_inventory;
pub mod finalize_progress;
pub mod mmap;
pub mod modset;
pub mod portable_content_manifest;
pub mod proc_mem;
pub mod production_content_vfs;
pub mod production_map_content_index;
pub mod production_vfs_winner_binding;
pub mod runtime_dlc_roster;
pub mod scientist_source_report;
pub mod spool_trim;
pub mod static_bundle_gate;
pub mod stitching;
pub mod stream_fold;
mod tactic_weight_fold;
pub mod transport_status;
pub mod windows_control_pipe;
pub mod world_fold;

/// Upper bound for each authenticated early-launch handshake phase. A cold
/// continue-save can spend several minutes between first D3D creation and a
/// coherent world census, so this shares the safe launcher's world-ready
/// budget instead of imposing a separate 30-second startup assumption.
pub const MAX_EARLY_HOOK_TIMEOUT_MS: u32 =
    compact_contract::early_render_control::MAX_EARLY_RENDER_TIMEOUT_MS;
