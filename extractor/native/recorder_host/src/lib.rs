#[cfg(test)]
mod politics_fixture;
#[cfg(test)]
mod map_control_fixture;

pub mod attach_content_proof;
pub mod auto_attach;
pub mod build_declaration;
pub mod campaign_identity;
pub mod compact_domain_registry;
pub mod content_census_cache;
pub mod control_transport;
pub mod direct_hourly_service;
pub mod domain_data_authority;
pub mod duplex_session;
pub mod durable_publish;
#[cfg(windows)]
pub mod early_render_host_channel;
pub mod early_render_launch;
pub mod emitted_token_inventory;
pub mod finalize_progress;
pub mod game_build;
pub mod gap_ledger;
pub mod harness_refusal;
pub mod hour_coverage;
pub mod hourly_service;
pub mod mem_account;
pub mod mmap;
pub mod modset;
pub mod native_design_catalog;
pub mod portable_content_manifest;
pub mod portable_manifest;
pub mod proc_mem;
pub mod production_content_vfs;
pub mod production_map_content_index;
pub mod production_vfs_winner_binding;
pub mod runtime_dlc_roster;
pub mod scientist_source_report;
pub mod sharing_violation_holders;
pub mod spool_trim;
pub mod static_artifact_epoch;
pub mod static_bundle_gate;
pub mod stitching;
pub mod stream_fold;
mod tactic_weight_fold;
pub mod transport_status;
pub mod windows_control_pipe;
pub mod world_fold;
pub mod xmap02_foundation_producer;
pub mod xmap14_combat_indicator_producer;

pub const SESSION_MANIFEST_SCHEMA: &str = "native_collector_session_v1";

/// Kernel buffer the capture named pipe is created with, both directions, and
/// the granularity the host drains it at.
///
/// These live in the LIBRARY rather than beside the drain loop in the binary
/// because a second place already depends on the read size:
/// [`auto_attach::proof::FirstCompactRecordGate`] bounds its own reassembly
/// buffer at "one maximal record plus one read chunk", and while the chunk was
/// 64 KiB that headroom was spelled as a bare `65_536`. Raising the read size
/// without raising that bound would make the gate refuse a capture whose first
/// record simply arrived in a bigger chunk, so the two numbers are one
/// constant now and cannot drift apart again.
///
/// WHY THEY ARE NOT 64 KiB (lane PN, 2026-08-21). The collector's sink is a
/// SYNCHRONOUS write of a whole frame's records — 8.9 MB per game hour on
/// `captures/oz_tip`, 4.63 GB over the capture. Through a 64 KiB pipe drained
/// 64 KiB at a time that is 136 kernel hand-offs per frame, and the collector's
/// own `rs_sink_time_total_us` measured **25.85 s inside a 60.76 s capture**
/// for bytes the host processed in 5.3 s (73,363 reads, `sink_write_us.avg`
/// 74.8 us, i.e. 875 MB/s). The collector saw an effective 179 MB/s. That
/// 74.8 us covers only `StreamWriter::write_all` — not the host's `ReadFile`,
/// hashing, proof parsing or scheduling — so the ~290 us of every 365 us
/// hand-off it does not account for is hand-off OVERHEAD, which is weaker than
/// "round-trip latency alone" and is all the evidence supports (Codex r1 #5).
/// What is certain is that it scales with the NUMBER of hand-offs, which is the
/// only thing these constants change.
///
/// A BLOCKED WRITER COSTS WALL, NOT CPU, so the claim here is the narrow one:
/// this latency sits in the publication path, and
/// `read_only_scheduler.rs::enqueue_request` evicts a pending hour — minting
/// `GapKind::Backpressure` for every domain — whenever a new hour arrives
/// before the previous one was taken up, the request ring being depth 1. 39 of
/// oz_tip's 53 lost hours are that eviction. The game separately runs at
/// 8.83 h/s attached against 18.20 h/s free-run (`CLOCK_BASELINE_V1.json`),
/// but this change is NOT shown to be a cause of that gap and no game-speed
/// improvement is claimed for it.
///
/// This is a TRANSPORT change, and the identity claim is scoped: the decoded
/// H4CR record stream and the spool bytes are identical to a 64 KiB capture,
/// because the same records in the same order reach the same `StreamWriter`.
/// PER-CALL telemetry does change, by construction — `spool_backpressure`'s
/// `sink_calls` falls ~16x and the `sink_write_us` histogram shifts, a call now
/// carrying 1 MiB instead of 64 KiB — and `memory_account` gains this path's
/// read buffer. Those are measurements OF the transport, not product semantics
/// (Codex r1 #4).
///
/// SIZES HERE ARE ADVISORY QUOTA, NOT A CEILING (Codex r1 #2). Windows may
/// round them, and a write larger than the available quota can make the system
/// expand pipe storage regardless — which is live here, since one frame is
/// ~8.9 MB against the 4 MiB asked for. The manifest's `host_memory_bound`
/// states what is CONFIGURED; it is not a proof of kernel-resident maximum.
pub const CAPTURE_PIPE_BUFFER_BYTES: u32 = 4 * 1024 * 1024;
/// Read granularity of the capture drain loop. See [`CAPTURE_PIPE_BUFFER_BYTES`].
pub const CAPTURE_READ_BUFFER_BYTES: usize = 1024 * 1024;

pub const MAX_EARLY_HOOK_TIMEOUT_MS: u32 =
    compact_contract::early_render_control::MAX_EARLY_RENDER_TIMEOUT_MS;
