//! Replies answered from their gate's prepared prefix while preparation runs.
//!
//! A product whose values need earlier observations has no stateless sample
//! for hours its gate has not accepted. While that gate's coverage is
//! partial, the product answers with what its pages hold through the gate's
//! last covered hour and never guesses past it. The reply's root carries the
//! gate's `coverage` and `pending`: the requested recorded hours after the
//! prefix, which later preparation answers. A value inside an answered row
//! that needs history the prefix has not reached stays unknown, with the
//! product's `preparing` reason.
//! Accepted coverage does not prove finality for backdated observations or
//! branch-wide classifications. Such products mark their revisable hours
//! approximate, or independently prove a closed window before claiming it exact.

use super::coverage::{self, Coverage};
use crate::{
    Source,
    products::{ContentOptions, country_groups::HourSpan},
};
use serde::Serialize;
use serde_json::Value;
use std::{io, sync::atomic::AtomicBool};

/// The root fields of a reply answered from a gate's prepared prefix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedPrefix")]
pub struct Prefix {
    /// The gate's accepted coverage the reply reads through. A product's
    /// approximate spans identify accepted hours that later commits can revise.
    /// Before its first publication, the empty extent ends immediately before
    /// `firstHour`.
    pub coverage: Coverage,
    /// The requested recorded hours after `coverage.lastHour`, in order and
    /// without recorded gaps: still being prepared. Empty when the reply
    /// answers every requested hour.
    pub pending: Vec<HourSpan>,
}

impl Prefix {
    /// The prefix `coverage` gives a request on `branch` through `requested`
    /// (its last hour; None asks for the whole branch).
    pub fn new(source: &Source, branch: u64, coverage: Coverage, requested: Option<i32>) -> Self {
        let pending = if coverage.complete {
            Vec::new()
        } else {
            pending(source, branch, coverage.last_hour, requested)
        };
        Self { coverage, pending }
    }

    /// The last hour the reply answers: the requested hour, or the prefix's
    /// last hour when the request reaches past it. None asks for, and once
    /// coverage is complete answers, the whole branch.
    pub fn through(&self, requested: Option<i32>) -> Option<i32> {
        if self.coverage.complete {
            return requested;
        }
        Some(requested.map_or(self.coverage.last_hour, |hour| {
            hour.min(self.coverage.last_hour)
        }))
    }

    /// Drops the hours of `spans` after the prefix. A whole-branch reader
    /// names the hours after its last observation unobserved through the
    /// branch's end; before completion those hours are pending, not missing.
    pub fn clip(&self, spans: &mut Vec<HourSpan>) {
        if self.coverage.complete {
            return;
        }
        let last = self.coverage.last_hour;
        spans.retain(|span| span.first_hour <= last);
        for span in spans {
            span.last_hour = span.last_hour.min(last);
        }
    }

    /// Takes the hours a reply answers from sampled keyframes (its
    /// `approximate` spans) out of `pending`: they have geometry, marked
    /// approximate, and only the hours left are still being prepared.
    pub fn sampled(&mut self, approximate: &[HourSpan]) {
        self.answered(approximate);
    }

    /// Remove independently answered hours (sampled spans or proven newest
    /// windows) from the pending tail.
    pub fn answered(&mut self, approximate: &[HourSpan]) {
        self.pending = subtract(&self.pending, approximate);
    }

    /// Adds the prefix's fields to a reply's root object; null and other
    /// replies are left as they are.
    pub fn mark(&self, data: &mut Value) -> io::Result<()> {
        self.mark_coverage(data, "coverage")
    }

    /// Marks preparation separately from a product's capture evidence.
    pub fn mark_coverage(&self, data: &mut Value, field: &str) -> io::Result<()> {
        if let Some(reply) = data.as_object_mut() {
            reply.insert(field.into(), serde_json::to_value(&self.coverage)?);
            reply.insert("pending".into(), serde_json::to_value(&self.pending)?);
        }
        Ok(())
    }

    /// A later observation can revise any earlier value or classification.
    /// Without a recorded bound on that revision, every presented prefix
    /// hour is approximate until the branch is complete. Recorded gaps and
    /// the pending tail are never presented as approximate values.
    pub fn revisable(&self, source: &Source, branch: u64) -> Vec<HourSpan> {
        if self.coverage.complete {
            return Vec::new();
        }
        recorded(
            source,
            branch,
            self.coverage.first_hour,
            Some(self.coverage.last_hour),
        )
    }

    pub fn mark_revisable(&self, source: &Source, branch: u64, data: &mut Value) -> io::Result<()> {
        self.mark(data)?;
        if let Some(reply) = data.as_object_mut() {
            reply.insert(
                "approximate".into(),
                serde_json::to_value(self.revisable(source, branch))?,
            );
        }
        Ok(())
    }
}

/// The prefix `gate`'s queries answer `branch` from, for a request through
/// `requested` ([`Prefix::new`]); None before the gate accepts any hour.
pub fn read(
    source: &Source,
    content: Option<&ContentOptions>,
    gate: &str,
    branch: u64,
    requested: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Prefix>> {
    Ok(
        coverage::product_coverage(source, content, branch, gate, cancelled)?
            .map(|coverage| Prefix::new(source, branch, coverage, requested)),
    )
}

/// A prefix reply before the first publication has no accepted hour. Its
/// empty extent ends immediately before the branch's first recorded hour.
pub fn read_or_empty(
    source: &Source,
    content: Option<&ContentOptions>,
    gate: &str,
    branch: u64,
    requested: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<Prefix> {
    let accepted = coverage::product_coverage(source, content, branch, gate, cancelled)?;
    Ok(Prefix::new(
        source,
        branch,
        extent(source, branch, accepted)?,
        requested,
    ))
}

/// Accepted preparation coverage, or an empty extent immediately before the
/// branch's first recorded hour. An empty extent accepts no recorded hour.
pub fn extent(source: &Source, branch: u64, accepted: Option<Coverage>) -> io::Result<Coverage> {
    if let Some(coverage) = accepted {
        return Ok(coverage);
    }
    let first = source
        .block_windows(branch)
        .iter()
        .map(|window| window.first_hour)
        .min()
        .ok_or_else(|| crate::invalid("No recorded hours for this branch"))?;
    let last = first
        .checked_sub(1)
        .ok_or_else(|| crate::invalid("Prepared prefix has no preceding hour"))?;
    Ok(Coverage {
        schema: coverage::SCHEMA,
        first_hour: first,
        last_hour: last,
        complete: false,
    })
}

/// Marks a cold object reply with an empty preparation extent and every
/// requested recorded hour pending, without recorded gaps.
pub fn mark_unprepared(
    source: &Source,
    branch: u64,
    through: Option<i32>,
    data: &mut Value,
) -> io::Result<()> {
    Prefix::new(source, branch, extent(source, branch, None)?, through).mark(data)
}

/// Subtract hour spans, preserving the input order and integer end points.
pub(crate) fn subtract(spans: &[HourSpan], answered: &[HourSpan]) -> Vec<HourSpan> {
    let mut cuts = answered.to_vec();
    cuts.sort_unstable_by_key(|span| span.first_hour);
    let mut left = Vec::with_capacity(spans.len());
    for span in spans {
        // The next pending hour of `span` not yet placed, past i32 at the end.
        let mut first = i64::from(span.first_hour);
        for sampled in &cuts {
            if i64::from(sampled.last_hour) < first || sampled.first_hour > span.last_hour {
                continue;
            }
            if i64::from(sampled.first_hour) > first {
                left.push(HourSpan {
                    first_hour: first as i32,
                    last_hour: sampled.first_hour - 1,
                });
            }
            first = i64::from(sampled.last_hour) + 1;
        }
        if first <= i64::from(span.last_hour) {
            left.push(HourSpan {
                first_hour: first as i32,
                last_hour: span.last_hour,
            });
        }
    }
    left
}

/// The hours of one interval outside this branch's recorded gaps.
pub(crate) fn without_gaps(source: &Source, branch: u64, first: i32, last: i32) -> Vec<HourSpan> {
    if first > last {
        return Vec::new();
    }
    let gaps: Vec<_> = source
        .replay_ref()
        .gaps
        .iter()
        .filter(|gap| gap.branch == branch && gap.first_hour <= last && first <= gap.last_hour)
        .map(|gap| HourSpan {
            first_hour: gap.first_hour,
            last_hour: gap.last_hour,
        })
        .collect();
    subtract(
        &[HourSpan {
            first_hour: first,
            last_hour: last,
        }],
        &gaps,
    )
}

/// The recorded hours of `branch` after `after` through `through` (the
/// branch's last block when None): its blocks' hours, joined where blocks
/// meet, without the recorded gaps between them.
pub fn pending(source: &Source, branch: u64, after: i32, through: Option<i32>) -> Vec<HourSpan> {
    let Some(first) = after.checked_add(1) else {
        return Vec::new();
    };
    recorded(source, branch, first, through)
}

fn recorded(source: &Source, branch: u64, first: i32, through: Option<i32>) -> Vec<HourSpan> {
    let mut windows = source.block_windows(branch).to_vec();
    windows.sort_unstable_by_key(|window| window.first_hour);
    let mut spans: Vec<HourSpan> = Vec::new();
    for window in windows {
        let from = window.first_hour.max(first);
        let to = through.map_or(window.last_hour, |through| window.last_hour.min(through));
        if from > to {
            continue;
        }
        for span in without_gaps(source, branch, from, to) {
            crate::products::country_groups::mark_unobserved(
                &mut spans,
                span.first_hour,
                span.last_hour,
            );
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{continuation_records, file_source, replay, source as frame};

    /// Blocks at hours 0..=1, 2..=3 and, after a recorded gap, 10..=11.
    fn source() -> Source {
        let mut recording = replay(vec![frame(1, 0, None), frame(1, 1, None)]);
        for hours in [[2, 3], [10, 11]] {
            recording = recording
                .append(&[continuation_records(
                    &recording,
                    1,
                    hours.iter().map(|&hour| frame(1, hour, None)).collect(),
                )])
                .unwrap();
        }
        let (source, root) = file_source(recording);
        let _ = std::fs::remove_dir_all(root);
        source
    }

    fn coverage(last_hour: i32, complete: bool) -> Coverage {
        Coverage {
            schema: coverage::SCHEMA,
            first_hour: 0,
            last_hour,
            complete,
        }
    }

    fn span(first_hour: i32, last_hour: i32) -> HourSpan {
        HourSpan {
            first_hour,
            last_hour,
        }
    }

    #[test]
    fn pending_names_the_recorded_hours_after_the_prefix_through_the_request() {
        let source = source();
        let partial = Prefix::new(&source, 1, coverage(1, false), None);
        assert_eq!(partial.pending, [span(2, 3), span(10, 11)]);
        assert_eq!(partial.through(None), Some(1));
        assert_eq!(partial.through(Some(10)), Some(1));
        assert_eq!(partial.through(Some(0)), Some(0));
        let bounded = Prefix::new(&source, 1, coverage(2, false), Some(10));
        assert_eq!(bounded.pending, [span(3, 3), span(10, 10)]);
        // A request inside the prefix, or through a gap only, waits for nothing.
        assert!(
            Prefix::new(&source, 1, coverage(3, false), Some(9))
                .pending
                .is_empty()
        );
        assert!(
            Prefix::new(&source, 1, coverage(1, false), Some(1))
                .pending
                .is_empty()
        );
        let complete = Prefix::new(&source, 1, coverage(11, true), None);
        assert!(complete.pending.is_empty());
        assert_eq!(complete.through(None), None);
        assert_eq!(complete.through(Some(5)), Some(5));
        assert!(pending(&source, 1, i32::MAX, None).is_empty());
        assert!(pending(&source, 7, 0, None).is_empty());
    }

    #[test]
    fn clipping_leaves_unobserved_hours_inside_the_prefix_only() {
        let source = source();
        let mut spans = vec![span(0, 0), span(1, 11), span(4, 9)];
        Prefix::new(&source, 1, coverage(1, false), None).clip(&mut spans);
        assert_eq!(spans, [span(0, 0), span(1, 1)]);
        let mut whole = vec![span(4, 9)];
        Prefix::new(&source, 1, coverage(11, true), None).clip(&mut whole);
        assert_eq!(whole, [span(4, 9)]);
    }

    #[test]
    fn sampled_hours_leave_only_the_unsampled_hours_pending() {
        let source = source();
        let mut prefix = Prefix::new(&source, 1, coverage(1, false), None);
        assert_eq!(prefix.pending, [span(2, 3), span(10, 11)]);
        prefix.sampled(&[span(3, 3), span(10, 10)]);
        assert_eq!(prefix.pending, [span(2, 2), span(11, 11)]);
        prefix.sampled(&[span(0, 20)]);
        assert!(prefix.pending.is_empty());
        let mut edge = Prefix {
            coverage: coverage(1, false),
            pending: vec![span(5, i32::MAX)],
        };
        edge.sampled(&[span(i32::MAX, i32::MAX)]);
        assert_eq!(edge.pending, [span(5, i32::MAX - 1)]);
        edge.sampled(&[]);
        assert_eq!(edge.pending, [span(5, i32::MAX - 1)]);
    }

    #[test]
    fn shared_span_join_keeps_overlapping_intervals_at_the_last_integer_hour() {
        let mut spans = vec![span(i32::MAX - 1, i32::MAX)];
        crate::products::country_groups::mark_unobserved(&mut spans, i32::MAX, i32::MAX);
        assert_eq!(spans, [span(i32::MAX - 1, i32::MAX)]);
    }

    #[test]
    fn subtraction_handles_unordered_overlapping_cuts_and_integer_edges() {
        assert_eq!(
            subtract(
                &[span(i32::MIN, i32::MAX)],
                &[
                    span(10, i32::MAX),
                    span(0, 2),
                    span(i32::MIN, -2),
                    span(1, 5),
                ]
            ),
            [span(-1, -1), span(6, 9)],
        );
    }

    #[test]
    fn marking_adds_coverage_and_pending_to_an_object_reply_only() {
        let source = source();
        let prefix = Prefix::new(&source, 1, coverage(1, false), Some(3));
        let mut data = serde_json::json!({"rows": [1]});
        prefix.mark(&mut data).unwrap();
        assert_eq!(
            data,
            serde_json::json!({"rows": [1],
                "coverage": {"schema": coverage::SCHEMA, "firstHour": 0, "lastHour": 1, "complete": false},
                "pending": [{"firstHour": 2, "lastHour": 3}]})
        );
        let mut null = Value::Null;
        prefix.mark(&mut null).unwrap();
        assert!(null.is_null());
    }

    #[test]
    fn named_and_cold_prefixes_preserve_capture_coverage() {
        let source = source();
        let prefix = Prefix::new(&source, 1, coverage(1, false), Some(3));
        let captured = serde_json::json!([{"firstHour": 0, "lastHour": 1}]);
        let mut data = serde_json::json!({"coverage": captured});
        prefix.mark_coverage(&mut data, "preparedCoverage").unwrap();
        assert_eq!(data["coverage"], captured);
        assert_eq!(
            data["preparedCoverage"],
            serde_json::to_value(&prefix.coverage).unwrap()
        );
        assert_eq!(
            data["pending"],
            serde_json::to_value(&prefix.pending).unwrap()
        );
        let cold = Prefix::new(&source, 1, extent(&source, 1, None).unwrap(), Some(3));
        cold.mark_coverage(&mut data, "preparedCoverage").unwrap();
        assert_eq!(data["coverage"], captured);
        assert_eq!(
            data["preparedCoverage"],
            serde_json::to_value(&cold.coverage).unwrap()
        );
        assert_eq!(
            data["pending"],
            serde_json::to_value(&cold.pending).unwrap()
        );
    }

    #[test]
    fn unbounded_revisions_mark_every_presented_recorded_hour_until_complete() {
        let source = source();
        let prefix = Prefix::new(&source, 1, coverage(10, false), Some(11));
        let mut data = serde_json::json!({"rows": []});
        prefix.mark_revisable(&source, 1, &mut data).unwrap();
        assert_eq!(
            data["approximate"],
            serde_json::json!([
                {"firstHour": 0, "lastHour": 3}, {"firstHour": 10, "lastHour": 10}
            ])
        );
        assert_eq!(
            data["pending"],
            serde_json::json!([{"firstHour": 11, "lastHour": 11}])
        );
        let complete = Prefix::new(&source, 1, coverage(11, true), None);
        complete.mark_revisable(&source, 1, &mut data).unwrap();
        assert_eq!(data["approximate"], serde_json::json!([]));
        assert_eq!(data["pending"], serde_json::json!([]));
    }
    #[test]
    fn a_cold_reply_names_every_recorded_hour_pending() {
        let source = source();
        let mut data = serde_json::json!({"rows":[]});
        mark_unprepared(&source, 1, Some(10), &mut data).unwrap();
        assert_eq!(
            data["coverage"],
            serde_json::to_value(coverage(-1, false)).unwrap()
        );
        assert_eq!(
            data["pending"],
            serde_json::json!([
                {"firstHour":0,"lastHour":3}, {"firstHour":10,"lastHour":10}
            ])
        );
    }

    #[test]
    fn gaps_inside_a_block_are_never_pending() {
        let mut replay = replay((0..=3).map(|hour| frame(1, hour, None)).collect());
        replay.gaps.push(replay::branches::Gap {
            branch: 1,
            first_hour: 1,
            last_hour: 2,
            reason: "synthetic gap".into(),
        });
        let (source, root) = file_source(replay);
        assert_eq!(pending(&source, 1, 0, None), [span(3, 3)]);
        let mut cold = serde_json::json!({});
        mark_unprepared(&source, 1, None, &mut cold).unwrap();
        assert_eq!(
            cold["pending"],
            serde_json::to_value([span(0, 0), span(3, 3)]).unwrap()
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
