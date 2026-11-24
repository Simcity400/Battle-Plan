use super::{binding::Bindings, content::Content, state_view};
use content_reader_gui::gui::native::Element;
use replay_data::rows::{AirCountryRelationRow, tables};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
use viewer_engagements::engagement_history::CombatFlags;
use viewer_engagements::combat_selection::Selection;
use viewer_foundation::perspective::Vision;

mod convoys;
mod counts;
mod dominance;
mod hovers;
mod picture;

pub(super) const WINDOW: &[u8] = b"navalregionview";
pub(super) const TEMPLATES: &[&[u8]] = &[WINDOW, crate::naval_environment::MODIFIER_ENTRY.as_bytes()];

const MINES: [&str; 4] = ["mine_text", "enemy_mine_text", "mine_icon", "enemy_mine_icon"];
const ACCESS_RULES: [&str; 3] = ["naval_access_rule_allowed", "naval_access_rule_avoid", "naval_access_rule_blocked"];

fn percent(share: i64) -> String {
    content_reader_parser::fixed5::fixed5_percent(share, 0, true)
}

fn mine_share(amount: i64, maximum: i64) -> i64 {
    if maximum == 0 || amount.wrapping_add(0x7fff_ffff_ffff_fffe) as u64 > 0xffff_ffff_ffff_fffc {
        return 0xffff_ffff;
    }
    amount.wrapping_mul(100_000) / maximum
}

fn links(frame: &DecodedFrame) -> Result<BTreeMap<u32, u32>, String> {
    Ok(frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect())
}

fn own_raw(frame: &DecodedFrame, vision: &Vision) -> Result<u32, String> {
    frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.canonical_slot == vision.ours())
        .map(|row| row.id)
        .ok_or_else(|| "the observed country has no Countries row".into())
}

fn text(out: &mut Bindings, root: &Element, name: &str, value: Result<String, String>) {
    for path in state_view::paths_named(root, name) {
        out.bind(&path, super::binding::TEXT, value.clone());
    }
}

fn mines(content: &Content, frame: &DecodedFrame, vision: &Vision, root: &Element, out: &mut Bindings, region: u32) {
    match viewer_foundation::naval_mine_feature::enabled(frame) {
        Ok(false) => {
            for name in MINES {
                for path in state_view::paths_named(root, name) {
                    super::binding::hide(&mut out.values, &path);
                }
            }
            return;
        }
        Ok(true) => {}
        Err(why) => {
            state_view::withhold_named(out, root, &MINES, &format!("naval region view mines: {why}"));
            return;
        }
    }
    let own = (|| -> Result<String, String> {
        let links = links(frame)?;
        let levels = replay_derive::naval_mine_levels::Levels::read(frame, &links)?;
        let maximum = content.assets.air_mine_maximum.clone()?;
        Ok(percent(mine_share(levels.amount(own_raw(frame, vision)?, region)?, maximum)))
    })();
    text(out, root, "mine_text", own.map_err(|why| format!("naval region view own mines: {why}")));
    let enemy = content
        .air_context
        .as_ref()
        .ok_or_else(|| "the regional mine content is unavailable".to_owned())
        .and_then(|context| viewer_counter_products::map::province_tooltips::naval_mines::enemy_share(context, frame, vision, region))
        .map(percent);
    text(out, root, "enemy_mine_text", enemy.map_err(|why| format!("naval region view enemy mines: {why}")));
}

fn access_mode(frame: &DecodedFrame, links: &BTreeMap<u32, u32>, viewer: u32, region: u32) -> Result<u8, String> {
    Ok(frame
        .complete::<tables::NavalRegionAccess>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.region == region && links.get(&row.country) == Some(&viewer))
        .map_or(0, |row| row.mode))
}

fn access(frame: &DecodedFrame, vision: &Vision, root: &Element, out: &mut Bindings, region: u32) {
    let moved = (|| -> Result<Vec<(String, [f32; 2])>, String> {
        let mode = access_mode(frame, &links(frame)?, vision.ours(), region)?;
        let Some(rule) = ACCESS_RULES.get(usize::from(mode)) else {
            return Ok(Vec::new());
        };
        let button = super::gui::elements(root)
            .into_iter()
            .find(|(path, _)| path.rsplit('/').next() == Some(rule))
            .and_then(|(_, element)| element.button().map(|button| button.position))
            .ok_or_else(|| format!("the access rule button {rule} is absent from the window"))?;
        super::gui::elements(root)
            .into_iter()
            .filter(|(path, _)| path.rsplit('/').next() == Some("naval_access_rule_highlight"))
            .map(|(path, element)| {
                let from = element.icon().map(|icon| icon.position).ok_or("the access rule highlight is not an icon")?;
                Ok((path, [button[0] as f32 - from[0], button[1] as f32 - from[1]]))
            })
            .collect()
    })();
    match moved {
        Ok(moved) => {
            for (path, offset) in moved {
                out.values.entry(path).or_default().offset = offset;
            }
        }
        Err(why) => state_view::withhold_named(out, root, &["naval_access_rule_highlight"], &format!("naval region view access rule: {why}")),
    }
}

#[allow(clippy::too_many_arguments, reason = "the window's read inputs, each from another owner")]
pub(super) fn bind(
    content: &Content,
    frame: &DecodedFrame,
    vision: &Vision,
    region: u32,
    engaged: &CombatFlags,
    paused: bool,
    sources: &viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap,
) -> io::Result<Bindings> {
    let mut out = Bindings::default();
    let Some(root) = content.assets.native_windows.gui_type(WINDOW) else {
        out.refuse("naval region view window is absent from the recorded content stack".into());
        return Ok(out);
    };
    state_view::seed(content, root, &mut out);
    let name = content
        .strategic_regions
        .get(&region)
        .ok_or_else(|| format!("naval region view name: region {region} is absent from the content stack"))
        .and_then(|definition| state_view::loc(content, &definition.name_key, &[]));
    text(&mut out, root, "region_name", name);
    access(frame, vision, root, &mut out, region);
    picture::bind(content, frame, root, &mut out, region, engaged, paused)?;
    counts::bind(content, frame, vision, root, &mut out, region);
    convoys::bind(content, frame, vision, root, &mut out, region);
    mines(content, frame, vision, root, &mut out, region);
    dominance::bind(content, frame, vision, root, &mut out, region);
    hovers::bind(content, frame, vision, root, &mut out, region, sources);
    super::tooltips::bind_authored(
        &mut out,
        content.assets.as_ref(),
        &content.localisation,
        &content.assets.native_windows,
        Selection::NavalRegion(region),
    );
    Ok(out)
}

fn enemies(frame: &DecodedFrame, vision: &Vision) -> Result<Vec<u32>, String> {
    replay_derive::naval_region_control::raw_roster(frame, vision.ours(), AirCountryRelationRow::ENEMY)
}

