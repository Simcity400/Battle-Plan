use std::sync::OnceLock;
use std::time::Instant;

/// CPU-side presentation point of a successfully submitted map frame, not a
/// GPU completion or scanout timestamp. Sampling never follows the log writer.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ShownStamp {
    ns: u128,
    branch: replay_data::BranchId,
    hour: replay_data::Hour,
}

impl std::fmt::Display for ShownStamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "shown_ns={} shown_branch={} shown_hour={}",
            self.ns, self.branch, self.hour
        )
    }
}

/// PROFILE-only monotonic time and the drawn picture, never the playhead.
/// The independent process-local origin is shared by every map in this process.
pub(super) fn shown_stamp(
    enabled: bool,
    shown: impl FnOnce() -> (replay_data::BranchId, replay_data::Hour),
) -> Option<ShownStamp> {
    shown_stamp_with(enabled, shown, || {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN.get_or_init(Instant::now).elapsed().as_nanos()
    })
}

fn shown_stamp_with(
    enabled: bool,
    shown: impl FnOnce() -> (replay_data::BranchId, replay_data::Hour),
    clock: impl FnOnce() -> u128,
) -> Option<ShownStamp> {
    enabled.then(|| {
        let ns = clock();
        let (branch, hour) = shown();
        ShownStamp { ns, branch, hour }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_stamps_preserve_wall_time_and_installed_identity_only_when_enabled() {
        assert_eq!(
            shown_stamp_with(
                false,
                || panic!("a disabled stamp reads no drawn picture"),
                || panic!("a disabled stamp reads no clock"),
            ),
            None
        );
        for (ns, branch, hour) in [(0_u128, 0, 0_i32), (7, 3, -12), (u128::MAX, replay_data::BranchId::MAX, i32::MAX)] {
            let stamp = shown_stamp_with(true, || (branch, hour), || ns).unwrap();
            assert_eq!(
                stamp.to_string(),
                format!("shown_ns={ns} shown_branch={branch} shown_hour={hour}")
            );
        }
    }
}
