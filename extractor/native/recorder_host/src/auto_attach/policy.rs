//! Timing policy: reconciliation cadence, readiness backoff, handshake deadline
//! (design §2, §4). These are policy constants, not rig-fitted measurements —
//! the design says so explicitly and the idle-cost gate bounds them.

/// Watcher health drives the reconciliation cadence. WMI is the low-latency
/// signal; when it fails we fall back to faster polling until it recovers
/// (design §2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WatcherHealth {
    /// WMI subscription alive: event-first, slow reconciliation.
    Healthy,
    /// WMI failed: poll-only until recovery.
    Degraded,
}

/// Reconciliation cadence in milliseconds (design §2):
/// healthy => 30 s; degraded => 2 s.
pub const HEALTHY_RECONCILE_MS: u64 = 30_000;
pub const DEGRADED_RECONCILE_MS: u64 = 2_000;

/// Process-lifetime handshake deadline: a liveness policy, not measured the game
/// startup (design §4). If a heartbeat does not arrive within this window the
/// attempt fails loudly and retains diagnostics.
pub const HANDSHAKE_DEADLINE_MS: u64 = 30_000;

/// Once the initial hello succeeds, the recorder solicits a fresh H4CT proof
/// every five seconds. A proof never extends another operation's deadline.
pub const HEARTBEAT_INTERVAL_MS: u64 = 5_000;

/// Three missed heartbeat intervals are terminal. This is an explicit product
/// liveness policy, not a rig-derived timeout.
pub const HEARTBEAT_LIVENESS_MS: u64 = 15_000;

/// Maximum time a successfully admitted collector may remain attached without
/// producing one semantically validated world record. Fresh heartbeats prove
/// process liveness but deliberately do not extend this five-minute product
/// deadline.
pub const FIRST_FRAME_DEADLINE_MS: u64 = 300_000;

/// A finalizer which produces no durable filesystem progress for this long is
/// considered stalled. Speed-5 capture can legitimately require a long fold
/// and pack, so continuous progress resets this deadman rather than being
/// mistaken for a hang.
pub const GRACEFUL_STOP_STALL_DEADLINE_MS: u64 = 120_000;

/// Absolute fault-containment bound for one cooperative stop. This is separate
/// from the progress deadman: a healthy large speed-5 recording may run longer
/// than two minutes, but an unbounded child can never retain the supervisor
/// forever. Neither bound authorizes reinjection.
pub const GRACEFUL_STOP_DEADLINE_MS: u64 = 3_600_000;

/// Readiness backoff for `WaitingForIdentity` / `WaitingForWindow`
/// (design §4): 250 ms, 500 ms, 1 s, then every 2 s while the process lives.
pub const READINESS_BACKOFF_MS: [u64; 3] = [250, 500, 1_000];
pub const READINESS_STEADY_MS: u64 = 2_000;

/// Reconciliation interval for the given health.
pub fn reconcile_interval_ms(health: WatcherHealth) -> u64 {
    match health {
        WatcherHealth::Healthy => HEALTHY_RECONCILE_MS,
        WatcherHealth::Degraded => DEGRADED_RECONCILE_MS,
    }
}

/// The readiness re-check delay for the `attempt`-th probe (0-based). Caps at
/// the steady 2 s cadence once the initial fast probes are spent.
pub fn readiness_delay_ms(attempt: usize) -> u64 {
    READINESS_BACKOFF_MS
        .get(attempt)
        .copied()
        .unwrap_or(READINESS_STEADY_MS)
}

/// Upper bound on *reconciliation* snapshots over a window (excludes the single
/// startup snapshot). Design §10 idle-cost gate degraded figure (<= 900 in
/// 30 min) is stated in these terms.
pub fn max_reconciliations_in_window(health: WatcherHealth, window_ms: u64) -> u64 {
    window_ms / reconcile_interval_ms(health)
}

/// Upper bound on TOTAL snapshots over a window, INCLUDING the one startup
/// snapshot. Design §10 healthy figure (<= 61 in 30 min) is stated in these
/// terms. NOTE: the design's degraded figure of "900" is stated as
/// reconciliations-only (see [`max_reconciliations_in_window`]); the inclusive
/// total is 901. The enforced policy — a 2 s cadence — is identical either way;
/// the discrepancy is a wording asymmetry in the design, recorded in the
/// dossier, not a behavioural difference.
pub fn max_snapshots_in_window(health: WatcherHealth, window_ms: u64) -> u64 {
    1 + max_reconciliations_in_window(health, window_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cadence_matches_design() {
        assert_eq!(reconcile_interval_ms(WatcherHealth::Healthy), 30_000);
        assert_eq!(reconcile_interval_ms(WatcherHealth::Degraded), 2_000);
    }

    #[test]
    fn readiness_backoff_ladder() {
        assert_eq!(readiness_delay_ms(0), 250);
        assert_eq!(readiness_delay_ms(1), 500);
        assert_eq!(readiness_delay_ms(2), 1_000);
        assert_eq!(readiness_delay_ms(3), 2_000);
        assert_eq!(readiness_delay_ms(99), 2_000);
    }

    #[test]
    fn idle_cost_bounds_hold() {
        // Design §10 idle-cost gate: 30 minutes = 1_800_000 ms.
        let thirty_min = 1_800_000u64;
        // Healthy figure is stated inclusive of the startup snapshot: 61.
        assert_eq!(
            max_snapshots_in_window(WatcherHealth::Healthy, thirty_min),
            61,
            "healthy: 1 startup + 60 reconciliations"
        );
        // Degraded figure is stated as reconciliations-only: 900.
        assert_eq!(
            max_reconciliations_in_window(WatcherHealth::Degraded, thirty_min),
            900,
            "degraded: 900 reconciliations at the 2 s cadence"
        );
    }

    #[test]
    fn heartbeat_and_graceful_stop_policy_is_versioned_and_nonzero() {
        assert!(HEARTBEAT_INTERVAL_MS > 0);
        assert!(HEARTBEAT_LIVENESS_MS >= HEARTBEAT_INTERVAL_MS * 3);
        assert!(FIRST_FRAME_DEADLINE_MS > HEARTBEAT_LIVENESS_MS);
        assert_eq!(GRACEFUL_STOP_STALL_DEADLINE_MS, 120_000);
        assert_eq!(GRACEFUL_STOP_DEADLINE_MS, 3_600_000);
    }
}
