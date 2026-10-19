//! Shared regional score views and the recording oracle's keyed delta reader.

use replay_data::rows::RaidAirScoreRow;
#[cfg(test)]
use replay_data::{rows::TableId, rows::TableRows};
#[cfg(test)]
use replay_frame::frame::TableUpdate;
use std::collections::HashMap;
#[cfg(test)]
use std::{collections::HashSet, io};

pub(super) type OwnerIndex = HashMap<u32, HashMap<u32, usize>>;

#[derive(Clone, Copy)]
pub(super) struct ScoreView<'a> {
    pub rows: &'a [RaidAirScoreRow],
    pub complete: bool,
    pub owners: Option<&'a OwnerIndex>,
    pub active: Option<&'a HashMap<u32, usize>>,
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct ScoreRead {
    rows: Vec<RaidAirScoreRow>,
    // Delta keys arrive in country order. Keep each owner's small region index
    // together: one owner lookup per changed group, rather than a random lookup
    // into a campaign-sized u128 map for every pair.
    indices: OwnerIndex,
    active: HashMap<u32, usize>,
    baseline: Option<(u64, i32)>,
    complete: bool,
}
#[cfg(test)]
impl ScoreRead {
    pub fn view(&self) -> ScoreView<'_> {
        ScoreView {
            rows: &self.rows,
            complete: self.complete,
            owners: Some(&self.indices),
            active: Some(&self.active),
        }
    }

    // A failed table becomes unavailable, just as in FrameDecoder. No partial
    // mutation may contribute regions or lend a baseline to the following hour.
    pub fn observe(
        &mut self,
        frame: &(impl replay_frame::frame::FrameSource + ?Sized),
        regions: &mut HashSet<u32>,
    ) -> usize {
        match self.update(frame, regions) {
            Ok((added, examined)) => {
                regions.extend(added);
                examined
            }
            Err(_) => {
                self.rows.clear();
                self.indices.clear();
                self.active.clear();
                self.baseline = None;
                self.complete = false;
                0
            }
        }
    }

    fn update(
        &mut self,
        frame: &(impl replay_frame::frame::FrameSource + ?Sized),
        regions: &HashSet<u32>,
    ) -> io::Result<(HashSet<u32>, usize)> {
        let update = frame.update(TableId::RaidAirScores.index());
        if frame.keyframe() {
            self.baseline = None;
        }
        let mut added = HashSet::new();
        let examined;
        match update {
            TableUpdate::Snapshot(TableRows::RaidAirScores(rows))
            | TableUpdate::Truncated {
                rows: TableRows::RaidAirScores(rows),
                ..
            } => {
                self.rows.clone_from(rows);
                for indices in self.indices.values_mut() {
                    indices.clear();
                }
                self.active.clear();
                self.complete = matches!(update, TableUpdate::Snapshot(_));
                let mut offset = 0;
                for owner_rows in rows.chunk_by(|a, b| a.country == b.country) {
                    let indices = self.indices.entry(owner_rows[0].country).or_default();
                    indices.reserve(owner_rows.len());
                    for (at, row) in owner_rows.iter().enumerate() {
                        indices.insert(row.region, offset + at);
                        if row.friendly > 0 || row.enemy > 0 {
                            *self.active.entry(row.region).or_default() += 1;
                        }
                        if (row.friendly != 0 || row.enemy != 0) && !regions.contains(&row.region) {
                            added.insert(row.region);
                        }
                    }
                    offset += owner_rows.len();
                }
                self.indices.retain(|_, indices| !indices.is_empty());
                examined = rows.len();
            }
            TableUpdate::Delta {
                baseline,
                changed,
                deleted,
            } => {
                self.check_baseline(frame, *baseline)?;
                for key in deleted {
                    let index = self
                        .indices
                        .get_mut(&((*key >> 32) as u32))
                        .and_then(|regions| regions.remove(&(*key as u32)))
                        .filter(|index| self.rows[*index].key() == *key)
                        .ok_or_else(|| super::invalid("delta deletes an absent air score key"))?;
                    let removed = self.rows.swap_remove(index);
                    update_activity(&mut self.active, Some(&removed), None);
                    if let Some(moved) = self.rows.get(index) {
                        self.indices
                            .get_mut(&moved.country)
                            .expect("indexed score owner")
                            .insert(moved.region, index);
                    }
                }
                for owner_changes in changed.chunk_by(|a, b| a.key >> 32 == b.key >> 32) {
                    let country = (owner_changes[0].key >> 32) as u32;
                    let indices = self.indices.entry(country).or_default();
                    for delta in owner_changes {
                        let region = delta.key as u32;
                        let index = indices.get(&region).copied();
                        let old = index.map(|index| &self.rows[index]);
                        let row = delta.apply::<RaidAirScoreRow>(old.cloned())?;
                        // A previously nonzero pair already published its region.
                        // Most score changes need no global catalogue lookup.
                        if (row.friendly != 0 || row.enemy != 0)
                            && old.is_none_or(|old| old.friendly == 0 && old.enemy == 0)
                            && !regions.contains(&row.region)
                        {
                            added.insert(row.region);
                        }
                        update_activity(&mut self.active, old, Some(&row));
                        if let Some(index) = index {
                            self.rows[index] = row;
                        } else {
                            indices.insert(region, self.rows.len());
                            self.rows.push(row);
                        }
                    }
                }
                examined = changed.len() + deleted.len();
            }
            TableUpdate::Unchanged { baseline } => {
                self.check_baseline(frame, *baseline)?;
                examined = 0;
            }
            TableUpdate::Unavailable { .. } | TableUpdate::Incompatible { .. } => {
                self.rows.clear();
                self.indices.clear();
                self.active.clear();
                self.complete = false;
                examined = 0;
            }
            _ => {
                return Err(super::invalid(
                    "air score update contains another table's rows",
                ));
            }
        }
        self.baseline = self.complete.then_some((frame.branch(), frame.hour()));
        Ok((added, examined))
    }

    fn check_baseline(
        &self,
        frame: &(impl replay_frame::frame::FrameSource + ?Sized),
        baseline: i32,
    ) -> io::Result<()> {
        if self.baseline == Some((frame.branch(), baseline)) && baseline < frame.hour() {
            Ok(())
        } else {
            Err(super::invalid(format!(
                "missing air score baseline at hour {baseline}"
            )))
        }
    }
}

// Scores usually change while their region remains active. Only zero crossings,
// insertions and deletions touch the region counts.
#[cfg(test)]
fn update_activity(
    active: &mut HashMap<u32, usize>,
    old: Option<&RaidAirScoreRow>,
    new: Option<&RaidAirScoreRow>,
) {
    let was = old
        .filter(|row| row.friendly > 0 || row.enemy > 0)
        .map(|row| row.region);
    let now = new
        .filter(|row| row.friendly > 0 || row.enemy > 0)
        .map(|row| row.region);
    if was == now {
        return;
    }
    if let Some(region) = was {
        let count = active.get_mut(&region).expect("active score region");
        *count -= 1;
        if *count == 0 {
            active.remove(&region);
        }
    }
    if let Some(region) = now {
        *active.entry(region).or_default() += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::DeltaRow, rows::RowCodec, rows::tables};
    use replay_frame::{frame::FrameDecoder, frame::FrameEncoder, frame::TableView};

    #[test]
    fn air_superiority_score_index_matches_decoder_and_visits_only_changed_pairs() {
        for countries in [1500, 5000] {
            let mut frame = dashboard_core::test_fixtures::scale_frame(1, 0, countries);
            let initial: Vec<_> = (0..countries)
                .map(|country| RaidAirScoreRow {
                    country: 100_000 + country,
                    region: 7000,
                    friendly: i64::from(country == 0),
                    enemy: 0,
                    impact: Default::default(),
                    friendly_missions: 0,
                    enemy_missions: 0,
                    air_spotting_scale: Some(0),
                })
                .collect();
            *frame.tables[TableId::RaidAirScores.index()] =
                TableView::Complete(TableRows::RaidAirScores(initial.clone()));
            let mut encoder = FrameEncoder::new();
            let mut decoder = FrameDecoder::for_tables(&[TableId::RaidAirScores]);
            let mut read = ScoreRead::default();
            let mut regions = HashSet::new();
            let mut expected_regions = HashSet::new();
            for hour in 0..12 {
                frame.hour = hour;
                match hour {
                    2 => {
                        let TableView::Complete(TableRows::RaidAirScores(rows)) =
                            &mut *frame.tables[TableId::RaidAirScores.index()]
                        else {
                            unreachable!()
                        };
                        rows[1].friendly = 5;
                    }
                    3 => {
                        let TableView::Complete(TableRows::RaidAirScores(rows)) =
                            &mut *frame.tables[TableId::RaidAirScores.index()]
                        else {
                            unreachable!()
                        };
                        rows.remove(0);
                        rows.push(RaidAirScoreRow {
                            country: 200_000,
                            region: 9900,
                            friendly: 8,
                            enemy: 0,
                            impact: Default::default(),
                            friendly_missions: 0,
                            enemy_missions: 0,
                            air_spotting_scale: Some(0),
                        });
                    }
                    4 => {
                        *frame.tables[TableId::RaidAirScores.index()] = TableView::Truncated {
                            rows: TableRows::RaidAirScores(initial[..3].to_vec()),
                            missing: 1,
                        }
                    }
                    5 | 7 | 11 => {
                        *frame.tables[TableId::RaidAirScores.index()] =
                            TableView::Complete(TableRows::RaidAirScores(initial.clone()))
                    }
                    6 => {
                        *frame.tables[TableId::RaidAirScores.index()] = TableView::Unavailable {
                            reason: "synthetic unavailable read".into(),
                        }
                    }
                    _ => {}
                }
                let mut encoded = encoder.encode(frame.clone(), hour == 0).unwrap();
                if hour == 8 {
                    encoded.tables[TableId::RaidAirScores.index()] =
                        TableUpdate::Unchanged { baseline: 6 };
                } else if hour == 9 {
                    encoded.tables[TableId::RaidAirScores.index()] =
                        TableUpdate::Snapshot(TableRows::RaidAirScores(initial.clone()));
                } else if hour == 10 {
                    encoded.tables[TableId::RaidAirScores.index()] = TableUpdate::Delta {
                        baseline: 9,
                        deleted: vec![u128::MAX >> 64],
                        changed: vec![],
                    };
                } else if hour == 11 {
                    // Recovery after the deliberately invalid deletion.
                    encoded.tables[TableId::RaidAirScores.index()] =
                        TableUpdate::Snapshot(TableRows::RaidAirScores(initial.clone()));
                }
                let decoded = decoder.advance(&encoded).unwrap();
                let examined = read.observe(&encoded, &mut regions);
                let expected =
                    dashboard_text::metadata::recorded_rows::<tables::RaidAirScores>(decoded)
                        .unwrap();
                let mut actual = read.rows.clone();
                actual.sort_by_key(RowCodec::key);
                assert_eq!(
                    actual.as_slice(),
                    expected,
                    "countries={countries}, hour={hour}"
                );
                assert_eq!(
                    read.complete,
                    decoded.complete::<tables::RaidAirScores>().is_ok()
                );
                let mut expected_active = HashMap::new();
                for row in expected {
                    if row.friendly > 0 || row.enemy > 0 {
                        *expected_active.entry(row.region).or_default() += 1;
                    }
                    assert_eq!(read.rows[read.indices[&row.country][&row.region]], *row);
                }
                assert_eq!(read.active, expected_active);
                for row in expected {
                    if row.friendly != 0 || row.enemy != 0 {
                        expected_regions.insert(row.region);
                    }
                }
                assert_eq!(regions, expected_regions);
                if hour == 1 {
                    assert_eq!(examined, 0);
                }
                if hour == 2 {
                    assert_eq!(examined, 1);
                }
                if hour == 3 {
                    assert_eq!(examined, 2);
                }
            }
            // A failed field decode clears the entire table and its baseline.
            let changed = vec![DeltaRow {
                format: replay_data::FORMAT_VERSION,
                key: initial[0].key(),
                mask: 1,
                recorded_fields: 2,
                values: vec![],
            }];
            let mut bad = dashboard_core::test_fixtures::scale_frame(1, 12, countries);
            *bad.tables[TableId::RaidAirScores.index()] =
                TableView::Complete(TableRows::RaidAirScores(initial));
            let mut bad = FrameEncoder::new().encode(bad, true).unwrap();
            bad.keyframe = false;
            bad.tables[TableId::RaidAirScores.index()] = TableUpdate::Delta {
                baseline: 11,
                changed,
                deleted: vec![],
            };
            let decoded = decoder.advance(&bad).unwrap();
            read.observe(&bad, &mut regions);
            assert!(read.rows.is_empty());
            assert!(decoded.complete::<tables::RaidAirScores>().is_err());
            assert_eq!(regions, expected_regions);
        }
    }
}
