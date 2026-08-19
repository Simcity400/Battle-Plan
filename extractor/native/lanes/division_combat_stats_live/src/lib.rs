pub use compact_contract::division_combat_stats::{
    CachedStatBlock, DivisionCombatStatsRow, DivisionCombatStatsSnapshot, RowObservation,
    ScaleFields, COMBAT_ROUND_RVA, DOMAIN_ID, MAX_COMBATS, MAX_ROWS, SIDE_ATTACKER, SIDE_DEFENDER,
};

pub const MAX_COMMITTED_PER_ROUND: usize = 256;

pub const D21_MAX_LAND_COMBAT_SIDE_UNITS: usize = 128;

pub const MAX_QUEUED_ROUNDS: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservedDivision {
    pub division_id: u32,
    pub observation: RowObservation,
    pub stats: Option<CachedStatBlock>,
    pub scale: Option<ScaleFields>,
    pub combat_side_bonus_raw: Option<i64>,
    pub modifier_count_before: u32,
    pub modifier_count_after: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SideClass {
    Primary,
    Sibling,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedRound {
    pub combat_id: u32,
    pub side: u32,
    pub side_class: SideClass,
    pub side_ptr: usize,
    pub committed_count: u32,
    pub divisions: Vec<ObservedDivision>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapturedFrame {
    pub rounds: Vec<ObservedRound>,
    pub dropped_queue_full: u64,
    pub dropped_lock_busy: u64,
    pub dropped_unrecognised_side: u64,
    pub dropped_unlinked_combat: u64,
    pub dropped_unreadable_side_state: u64,
    pub dropped_oversized_roster: u64,
    pub dropped_unresolvable_division: u64,
}

impl CapturedFrame {
    pub fn is_complete(&self) -> bool {
        self.dropped_queue_full == 0
            && self.dropped_lock_busy == 0
            && self.dropped_unrecognised_side == 0
            && self.dropped_unlinked_combat == 0
            && self.dropped_unreadable_side_state == 0
            && self.dropped_oversized_roster == 0
            && self.dropped_unresolvable_division == 0
    }
}

#[cfg(windows)]
pub mod a729_hook;

#[cfg(all(windows, any(test, feature = "test-support")))]
pub use a729_hook::force_lifecycle_installed_for_test;

#[cfg(windows)]
pub use a729_hook::{
    abort_partial_install_and_uninstall, arm_capture, capture_armed, disarm_capture, drain_latest,
    install, installed, installed_hook_receipts, take_released_hook_receipts, teardown_pending,
    uninstall, DivisionCombatStatsDrainError, DivisionCombatStatsHookError, HOOK_RECEIPT_COUNT,
    ROUND_LOGICAL_NAME, ROUND_SENTINEL,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_complete_only_when_every_drop_reason_is_zero() {
        assert!(CapturedFrame::default().is_complete());
        let setters: [fn(&mut CapturedFrame); 7] = [
            |f| f.dropped_queue_full = 1,
            |f| f.dropped_lock_busy = 1,
            |f| f.dropped_unrecognised_side = 1,
            |f| f.dropped_unlinked_combat = 1,
            |f| f.dropped_unreadable_side_state = 1,
            |f| f.dropped_oversized_roster = 1,
            |f| f.dropped_unresolvable_division = 1,
        ];
        for (index, set) in setters.iter().enumerate() {
            let mut frame = CapturedFrame::default();
            set(&mut frame);
            assert!(
                !frame.is_complete(),
                "drop reason {index} does not make the frame incomplete"
            );
        }
    }

    #[test]
    fn the_round_roster_cap_cannot_be_the_binding_limit() {
        assert!(
            MAX_COMMITTED_PER_ROUND > D21_MAX_LAND_COMBAT_SIDE_UNITS,
            "a roster D21 accepts ({D21_MAX_LAND_COMBAT_SIDE_UNITS}) must fit under \
             this lane's cap ({MAX_COMMITTED_PER_ROUND}), or the cap silently \
             refuses real sides"
        );
        assert!(MAX_COMMITTED_PER_ROUND <= MAX_ROWS);
        assert_eq!(MAX_COMMITTED_PER_ROUND, 256);
        assert_eq!(D21_MAX_LAND_COMBAT_SIDE_UNITS, 128);
        assert_eq!(MAX_QUEUED_ROUNDS, 4_096);
    }

    #[test]
    fn the_lane_declares_no_acquisition_offset_of_its_own() {
        let source = include_str!("a729_hook.rs");
        let hook = source
            .split_once("#[cfg(test)]")
            .map(|(production, _)| production)
            .expect("a729_hook.rs has a test module, so the split cannot fail");
        let sentinel_start = hook
            .find("pub const ROUND_SENTINEL")
            .expect("the entry sentinel is declared in production code");
        let sentinel_end = sentinel_start
            + hook[sentinel_start..]
                .find("];")
                .expect("the sentinel declaration is closed")
            + 2;
        let scanned = format!("{}{}", &hook[..sentinel_start], &hook[sentinel_end..]);
        let decommented: String = scanned
            .lines()
            .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
            .collect::<Vec<_>>()
            .join("\n");
        for (number, line) in decommented.lines().enumerate() {
            assert!(
                !line.contains("0x"),
                "a729_hook.rs (production, line {} of the scanned text): hex literal in `{}` — every engine offset must be read through `contract::`",
                number + 1,
                line.trim()
            );
        }
        for keyword in ["checked_add(", "checked_sub(", "at("] {
            for (found, _) in decommented.match_indices(keyword) {
                let after = &decommented[found + keyword.len()..];
                let argument = if keyword == "at(" {
                    match after.split_once(',') {
                        Some((_, rest)) => rest.trim_start(),
                        None => continue,
                    }
                } else {
                    after
                };
                let mut depth = 1i32;
                let mut previous = '(';
                for c in argument.chars() {
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        ',' if depth == 1 && keyword == "at(" => break,
                        _ => {}
                    }
                    let previous_is_ident =
                        previous.is_ascii_alphanumeric() || previous == '_' || previous == ':';
                    assert!(
                        !(c.is_ascii_digit() && !previous_is_ident),
                        "a729_hook.rs production: `{}{}` uses a numeric literal as an engine displacement — every offset must come from `contract::`",
                        keyword,
                        argument.chars().take(40).collect::<String>()
                    );
                    previous = c;
                }
            }
        }
        assert_eq!(DOMAIN_ID, 99);
        assert_eq!(SIDE_ATTACKER, 1);
        assert_eq!(SIDE_DEFENDER, 2);
    }
}
