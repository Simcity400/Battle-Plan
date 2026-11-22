use super::{Content, Element, links};
use crate::binding::{self, Bindings};
use crate::modifier_text::Definitions;
use crate::state_view::{loc, paths_named};
use crate::tooltips::{hover_text, set_hover};
use content_reader_localisation::localisation::{Argument, crt_case_text};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap as Sources;
use viewer_foundation::{perspective::Vision, tooltip_text::{ensure_newlines, viewing_tag}};

const ACCESS: [(&str, &str); 3] = [
    ("naval_access_rule_allowed", "NAVAL_REGION_ACCESS_ALLOWED"),
    ("naval_access_rule_avoid", "NAVAL_REGION_ACCESS_AVOID"),
    ("naval_access_rule_blocked", "NAVAL_REGION_ACCESS_BLOCKED"),
];
const BARS: [&str; 3] = ["progress", "progress_enemy", "neutral_progress"];

fn access(content: &Content, level: &str) -> Result<String, String> {
    let name = loc(content, level, &[])?;
    let mut text = loc(content, "NAVAL_REGION_ACCESS_LEVEL", &[("LVL", Argument::Text(&name))])?;
    ensure_newlines(&mut text, 1);
    text.push_str(&loc(content, &format!("{level}_DESC"), &[])?);
    Ok(text)
}

fn terrain(content: &Content, frame: &DecodedFrame, vision: &Vision, region: u32, dropped: &mut Vec<String>) -> Result<String, String> {
    let terrain = content.assets.naval_terrain.as_ref().map_err(Clone::clone)?;
    let definition = content.strategic_regions.get(&region).ok_or_else(|| format!("region {region} is absent from the content stack"))?;
    let name = definition
        .naval_terrain
        .as_deref()
        .map(str::as_bytes)
        .filter(|name| terrain.has_category(name))
        .unwrap_or(b"ocean");
    let countries = frame.complete::<tables::Countries>().map_err(|e| e.to_string())?;
    let definitions = Definitions::read(frame, &content.modifier_names).map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    terrain.prefix(name, &definitions, content.assets.as_ref(), &content.localisation, viewing_tag(countries, vision), dropped)
}

fn marked(content: &Content, frame: &DecodedFrame, raw: u32, slot: u32) -> Result<String, String> {
    let names = content_reader_country_names::country_names::CountryNames::read(frame, &content.localisation);
    let name = crt_case_text(&names.name_with(slot, "_DEF")?, true, true);
    let flags = binding::flags(content, frame).map_err(|e| e.to_string())?;
    Ok(format!("{}{} {name}", crate::INLINE_FLAG, binding::country_flag(&flags, raw)?))
}

fn leader(content: &Content, frame: &DecodedFrame, region: u32) -> Result<Option<String>, String> {
    let Some(row) = frame
        .complete::<tables::NavalSupremacy>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.region == region && row.region_slot == 0)
    else {
        return Ok(None);
    };
    let slot = replay_derive::naval_region_control::canonical(&links(frame)?, row.country)?;
    marked(content, frame, row.country, slot).map(Some)
}

fn dominance(content: &Content, frame: &DecodedFrame, vision: &Vision, region: u32, sources: &Sources) -> Result<String, String> {
    let context = content.air_context.as_ref().ok_or("the naval dominance content is unavailable")?;
    let links = links(frame)?;
    viewer_counter_products::map::province_tooltips::dominance_hover(context, frame, vision.ours(), region, &links, sources, |raw, slot| {
        marked(content, frame, raw, slot)
    })
}

pub(super) fn bind(content: &Content, frame: &DecodedFrame, vision: &Vision, root: &Element, out: &mut Bindings, region: u32, sources: &Sources) {
    let mut dropped = Vec::new();
    let text = terrain(content, frame, vision, region, &mut dropped);
    for reason in dropped {
        out.refuse(reason);
    }
    hover(out, root, "terrain_picture", text.map_err(|why| format!("naval region view terrain hover: {why}")));
    for (name, level) in ACCESS {
        hover(out, root, name, access(content, level).map_err(|why| format!("naval region view access hover: {why}")));
    }
    match leader(content, frame, region) {
        Ok(name) => {
            for path in paths_named(root, "flag") {
                out.values.entry(path).or_default().tooltip = name.as_deref().and_then(hover_text);
            }
        }
        Err(why) => out.refuse(format!("naval region view flag hover: {why}")),
    }
    let text = dominance(content, frame, vision, region, sources).map_err(|why| format!("naval region view dominance hover: {why}"));
    for name in BARS {
        hover(out, root, name, text.clone());
    }
}

fn hover(out: &mut Bindings, root: &Element, name: &str, text: Result<String, String>) {
    for path in paths_named(root, name) {
        match &text {
            Ok(text) => set_hover(&mut out.values, path, hover_text(text), None),
            Err(why) => {
                set_hover(&mut out.values, path, None, None);
                out.refuse(why.clone());
            }
        }
    }
}
