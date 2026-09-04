//! D53's frame-thread spend, hour by hour.
//!
//! The d53 read lands on the frame's CRITICAL PATH: capture e2e_04 measured it
//! at 158.7 ms p50 against a 166.5 ms game hour, 85.5% of the whole capture's
//! critical path, and the frame builder lost 182 of 936 hours because of it
//! (runs/d53c_keep_up/ATTRIBUTION.json). A domain that cannot pay for the hour
//! must name the hour, not spend the frame: every hour this ledger defers is a
//! typed d53 gap, and no other domain loses anything.

/// What d53 may spend on the frame thread per game hour, as a long-run
/// average. It is d53's OWN published per-record budget
/// (`perf_budget/domain_budget_v1.json`, diplomacy_intel_operatives_live
/// `read_time_p50_ms_max`), so an admitted hour costs no more than the domain
/// was ever priced at. The frame fits: capture e2e_03's critical path without
/// d53 was 83.6 ms p50, and 83.6 + 65.2 = 148.8 ms is inside both the global
/// `crit_wall_p50_ms_max` of 161 ms and the 166.5 ms hour.
pub const DOMAIN53_KEEP_UP_SHARE_US: i64 = 65_200;

/// Why an hour did not run the d53 read. The deferral is a typed gap on the
/// wire, so the cause has to be one string the manifest can count.
pub const DOMAIN53_KEEP_UP_DEFERRED_CAUSE: &str = "domain53_keep_up_deferred";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain53KeepUpVerdict {
    /// The hour is paid for: run the read.
    Admit,
    /// The hour is not paid for: publish the typed gap and spend nothing.
    /// Carries the credit it was short by, in microseconds.
    Defer { short_by_us: u64 },
}

/// The ledger. One per attached collector, carried by the d53 tracker.
///
/// Credit accrues one share per frame and is spent by the measured read.
/// Debt is carried signed, so a read that overran its share is paid back by
/// the hours after it rather than forgiven, and the long-run admitted rate is
/// exactly share over cost. Credit is capped at one read plus one share --
/// the most an hour that was about to admit can be holding -- so a stretch
/// where d53 got cheap or was disabled cannot bank a burst for later.
#[derive(Clone, Copy, Debug, Default)]
pub struct Domain53KeepUp {
    credit_us: i64,
    /// What the last admitted read actually cost. Zero before the first one,
    /// which is why the first frame always admits: the ledger measures the
    /// domain rather than assuming a price for it.
    last_cost_us: i64,
    admitted: u64,
    deferred: u64,
    /// An admitted hour that has not been charged yet. A frame that never
    /// consulted the ledger -- d53's inbound-status short circuit builds an
    /// empty record without reading anything -- must not move it, and no hour
    /// may be charged twice.
    owes_charge: bool,
}

impl Domain53KeepUp {
    /// Accrue this hour's share and decide. Called once per frame on the
    /// scheduler thread, BEFORE anything that costs: on `Defer` the frame
    /// reaps no plan job, spawns no walk, copies nothing and decodes nothing.
    pub fn admit(&mut self) -> Domain53KeepUpVerdict {
        self.accrue(DOMAIN53_KEEP_UP_SHARE_US);
        if self.credit_us >= self.last_cost_us {
            self.admitted = self.admitted.saturating_add(1);
            self.owes_charge = true;
            Domain53KeepUpVerdict::Admit
        } else {
            self.deferred = self.deferred.saturating_add(1);
            Domain53KeepUpVerdict::Defer {
                short_by_us: (self.last_cost_us - self.credit_us).max(0) as u64,
            }
        }
    }

    /// Charge an admitted hour with what its read measured. Every exit of the
    /// d53 finish half calls this, including the fail-closed ones: a refused
    /// read costs the frame whatever it spent before refusing.
    pub fn charge(&mut self, read_time_us: u64) {
        if !self.owes_charge {
            return;
        }
        self.owes_charge = false;
        let cost = i64::try_from(read_time_us).unwrap_or(i64::MAX);
        self.last_cost_us = cost;
        self.credit_us = self.credit_us.saturating_sub(cost);
    }

    pub fn admitted(&self) -> u64 {
        self.admitted
    }

    pub fn deferred(&self) -> u64 {
        self.deferred
    }

    pub fn credit_us(&self) -> i64 {
        self.credit_us
    }

    fn accrue(&mut self, share_us: i64) {
        // In steady state this cap never binds: an hour accrues only while
        // credit is below one read's cost, so it lands below cost + share.
        let cap = self.last_cost_us.saturating_add(share_us).max(share_us);
        self.credit_us = self.credit_us.saturating_add(share_us).min(cap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first hour admits whatever d53 costs, because the ledger has not
    /// measured it yet, and the hours after it pay for what that one spent.
    #[test]
    fn the_first_hour_admits_and_the_measured_cost_prices_every_hour_after_it() {
        let mut ledger = Domain53KeepUp::default();
        assert_eq!(ledger.admit(), Domain53KeepUpVerdict::Admit);
        ledger.charge(158_710);
        assert_eq!(ledger.credit_us(), 65_200 - 158_710);
        for _ in 0..3 {
            assert!(matches!(ledger.admit(), Domain53KeepUpVerdict::Defer { .. }));
        }
        assert_eq!(ledger.admit(), Domain53KeepUpVerdict::Admit);
        assert_eq!(ledger.admitted(), 2);
        assert_eq!(ledger.deferred(), 3);
    }

    /// The long-run rate IS the share over the cost, so d53's average frame
    /// cost is its published budget whatever one read happens to cost. Run
    /// against capture e2e_04's measured read (158.7 ms) over its 936 hours.
    #[test]
    fn the_long_run_spend_per_hour_is_the_share() {
        const MEASURED_READ_US: u64 = 158_710;
        const HOURS: u64 = 936;
        let mut ledger = Domain53KeepUp::default();
        let mut spent = 0u64;
        for _ in 0..HOURS {
            if ledger.admit() == Domain53KeepUpVerdict::Admit {
                ledger.charge(MEASURED_READ_US);
                spent += MEASURED_READ_US;
            }
        }
        assert_eq!(ledger.admitted() + ledger.deferred(), HOURS);
        let per_hour = spent / HOURS;
        assert!(
            per_hour <= DOMAIN53_KEEP_UP_SHARE_US as u64,
            "{per_hour} us per hour against a {DOMAIN53_KEEP_UP_SHARE_US} us share",
        );
        // Nothing is thrown away either: the ledger runs d53 as often as the
        // share pays for, which at this cost is 41% of hours.
        let admitted_share = ledger.admitted() as f64 / HOURS as f64;
        let affordable = DOMAIN53_KEEP_UP_SHARE_US as f64 / MEASURED_READ_US as f64;
        assert!(
            (admitted_share - affordable).abs() < 0.02,
            "{admitted_share} against {affordable}"
        );
    }

    /// A cheap read is never deferred: if d53 fits its share, it runs every
    /// hour, which is what the hourly baseline requires.
    #[test]
    fn a_read_inside_the_share_runs_every_hour() {
        let mut ledger = Domain53KeepUp::default();
        for _ in 0..500 {
            assert_eq!(ledger.admit(), Domain53KeepUpVerdict::Admit);
            ledger.charge(DOMAIN53_KEEP_UP_SHARE_US as u64);
        }
        assert_eq!(ledger.deferred(), 0);
    }

    /// Credit does not bank: an hour d53 never ran cannot buy two reads in a
    /// row and hand the frame builder a burst it has no time for.
    #[test]
    fn credit_never_banks_a_second_read() {
        const COST: u64 = 158_710;
        let mut ledger = Domain53KeepUp::default();
        ledger.admit();
        ledger.charge(COST);
        // A thousand hours where d53 was never charged: the ledger still
        // holds at most one read plus one hour's share.
        for _ in 0..1_000 {
            ledger.admit();
        }
        assert_eq!(ledger.credit_us(), COST as i64 + DOMAIN53_KEEP_UP_SHARE_US);
        // And it still cannot pay for two reads back to back.
        ledger.charge(COST);
        assert!(matches!(ledger.admit(), Domain53KeepUpVerdict::Defer { .. }));
    }

    /// Only an hour the ledger admitted moves it: d53's inbound-status short
    /// circuit returns a record without consulting the ledger, and an hour it
    /// deferred spent nothing, so neither may be charged -- and no hour may be
    /// charged twice.
    #[test]
    fn only_an_admitted_hour_is_charged_and_only_once() {
        let mut ledger = Domain53KeepUp::default();
        ledger.charge(500_000);
        assert_eq!(ledger.credit_us(), 0, "a frame that never admitted");
        assert_eq!(ledger.admit(), Domain53KeepUpVerdict::Admit);
        ledger.charge(158_710);
        ledger.charge(158_710);
        assert_eq!(ledger.credit_us(), DOMAIN53_KEEP_UP_SHARE_US - 158_710);
        assert!(matches!(ledger.admit(), Domain53KeepUpVerdict::Defer { .. }));
        ledger.charge(158_710);
        assert_eq!(
            ledger.credit_us(),
            2 * DOMAIN53_KEEP_UP_SHARE_US - 158_710,
            "the deferred hour accrued and spent nothing"
        );
    }

    /// WHAT A DROPPED SNAPSHOT PLAN COSTS. The frame that drops the plan
    /// spent a whole read before refusing, so the ledger charges that read
    /// and the hours it then defers are typed gaps that spend nothing --
    /// which is exactly the window the off-frame replacement walk needs.
    ///
    /// Run against capture e2e_05's measured shape
    /// (`runs/d53d_capture05_gaps/FINDING.json`): 1,244 frames in 120 s, a
    /// 158.7 ms read, an 80.1 ms mean walk (4,162,835 us over 52), one plan
    /// drop every sixth read (60 drops, 362 admitted hours), and 9 us as the
    /// read time a refusal that read nothing reports.
    ///
    /// Both halves of the e2e_05 defect are pinned here. The walk could not
    /// START for 221 ms after a drop, and because the refusal was charged its
    /// wire read time instead of what it spent, every hour of that window was
    /// ADMITTED and lost -- just under three per drop, which is the 177
    /// pending hours the capture counted against its 60 drops.
    #[test]
    fn a_dropped_plan_costs_at_most_the_hour_it_was_dropped_in() {
        const FRAME_US: u64 = 96_500;
        const READ_US: u64 = 158_710;
        const WALK_MEAN_US: u64 = 80_054;
        /// The slowest walk either capture measured (e2e_04), which outlasts
        /// a frame; the mean is what e2e_05 actually paid.
        const WALK_MAX_US: u64 = 221_000;
        const REFUSAL_WIRE_US: u64 = 9;
        const HOURS: u64 = 1_244;
        const READS_PER_DROP: u64 = 6;

        /// `(drops, pending_hours)` over the run. `drop_backoff_us` is how
        /// long the replacement walk waits before it may start; `drop_cost_us`
        /// is what the ledger is charged for the hour that dropped the plan.
        fn run(drop_backoff_us: u64, drop_cost_us: u64, walk_us: u64) -> (u64, u64) {
            let mut ledger = Domain53KeepUp::default();
            let mut plan = false;
            let mut walk_ends_at: Option<u64> = None;
            let mut walk_may_start_at = 0u64;
            let (mut reads, mut drops, mut pending) = (0u64, 0u64, 0u64);
            for hour in 0..HOURS {
                let now = hour * FRAME_US;
                if walk_ends_at.is_some_and(|at| at <= now) {
                    plan = true;
                    walk_ends_at = None;
                }
                if matches!(ledger.admit(), Domain53KeepUpVerdict::Defer { .. }) {
                    continue;
                }
                if !plan {
                    if walk_ends_at.is_none() && now >= walk_may_start_at {
                        walk_ends_at = Some(now + walk_us);
                    }
                    pending += 1;
                    ledger.charge(REFUSAL_WIRE_US);
                    continue;
                }
                reads += 1;
                if reads % READS_PER_DROP == 0 {
                    drops += 1;
                    plan = false;
                    walk_may_start_at = now + drop_backoff_us;
                    if drop_backoff_us == 0 {
                        walk_ends_at = Some(now + walk_us);
                    }
                    ledger.charge(drop_cost_us);
                } else {
                    ledger.charge(READ_US);
                }
            }
            (drops, pending)
        }

        let (drops, pending) = run(0, READ_US, WALK_MEAN_US);
        assert!(drops > 0, "the run must actually drop plans");
        assert_eq!(
            pending, 1,
            "only the cold start has no plan: {drops} drop(s) cost {pending} pending hour(s)"
        );

        // The 221 ms backoff, charged the refusal's wire time: the capture's
        // own ratio, just under three lost hours for every plan dropped.
        let (drops_before, pending_before) = run(221_000, REFUSAL_WIRE_US, WALK_MEAN_US);
        assert!(
            pending_before >= 2 * drops_before,
            "{pending_before} pending hour(s) for {drops_before} drop(s)"
        );

        // At the slowest walk ever measured the drop hour still bounds the
        // loss: the walk outlasts one frame, so a drop can cost the hour
        // after it as well -- and NOT MORE, because the charge defers the
        // rest. Uncharged, the same walk loses strictly more.
        let (drops_max, pending_max) = run(0, READ_US, WALK_MAX_US);
        assert!(
            pending_max <= 1 + drops_max,
            "{pending_max} pending hour(s) for {drops_max} drop(s) at the slowest walk"
        );
        let (_, pending_uncharged) = run(0, REFUSAL_WIRE_US, WALK_MAX_US);
        assert!(
            pending_uncharged > pending_max,
            "an uncharged drop admits the very next hour into a walk still running"
        );
    }

    /// The share is d53's own published read-time budget, and the frame fits
    /// with it: capture e2e_03's critical path without d53 plus the share is
    /// inside both the global critical-path budget and the game hour.
    #[test]
    fn the_share_is_the_published_budget_and_the_frame_fits_it() {
        const D53_READ_TIME_P50_MS_MAX: i64 = 65_200;
        const E2E_03_CRIT_WALL_WITHOUT_D53_US: i64 = 83_600;
        const GLOBAL_CRIT_WALL_P50_MS_MAX_US: i64 = 161_000;
        const GAME_HOUR_AT_SPEED_5_US: i64 = 166_500;
        assert_eq!(DOMAIN53_KEEP_UP_SHARE_US, D53_READ_TIME_P50_MS_MAX);
        let frame = E2E_03_CRIT_WALL_WITHOUT_D53_US + DOMAIN53_KEEP_UP_SHARE_US;
        assert!(frame <= GLOBAL_CRIT_WALL_P50_MS_MAX_US);
        assert!(frame <= GAME_HOUR_AT_SPEED_5_US);
    }
}
