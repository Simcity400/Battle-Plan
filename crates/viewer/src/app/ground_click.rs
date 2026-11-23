use content_reader_map::map::ProvinceKind;
use viewer_map_modes::map_mode::{MapMode, Stock};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ground {
    AirRegion(u32),
    NavalRegion(u32),
    State(u32),
    Close,
    Refused(String),
}

pub(crate) struct Picked {
    pub province: u32,
    pub kind: Option<ProvinceKind>,
    pub state: bool,
    pub region: Option<u32>,
    pub naval: Result<bool, String>,
}

pub(crate) fn route(mode: &MapMode, picked: &Picked) -> Option<Ground> {
    let region = |picked: &Picked| -> Option<Ground> {
        let region = picked.region?;
        Some(match &picked.naval {
            Ok(true) => Ground::NavalRegion(region),
            Ok(false) => Ground::Close,
            Err(why) => Ground::Refused(why.clone()),
        })
    };
    match mode {
        MapMode::Stock(Stock::StrategicAir) => picked.region.map(Ground::AirRegion),
        MapMode::Stock(Stock::StrategicNavy) => region(picked),
        _ => match picked.kind {
            Some(ProvinceKind::Sea) => region(picked),
            Some(ProvinceKind::Land) if picked.state => Some(Ground::State(picked.province)),
            Some(ProvinceKind::Land | ProvinceKind::Lake) => Some(Ground::Close),
            None => Some(Ground::Refused(format!("province {} has no class in the content", picked.province))),
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Selected {
    Region(u32),
    State(u32),
}

pub(crate) fn selects(mode: &MapMode, picked: &Picked) -> Option<Selected> {
    let region = picked.region.map(Selected::Region);
    match mode {
        MapMode::Stock(Stock::StrategicAir | Stock::StrategicNavy) => region,
        _ => match picked.kind {
            Some(ProvinceKind::Sea) => region,
            Some(ProvinceKind::Land) if picked.state => Some(Selected::State(picked.province)),
            _ => None,
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Outline {
    pub provinces: std::sync::Arc<[u32]>,
    pub state: bool,
}

pub(crate) fn state_provinces(frame: &replay_frame::frame::DecodedFrame, province: u32) -> std::io::Result<Vec<u32>> {
    let rows = frame
        .complete::<replay_data::rows::tables::Provinces>()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let state = rows
        .binary_search_by_key(&province, |row| row.id)
        .map(|at| rows[at].state)
        .map_err(|_| std::io::Error::other(format!("Provinces province {province} is absent")))?;
    Ok(rows.iter().filter(|row| state != 0 && row.state == state).map(|row| row.id).collect())
}

pub(crate) fn naval_region(region: u32, kinds: impl IntoIterator<Item = Option<ProvinceKind>>) -> Result<bool, String> {
    let mut unclassed = false;
    for kind in kinds {
        match kind {
            Some(ProvinceKind::Sea) => return Ok(true),
            Some(_) => {}
            None => unclassed = true,
        }
    }
    if unclassed {
        return Err(format!("strategic region {region} holds a province with no class in the content"));
    }
    Ok(false)
}

pub(crate) fn has_state(frame: &replay_frame::frame::DecodedFrame, province: u32) -> std::io::Result<bool> {
    let rows = frame
        .complete::<replay_data::rows::tables::Provinces>()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    rows.binary_search_by_key(&province, |row| row.id)
        .map(|at| rows[at].state != 0)
        .map_err(|_| std::io::Error::other(format!("Provinces province {province} is absent")))
}

impl super::Viewer {
    pub(super) fn select_province(
        &mut self,
        document: &crate::document::Document,
        frame: Option<&replay_frame::frame::DecodedFrame>,
        province: u32,
        ctx: &eframe::egui::Context,
    ) {
        let mode = self.shown_mode().clone();
        self.click(document, super::held_clicks::Click::Province { province, mode }, frame, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_map_modes::map_mode::MapMode;

    fn picked(kind: Option<ProvinceKind>, state: bool, naval: Result<bool, String>) -> Picked {
        Picked { province: 7, kind, state, region: Some(40), naval }
    }

    #[test]
    fn every_mode_routes_each_province_class_as_the_game_does() {
        use ProvinceKind::*;
        let cases = [
            (Some(Land), true, Ok(false)),
            (Some(Land), false, Ok(false)),
            (Some(Lake), false, Ok(false)),
            (Some(Sea), false, Ok(true)),
            (Some(Sea), false, Ok(false)),
        ];
        for stock in Stock::ALL {
            let mode = MapMode::Stock(stock);
            for (kind, state, naval) in cases.clone() {
                let got = route(&mode, &picked(kind, state, naval.clone()));
                let expected = match stock {
                    Stock::StrategicAir => Ground::AirRegion(40),
                    Stock::StrategicNavy if naval == Ok(true) => Ground::NavalRegion(40),
                    Stock::StrategicNavy => Ground::Close,
                    _ => match kind {
                        Some(Sea) if naval == Ok(true) => Ground::NavalRegion(40),
                        Some(Land) if state => Ground::State(7),
                        _ => Ground::Close,
                    },
                };
                assert_eq!(got, Some(expected), "{stock:?} {kind:?} state {state} naval {naval:?}");
            }
        }
        let scripted = MapMode::Scripted("custom".into());
        assert_eq!(route(&scripted, &picked(Some(Land), true, Ok(false))), Some(Ground::State(7)));
    }

    #[test]
    fn a_click_off_every_region_or_class_is_named_or_does_nothing() {
        let off = Picked { region: None, ..picked(Some(ProvinceKind::Sea), false, Ok(true)) };
        assert_eq!(route(&MapMode::Stock(Stock::Default), &off), None);
        assert_eq!(route(&MapMode::Stock(Stock::StrategicAir), &off), None);
        assert!(matches!(route(&MapMode::Stock(Stock::Default), &picked(None, false, Ok(false))), Some(Ground::Refused(_))));
        let undecided = picked(Some(ProvinceKind::Sea), false, Err("mixed".into()));
        assert_eq!(route(&MapMode::Stock(Stock::StrategicNavy), &undecided), Some(Ground::Refused("mixed".into())));
    }

    #[test]
    fn a_region_pick_selects_the_region_and_a_land_pick_its_state() {
        use ProvinceKind::*;
        let default = MapMode::Stock(Stock::Default);
        assert_eq!(selects(&default, &picked(Some(Land), true, Ok(false))), Some(Selected::State(7)));
        assert_eq!(selects(&default, &picked(Some(Land), false, Ok(false))), None);
        assert_eq!(selects(&default, &picked(Some(Lake), false, Ok(false))), None);
        assert_eq!(selects(&default, &picked(Some(Sea), false, Ok(false))), Some(Selected::Region(40)));
        for strategic in [Stock::StrategicAir, Stock::StrategicNavy] {
            assert_eq!(selects(&MapMode::Stock(strategic), &picked(Some(Land), true, Ok(false))), Some(Selected::Region(40)));
        }
    }

    #[test]
    fn a_region_with_any_sea_province_opens_the_naval_view() {
        use ProvinceKind::*;
        assert_eq!(naval_region(1, [Some(Sea), Some(Sea)]), Ok(true));
        assert_eq!(naval_region(1, [Some(Land), Some(Lake)]), Ok(false));
        assert_eq!(naval_region(1, [Some(Land), Some(Sea), Some(Lake)]), Ok(true), "a sea region with coastal land opens it");
        assert_eq!(naval_region(1, [None, Some(Sea)]), Ok(true), "one sea province decides it");
        assert!(naval_region(1, [Some(Land), None]).is_err());
        assert_eq!(naval_region(1, []), Ok(false));
    }
}
