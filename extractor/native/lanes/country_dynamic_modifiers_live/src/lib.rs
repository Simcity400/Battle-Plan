use serde::{Deserialize, Serialize};

#[path = "../timed_instance_exact.rs"]
pub mod timed_instance_exact;

#[cfg(windows)]
pub mod a729_hook;

pub const DOMAIN_ID: u16 = 67;
pub const PRODUCTION_DMI2_SCHEDULING_ENABLED: bool = true;
pub const LIVE_FIXTURE_ACCEPTED: bool = false;
pub const ACCEPTED_LIVE_FIXTURE_VERIFICATION_SHA256: Option<&str> = None;
pub const EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const ZERO_REMOVAL_JOB_ID: &str =
    "20260720T101858Z-w5-a18-dmi2-synchronous-zero-removal-rec-845d4d32";
pub const ZERO_REMOVAL_MANIFEST_SHA256: &str =
    "9281a49e3f046984a699c3c42fd1ad172361d670aed9586631329255b038cd25";

pub fn production_dmi2_admission_ready() -> bool {
    PRODUCTION_DMI2_SCHEDULING_ENABLED
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicModifierWitnessRow {
    pub modifier_id: String,
    pub enabled: bool,
    pub values_raw: Vec<i64>,
    pub remaining_days: Option<u32>,
    pub pointer_masked_entry_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EraseBoundaryWitness {
    pub start_index: u32,
    pub end_index: u32,
    pub rows_before_erase: Vec<DynamicModifierWitnessRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionZeroRemoval {
    pub sequence: u64,
    pub owner_tag_db_id: u32,
    pub observed_game_hour_raw: i32,
    pub row: DynamicModifierWitnessRow,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyUpdateWitness {
    pub sequence: u64,
    pub thread_id: u32,
    pub owner_tag_db_id: u32,
    pub game_hour_raw_before: i32,
    pub game_hour_raw_after: i32,
    pub before: Vec<DynamicModifierWitnessRow>,
    pub erase_boundaries: Vec<EraseBoundaryWitness>,
    pub after: Vec<DynamicModifierWitnessRow>,
}

impl DailyUpdateWitness {
    pub fn validates_exact_daily_transform(&self) -> bool {
        if !unique_structural_rows(&self.before) || !unique_structural_rows(&self.after) {
            return false;
        }
        let Some(mut current) = self
            .before
            .iter()
            .map(row_after_daily_decrement)
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        let mut prior_start = usize::MAX;
        for boundary in &self.erase_boundaries {
            let Ok(start) = usize::try_from(boundary.start_index) else {
                return false;
            };
            let Ok(end) = usize::try_from(boundary.end_index) else {
                return false;
            };
            if end != start.saturating_add(1)
                || end > current.len()
                || start >= prior_start
                || !unique_structural_rows(&boundary.rows_before_erase)
                || current.get(start..end).map_or(true, |expected| {
                    expected
                        .iter()
                        .zip(&boundary.rows_before_erase)
                        .any(|(left, right)| !same_transform_row(left, right))
                })
            {
                return false;
            }
            current.drain(start..end);
            prior_start = start;
        }
        current.len() == self.after.len()
            && current
                .iter()
                .zip(&self.after)
                .all(|(left, right)| same_transform_row(left, right))
    }

    pub fn contains_measured_zero_removal(&self) -> bool {
        self.validates_exact_daily_transform()
            && self.before.iter().any(|before| {
                before.remaining_days == Some(1)
                    && self.erase_boundaries.iter().any(|boundary| {
                        boundary.rows_before_erase.iter().any(|zero| {
                            zero.remaining_days == Some(0)
                                && row_after_daily_decrement(before)
                                    .as_ref()
                                    .is_some_and(|expected| same_transform_row(expected, zero))
                        })
                    })
                    && !self
                        .after
                        .iter()
                        .any(|after| after.modifier_id == before.modifier_id)
            })
    }

    pub fn contains_exact_decrement(&self) -> bool {
        self.validates_exact_daily_transform()
            && self.before.iter().any(|before| {
                let Some(previous) = before.remaining_days else {
                    return false;
                };
                previous > 0
                    && self.after.iter().any(|after| {
                        after.modifier_id == before.modifier_id
                            && after.remaining_days == Some(previous - 1)
                            && row_after_daily_decrement(before)
                                .as_ref()
                                .is_some_and(|expected| same_transform_row(expected, after))
                    })
            })
    }
}

fn unique_structural_rows(rows: &[DynamicModifierWitnessRow]) -> bool {
    let mut ids = std::collections::BTreeSet::new();
    rows.iter()
        .all(|row| row_is_structural(row) && ids.insert(row.modifier_id.as_str()))
}

fn same_transform_row(left: &DynamicModifierWitnessRow, right: &DynamicModifierWitnessRow) -> bool {
    left.modifier_id == right.modifier_id
        && left.enabled == right.enabled
        && left.values_raw == right.values_raw
        && left.remaining_days == right.remaining_days
}

fn row_is_structural(row: &DynamicModifierWitnessRow) -> bool {
    let bytes = row.pointer_masked_entry_bytes.as_slice();
    if bytes.len() != 0x40
        || [0usize, 0x18, 0x28]
            .iter()
            .any(|start| bytes[*start..*start + 8].iter().any(|byte| *byte != 0))
        || !matches!(bytes[0x20], 0 | 1)
        || (bytes[0x20] != 0) != row.enabled
    {
        return false;
    }
    let days_raw = i32::from_le_bytes(bytes[0x10..0x14].try_into().expect("fixed row"));
    let expected_days = match row.remaining_days {
        None => -1,
        Some(days) => match i32::try_from(days) {
            Ok(days) => days,
            Err(_) => return false,
        },
    };
    let capacity = u32::from_le_bytes(bytes[0x30..0x34].try_into().expect("fixed row"));
    let count = u32::from_le_bytes(bytes[0x34..0x38].try_into().expect("fixed row"));
    days_raw == expected_days
        && count <= capacity
        && usize::try_from(count).ok() == Some(row.values_raw.len())
}

fn row_after_daily_decrement(row: &DynamicModifierWitnessRow) -> Option<DynamicModifierWitnessRow> {
    if !row_is_structural(row) {
        return None;
    }
    let mut after = row.clone();
    after.remaining_days = row.remaining_days.map(|days| days.saturating_sub(1));
    let days_raw = after.remaining_days.map_or(-1, |days| {
        i32::try_from(days).expect("validated witness day")
    });
    after.pointer_masked_entry_bytes[0x10..0x14].copy_from_slice(&days_raw.to_le_bytes());
    Some(after)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleWitnessDiagnostics {
    pub retained_updates: u64,
    pub ignored_without_timed_instance: u64,
    pub dropped_reentrant: u64,
    pub dropped_unreadable: u64,
    pub dropped_lock_busy: u64,
    pub dropped_overflow: u64,
    pub completeness_poisoned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleWitnessReceipt {
    pub schema: String,
    pub executable_sha256: String,
    pub exact20_job_id: String,
    pub exact20_manifest_sha256: String,
    pub update_function_rva: u32,
    pub erase_function_rva: u32,
    pub process_identity: LifecycleWitnessProcessIdentity,
    pub evidence_install: bool,
    pub hook_epoch: u64,
    pub witnesses: Vec<DailyUpdateWitness>,
    pub diagnostics: LifecycleWitnessDiagnostics,
    pub production_admission_ready: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleWitnessProcessIdentity {
    pub pid: u32,
    pub create_time_100ns: u64,
    pub executable_sha256: String,
}

impl LifecycleWitnessReceipt {
    pub fn fixture_has_exact_zero_path(&self) -> bool {
        if !self
            .witnesses
            .iter()
            .all(DailyUpdateWitness::validates_exact_daily_transform)
            || !self
                .witnesses
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        {
            return false;
        }
        let exact_path = self.witnesses.iter().enumerate().any(|(index, decrement)| {
            decrement.before.iter().any(|before| {
                let Some(previous) = before.remaining_days else {
                    return false;
                };
                previous > 1
                    && decrement
                        .after
                        .iter()
                        .find(|after| after.modifier_id == before.modifier_id)
                        .is_some_and(|after| {
                            after.remaining_days == Some(previous - 1)
                                && row_after_daily_decrement(before)
                                    .as_ref()
                                    .is_some_and(|expected| same_transform_row(expected, after))
                                && exact_zero_path_after(
                                    &self.witnesses[index + 1..],
                                    decrement,
                                    after,
                                )
                        })
            })
        });
        self.evidence_install
            && self.hook_epoch != 0
            && !self.diagnostics.completeness_poisoned
            && self.diagnostics.dropped_reentrant == 0
            && self.diagnostics.dropped_unreadable == 0
            && self.diagnostics.dropped_lock_busy == 0
            && self.diagnostics.dropped_overflow == 0
            && self.diagnostics.retained_updates == self.witnesses.len() as u64
            && exact_path
    }
}

fn exact_zero_path_after(
    later: &[DailyUpdateWitness],
    initial: &DailyUpdateWitness,
    initial_after: &DynamicModifierWitnessRow,
) -> bool {
    let mut expected_before = initial_after.clone();
    let mut prior_hour = initial.game_hour_raw_after;
    for witness in later {
        if witness.owner_tag_db_id != initial.owner_tag_db_id {
            continue;
        }
        if witness.thread_id != initial.thread_id
            || witness.game_hour_raw_before != witness.game_hour_raw_after
            || witness.game_hour_raw_before.checked_sub(prior_hour) != Some(24)
        {
            return false;
        }
        let Some(before) = witness
            .before
            .iter()
            .find(|row| row.modifier_id == expected_before.modifier_id)
        else {
            return false;
        };
        if !same_transform_row(before, &expected_before) {
            return false;
        }
        let Some(decremented) = row_after_daily_decrement(before) else {
            return false;
        };
        if before.remaining_days == Some(1) {
            return witness.erase_boundaries.iter().any(|boundary| {
                boundary
                    .rows_before_erase
                    .iter()
                    .any(|row| same_transform_row(row, &decremented))
            }) && !witness
                .after
                .iter()
                .any(|row| row.modifier_id == before.modifier_id);
        }
        let Some(after) = witness
            .after
            .iter()
            .find(|row| row.modifier_id == before.modifier_id)
        else {
            return false;
        };
        if !same_transform_row(after, &decremented) {
            return false;
        }
        expected_before = after.clone();
        prior_hour = witness.game_hour_raw_after;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, days: Option<u32>) -> DynamicModifierWitnessRow {
        let mut pointer_masked_entry_bytes = vec![0; 0x40];
        let days_raw = days.map_or(-1, |value| value as i32);
        pointer_masked_entry_bytes[0x10..0x14].copy_from_slice(&days_raw.to_le_bytes());
        pointer_masked_entry_bytes[0x20] = 1;
        pointer_masked_entry_bytes[0x30..0x34].copy_from_slice(&2u32.to_le_bytes());
        pointer_masked_entry_bytes[0x34..0x38].copy_from_slice(&2u32.to_le_bytes());
        DynamicModifierWitnessRow {
            modifier_id: id.into(),
            enabled: true,
            values_raw: vec![10_000, -5_000],
            remaining_days: days,
            pointer_masked_entry_bytes,
        }
    }

    #[test]
    fn zero_is_not_inferred_from_absence() {
        let witness = DailyUpdateWitness {
            sequence: 1,
            thread_id: 1,
            owner_tag_db_id: 33,
            game_hour_raw_before: 100,
            game_hour_raw_after: 100,
            before: vec![row("timed", Some(1))],
            erase_boundaries: vec![],
            after: vec![],
        };
        assert!(!witness.contains_measured_zero_removal());
    }

    #[test]
    fn synchronous_zero_erase_is_distinct_from_decrement() {
        let decrement = DailyUpdateWitness {
            sequence: 1,
            thread_id: 1,
            owner_tag_db_id: 33,
            game_hour_raw_before: 100,
            game_hour_raw_after: 100,
            before: vec![row("timed", Some(2)), row("permanent", None)],
            erase_boundaries: vec![],
            after: vec![row("timed", Some(1)), row("permanent", None)],
        };
        assert!(decrement.contains_exact_decrement());
        assert!(!decrement.contains_measured_zero_removal());

        let removal = DailyUpdateWitness {
            sequence: 2,
            game_hour_raw_before: 124,
            game_hour_raw_after: 124,
            before: vec![row("timed", Some(1)), row("permanent", None)],
            erase_boundaries: vec![EraseBoundaryWitness {
                start_index: 0,
                end_index: 1,
                rows_before_erase: vec![row("timed", Some(0))],
            }],
            after: vec![row("permanent", None)],
            ..decrement.clone()
        };
        assert!(removal.contains_measured_zero_removal());
        assert!(!removal.contains_exact_decrement());

        let receipt = LifecycleWitnessReceipt {
            schema: "hoi4.a18.dynamic_modifier_lifecycle_witness.v1".into(),
            executable_sha256: EXECUTABLE_SHA256.into(),
            exact20_job_id: ZERO_REMOVAL_JOB_ID.into(),
            exact20_manifest_sha256: ZERO_REMOVAL_MANIFEST_SHA256.into(),
            update_function_rva: 0x54beb0,
            erase_function_rva: 0x604d30,
            process_identity: LifecycleWitnessProcessIdentity {
                pid: 1,
                create_time_100ns: 1,
                executable_sha256: EXECUTABLE_SHA256.into(),
            },
            evidence_install: true,
            hook_epoch: 1,
            witnesses: vec![decrement.clone(), removal.clone()],
            diagnostics: LifecycleWitnessDiagnostics {
                retained_updates: 2,
                ..LifecycleWitnessDiagnostics::default()
            },
            production_admission_ready: false,
        };
        assert!(receipt.fixture_has_exact_zero_path());

        let mut disconnected = receipt;
        disconnected.witnesses[1].owner_tag_db_id += 1;
        assert!(!disconnected.fixture_has_exact_zero_path());
    }

    #[test]
    fn production_admission_does_not_require_the_optional_fixture_receipt() {
        assert!(production_dmi2_admission_ready());
        assert!(ACCEPTED_LIVE_FIXTURE_VERIFICATION_SHA256.is_none());
    }
}
