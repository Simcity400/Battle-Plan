//! The terrain map mode's province hover, which replaces the political location text.
//!
//! A sea province (neither land nor lake in its definition flags) shows the naval terrain
//! text of its strategic region's `naval_terrain`, or `ocean`, as the naval combat
//! background does ([`naval_terrain`](super::naval_terrain)). A land or lake province shows
//! its definition's terrain category: the highlighted name on its own line, then the land
//! combat terrain text ([`land_terrain`](super::land_terrain)), then its local province
//! effects ([`local`]).
use std::sync::Arc;

use installed_content_core::terrain::TerrainCategory;
use installed_content_localisation::localisation::Localisation;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use viewer_foundation::{
    map::perspective::Vision,
    tooltip_text::{TextIcons, Tooltip, token, viewing_tag},
};

use super::{content::Content, modifier_text::Definitions};

mod local;

/// Definition flags of a land and of a lake province.
const LAND_OR_LAKE: u8 = 1 | 2;

/// The hover of `province` this hour.
pub fn hover(content: &Content, frame: &DecodedFrame, province: u32, vision: &Vision) -> Tooltip {
    let mut out = Tooltip::default();
    let mut dropped = Vec::new();
    let mut local = None;
    match text(content, frame, province, vision, &mut dropped, &mut local) {
        Ok(text) => out.title = text,
        Err(why) => out.refuse("terrain", why),
    }
    for why in dropped {
        out.refuse("terrain modifier", why);
    }
    if let Some(why) = local {
        out.refuse("local province effects", why);
    }
    out
}

fn text(
    content: &Content,
    frame: &DecodedFrame,
    province: u32,
    vision: &Vision,
    dropped: &mut Vec<String>,
    refused: &mut Option<String>,
) -> Result<String, String> {
    let assets = content.assets.as_ref();
    let loc = &content.localisation;
    let terrain = content
        .movement
        .as_ref()
        .ok_or("the provinces' terrain records did not read")?
        .map
        .terrain(province)
        .ok_or_else(|| format!("province {province} has no terrain record"))?;
    let database = assets.land_terrain.as_ref().map_err(Clone::clone)?;
    let definitions = Definitions::read(frame, &content.modifier_names)
        .map_err(|why| why.to_string())?
        .map_err(|why| why.to_string())?;
    if terrain.flags & LAND_OR_LAKE == 0 {
        let named = content
            .region_of
            .get(&province)
            .and_then(|region| content.strategic_regions.get(region))
            .and_then(|region| region.naval_terrain.as_deref());
        let category = database
            .naval_category(named)
            .ok_or("the terrain database declares no ocean")?;
        let countries = frame
            .complete::<tables::Countries>()
            .map_err(|why| why.to_string())?;
        let naval = assets.naval_terrain.as_ref().map_err(Clone::clone)?;
        return naval.prefix(
            &database.categories[category].name,
            &definitions,
            assets,
            loc,
            viewing_tag(countries, vision),
            dropped,
        );
    }
    let category = usize::try_from(terrain.index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| database.categories.get(index))
        .ok_or_else(|| format!("province {province} is on the null terrain"))?;
    land(
        category,
        &definitions,
        assets,
        loc,
        local::effects(content, frame, &definitions, province),
        dropped,
        refused,
    )
}

/// A land or lake province's text: its category's highlighted name on its own line, the
/// land terrain text, then the local effects, whose refusal leaves the rest.
fn land(
    category: &TerrainCategory,
    definitions: &Definitions<'_>,
    icons: &dyn TextIcons,
    loc: &Localisation,
    local: Result<String, String>,
    dropped: &mut Vec<String>,
    refused: &mut Option<String>,
) -> Result<String, String> {
    let mut text = format!("\u{11}H{}\u{11}!\n", token(icons, loc, &category.name)?);
    text.push_str(&super::land_terrain::effects(
        category,
        definitions,
        icons,
        loc,
        dropped,
    )?);
    match local {
        Ok(local) => text.push_str(&local),
        Err(why) => *refused = Some(why),
    }
    Ok(text)
}

/// The hover of the last province asked for, kept while the hour and viewing country stay.
#[derive(Default)]
pub struct Cache {
    held: Option<Held>,
}

/// The province and viewing country a hover was built for, its hour and the hover.
type Held = ((u32, u32), Arc<DecodedFrame>, Arc<Tooltip>);

impl Cache {
    pub fn request(
        &mut self,
        content: &Content,
        frame: &Arc<DecodedFrame>,
        province: u32,
        vision: &Vision,
    ) -> Arc<Tooltip> {
        let key = (province, vision.ours());
        if let Some((held, held_frame, text)) = &self.held
            && *held == key
            && Arc::ptr_eq(held_frame, frame)
        {
            return Arc::clone(text);
        }
        let text = Arc::new(hover(content, frame, province, vision));
        self.held = Some((key, Arc::clone(frame), Arc::clone(&text)));
        text
    }
}

#[cfg(test)]
mod tests;
