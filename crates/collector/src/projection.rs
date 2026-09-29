//! Exact projections from the current owned native sample into replay baselines.
//!
//! Only ordering scratch is retained. Every reuse compares all current row
//! payloads, after the caller's acquisition and arena checks, with the encoder's
//! complete baseline. A stable permutation avoids repeatedly sorting the same
//! census; every use verifies its order against the current native keys.
//! Portrait acquisition may retain owned descriptors certified current by its
//! native mutation observer; this is distinct from carrying an older sample.

use crate::spooling::{table, unavailable};
use engine::walk::Outcome;
use replay::{
    BranchId, Hour,
    frame::{FrameEncoder, TABLE_COUNT, TableInput, TableView},
    rows::{RowCodec, TableRows, TableSchema},
};
use serde_json::{Value, json};

#[derive(Default)]
struct Ordering {
    indices: Vec<usize>,
    compared: u64,
    reused: u64,
    converted: u64,
    updated: u64,
    updated_rows: u64,
    sorted: u64,
}

pub(crate) struct ProjectionScratch {
    tables: [Ordering; TABLE_COUNT],
}

impl Default for ProjectionScratch {
    fn default() -> Self {
        Self {
            tables: std::array::from_fn(|_| Ordering::default()),
        }
    }
}

impl ProjectionScratch {
    pub(crate) fn json(&self) -> Value {
        json!({
            "scope": "current complete native payload compared with encoder baseline; cumulative conversion decisions including later failed writes",
            "tables": replay::rows::TableId::ALL.into_iter().zip(&self.tables).filter(|(_, order)| order.compared != 0 || order.converted != 0).map(|(table, order)| json!({
                "table": format!("{table:?}"),
                "compared": order.compared,
                "reused": order.reused,
                "converted": order.converted,
                "updated": order.updated,
                "updated_rows": order.updated_rows,
                "sorted": order.sorted,
                "ordering_capacity_bytes": order.indices.capacity() * std::mem::size_of::<usize>(),
            })).collect::<Vec<_>>(),
        })
    }
}

pub(crate) struct Projection<'a> {
    encoder: &'a FrameEncoder,
    branch: BranchId,
    hour: Hour,
    force: bool,
    scratch: &'a mut ProjectionScratch,
}

impl<'a> Projection<'a> {
    pub(crate) fn new(
        encoder: &'a FrameEncoder,
        branch: BranchId,
        hour: Hour,
        force: bool,
        scratch: &'a mut ProjectionScratch,
    ) -> Self {
        Self {
            encoder,
            branch,
            hour,
            force,
            scratch,
        }
    }

    fn table<S: TableSchema, T>(
        &mut self,
        outcome: Outcome,
        source: &[T],
        key: impl Fn(&T) -> u128,
        equal: impl Fn(&T, &S::Row) -> bool,
        convert: impl Fn(&T) -> S::Row,
    ) -> TableInput {
        let Some(source) = source.get(..outcome.len()) else {
            return unavailable("walker row count exceeds allocated buffer").into();
        };
        match outcome {
            Outcome::Unavailable { reason } => return unavailable(reason).into(),
            Outcome::Truncated { unread_rows: 0, .. } => {
                return unavailable("walker marked truncation without a missing count").into();
            }
            _ => {}
        }
        let order = &mut self.scratch.tables[S::ID.index()];
        let natural = source.windows(2).all(|rows| key(&rows[0]) < key(&rows[1]));
        if !natural {
            if order.indices.len() != source.len() {
                order.indices.clear();
                order.indices.extend(0..source.len());
            }
            if !order
                .indices
                .windows(2)
                .all(|pair| key(&source[pair[0]]) < key(&source[pair[1]]))
            {
                order
                    .indices
                    .sort_unstable_by_key(|index| key(&source[*index]));
                order.sorted = order.sorted.saturating_add(1);
            }
            if order
                .indices
                .windows(2)
                .any(|pair| key(&source[pair[0]]) == key(&source[pair[1]]))
            {
                return unavailable("walker produced duplicate engine ids").into();
            }
        }
        let at = |index| &source[if natural { index } else { order.indices[index] }];
        // Staggered tables also need a recovery cache for skipped hours. Their
        // existing immutable-cache path owns that contract; this path is hourly.
        if matches!(outcome, Outcome::Rows { .. })
            && !replay::cadence::is_staggered(S::ID)
            && let Some((token, baseline)) =
                self.encoder
                    .comparison_baseline::<S>(self.branch, self.hour, self.force)
        {
            order.compared = order.compared.saturating_add(1);
            if token.allows_row_updates() && source.len() == baseline.len() {
                let mut indices = Vec::new();
                let same_keys = baseline.iter().enumerate().all(|(index, previous)| {
                    let current = at(index);
                    if key(current) != previous.key() {
                        return false;
                    }
                    if !equal(current, previous) {
                        indices.push(index);
                    }
                    true
                });
                if same_keys {
                    if indices.is_empty() {
                        order.reused = order.reused.saturating_add(1);
                        return TableInput::ComparedEqual(token);
                    }
                    let rows = TableRows::from_rows::<S>(
                        indices.iter().map(|&index| convert(at(index))).collect(),
                    );
                    order.updated = order.updated.saturating_add(1);
                    order.updated_rows = order.updated_rows.saturating_add(indices.len() as u64);
                    return TableInput::ComparedChanges {
                        token,
                        indices,
                        rows,
                    };
                }
            }
            if source.len() == baseline.len()
                && baseline.iter().enumerate().all(|(index, previous)| {
                    let current = at(index);
                    key(current) == previous.key() && equal(current, previous)
                })
            {
                order.reused = order.reused.saturating_add(1);
                return TableInput::ComparedEqual(token);
            }
        }
        let rows =
            TableRows::from_rows::<S>((0..source.len()).map(|index| convert(at(index))).collect());
        order.converted = order.converted.saturating_add(1);
        match outcome {
            Outcome::Truncated { unread_rows, .. } => TableView::Truncated {
                rows,
                missing: unread_rows,
            },
            _ => TableView::Complete(rows),
        }
        .into()
    }
}

/// `key` must be the replay key and `equal` must compare every replay field.
/// All source and arena checks are performed before calling this function.
pub(crate) fn project_table<S: TableSchema, T>(
    projection: Option<&mut Projection<'_>>,
    outcome: Outcome,
    source: &[T],
    key: impl Fn(&T) -> u128,
    equal: impl Fn(&T, &S::Row) -> bool,
    convert: impl Fn(&T) -> S::Row,
) -> TableInput {
    match projection {
        Some(projection) => projection.table::<S, _>(outcome, source, key, equal, convert),
        None => table::<S, _>(outcome, source, convert).into(),
    }
}

/// Scalar projections compare without allocating an intermediate replay row
/// vector. Use `project_table` for rows owning strings or nested vectors.
pub(crate) fn scalar_table<S: TableSchema, T>(
    projection: Option<&mut Projection<'_>>,
    outcome: Outcome,
    source: &[T],
    key: impl Fn(&T) -> u128,
    convert: impl Fn(&T) -> S::Row,
) -> TableInput {
    project_table::<S, _>(
        projection,
        outcome,
        source,
        key,
        |native, previous| convert(native) == *previous,
        &convert,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::{DecodedFrame, FrameDecoder, FrameInput, Metrics},
        rows::{StateRow, TableId, tables},
    };
    use std::cell::Cell;

    fn input(hour: Hour, table: TableInput) -> FrameInput {
        let mut frame = FrameInput {
            branch: 1,
            hour,
            tables: std::array::from_fn(|_| unavailable("fixture table").into()),
            metrics: Metrics::default(),
        };
        frame.tables[TableId::States.index()] = table;
        frame
    }

    fn project(
        encoder: &FrameEncoder,
        scratch: &mut ProjectionScratch,
        hour: Hour,
        outcome: Outcome,
        rows: &[StateRow],
        conversions: &Cell<usize>,
    ) -> TableInput {
        project_table::<tables::States, _>(
            Some(&mut Projection::new(encoder, 1, hour, false, scratch)),
            outcome,
            rows,
            |row| u128::from(row.id),
            |row, previous| row == previous,
            |row| {
                conversions.set(conversions.get() + 1);
                row.clone()
            },
        )
    }

    #[test]
    fn current_native_rows_reuse_without_materialization_and_keep_exact_delta_behavior() {
        let mut projected = FrameEncoder::new();
        let mut ordinary = FrameEncoder::new();
        let mut projected_decoder = FrameDecoder::new();
        let mut ordinary_decoder = FrameDecoder::new();
        let mut scratch = ProjectionScratch::default();
        let conversions = Cell::new(0);
        let state = |id, owner| StateRow {
            id,
            owner,
            controller: owner + 1,
            movement_infrastructure: 0,
            ..Default::default()
        };
        let mut rows = vec![state(3, 30), state(1, 10), state(2, 20)];
        for hour in 0..8 {
            match hour {
                1 | 2 => rows.rotate_left(1),
                3 => rows.sort_unstable_by_key(|row| row.id),
                4 => rows[1].owner += 100,
                5 => {
                    rows.remove(0);
                    rows.push(state(4, 40));
                }
                6 => rows.reverse(),
                _ => {}
            }
            let outcome = Outcome::Rows { len: rows.len() };
            let before = conversions.get();
            let current = project(&projected, &mut scratch, hour, outcome, &rows, &conversions);
            let reused = matches!(hour, 1 | 2 | 3 | 6 | 7);
            assert_eq!(matches!(current, TableInput::ComparedEqual(_)), reused);
            assert_eq!(
                conversions.get() - before,
                if reused {
                    0
                } else if hour == 4 {
                    1
                } else {
                    rows.len()
                }
            );
            let baseline = table::<tables::States, _>(outcome, &rows, Clone::clone);
            let actual = projected.encode_input(input(hour, current), false).unwrap();
            let expected = ordinary
                .encode_input(input(hour, baseline.into()), false)
                .unwrap();
            assert_eq!(actual, expected);
            let actual: DecodedFrame = projected_decoder.apply(&actual).unwrap();
            assert_eq!(actual, ordinary_decoder.apply(&expected).unwrap());
        }
        assert_eq!(scratch.tables[TableId::States.index()].reused, 5);
    }

    #[test]
    fn duplicate_partial_and_failed_sources_never_claim_an_equal_complete_table() {
        let mut encoder = FrameEncoder::new();
        let mut scratch = ProjectionScratch::default();
        let conversions = Cell::new(0);
        let rows = [
            StateRow {
                id: 3,
                owner: 7,
                controller: 9,
                movement_infrastructure: 0,
                ..Default::default()
            },
            StateRow {
                id: 1,
                owner: 2,
                controller: 4,
                movement_infrastructure: 0,
                ..Default::default()
            },
        ];
        let complete = Outcome::Rows { len: 2 };
        let first = project(&encoder, &mut scratch, 0, complete, &rows, &conversions);
        encoder.encode_input(input(0, first), false).unwrap();
        for (outcome, source) in [
            (complete, vec![rows[0].clone(), rows[0].clone()]),
            (Outcome::Rows { len: 3 }, rows.to_vec()),
            (
                Outcome::Unavailable {
                    reason: "fresh fault",
                },
                rows.to_vec(),
            ),
            (
                Outcome::Truncated {
                    len: 2,
                    unread_rows: 0,
                },
                rows.to_vec(),
            ),
        ] {
            let rejected = project(&encoder, &mut scratch, 1, outcome, &source, &conversions);
            assert!(matches!(
                rejected,
                TableInput::Sampled(TableView::Unavailable { .. })
            ));
        }
        let partial = project(
            &encoder,
            &mut scratch,
            1,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1,
            },
            &rows,
            &conversions,
        );
        assert!(matches!(
            partial,
            TableInput::Sampled(TableView::Truncated { missing: 1, .. })
        ));
        encoder.encode_input(input(1, partial), false).unwrap();
        assert!(matches!(
            project(&encoder, &mut scratch, 2, complete, &rows, &conversions),
            TableInput::Sampled(TableView::Complete(_))
        ));
    }

    #[test]
    fn nonconsecutive_branch_and_recovery_hours_require_current_owned_rows() {
        let mut encoder = FrameEncoder::new();
        let mut scratch = ProjectionScratch::default();
        let conversions = Cell::new(0);
        let rows = [StateRow {
            id: 1,
            owner: 2,
            controller: 4,
            movement_infrastructure: 0,
            ..Default::default()
        }];
        let outcome = Outcome::Rows { len: 1 };
        let first = project(&encoder, &mut scratch, 0, outcome, &rows, &conversions);
        encoder.encode_input(input(0, first), false).unwrap();
        for (branch, hour, force) in [(1, 2, false), (2, 1, false), (1, 1, true)] {
            let current = scalar_table::<tables::States, _>(
                Some(&mut Projection::new(
                    &encoder,
                    branch,
                    hour,
                    force,
                    &mut scratch,
                )),
                outcome,
                &rows,
                |row| u128::from(row.id),
                Clone::clone,
            );
            assert!(matches!(
                current,
                TableInput::Sampled(TableView::Complete(_))
            ));
        }
    }
}
