use std::{collections::BTreeMap, io};

use super::{Inputs, Stock, color4, style};
use content_reader_defines::defines::Defines;
use content_reader_map::map::ProvinceKind;
use content_reader_regions::regions::StrategicRegion;
use replay_data::{rows::RaidAirScoreRow, rows::tables};

mod fold;
#[cfg(test)]
mod oracle;

use map_display_data::landscape::StrategicLayer as Layer;

pub struct Content {
    pub regions: BTreeMap<u32, StrategicRegion>,
    fold: std::sync::Mutex<fold::Fold>,
    air: [[f32; 4]; 4],
    navy: [f32; 4],
}

impl Content {
    #[cfg(test)]
    pub(super) fn fixture() -> Self {
        Self {
            regions: BTreeMap::new(),
            fold: Default::default(),
            air: [
                [0.8, 0.0, 0.0, 1.0],
                [0.0, 0.8, 0.0, 1.0],
                [0.8, 0.8, 0.0, 1.0],
                [0.4, 0.4, 0.4, 1.0],
            ],
            navy: [0.0, 0.0, 1.0, 0.0],
        }
    }
    pub fn new(defines: &Defines, regions: &BTreeMap<u32, StrategicRegion>) -> io::Result<Self> {
        Ok(Self {
            regions: regions.clone(),
            fold: Default::default(),
            air: [
                color4(defines, "NGraphics", "STRATEGIC_AIR_COLOR_BAD")?,
                color4(defines, "NGraphics", "STRATEGIC_AIR_COLOR_GOOD")?,
                color4(defines, "NGraphics", "STRATEGIC_AIR_COLOR_AVERAGE")?,
                color4(defines, "NGraphics", "STRATEGIC_AIR_COLOR_NEUTRAL")?,
            ],
            navy: color4(defines, "NGraphics", "STRATEGIC_NAVY_COLOR_NEUTRAL")?,
        })
    }

    pub fn bind(&self, map: &std::sync::Arc<content_reader_map::map::MapData>) {
        self.fold
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bank = Some(std::sync::Arc::downgrade(map));
    }
    pub fn layer(
        &self,
        mode: Stock,
        inputs: &Inputs,
        warnings: &mut Vec<String>,
    ) -> io::Result<Layer> {
        self.fold
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .read(self, mode, inputs, warnings)
    }
}

fn air_color(row: &RaidAirScoreRow, friendly_missions: u16) -> Result<usize, String> {
    if row.friendly == 0 && row.enemy == 0 {
        return Ok(if friendly_missions == 0 && row.enemy_missions == 0 {
            3
        } else if friendly_missions > row.enemy_missions {
            1
        } else {
            0
        });
    }
    let total = row.friendly.wrapping_add(row.enemy);
    if total == 0 || [i64::MIN, i64::MIN + 1, i64::MAX].contains(&row.friendly) {
        return Ok(1);
    }
    let ratio = row
        .friendly
        .wrapping_mul(100_000)
        .checked_div(total)
        .ok_or("air-superiority ratio division overflow")?;
    Ok(if ratio > 60_000 {
        1
    } else if ratio < 40_000 {
        0
    } else {
        2
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn superiority_thresholds_and_zero_weight_mission_counts_follow_the_game() {
        let scores = |friendly, enemy| RaidAirScoreRow {
            friendly,
            enemy,
            ..Default::default()
        };
        assert_eq!(air_color(&scores(60_000, 40_000), 0), Ok(2));
        assert_eq!(air_color(&scores(60_001, 39_999), 0), Ok(1));
        assert_eq!(air_color(&scores(40_000, 60_000), 0), Ok(2));
        assert_eq!(air_color(&scores(39_999, 60_001), 0), Ok(0));
        let mut zero = scores(0, 0);
        assert_eq!(air_color(&zero, 0), Ok(3));
        zero.enemy_missions = 3;
        assert_eq!(air_color(&zero, 3), Ok(0));
        assert_eq!(air_color(&zero, 4), Ok(1));
        assert_eq!(air_color(&zero, 0), Ok(0));
    }

    #[test]
    fn strategic_fold_matches_the_frozen_kernel_through_sparse_edits_and_clock_moves() {
        use crate::map_mode::tests::{frame, map};
        use replay_data::rows::{CountryLinkRow, TableId, TableRows, WarRelationRow};
        use replay_frame::frame::TableView;
        let mut content = Content::fixture();
        for id in [0, 17, 30, u32::MAX] {
            content.regions.insert(
                id,
                StrategicRegion {
                    id,
                    provinces: vec![1, 5],
                    ..Default::default()
                },
            );
        }
        let parent = super::super::tests::content(Default::default());
        let map = std::sync::Arc::new(map());
        content.bind(&map);
        let mut source = frame();
        source.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                first_country: 200,
                second_country: 300,
                start_hour: 10,
                end_hour: 11,
                ..Default::default()
            }]))
            .into();
        for step in 0..9 {
            source.hour = [9, 10, 11, 12, 11, 10, 13, 14, 15][step];
            source.tables[TableId::RaidAirScores.index()] =
                TableView::Complete(TableRows::RaidAirScores(vec![
                    RaidAirScoreRow {
                        country: 100,
                        region: 17,
                        friendly: step as i64,
                        enemy: 30,
                        ..Default::default()
                    },
                    RaidAirScoreRow {
                        country: 200,
                        region: 17,
                        friendly_missions: 2,
                        enemy_missions: 1,
                        ..Default::default()
                    },
                    RaidAirScoreRow {
                        country: 300,
                        region: 30,
                        friendly: 60,
                        enemy: 40,
                        ..Default::default()
                    },
                ]))
                .into();
            if step == 4 {
                source.tables[TableId::CountryLinks.index()] =
                    TableView::Complete(TableRows::CountryLinks(vec![
                        CountryLinkRow {
                            id: 100,
                            canonical_slot: 0,
                        },
                        CountryLinkRow {
                            id: 200,
                            canonical_slot: 2,
                        },
                        CountryLinkRow {
                            id: 300,
                            canonical_slot: 2,
                        },
                    ]))
                    .into();
            }
            if step == 6 {
                source.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
                    reason: "synthetic lazy gap".into(),
                }
                .into();
            }
            for selected in [None, Some(0), Some(1), Some(2), Some(1501)] {
                let inputs = Inputs {
                    content: &parent,
                    map: &map,
                    frame: &source,
                    selected,
                    world: None,
                    colour: None,
                };
                for mode in [Stock::StrategicAir, Stock::StrategicNavy] {
                    let mut actual_warnings = Vec::new();
                    let mut expected_warnings = Vec::new();
                    let actual = content.layer(mode, &inputs, &mut actual_warnings).unwrap();
                    let expected = content.oracle_layer(mode, &inputs, &mut expected_warnings);
                    assert_eq!(actual, expected);
                    assert_eq!(actual_warnings, expected_warnings);
                    let mut quiet_warnings = Vec::new();
                    let quiet = content.layer(mode, &inputs, &mut quiet_warnings).unwrap();
                    assert_eq!(quiet, actual);
                    assert_eq!(quiet_warnings, actual_warnings);
                    if !(mode == Stock::StrategicAir
                        && selected.is_none_or(|slot| slot == 1501 || slot == 1 && step >= 4))
                    {
                        assert!(quiet.styles.ptr_eq(&actual.styles));
                    }
                    assert!(std::sync::Arc::ptr_eq(
                        &quiet.region_of.0,
                        &actual.region_of.0
                    ));
                }
            }
        }
    }

    #[test]
    fn air_covers_land_and_water_and_navy_keeps_only_sea_without_shifting_region_colours() {
        use crate::map_mode::tests::{frame, map};
        use replay_data::{rows::TableId, rows::TableRows};
        use replay_frame::frame::TableView;
        let mut content = Content::fixture();
        content.regions.insert(
            17,
            StrategicRegion {
                id: 17,
                provinces: vec![1, 5],
                ..Default::default()
            },
        );
        let mut frame = frame();
        frame.tables[TableId::RaidAirScores.index()] =
            TableView::Complete(TableRows::RaidAirScores(vec![RaidAirScoreRow {
                country: 100,
                region: 17,
                friendly: 80_000,
                enemy: 20_000,
                ..Default::default()
            }]))
            .into();
        let parent = super::super::tests::content(Default::default());
        let map = map();
        let inputs = Inputs {
            content: &parent,
            map: &map,
            frame: &frame,
            selected: Some(0),
            world: None,
            colour: None,
        };
        let mut warnings = Vec::new();
        let air = content
            .layer(Stock::StrategicAir, &inputs, &mut warnings)
            .unwrap();
        assert_eq!((air.region_of[1], air.region_of[5]), (18, 18));
        assert_eq!(air.styles[&18].color, super::super::pack(content.air[1]));
        assert!(!air.naval_fx);
        assert!(warnings.is_empty());
        let navy = content
            .layer(Stock::StrategicNavy, &inputs, &mut warnings)
            .unwrap();
        assert_eq!((navy.region_of[1], navy.region_of[5]), (0, 18));
        assert_eq!(navy.styles[&18].color, super::super::pack(content.navy));
        assert!(navy.naval_fx);
        frame.tables[TableId::RaidAirScores.index()] = TableView::Unavailable {
            reason: "synthetic score refusal".into(),
        }
        .into();
        let absent = Inputs {
            content: &parent,
            map: &map,
            frame: &frame,
            selected: Some(0),
            world: None,
            colour: None,
        };
        let refused = content
            .layer(Stock::StrategicAir, &absent, &mut warnings)
            .unwrap();
        assert!(refused.styles.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("1 strategic-region colours withheld"));
    }
}
