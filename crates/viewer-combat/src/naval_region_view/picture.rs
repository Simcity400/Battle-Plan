use super::{Content, Element};
use crate::binding::{self, Bindings};
use crate::naval_environment::{MODIFIER_ENTRY, night_picture, picture_names, region_custom_sea, weather_entries, weather_suffix};
use crate::state_view::{paths_named, withhold_named};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::io;
use viewer_engagements::engagement_history::CombatFlags;

const NIGHT: i64 = 50_000;

fn mean_night(factors: &[i64]) -> i64 {
    let count = (factors.len() as i64).wrapping_mul(100_000);
    let sum = factors.iter().fold(0_i64, |sum, factor| sum.wrapping_add(*factor));
    if count == 0 || (sum as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        return 0xffff_ffff;
    }
    sum.wrapping_mul(100_000) / count
}

fn ongoing(frame: &DecodedFrame, engaged: &CombatFlags, provinces: &[u32]) -> Result<bool, String> {
    let table = frame.complete::<tables::NavalProvinceCombats>();
    let complete = table.is_ok();
    let rows = table.unwrap_or_default();
    let mut undecided = false;
    for province in provinces {
        let row = rows.binary_search_by_key(province, |row| row.province).ok().map(|at| &rows[at]);
        match engaged.engaged_entries(row, complete, 1) {
            Some(true) => return Ok(true),
            Some(false) => {}
            None => undecided = true,
        }
    }
    if undecided {
        return Err("whether a combat in the region is engaged is not recorded".into());
    }
    Ok(false)
}

fn name(content: &Content, frame: &DecodedFrame, region: u32, engaged: &CombatFlags, paused: bool) -> Result<String, String> {
    let definition = content.strategic_regions.get(&region).ok_or("the region is absent from the content stack")?;
    let weather = frame
        .complete::<tables::WeatherRegions>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.id == region)
        .ok_or("the region has no WeatherRegions row")?;
    let factors = definition
        .provinces
        .iter()
        .map(|&province| crate::air_menu::summary::clock(content, frame, province).map(|clock| clock.night_factor))
        .collect::<Result<Vec<_>, String>>()?;
    let night = if mean_night(&factors) > NIGHT {
        match frame.complete::<tables::Clock>().map_err(|e| e.to_string())? {
            [clock] => night_picture(clock.speed, paused),
            _ => return Err("the game speed needs one Clock row".into()),
        }
    } else {
        false
    };
    let ongoing = ongoing(frame, engaged, &definition.provinces)?;
    let names = picture_names(definition.naval_terrain.as_deref(), night, weather_suffix(weather), ongoing);
    names
        .iter()
        .find(|name| content.assets.has_sprite(name))
        .cloned()
        .ok_or_else(|| format!("the content declares none of the pictures {}", names.join(", ")))
}

pub(super) fn bind(content: &Content, frame: &DecodedFrame, root: &Element, out: &mut Bindings, region: u32, engaged: &CombatFlags, paused: bool) -> io::Result<()> {
    let picture = name(content, frame, region, engaged, paused);
    for path in paths_named(root, "terrain_picture") {
        out.bind(&path, binding::SPRITE, picture.clone().map_err(|why| format!("naval region view terrain picture: {why}")));
    }
    let weather = frame
        .complete::<tables::WeatherRegions>()
        .map_err(|e| e.to_string())
        .and_then(|rows| rows.iter().find(|row| row.id == region).cloned().ok_or_else(|| "the region has no WeatherRegions row".into()));
    let weather = match weather {
        Ok(weather) => weather,
        Err(why) => {
            withhold_named(out, root, &["modifier_grid"], &format!("naval region view modifiers: {why}"));
            return Ok(());
        }
    };
    let custom_sea = region_custom_sea(&content.region_of, frame, &weather, region);
    let entries = weather_entries(out, content, frame, &weather, region, custom_sea.as_deref().map_err(Clone::clone))?;
    for path in paths_named(root, "modifier_grid") {
        let widget = out.values.entry(path).or_default();
        widget.entry_template = Some(MODIFIER_ENTRY.into());
        widget.entries = entries.clone();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_night_factor_is_the_provinces_mean() {
        assert_eq!(mean_night(&[100_000, 0]), 50_000);
        assert_eq!(mean_night(&[100_000, 100_000, 1]), 66_667);
        assert_eq!(mean_night(&[]), 0xffff_ffff);
    }
}
