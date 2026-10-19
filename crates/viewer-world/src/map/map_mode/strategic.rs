//! Strategic-region colours. The Air layer includes land and water; the Navy
//! layer includes sea provinces and retains the ordinary naval-control FX.
use std::{collections::BTreeMap, io};

use super::{Inputs, Stock, Style, color4, style};
use installed_content_core::{defines::Defines, map::ProvinceKind};
use installed_content_regions::regions::StrategicRegion;
use replay_data::{rows::RaidAirScoreRow, rows::tables};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layer {
    pub region_of: Vec<u32>,
    pub styles: BTreeMap<u32, Style>,
    /// Air does not draw naval-control artwork.
    pub naval_fx: bool,
}

pub struct Content {
    pub regions: BTreeMap<u32, StrategicRegion>,
    air: [[f32; 4]; 4],
    navy: [f32; 4],
}

impl Content {
    #[cfg(test)]
    pub(super) fn fixture() -> Self {
        Self {
            regions: BTreeMap::new(),
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
            air: [
                color4(defines, "NGraphics", "STRATEGIC_AIR_COLOR_BAD")?,
                color4(defines, "NGraphics", "STRATEGIC_AIR_COLOR_GOOD")?,
                color4(defines, "NGraphics", "STRATEGIC_AIR_COLOR_AVERAGE")?,
                color4(defines, "NGraphics", "STRATEGIC_AIR_COLOR_NEUTRAL")?,
            ],
            navy: color4(defines, "NGraphics", "STRATEGIC_NAVY_COLOR_NEUTRAL")?,
        })
    }

    pub fn layer(&self, mode: Stock, inputs: &Inputs, warnings: &mut Vec<String>) -> Layer {
        let air = mode == Stock::StrategicAir;
        let mut layer = Layer {
            region_of: vec![
                0;
                inputs
                    .map
                    .definitions
                    .keys()
                    .next_back()
                    .map_or(1, |id| *id as usize + 1)
            ],
            naval_fx: !air,
            ..Default::default()
        };
        // Hold/active highlights are supplied by each view's separate local
        // task-force and fleet selection product, independently of supremacy.
        let colors = if air {
            self.air_colors(inputs)
        } else {
            Ok(BTreeMap::new())
        };
        let mut refused = BTreeMap::<String, usize>::new();
        for (&id, region) in &self.regions {
            let Some(key) = id.checked_add(1) else {
                *refused
                    .entry("region identity cannot be represented".into())
                    .or_default() += 1;
                continue;
            };
            for province in &region.provinces {
                if inputs
                    .map
                    .definitions
                    .get(province)
                    .is_some_and(|p| air || p.kind == ProvinceKind::Sea)
                {
                    layer.region_of[*province as usize] = key;
                }
            }
            match colors.as_ref().map_err(Clone::clone).and_then(|colors| {
                colors
                    .get(&id)
                    .cloned()
                    .unwrap_or(Ok(if air { self.air[3] } else { self.navy }))
            }) {
                Ok(color) => {
                    layer.styles.insert(key, style(color));
                }
                Err(reason) => {
                    *refused.entry(reason).or_default() += 1;
                }
            }
        }
        for (reason, count) in refused {
            warnings.push(format!(
                "{} map mode: {count} strategic-region colours withheld: {reason}",
                if air { "Air" } else { "Navy" }
            ));
        }
        layer
    }

    fn air_colors(
        &self,
        inputs: &Inputs,
    ) -> Result<BTreeMap<u32, Result<[f32; 4], String>>, String> {
        let selected = inputs.selected.ok_or("viewing country is absent")?;
        let countries = super::Countries::read(inputs.frame).map_err(|e| e.to_string())?;
        countries
            .raw
            .get(&selected)
            .ok_or("viewing country has no Countries row")?;
        let rows = inputs
            .frame
            .complete::<tables::RaidAirScores>()
            .map_err(|e| e.to_string())?;
        let mut counts_allowed = None;
        let mut out = BTreeMap::new();
        for row in rows
            .iter()
            .filter(|row| countries.slot(row.country) == Some(selected))
        {
            let friendly = if row.friendly == 0 && row.enemy == 0 && row.friendly_missions != 0 {
                counts_allowed
                    .get_or_insert_with(|| {
                        let counter = inputs
                            .frame
                            .complete::<tables::CountryCounterInputs>()
                            .map_err(|e| e.to_string())?
                            .iter()
                            .find(|row| countries.slot(row.country) == Some(selected))
                            .ok_or("viewing country's owned-state count is absent")?;
                        if counter.owned_state_count < 1 {
                            return Ok(false);
                        }
                        if selected == 0 {
                            return Err("Country0's enemy roster is not recorded".into());
                        }
                        Ok(!replay_derive::directed_war_derive::ReceiverWars::read(
                            inputs.frame,
                            selected,
                        )?
                        .targets()
                        .is_empty())
                    })
                    .clone()
            } else {
                Ok(false)
            };
            let color = friendly.and_then(|counts| {
                air_color(row, if counts { row.friendly_missions } else { 0 })
                    .map(|index| self.air[index])
            });
            out.insert(row.region, color);
        }
        // A complete sparse table witnesses each air object's final region.
        // With no object the game's null lookup supplies all zero operands.
        Ok(out)
    }
}

/// The native fixed5 comparison, including the inclusive contested interval
/// and zero-weight active-mission fallback. Division overflow remains a refusal.
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
    fn air_covers_land_and_water_and_navy_keeps_only_sea_without_shifting_region_colours() {
        use crate::map::map_mode::tests::{frame, map};
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
        let air = content.layer(Stock::StrategicAir, &inputs, &mut warnings);
        assert_eq!((air.region_of[1], air.region_of[5]), (18, 18));
        assert_eq!(air.styles[&18].color, super::super::pack(content.air[1]));
        assert!(!air.naval_fx);
        assert!(warnings.is_empty());
        let navy = content.layer(Stock::StrategicNavy, &inputs, &mut warnings);
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
        let refused = content.layer(Stock::StrategicAir, &absent, &mut warnings);
        assert!(refused.styles.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("1 strategic-region colours withheld"));
    }
}
