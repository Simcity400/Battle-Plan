use super::{Arg, Tooltip, counter_content, tooltip_text_state_name};
use content_reader_localisation::localisation::Localisation;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use viewer_unit_reads::naval_count_approx;
use viewer_foundation::perspective::Vision;
use viewer_foundation::tooltip_text::localise;

const INTEL_FEATURE: u32 = 1 << 5;
const FULL_RATIO: i64 = 100_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    pub state: Result<Option<String>, String>,
    pub level: i32,
    pub own: bool,
    pub intel_feature: Result<bool, String>,
    pub sees: Result<bool, String>,
    pub queue: Result<i32, String>,
    pub occupied: Result<bool, String>,
    pub fleet_selected: bool,
}

pub fn build(
    content: &counter_content::Content,
    frame: &DecodedFrame,
    vision: &Vision,
    province: u32,
    fleet_selected: bool,
) -> Tooltip {
    match facts(content, frame, vision, province, fleet_selected) {
        Ok(facts) => lines(&content.localisation, &content.naval_counts, &facts),
        Err(reason) => {
            let mut out = Tooltip::default();
            out.refuse("naval base", reason);
            out
        }
    }
}

fn error(e: impl ToString) -> String {
    e.to_string()
}

fn facts(
    content: &counter_content::Content,
    frame: &DecodedFrame,
    vision: &Vision,
    province: u32,
    fleet_selected: bool,
) -> Result<Facts, String> {
    let bases = frame.complete::<tables::NavalBases>().map_err(error)?;
    let base = bases
        .binary_search_by_key(&province, |row| row.province)
        .map(|at| &bases[at])
        .map_err(|_| format!("NavalBases holds no base in province {province}"))?;
    let links = frame.complete::<tables::CountryLinks>().map_err(error)?;
    let owner = links
        .binary_search_by_key(&base.country, |row| row.id)
        .map(|at| links[at].canonical_slot)
        .map_err(|_| format!("the holder {} of the naval base has no CountryLinks row", base.country))?;
    let state = (|| {
        let provinces = frame.complete::<tables::Provinces>().map_err(error)?;
        let id = provinces
            .binary_search_by_key(&province, |row| row.id)
            .map(|at| provinces[at].state)
            .map_err(|_| format!("Provinces province {province} is absent"))?;
        if id == 0 {
            return Ok(None);
        }
        let states = frame.complete::<tables::States>().map_err(error)?;
        let row = states.iter().find(|row| row.id == id).ok_or("state row absent")?;
        tooltip_text_state_name(
            &content.localisation,
            &content.recorded_language,
            id,
            &row.custom_name,
            &row.owner_name,
            || {
                let key = content
                    .state_name_keys
                    .get(&id)
                    .ok_or("state content name absent")?
                    .as_ref()
                    .map_err(Clone::clone)?;
                localise(&content.localisation, key, &[])
            },
        )
        .map(Some)
    })();
    let intel_feature = match frame.complete::<tables::SimulationFeatures>() {
        Ok([features]) => Ok(features.dlc_mask & INTEL_FEATURE != 0),
        Ok(_) => Err("the DLC mask needs one SimulationFeatures row".to_owned()),
        Err(e) => Err(e.to_string()),
    };
    let queue = (|| {
        let bases = frame.complete::<tables::NavalRepairBases>().map_err(error)?;
        if bases.binary_search_by_key(&province, |row| row.province).is_err() {
            return Err(format!("the repair queue of the naval base in province {province} is not recorded"));
        }
        let queue = frame.complete::<tables::NavalRepairQueue>().map_err(error)?;
        let count = queue.iter().filter(|row| row.province == province).count();
        i32::try_from(count).map_err(|_| "repair queue count overflow".to_owned())
    })();
    let occupied = frame
        .complete::<tables::TaskForces>()
        .map(|forces| forces.iter().any(|force| force.province == province))
        .map_err(error);
    Ok(Facts {
        state,
        level: base.level,
        own: owner == vision.ours(),
        intel_feature,
        sees: vision.shares_vision_or_fog(owner).map_err(|why| why.clause()),
        queue,
        occupied,
        fleet_selected,
    })
}

pub fn lines(loc: &Localisation, defines: &naval_count_approx::Defines, facts: &Facts) -> Tooltip {
    let mut out = Tooltip::default();
    let push = |out: &mut Tooltip, name: &str, text: Result<String, String>| match text {
        Ok(text) => out.title.push_str(&text),
        Err(reason) => out.refuse(name, reason),
    };
    match &facts.state {
        Ok(Some(name)) => push(&mut out, "naval base header", localise(loc, "HEADER", &[("KEY", Arg::Text(name))])),
        Ok(None) => {}
        Err(reason) => out.refuse("naval base header", reason.clone()),
    }
    let level = (|| {
        let sees = facts.sees.clone()?;
        if !facts.intel_feature.clone()? {
            return Ok(if sees { 4 } else { 0 });
        }
        if !sees {
            return Err("the viewing country's navy intel ratio against its global scale is not reproduced".to_owned());
        }
        defines.port_detail_level(FULL_RATIO).ok_or_else(|| {
            "NIntel.NAVY_MAPICON_NAVAL_PORT_VISIBILITY_DETAIL_THRESHOLDS does not hold the five thresholds the game reads".to_owned()
        })
    })();
    let level = match level {
        Ok(level) => level,
        Err(reason) => {
            out.refuse("naval base intel level", reason);
            return out;
        }
    };
    if level < 1 {
        push(&mut out, "naval base level", localise(loc, "NAVAL_BASE_LEVEL_UNKNOWN", &[]).map(|text| text + "\n"));
    } else {
        push(
            &mut out,
            "naval base level",
            localise(loc, "NAVAL_BASE_LEVEL", &[("VAL", Arg::Int32(facts.level))]).map(|text| text + "\n"),
        );
        if level < 4 {
            if level >= 2 {
                out.refuse("naval base taskforce count", "the intel count range of the docked task forces is not reproduced".into());
            }
        } else {
            match &facts.occupied {
                Ok(false) => {}
                Ok(true) => out.refuse("naval base fleet list", "the province's native unit order is not recorded".into()),
                Err(reason) => out.refuse("naval base fleet list", reason.clone()),
            }
        }
        match &facts.queue {
            Ok(queue) if *queue > 0 => push(
                &mut out,
                "naval base repairs",
                localise(loc, "DOCKYARD_REPAIR_INFO", &[("MAX", Arg::Int32(facts.level)), ("CUR", Arg::Int32(*queue))]),
            ),
            Ok(_) => {}
            Err(reason) => out.refuse("naval base repairs", reason.clone()),
        }
        if facts.own {
            out.refuse("naval base repair shortfall", "the dockyard allocation it compares with the base's need is not reproduced".into());
        }
    }
    out.title.push('\n');
    if facts.fleet_selected {
        out.refuse("naval base order hints", "the selected fleet's order check is not reproduced".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc() -> Localisation {
        [
            ("HEADER", "$KEY$\n"),
            ("NAVAL_BASE_LEVEL_UNKNOWN", "Naval base"),
            ("NAVAL_BASE_LEVEL", "Naval base: $VAL$"),
            ("DOCKYARD_REPAIR_INFO", "$CUR$ waiting, $MAX$ at most"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
    }

    fn facts() -> Facts {
        Facts {
            state: Ok(Some("Kent".into())),
            level: 6,
            own: false,
            intel_feature: Ok(false),
            sees: Ok(true),
            queue: Ok(0),
            occupied: Ok(false),
            fleet_selected: false,
        }
    }

    fn build(facts: &Facts) -> Tooltip {
        lines(&loc(), &naval_count_approx::Defines::VANILLA, facts)
    }

    #[test]
    fn a_visible_base_shows_its_level_and_no_selection_hint() {
        let tip = build(&facts());
        assert_eq!(tip.title, "Kent\nNaval base: 6\n\n");
        assert_eq!(tip.body, "");
        assert!(tip.refused.is_empty(), "{:?}", tip.refused);
    }

    #[test]
    fn without_full_visibility_the_level_is_unknown() {
        let tip = build(&Facts { sees: Ok(false), ..facts() });
        assert_eq!(tip.title, "Kent\nNaval base\n\n");
        assert_eq!(tip.body, "");
        assert!(tip.refused.is_empty(), "{:?}", tip.refused);
        let dlc = build(&Facts { sees: Ok(false), intel_feature: Ok(true), ..facts() });
        assert_eq!(dlc.title, "Kent\n");
        assert!(dlc.refused.keys().any(|line| line.starts_with("naval base intel level")));
    }

    #[test]
    fn repairs_show_for_a_queue_and_the_shortfall_and_fleet_list_are_named_gaps() {
        let tip = build(&Facts { queue: Ok(3), own: true, occupied: Ok(true), state: Ok(None), ..facts() });
        assert_eq!(tip.title, "Naval base: 6\n3 waiting, 6 at most\n");
        assert!(tip.refused.keys().any(|line| line.starts_with("naval base fleet list")));
        assert!(tip.refused.keys().any(|line| line.starts_with("naval base repair shortfall")));
        let foreign = build(&Facts { queue: Ok(3), ..facts() });
        assert!(!foreign.refused.keys().any(|line| line.starts_with("naval base repair shortfall")));
    }

    #[test]
    fn the_intel_feature_reads_the_detail_thresholds_at_full_ratio() {
        let tip = build(&Facts { intel_feature: Ok(true), ..facts() });
        assert_eq!(tip.title, "Kent\nNaval base: 6\n\n");
        let mut defines = naval_count_approx::Defines::VANILLA;
        defines.port_detail = Some([0, 10_000, 30_000, 70_000, 200_000]);
        let partial = lines(&loc(), &defines, &Facts { intel_feature: Ok(true), occupied: Ok(true), ..facts() });
        assert_eq!(partial.title, "Kent
Naval base: 6

");
        assert_eq!(partial.refused.len(), 1, "{:?}", partial.refused);
        assert!(partial.refused.keys().any(|line| line.starts_with("naval base taskforce count")));
        defines.port_detail = Some([0, 10_000, 200_000, 200_000, 200_000]);
        let level_one = lines(&loc(), &defines, &Facts { intel_feature: Ok(true), ..facts() });
        assert_eq!(level_one.title, "Kent\nNaval base: 6\n\n");
        defines.port_detail = None;
        let unread = lines(&loc(), &defines, &Facts { intel_feature: Ok(true), ..facts() });
        assert!(unread.refused.keys().any(|line| line.starts_with("naval base intel level")));
    }
}
