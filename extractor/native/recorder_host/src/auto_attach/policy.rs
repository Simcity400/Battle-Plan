#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WatcherHealth {
    Healthy,
    Degraded,
}

pub const HEALTHY_RECONCILE_MS: u64 = 30_000;
pub const DEGRADED_RECONCILE_MS: u64 = 2_000;

/// Process-lifetime handshake deadline: a liveness policy, not measured the game
pub const HANDSHAKE_DEADLINE_MS: u64 = 30_000;

const _: () = assert!(
    compact_contract::passive_ready::DIRECT_CAPTURE_IDENTITY_READ_BUDGET_MS * 2
        < HANDSHAKE_DEADLINE_MS
);

pub const HEARTBEAT_INTERVAL_MS: u64 = 5_000;

pub const HEARTBEAT_LIVENESS_MS: u64 = 15_000;

pub const FIRST_FRAME_DEADLINE_MS: u64 = 300_000;

pub const GRACEFUL_STOP_STALL_DEADLINE_MS: u64 = 120_000;

pub const GRACEFUL_STOP_DEADLINE_MS: u64 = 3_600_000;

pub const READINESS_BACKOFF_MS: [u64; 3] = [250, 500, 1_000];
pub const READINESS_STEADY_MS: u64 = 2_000;

pub fn reconcile_interval_ms(health: WatcherHealth) -> u64 {
    match health {
        WatcherHealth::Healthy => HEALTHY_RECONCILE_MS,
        WatcherHealth::Degraded => DEGRADED_RECONCILE_MS,
    }
}

pub fn readiness_delay_ms(attempt: usize) -> u64 {
    READINESS_BACKOFF_MS
        .get(attempt)
        .copied()
        .unwrap_or(READINESS_STEADY_MS)
}

pub fn max_reconciliations_in_window(health: WatcherHealth, window_ms: u64) -> u64 {
    window_ms / reconcile_interval_ms(health)
}

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
        let thirty_min = 1_800_000u64;
        assert_eq!(
            max_snapshots_in_window(WatcherHealth::Healthy, thirty_min),
            61,
            "healthy: 1 startup + 60 reconciliations"
        );
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
