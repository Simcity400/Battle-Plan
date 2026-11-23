use crate::{
    dependency_index::{Clock, DependencyIndex},
    witness::Operand,
};
use content_reader_map::map::MapData;
use map_display_data::{province_field::Storage, retained::Retained};
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use std::{io, sync::Arc};
use viewer_borders::demilitarized::{self, Zone};
use map_display_data::landscape::Regions;

#[derive(Default)]
pub(crate) struct Zones(DependencyIndex<(Operand<MapData>, Operand<Storage>), Retained<Zone>>);
impl Zones {
    pub(crate) fn read(
        &self,
        frame: &DecodedFrame,
        map: &Arc<MapData>,
        regions: &Regions,
    ) -> io::Result<Retained<Zone>> {
        let _cost = replay_frame::cpu::scope("territory zones", Some("territory"));
        let value = self.0.project_fields(
            frame,
            (Operand::new(map), Operand::new(&regions.state_of.0)),
            Clock::Stable,
            |table| match table {
                TableId::StateFlags => super::landscape::named_fields(table, &["demilitarized"]),
                TableId::States => 0,
                _ => table.full_mask(),
            },
            || zone(frame, map, regions),
        )?;
        Ok(value.as_ref().clone())
    }
}
fn zone(frame: &DecodedFrame, map: &MapData, regions: &Regions) -> io::Result<Retained<Zone>> {
    let states = demilitarized::recorded_states(frame)?;
    Ok(Retained::new(Zone::new(map, regions, &states)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{ClockRow, StateFlagRow, StateRow, TableRows};
    use replay_frame::frame::{FrameDecoder, FrameEncoder, Metrics, TableView};
    fn inputs() -> (DecodedFrame, Arc<MapData>, Regions) {
        let mut source = DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|i| {
                let table = TableId::ALL[i];
                let rows = match table.key_bits() {
                    0 => TableRows::one_default(table),
                    _ => TableRows::empty(table),
                };
                if rows.validate_keys().is_err() {
                    return TableView::Unavailable {
                        reason: "synthetic zone frames did not read this table".into(),
                    }
                    .into();
                }
                TableView::Complete(rows).into()
            }),
        };
        source.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![ClockRow {
                hour: source.hour,
                ..Default::default()
            }]))
            .into();
        source.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
            (1..=1501)
                .map(|id| StateRow {
                    id,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        source.tables[TableId::StateFlags.index()] =
            TableView::Complete(TableRows::StateFlags(vec![StateFlagRow {
                state: 7,
                demilitarized: 1,
                ..Default::default()
            }]))
            .into();
        let map = Arc::new(MapData {
            width: 2,
            height: 1,
            province_ids: vec![1, 2],
            definitions: (1..=2)
                .map(|id| {
                    (
                        id,
                        content_reader_map::map::ProvinceDefinition {
                            id,
                            rgb: [0; 3],
                            kind: content_reader_map::map::ProvinceKind::Land,
                            continent: 0,
                        },
                    )
                })
                .collect(),
            country_colors: Default::default(),
        });
        let regions = Regions {
            country_of: vec![0, 1, 1].into(),
            owner_of: vec![0, 1, 1].into(),
            state_of: map_display_data::province_field::ProvinceField::filled(3)
                .unwrap()
                .patched_sorted([(1, 7), (2, 9)])
                .unwrap(),
            country_colors: Default::default(),
            warnings: vec![],
            strategic_attributes: Default::default(),
            mode: None,
        };
        (source, map, regions)
    }
    fn next_hour(source: &mut DecodedFrame) {
        source.hour += 1;
        let TableView::Complete(TableRows::Clock(clock)) =
            &mut *source.tables[TableId::Clock.index()]
        else {
            unreachable!("fixture clock")
        };
        clock[0].hour = source.hour;
    }
    #[test]
    fn quiet_zones_and_unrelated_state_edits_share_the_admitted_planes() {
        let (mut source, map, regions) = inputs();
        let zones = Zones::default();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new().with_change_facts();
        let update = encoder.encode(source.clone(), true).unwrap();
        let first = zones
            .read(decoder.advance(&update).unwrap(), &map, &regions)
            .unwrap();
        assert!(first.splits(1, 2));
        next_hour(&mut source);
        if let TableView::Complete(TableRows::States(rows)) =
            &mut *source.tables[TableId::States.index()]
        {
            rows[1500].manpower_available = 99;
        }
        let update = encoder.encode(source.clone(), false).unwrap();
        let quiet = zones
            .read(decoder.advance(&update).unwrap(), &map, &regions)
            .unwrap();
        assert!(first.ptr_eq(&quiet));
        next_hour(&mut source);
        if let TableView::Complete(TableRows::States(rows)) =
            &mut *source.tables[TableId::States.index()]
        {
            rows.remove(6);
        }
        let update = encoder.encode(source.clone(), false).unwrap();
        let removed = zones
            .read(decoder.advance(&update).unwrap(), &map, &regions)
            .unwrap();
        assert!(
            !quiet.ptr_eq(&removed),
            "zero field mask must still observe key deletion"
        );
        assert!(!removed.splits(1, 2));
        next_hour(&mut source);
        if let TableView::Complete(TableRows::StateFlags(rows)) =
            &mut *source.tables[TableId::StateFlags.index()]
        {
            rows[0].demilitarized = 0;
        }
        let update = encoder.encode(source, false).unwrap();
        let changed = zones
            .read(decoder.advance(&update).unwrap(), &map, &regions)
            .unwrap();
        assert!(!first.ptr_eq(&changed));
        assert!(!changed.splits(1, 2));
    }
}
