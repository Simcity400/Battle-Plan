
use super::fresh_search::{available, index};
use content_reader_unit_path::unit_path::Fact;
use replay_data::{
    rows::FreshTaskForceCounterEta, rows::ShipRow, rows::TableSchema, rows::TaskForceMapMotionRow,
    rows::TaskForceRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, sync::OnceLock};
#[cfg(test)]
use dashboard_text::table_access::TokenError;
use dashboard_text::table_access::TokenNames;

type Index<'a, K, R> = OnceLock<Fact<BTreeMap<K, &'a R>>>;

fn rows<'s, 'a, T: TableSchema, K: Ord>(
    cell: &'s Index<'a, K, T::Row>,
    frame: &'a DecodedFrame,
    key: impl Fn(&T::Row) -> K,
) -> Fact<&'s BTreeMap<K, &'a T::Row>> {
    available(cell.get_or_init(|| index::<T, _>(frame, key)))
}

pub(super) struct NavalInputs<'a> {
    frame: &'a DecodedFrame,
    forces: Index<'a, u64, TaskForceRow>,
    motions: Index<'a, u64, TaskForceMapMotionRow>,
    ships: OnceLock<Fact<BTreeMap<u64, Vec<&'a ShipRow>>>>,
    tokens: OnceLock<Fact<TokenNames<'a>>>,
}

impl<'a> NavalInputs<'a> {
    pub(super) fn new(frame: &'a DecodedFrame) -> Self {
        Self {
            frame,
            forces: OnceLock::new(),
            motions: OnceLock::new(),
            ships: OnceLock::new(),
            tokens: OnceLock::new(),
        }
    }

    pub(super) fn tf(&self, handle: u64) -> Fact<&'a TaskForceRow> {
        rows::<tables::TaskForces, _>(&self.forces, self.frame, |row| row.id)?
            .get(&handle)
            .copied()
            .ok_or_else(|| format!("fresh naval ETA task force {handle:#x} is not recorded"))
    }

    pub(super) fn motion(&self, handle: u64) -> Fact<&'a TaskForceMapMotionRow> {
        rows::<tables::TaskForceMapMotion, _>(&self.motions, self.frame, |row| row.task_force)?
            .get(&handle)
            .copied()
            .ok_or_else(|| format!("fresh naval ETA task force {handle:#x} has no motion row"))
    }

    pub(super) fn fresh(&self, handle: u64) -> Fact<&'a FreshTaskForceCounterEta> {
        let fresh = self
            .motion(handle)?
            .fresh
            .require("TaskForceMapMotion.fresh")?
            .as_ref()
            .ok_or("fresh naval ETA task-force packet was refused")?;
        if !fresh.sample_hour.fresh_at(self.frame.hour) {
            return Err("fresh naval ETA task-force packet belongs to another hour".into());
        }
        Ok(fresh)
    }

    pub(super) fn tokens(&self) -> Fact<TokenNames<'a>> {
        available(self.tokens.get_or_init(|| {
            self.frame
                .complete::<tables::LexicalTokens>()
                .map_err(|error| error.to_string())?;
            TokenNames::new(self.frame).map_err(|error| error.to_string())
        }))
        .copied()
    }

    pub(super) fn ships(&self, handle: u64) -> Fact<&[&'a ShipRow]> {
        let groups = available(self.ships.get_or_init(|| {
            let mut out: BTreeMap<u64, Vec<&'a ShipRow>> = BTreeMap::new();
            for row in index::<tables::Ships, _>(self.frame, |row| row.id)?.into_values() {
                out.entry(row.task_force).or_default().push(row);
            }
            Ok(out)
        }))?;
        Ok(groups.get(&handle).map_or(&[], Vec::as_slice))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::Recorded, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};

    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 20,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture domain unavailable".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        }
    }
    fn put(frame: &mut DecodedFrame, rows: TableRows) {
        let table = rows.table();
        *frame.tables[table.index()] = TableView::Complete(rows);
    }

    #[test]
    fn the_fresh_speed_packet_requires_the_exact_hour() {
        let mut frame = frame();
        for (hour, current) in [(19, false), (20, true)] {
            put(
                &mut frame,
                TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
                    task_force: 7,
                    fresh: Recorded::Value(Some(FreshTaskForceCounterEta {
                        sample_hour: replay_data::rows::PacketHour::Stamped(hour),
                        base_speed: Some(100_000),
                        ..Default::default()
                    })),
                    ..Default::default()
                }]),
            );
            let inputs = NavalInputs::new(&frame);
            assert_eq!(inputs.fresh(7).is_ok(), current);
            if !current {
                assert!(inputs.fresh(7).unwrap_err().contains("another hour"));
            }
        }
    }

    #[test]
    fn ships_group_by_task_force_and_an_empty_roster_is_known() {
        let mut frame = frame();
        let inputs = NavalInputs::new(&frame);
        assert!(inputs.ships(7).is_err());
        put(
            &mut frame,
            TableRows::Ships(vec![
                ShipRow {
                    id: 17,
                    task_force: 7,
                    ..Default::default()
                },
                ShipRow {
                    id: 18,
                    task_force: 8,
                    ..Default::default()
                },
            ]),
        );
        let inputs = NavalInputs::new(&frame);
        assert_eq!(
            inputs
                .ships(7)
                .unwrap()
                .iter()
                .map(|ship| ship.id)
                .collect::<Vec<_>>(),
            [17]
        );
        assert!(inputs.ships(9).unwrap().is_empty());
    }

    #[test]
    fn class_tokens_require_complete_strict_key_order() {
        use replay_data::rows::LexicalTokenRow;
        let mut frame = frame();
        assert!(NavalInputs::new(&frame).tokens().is_err());
        put(
            &mut frame,
            TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 17,
                literal: b"authored_naval_class".to_vec(),
            }]),
        );
        let inputs = NavalInputs::new(&frame);
        assert_eq!(
            inputs.tokens().unwrap().name(17),
            Ok("authored_naval_class")
        );
        assert_eq!(
            inputs.tokens().unwrap().name(18),
            Err(TokenError::Absent(18))
        );
        put(
            &mut frame,
            TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 17,
                    literal: b"first".to_vec(),
                },
                LexicalTokenRow {
                    token: 17,
                    literal: b"second".to_vec(),
                },
            ]),
        );
        assert!(
            NavalInputs::new(&frame)
                .tokens()
                .unwrap_err()
                .contains("are not in order")
        );
    }
}
