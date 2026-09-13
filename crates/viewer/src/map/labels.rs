//! Victory-point and capital labels (R2 section 5, algorithm D).
//!
//! Each label is a `victory_point_mapicon` or `capital_mapicon` instance from
//! `interface/mapicons.gui`: one `GFX_onmap_victorypoints_strip` frame and one
//! `hoi_20bs` text box, anchored to a world position and drawn in device pixels
//! by [`super::sprites`]. Both windows are layout-identical, so one layout
//! serves both and the only difference is the sprite frame and the cutoff.
//!
//! Label priority is `DEFAULT_PRIORITY_VICTORY_POINTS = DEFAULT_PRIORITY_CAPITAL
//! = 5` against `DEFAULT_PRIORITY_UNITS_STACK = 10` (R2 5.5), so unit counters
//! draw over these. Counters are a later pass; pushing the labels first is what
//! keeps that order when they arrive.
//!
//! # Draw distances
//!
//! R2 section 5.5 reads `VICTORY_POINT_MAP_ICON_TEXT_CUTOFF = {150, 250, 500}`
//! as a per-level cutoff. That reading does not reproduce
//! `array_loaded.png`: at the fitted camera height 310.178 the screenshot
//! carries text on Kiel, Hannover, Metz, Karlsruhe and Breslau, all of which
//! are 10 victory points and would be level 1 with a 250 cutoff, and it draws
//! no icon at all on Malmö or any other 5-point province. What does reproduce
//! it is the reading the defines' own comment gives —
//! `VICTORY_POINT_MAP_ICON_MAX_VICTORY_POINTS_FOR_PERCENT = 22` is the
//! "max value for point on the above range", i.e. the ranges
//! `..._DOT_CUTOFF_MIN/MAX` and `..._TEXT_CUTOFF_MIN/MAX` are interpolated by
//! `min(vp, 22) / 22`:
//!
//! | province | vp | dot cutoff | text cutoff | at 310.178 |
//! |---|---|---|---|---|
//! | Malmö | 5 | 304.5 | 259.1 | neither, and the screenshot draws neither |
//! | Kiel | 10 | 509.1 | 418.2 | both, and the screenshot draws both |
//! | Frankfurt | 20 | 918.2 | 736.4 | both |
//!
//! Capitals are the "special snowflakes" the same block calls them: their icon
//! lives to `CAPITAL_ICON_CUTOFF = 1100` and their text to
//! `VICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX = 1000` whatever the value, which
//! is why Luxembourg (5 points, a capital) keeps both in the screenshot.
//! **UNVERIFIED**: the interpolation is fitted to one reference frame, not read
//! out of the binary. `VICTORY_POINT_MAP_ICON_TEXT_CUTOFF = {150, 250, 500}`
//! has no consumer here at all, so a later fit with a second reference pair
//! should revisit which of the two readings the engine actually uses.
//!
//! # Allegiance colour
//!
//! The perspective and the relation table live in [`super::allegiance`], which
//! the unit counters share. This pass takes the **war-only** reading
//! ([`super::allegiance::war_table`]), where our own faction partners stay
//! grey; that module's docs carry the evidence for why, and why the counters
//! read the same tables differently.
//!
//! Allegiance is read off the **province owner**, not its controller: in
//! `array_loaded.png` Praha is owned by Czechoslovakia and controlled by the
//! Soviet Union and its dot is grey, which the controller reading could not
//! produce.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::{
    ResolvedLayers,
    gui::{self, Length},
    localisation::Localisation,
    map::{MapData, ProvinceKind},
    positions,
};
use replay::{frame::DecodedFrame, rows::tables};

use super::{
    allegiance::{self, Allegiance},
    borders::{HEIGHT_SCALE, WATER_HEIGHT, world_from_pixel},
    camera::Camera,
    sprites::{self, Batch, SpriteSheet},
    text,
};

/// `00_graphics.lua:948`: the victory-point value each icon size starts at.
pub const MAP_ICON_AFTER: [i32; 3] = [0, 9, 20];
/// `00_graphics.lua:953`: the low end of the dot's camera-height range.
pub const DOT_CUTOFF_MIN: f32 = 100.0;
/// `00_graphics.lua:954`: the high end of the dot's camera-height range.
pub const DOT_CUTOFF_MAX: f32 = 1000.0;
/// `00_graphics.lua:951`: the low end of the text's camera-height range.
pub const TEXT_CUTOFF_MIN: f32 = 100.0;
/// `00_graphics.lua:952`: the high end of the text's camera-height range.
pub const TEXT_CUTOFF_MAX: f32 = 800.0;
/// `00_graphics.lua:955`: the value that reaches the top of both ranges.
pub const MAX_VICTORY_POINTS_FOR_PERCENT: f32 = 22.0;
/// `00_graphics.lua:963`: capitals keep their icon this far out.
pub const CAPITAL_ICON_CUTOFF: f32 = 1100.0;
/// `00_graphics.lua:950`: `VICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX`.
pub const CAPITAL_TEXT_CUTOFF: f32 = 1000.0;
// counters
/// `DEFAULT_PRIORITY_VICTORY_POINTS = DEFAULT_PRIORITY_CAPITAL = 5`
/// (`00_graphics.lua:143,158`), the sort priority both label windows take.
pub const PRIORITY: u32 = 5;

/// Unit-stack type 38 `Victory point`, the label anchor (R2 section 9.4).
pub const ANCHOR_STACK: u32 = 38;
/// Unit-stack type 10 `Defending`, which slot 38 defaults from in the loader.
pub const ANCHOR_FALLBACK_STACK: u32 = 10;

/// The file the two label windows live in.
pub const MAPICONS_GUI: &str = "interface/mapicons.gui";
/// `mapicons.gui:628-652`.
pub const VICTORY_POINT_WINDOW: &str = "victory_point_mapicon";
/// `mapicons.gui:655-680`.
pub const CAPITAL_WINDOW: &str = "capital_mapicon";

/// R1 "Map-icon anchoring": the 160 x 30 container is centred on the projected
/// anchor, so its origin is half its height above it. R1 fitted the whole chain
/// against `array_loaded.png` to rms 0.6 px on type-38 anchors.
pub const CONTAINER_OFFSET: [f32; 2] = [0.0, -15.0];

/// The shape group of a label: 0 star, 1 large dot, 2 pentagon, 3 small square.
pub fn shape_group(capital: bool, victory_points: i32) -> u32 {
    if capital {
        0
    } else if victory_points >= MAP_ICON_AFTER[2] {
        1
    } else if victory_points >= MAP_ICON_AFTER[1] {
        2
    } else {
        3
    }
}

/// The zero-based strip frame: `shapeGroup * 5 + colourIndex`.
pub fn frame_index(shape: u32, allegiance: Allegiance) -> u32 {
    shape * 5 + allegiance.index()
}

/// `min(vp, 22) / 22`, the fraction that places a value on the cutoff ranges.
fn percent(victory_points: i32) -> f32 {
    (victory_points.max(0) as f32 / MAX_VICTORY_POINTS_FOR_PERCENT).clamp(0.0, 1.0)
}

/// The camera height the icon stops being drawn above.
pub fn dot_cutoff(capital: bool, victory_points: i32) -> f32 {
    if capital {
        CAPITAL_ICON_CUTOFF
    } else {
        DOT_CUTOFF_MIN + (DOT_CUTOFF_MAX - DOT_CUTOFF_MIN) * percent(victory_points)
    }
}

/// The camera height the name text stops being drawn above.
pub fn text_cutoff(capital: bool, victory_points: i32) -> f32 {
    if capital {
        CAPITAL_TEXT_CUTOFF
    } else {
        TEXT_CUTOFF_MIN + (TEXT_CUTOFF_MAX - TEXT_CUTOFF_MIN) * percent(victory_points)
    }
}

/// The two child offsets of the label windows, as `mapicons.gui` declares them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelLayout {
    /// `iconType "icon"` position, `(-5, 0)`.
    pub icon: [f32; 2],
    /// `instantTextboxType "text"` position, `(26, 5)`.
    pub text: [f32; 2],
    /// The text box's `maxWidth`, 149.
    pub max_width: f32,
    /// The text box's `format`, `left`.
    pub align: text::Align,
}

impl LabelLayout {
    /// Reads `victory_point_mapicon` out of `interface/mapicons.gui`.
    ///
    /// `capital_mapicon` is layout-identical (R2 section 5.1); the two are
    /// compared and a difference is reported rather than silently ignored.
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let document = gui::read(layers, MAPICONS_GUI)?;
        let read = |window: &str| -> io::Result<Self> {
            let items = gui::find_any(&document, &["containerWindowType"], window)
                .ok_or_else(|| invalid(format!("{MAPICONS_GUI} has no {window}")))?;
            let icon = gui::find_child(items, &["iconType"], "icon")
                .ok_or_else(|| invalid(format!("{window} has no icon child")))?;
            // The shipped file spells the same element both ways.
            let text_box =
                gui::find_child(items, &["instantTextboxType", "instantTextBoxType"], "text")
                    .ok_or_else(|| invalid(format!("{window} has no text child")))?;
            Ok(Self {
                icon: pixels(gui::position(icon)?, window, "icon position")?,
                text: pixels(gui::position(text_box)?, window, "text position")?,
                // `maxWidth` is a plain key beside `maxHeight`, not a `size`
                // block, so it is read directly rather than through `size`.
                max_width: gui::number(text_box, "maxWidth")?
                    .ok_or_else(|| invalid(format!("{window} text box declares no maxWidth")))?
                    as f32,
                align: text::Align::parse(gui::format(text_box)),
            })
        };
        let victory_point = read(VICTORY_POINT_WINDOW)?;
        let capital = read(CAPITAL_WINDOW)?;
        if victory_point != capital {
            return Err(invalid(format!(
                "{VICTORY_POINT_WINDOW} and {CAPITAL_WINDOW} no longer share a layout: \
                 {victory_point:?} against {capital:?}"
            )));
        }
        Ok(victory_point)
    }
}

fn pixels(pair: Option<(Length, Length)>, window: &str, what: &str) -> io::Result<[f32; 2]> {
    let (x, y) = pair.ok_or_else(|| invalid(format!("{window} {what} is absent")))?;
    let one = |length: Length| match length {
        Length::Pixels(value) => Ok(value as f32),
        Length::Percent(_) => Err(invalid(format!(
            "{window} {what} is a percentage; the icon layer draws in device pixels"
        ))),
    };
    Ok([one(x)?, one(y)?])
}

/// Static content the label pass reads once per document.
pub struct LabelContent {
    pub layout: LabelLayout,
    /// World anchor per province, type 38 then type 10 then the centroid.
    pub anchors: BTreeMap<u32, [f32; 3]>,
    /// Provinces that fell back to their centroid, stated once.
    pub centroid_fallbacks: Vec<u32>,
}

impl LabelContent {
    /// Reads the layout and the unit stacks, and fills the anchor table.
    pub fn load(layers: &ResolvedLayers, map: &MapData, heightmap: &[u8]) -> io::Result<Self> {
        let layout = LabelLayout::read(layers)?;
        let stacks = positions::read_unit_stacks(layers)?;
        let mut anchors: BTreeMap<u32, [f32; 3]> = BTreeMap::new();
        let mut fallback: BTreeMap<u32, [f32; 3]> = BTreeMap::new();
        for stack in &stacks {
            if stack.kind == ANCHOR_STACK {
                anchors.insert(stack.province, [stack.x, stack.y, stack.z]);
            } else if stack.kind == ANCHOR_FALLBACK_STACK {
                fallback.insert(stack.province, [stack.x, stack.y, stack.z]);
            }
        }
        // The loader copies slot 10 into slot 38 wherever the file left 38
        // unset (R2 section 9.3); doing the same here needs no special case.
        for (province, anchor) in fallback {
            anchors.entry(province).or_insert(anchor);
        }
        let missing: BTreeSet<u32> = map
            .definitions
            .keys()
            .copied()
            .filter(|id| *id != 0 && !anchors.contains_key(id))
            .collect();
        let mut centroid_fallbacks: Vec<u32> = Vec::new();
        if !missing.is_empty() {
            for (province, centre) in centroids(map, &missing) {
                anchors.insert(
                    province,
                    [centre[0], height_at(map, heightmap, centre), centre[1]],
                );
                centroid_fallbacks.push(province);
            }
            centroid_fallbacks.sort_unstable();
        }
        Ok(Self {
            layout,
            anchors,
            centroid_fallbacks,
        })
    }
}

/// Mean world `(x, z)` of every pixel of the wanted provinces, in one pass.
fn centroids(map: &MapData, wanted: &BTreeSet<u32>) -> BTreeMap<u32, [f32; 2]> {
    let mut sums: BTreeMap<u32, (f64, f64, f64)> = BTreeMap::new();
    for (index, id) in map.province_ids.iter().enumerate() {
        if !wanted.contains(id) {
            continue;
        }
        let x = (index % map.width as usize) as f64;
        let y = (index / map.width as usize) as f64;
        let entry = sums.entry(*id).or_insert((0.0, 0.0, 0.0));
        entry.0 += x;
        entry.1 += y;
        entry.2 += 1.0;
    }
    sums.into_iter()
        .map(|(id, (sx, sy, n))| {
            let (x, z) = world_from_pixel((sx / n) as f32, (sy / n) as f32);
            (id, [x, z])
        })
        .collect()
}

/// Terrain height at a world `(x, z)`, clamped up to the water plane.
fn height_at(map: &MapData, heightmap: &[u8], world: [f32; 2]) -> f32 {
    if map.width == 0 || map.height == 0 || heightmap.len() < (map.width * map.height) as usize {
        return WATER_HEIGHT;
    }
    let (px, py) = super::borders::pixel_from_world(world[0], world[1]);
    let column = (px.floor() as i64).rem_euclid(map.width as i64) as usize;
    let row = (py.floor() as i64).clamp(0, map.height as i64 - 1) as usize;
    (heightmap[row * map.width as usize + column] as f32 * HEIGHT_SCALE).max(WATER_HEIGHT)
}

/// Everything one frame of labels needs besides the decoded hour.
pub struct LabelInputs<'a> {
    pub content: &'a LabelContent,
    pub localisation: &'a Localisation,
    pub font: &'a text::Font,
    pub sheet: &'a SpriteSheet,
    pub map: &'a MapData,
    pub camera: Camera,
    pub viewport: [u32; 2],
}

/// What one frame of labels drew, for reporting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LabelReport {
    pub icons: usize,
    pub texts: usize,
    /// The tag the allegiance colours are measured against, if one was found.
    pub perspective: Option<String>,
}

/// Pushes one hour's victory-point and capital labels onto `batch`.
pub fn build(
    inputs: &LabelInputs<'_>,
    frame: &DecodedFrame,
    batch: &mut Batch,
) -> io::Result<LabelReport> {
    let provinces = frame.complete::<tables::Provinces>()?;
    let links: BTreeMap<u32, u32> = frame
        .complete::<tables::CountryLinks>()?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    let tags: BTreeMap<u32, String> = frame
        .complete::<tables::Countries>()?
        .iter()
        .map(|country| {
            (
                country.canonical_slot,
                String::from_utf8_lossy(&country.tag).into_owned(),
            )
        })
        .collect();
    let ours = allegiance::perspective(frame, &links)?;
    let allegiances = allegiance::war_table(frame, &links, ours)?;
    let capitals = capital_provinces(frame, &links)?;

    let height = inputs.camera.height;
    let mut report = LabelReport {
        perspective: ours.and_then(|slot| tags.get(&slot).cloned()),
        ..Default::default()
    };
    let color = inputs.font.color();
    for province in provinces {
        let Some(definition) = inputs.map.definitions.get(&province.id) else {
            continue;
        };
        if matches!(definition.kind, ProvinceKind::Sea | ProvinceKind::Lake) {
            continue;
        }
        let capital = capitals.contains(&province.id);
        let victory_points = province.victory_points;
        if victory_points <= 0 && !capital {
            continue;
        }
        if height > dot_cutoff(capital, victory_points) {
            continue;
        }
        let Some(anchor) = inputs.content.anchors.get(&province.id) else {
            continue;
        };
        let Some(screen) = inputs.camera.project_wrapped(*anchor, inputs.viewport) else {
            continue;
        };
        let origin = [
            screen[0] + CONTAINER_OFFSET[0],
            screen[1] + CONTAINER_OFFSET[1],
        ];
        let layout = &inputs.content.layout;
        let icon = [origin[0] + layout.icon[0], origin[1] + layout.icon[1]];
        // A label wholly outside the frame contributes nothing; the widest one
        // is the container, so one container of margin is enough.
        if icon[0] + 160.0 < 0.0
            || icon[1] + 30.0 < 0.0
            || icon[0] > inputs.viewport[0] as f32
            || icon[1] > inputs.viewport[1] as f32
        {
            continue;
        }
        let owner = links.get(&province.owner).copied();
        let allegiance = owner
            .and_then(|slot| allegiances.get(&slot).copied())
            .unwrap_or(Allegiance::Other);
        let shape = shape_group(capital, victory_points);
        // counters: `icon` is `iconType "icon"`, child 0 of the window.
        batch.at_layer(sprites::layer(PRIORITY, report.icons as u32, 0));
        batch.sprite(
            inputs.sheet,
            frame_index(shape, allegiance),
            icon,
            [1.0, 1.0, 1.0, 1.0],
        );
        report.icons += 1;
        if height > text_cutoff(capital, victory_points) {
            continue;
        }
        let tag = owner.and_then(|slot| tags.get(&slot)).map(String::as_str);
        let Some(name) = victory_point_keys(tag, province.id)
            .iter()
            .find_map(|key| inputs.localisation.get(key))
            .map(str::to_owned)
        else {
            continue;
        };
        let glyphs = text::layout(
            inputs.font,
            &name,
            [origin[0] + layout.text[0], origin[1] + layout.text[1]],
            layout.align,
            Some(layout.max_width),
        );
        if glyphs.is_empty() {
            continue;
        }
        // counters: the text box, which both windows declare after the icon.
        batch.at_layer(sprites::layer(PRIORITY, report.icons as u32 - 1, 1));
        batch.text(&glyphs, color);
        report.texts += 1;
    }
    Ok(report)
}

/// The provinces a `capital_mapicon` is drawn on.
///
/// `Politics.capital` is `CCountry + 0x1018`, and the recorded value for
/// Germany at the reference hour is **64**, which is the state id of
/// Brandenburg and not the province id of Berlin (6521): the field holds a
/// **state**, the same way `history/countries/*.txt` writes `capital = 64`.
/// The icon is drawn on a province, so the state has to be resolved to one.
///
/// **UNVERIFIED**: the engine's own state-to-capital-province rule is not
/// traced. The province of the state with the most victory points is used,
/// ties going to the lowest province id, which puts the star on Berlin for
/// state 64, København for 37, Brussels for 6 and Luxembourg for its own
/// state — the four capitals `array_loaded.png` shows. A state with no
/// victory-point province at all falls back to its lowest province id.
///
/// Only a country that owns at least one province counts. the game ships a
/// `Politics` row for every releasable tag — Schleswig, Hannover, Bavaria,
/// Saxony and so on — each with a capital state, and taking all of them puts a
/// star on Kiel, Essen, Erfurt and Dresden, which `array_loaded.png` does not.
fn capital_provinces(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
) -> io::Result<BTreeSet<u32>> {
    let provinces = frame.complete::<tables::Provinces>()?;
    let existing: BTreeSet<u32> = provinces
        .iter()
        .filter_map(|province| links.get(&province.owner).copied())
        .collect();
    let wanted: BTreeSet<u32> = frame
        .complete::<tables::Politics>()?
        .iter()
        .filter(|politics| politics.capital > 0)
        .filter(|politics| {
            links
                .get(&politics.country)
                .is_some_and(|slot| existing.contains(slot))
        })
        .map(|politics| politics.capital as u32)
        .collect();
    let mut best: BTreeMap<u32, (i32, u32)> = BTreeMap::new();
    for province in provinces {
        if !wanted.contains(&province.state) {
            continue;
        }
        let candidate = (province.victory_points, province.id);
        best.entry(province.state)
            .and_modify(|current| {
                if candidate.0 > current.0 || (candidate.0 == current.0 && candidate.1 < current.1)
                {
                    *current = candidate;
                }
            })
            .or_insert(candidate);
    }
    Ok(best.into_values().map(|(_, province)| province).collect())
}

/// The label key probe order: `<TAG>_VICTORY_POINTS_<id>` then
/// `VICTORY_POINTS_<id>` (R2 section 4.8, the same owner-override pattern the
/// state names use). The caller takes the first key its localisation has.
pub fn victory_point_keys(tag: Option<&str>, province: u32) -> Vec<String> {
    match tag.filter(|tag| !tag.is_empty()) {
        Some(tag) => vec![
            format!("{tag}_VICTORY_POINTS_{province}"),
            format!("VICTORY_POINTS_{province}"),
        ],
        None => vec![format!("VICTORY_POINTS_{province}")],
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shape_group_follows_the_victory_point_levels() {
        // A capital is a star whatever it is worth (R2 section 5.2).
        assert_eq!(shape_group(true, 0), 0);
        assert_eq!(shape_group(true, 50), 0);
        assert_eq!(shape_group(false, 50), 1);
        assert_eq!(shape_group(false, 20), 1);
        assert_eq!(shape_group(false, 19), 2);
        assert_eq!(shape_group(false, 9), 2);
        assert_eq!(shape_group(false, 8), 3);
        assert_eq!(shape_group(false, 1), 3);
    }

    #[test]
    fn the_frame_index_walks_shapes_slowest_and_colours_fastest() {
        assert_eq!(frame_index(0, Allegiance::Enemy), 0);
        assert_eq!(frame_index(0, Allegiance::Other), 4);
        assert_eq!(frame_index(1, Allegiance::Enemy), 5);
        // Budapest: 20 points, Hungarian, a faction partner of ours and grey.
        assert_eq!(frame_index(shape_group(false, 20), Allegiance::Other), 9);
        // Berlin: a German capital with Germany the player, green star.
        assert_eq!(frame_index(shape_group(true, 50), Allegiance::Ours), 2);
        // København: a capital of a country that is not ours.
        assert_eq!(frame_index(shape_group(true, 20), Allegiance::Other), 4);
        // Praha: 30 points, owner not ours, grey large dot.
        assert_eq!(frame_index(shape_group(false, 30), Allegiance::Other), 9);
        // The last frame of the strip is the small grey square.
        assert_eq!(frame_index(shape_group(false, 1), Allegiance::Other), 19);
    }

    #[test]
    fn the_cutoffs_reproduce_the_reference_frame() {
        let reference = 310.178;
        // Malmö, 5 points: neither the dot nor the text survives.
        assert!(dot_cutoff(false, 5) < reference);
        assert!(text_cutoff(false, 5) < reference);
        // Kiel and Hannover, 10 points: both do.
        assert!(dot_cutoff(false, 10) > reference);
        assert!(text_cutoff(false, 10) > reference);
        // Frankfurt, 20 points.
        assert!(text_cutoff(false, 20) > reference);
        // Luxembourg, 5 points but a capital.
        assert!(dot_cutoff(true, 5) > reference);
        assert!(text_cutoff(true, 5) > reference);
        // The ranges end where the defines put them.
        assert_eq!(dot_cutoff(false, 0), DOT_CUTOFF_MIN);
        assert_eq!(dot_cutoff(false, 22), DOT_CUTOFF_MAX);
        assert_eq!(dot_cutoff(false, 99), DOT_CUTOFF_MAX);
        assert_eq!(text_cutoff(false, 22), TEXT_CUTOFF_MAX);
        assert_eq!(dot_cutoff(true, 0), CAPITAL_ICON_CUTOFF);
        assert_eq!(text_cutoff(true, 0), CAPITAL_TEXT_CUTOFF);
        // A negative recorded value never reaches below the bottom of a range.
        assert_eq!(dot_cutoff(false, -5), DOT_CUTOFF_MIN);
    }

    #[test]
    fn the_text_key_probes_the_owner_override_first() {
        assert_eq!(
            victory_point_keys(Some("GER"), 692),
            ["GER_VICTORY_POINTS_692", "VICTORY_POINTS_692"]
        );
        assert_eq!(victory_point_keys(None, 6521), ["VICTORY_POINTS_6521"]);
        assert_eq!(victory_point_keys(Some(""), 6521), ["VICTORY_POINTS_6521"]);
    }
}
