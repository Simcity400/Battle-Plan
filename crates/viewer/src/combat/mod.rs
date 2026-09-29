//! Game-authored land and naval combat windows, shared by playback and PNG export.
mod assets;
mod battle_name;
mod binding;
mod commander_tooltips;
mod defines;
mod details;
mod division_icons;
mod division_tooltips;
pub(crate) mod engaged;
mod fixed;
mod hq;
pub(crate) mod icon_frames;
pub(crate) mod land_details;
mod layout;
pub(crate) mod modifier_text;
mod naval_details;
mod naval_environment;
mod presentation;
pub(crate) mod progress;
mod reinforcement_speed;
mod tactics;
mod template_icons;
pub(crate) mod text;
pub(crate) mod texture;
mod tooltip_painter;
mod tooltips;
mod trigger_names;

use crate::{
    document::Document,
    map::{sprites::Batch, text as map_text},
};
pub use assets::Assets;
use assets::Sprite;
pub(crate) use battle_name::battle_name_candidates;
use binding::Bindings;
/// The one screen-rectangle hit test; the map's counter hits share it.
pub(crate) use layout::contains;
use layout::{Node, boolean, number, string};
use std::{borrow::Cow, collections::BTreeMap, io};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Land(u64),
    Naval(u64),
    NavalResult(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selector {
    Id(u64),
    Province(u32),
}

impl Selector {
    pub fn resolve(self, document: &Document) -> io::Result<Selection> {
        use crate::map::tables::rows;
        use replay::rows::tables;
        let frame = document.frame()?;
        let mut matches = Vec::new();
        for row in rows::<tables::LandCombats>(frame)?.rows() {
            if match self {
                Self::Id(id) => id == row.id,
                Self::Province(id) => id == row.province,
            } {
                matches.push(Selection::Land(row.id));
            }
        }
        for row in rows::<tables::NavalCombats>(frame)?.rows() {
            if match self {
                Self::Id(id) => id == row.id,
                Self::Province(id) => id == row.province,
            } {
                matches.push(Selection::Naval(row.id));
            }
        }
        if matches.is_empty() {
            let results = rows::<tables::NavalResults>(frame)?;
            let newest = results
                .rows()
                .iter()
                .filter(|row| {
                    row.result_hour <= frame.hour
                        && row.readiness != 0
                        && match self {
                            Self::Id(id) => id == row.id,
                            Self::Province(id) => id == row.province,
                        }
                })
                .max_by_key(|row| (row.result_hour, row.id));
            if let Some(row) = newest {
                matches.push(Selection::NavalResult(row.id));
            }
        }
        match matches.as_slice() {
            [selection] => Ok(*selection),
            [] => Err(invalid(format!(
                "no recorded combat matches {self:?} at hour {}",
                frame.hour
            ))),
            _ => Err(invalid(format!(
                "{self:?} matches multiple combats; use --combat with the full combat handle"
            ))),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub selection: Selection,
    pub rect: [f32; 4],
}
impl Hit {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        layout::contains(self.rect, point)
    }
}

/// An open land window refills its lists only when they change membership.
/// 1415CA230 (committed) and 1415CA460 (reserves) first walk the grid's
/// existing entries: when the grid holds as many entries as the side's
/// `CLandCombatant+0xC8`/`+0xE0` array, and each entry's unit still belongs to
/// this combat and is found anywhere in that array, the entries are only
/// refreshed (1415C7E00/1415C8570) and keep their order. Otherwise the grid is
/// cleared and rebuilt in array index order. The window's `+0xAF4` flag forces
/// a rebuild on construction and when the `+0x2AC` interface setting changes;
/// neither is a recorded input, and a newly opened window starts empty.
fn keep_list_order(next: &mut [(u64, Values)], shown: &[(u64, Values)]) {
    let members = |list: &[(u64, Values)]| {
        let mut units: Vec<u64> = list.iter().map(|(unit, _)| *unit).collect();
        units.sort_unstable();
        units
    };
    if members(next) != members(shown) {
        return;
    }
    let rank: BTreeMap<u64, usize> = shown
        .iter()
        .enumerate()
        .map(|(index, (unit, _))| (*unit, index))
        .collect();
    next.sort_by_key(|(unit, _)| rank[unit]);
}

#[derive(Default)]
pub struct Window {
    pub selection: Option<Selection>,
    pub offset: [f32; 2],
    /// Committed and reserve lists per side, then the tactics popup list.
    pub scroll: [usize; 5],
    pub dragging: bool,
    /// The possible-tactics popup (1415C7D20): window presentation state,
    /// never recorded.
    pub tactics_open: bool,
    tactics_position: Option<[f32; 2]>,
    dragging_tactics: bool,
    hover: tooltip_painter::HoverState,
    /// The bindings for (branch, hour, selection, perspective), whether the
    /// engaged index covered that hour, and whether the window shows.
    cached: Option<(
        replay::BranchId,
        i32,
        Selection,
        crate::map::perspective::Choice,
        Bindings,
        bool,
        bool,
    )>,
}

/// 1415C8950 hides an active land window when 1413CD880 finds neither side
/// engaged. An undecided flag keeps the window, whose marker reports the gap.
fn visible(
    frame: &replay::frame::DecodedFrame,
    selection: Selection,
    ours: u32,
    engaged: Option<&crate::engaged_index::EngagedIndex>,
) -> io::Result<bool> {
    use crate::map::tables::rows;
    use replay::rows::tables;
    let Selection::Land(id) = selection else {
        return Ok(true);
    };
    let links: BTreeMap<u32, u32> = rows::<tables::CountryLinks>(frame)?
        .rows()
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let relations = crate::map::visibility::Relations::read(frame, &links);
    // A load-rebuilt flag's provenance is named with the hour's marker lines
    // (CounterReport::combat_limits), which cover this combat too.
    let (flags, _) = crate::engaged_index::combats(
        frame,
        crate::engaged_index::LAND,
        ours,
        &links,
        relations.as_ref().map_err(|reason| *reason),
        engaged,
    )?;
    Ok(flags.get(&id) != Some(&engaged::Engaged::No))
}
impl Window {
    /// What the open window cannot show exactly: its bindings' gaps and the
    /// combat artwork's. Built on request, never per frame; empty when no
    /// window is open.
    pub fn warnings(&self, document: &Document) -> Vec<String> {
        let Some(cache) = self.cached.as_ref().filter(|_| self.selection.is_some()) else {
            return Vec::new();
        };
        let mut warnings = cache.4.warnings.clone();
        warnings.extend(document.icons.combat.warnings.iter().cloned());
        warnings
    }
    pub fn open(&mut self, selection: Selection) {
        *self = Self {
            selection: Some(selection),
            ..Self::default()
        };
    }
    pub fn close(&mut self) {
        *self = Self::default();
    }
    /// The popup's own controls; true when `point` was one of them.
    pub fn handle_click(&mut self, drawn: &Drawn, point: [f32; 2]) -> bool {
        if drawn
            .tactics_close
            .is_some_and(|rect| layout::contains(rect, point))
        {
            self.tactics_open = false;
            return true;
        }
        if drawn
            .tactics_rect
            .is_some_and(|rect| layout::contains(rect, point))
        {
            return true;
        }
        if drawn
            .tactics_button
            .is_some_and(|rect| layout::contains(rect, point))
        {
            self.tactics_open = !self.tactics_open;
            self.tactics_position = None;
            self.scroll[4] = 0;
            return true;
        }
        false
    }
    /// A drag begins on the popup's or the window's title strip.
    pub fn start_drag(&mut self, drawn: Option<&Drawn>, point: Option<[f32; 2]>) {
        self.dragging = false;
        self.dragging_tactics = false;
        let (Some(drawn), Some(point)) = (drawn, point) else {
            return;
        };
        let title = |mut rect: [f32; 4]| {
            rect[3] = 35.0;
            layout::contains(rect, point)
        };
        if drawn.tactics_rect.is_some_and(title) {
            self.dragging = true;
            self.dragging_tactics = true;
        } else if !drawn
            .tactics_rect
            .is_some_and(|rect| layout::contains(rect, point))
            && title(drawn.rect)
        {
            self.dragging = true;
        }
    }
    /// Moves the window or the popup being dragged, in interface pixels.
    pub fn drag_by(&mut self, delta: [f32; 2]) {
        let target = if self.dragging_tactics {
            self.tactics_position.get_or_insert([0.0; 2])
        } else {
            &mut self.offset
        };
        for (position, movement) in target.iter_mut().zip(delta) {
            *position += movement;
        }
    }
    /// Draws the hovered widget's tooltip over `drawn` once its delays pass,
    /// at the pointer (interface pixels) within `extent`. Call every frame,
    /// with `None` when the pointer is elsewhere, so hiding can finish.
    pub fn draw_tooltip(
        &mut self,
        drawn: &mut Drawn,
        assets: &Assets,
        point: Option<[f32; 2]>,
        extent: [f32; 2],
    ) -> io::Result<()> {
        let Ok(defines) = &assets.tooltip_defines else {
            return Ok(());
        };
        let hit = point
            .and_then(|point| {
                drawn
                    .tooltips
                    .iter()
                    .rev()
                    .find(|hit| layout::contains(hit.rect, point))
            })
            .map(|hit| {
                (
                    hit.text.as_deref().unwrap_or_default(),
                    hit.description.as_deref(),
                    hit.rect,
                )
            });
        if let Some((text, anchor)) =
            self.hover
                .shown(hit, point, std::time::Instant::now(), defines.timing)
        {
            tooltip_painter::draw(assets, drawn, text, anchor, extent)?;
        }
        Ok(())
    }
    /// When the tooltip next changes with a still pointer, for a repaint.
    pub fn tooltip_repaint_after(&self, assets: &Assets) -> Option<std::time::Duration> {
        let defines = assets.tooltip_defines.as_ref().ok()?;
        self.hover
            .next_change(std::time::Instant::now(), defines.timing)
    }
    /// Lays the open window out on a frame of `extent` interface pixels; the
    /// batch and every hit rectangle are in interface pixels too
    /// ([`crate::map::ui_scale`]). `engaged` is the window's engaged-flag
    /// index; without it (a dump) the flag is what a load at this hour
    /// rebuilds. Only a draw with `hovers` collects the hover regions
    /// [`Self::draw_tooltip`] needs, so a frame that lays the window out
    /// twice (once for its hit tests) does the hover work once.
    pub(crate) fn draw(
        &mut self,
        document: &Document,
        extent: [f32; 2],
        perspective: &crate::map::perspective::Choice,
        engaged: Option<&crate::engaged_index::EngagedIndex>,
        hovers: bool,
    ) -> io::Result<Option<Drawn>> {
        let Some(selection) = self.selection else {
            return Ok(None);
        };
        let branch = document.playback.branch();
        let hour = document.playback.hour();
        let covered = engaged.is_some_and(|index| index.covers(branch, hour));
        if !self.cached.as_ref().is_some_and(|(b, h, s, p, _, c, _)| {
            (*b, *h, *s, *c) == (branch, hour, selection, covered) && p == perspective
        }) {
            let vision = document.vision(perspective)?;
            let window_shown = visible(document.frame()?, selection, vision.ours(), engaged)?;
            let mut bindings = Bindings::read(document, selection, &vision)?;
            if let Some((_, _, previous, _, shown, _, _)) = &self.cached
                && *previous == selection
            {
                for (next, shown) in bindings.entries.iter_mut().zip(&shown.entries) {
                    keep_list_order(next, shown);
                }
            }
            self.cached = Some((
                branch,
                hour,
                selection,
                perspective.clone(),
                bindings,
                covered,
                window_shown,
            ));
        }
        let (.., bindings, _, shown) = self.cached.as_ref().expect("bound above");
        if !shown {
            return Ok(None);
        }
        let assets = &document.icons.combat;
        let root = &assets.windows[match selection {
            Selection::Land(_) => "landcombatview",
            Selection::Naval(_) => "navalcombatview",
            Selection::NavalResult(_) => "navalcombatresultswindow",
        }];
        let mut rect = root.rect([0.0, 0.0, extent[0], extent[1]])?;
        rect[0] += self.offset[0];
        rect[1] += self.offset[1];
        let mut drawn = Drawn {
            rect,
            ..Drawn::default()
        };
        drawn.batch.at_layer(window_layer(0));
        let mut painter = Painter {
            assets,
            drawn: &mut drawn,
            hovers,
            layer_base: 0,
        };
        painter.node(root, rect, "", &bindings.values)?;
        for side in 0..4 {
            if !matches!(selection, Selection::Land(_)) {
                break;
            }
            let rows = &bindings.entries[side];
            let grid = format!(
                "{}_window/{}_window/list",
                if side % 2 == 0 {
                    "attacker"
                } else {
                    "defender"
                },
                if side < 2 { "committed" } else { "reserves" }
            );
            let entry = if side >= 2 {
                "reserve_combat_entry"
            } else if side == 0 {
                "division_combat_attacker_entry"
            } else {
                "division_combat_defender_entry"
            };
            let grid_rect = root.path_rect(rect, &grid)?;
            let grid_node = root.child(&grid).expect("path resolved above");
            let columns = number(&grid_node.items, "max_slots_horizontal", 1.0).max(1.0) as usize;
            let slot_width = installed_content::gui::slot_size(&grid_node.items)?
                .map(|(x, _)| match x {
                    installed_content::gui::Length::Pixels(x) => x as f32,
                    _ => 0.0,
                })
                .unwrap_or(grid_rect[2]);
            let step = installed_content::gui::slot_size(&grid_node.items)?
                .map(|(_, y)| match y {
                    installed_content::gui::Length::Pixels(y) => y as f32,
                    _ => 40.0,
                })
                .unwrap_or(40.0)
                .max(1.0);
            let parent_grid = grid.rsplit_once('/').expect("nested list").0;
            let parent_rect = root.path_rect(rect, parent_grid)?;
            let height = parent_rect[1] + parent_rect[3] - grid_rect[1];
            let clip = if side < 2 {
                [grid_rect[0] - 32.0, grid_rect[1], 280.0, height]
            } else {
                parent_rect
            };
            painter.drawn.lists[side] = Some(clip);
            let count = (height / step).ceil() as usize * columns;
            // A list scrolls once its rows overflow the box. pairJ_land_6.png
            // shows the attacker's slider for five 40-high committed rows in
            // the 165-high list, though a fifth row is partly drawn without it.
            let hidden_rows = rows
                .len()
                .div_ceil(columns)
                .saturating_sub((height / step).floor() as usize);
            let start = self.scroll[side].min(hidden_rows) * columns;
            for (index, (_, row)) in rows.iter().skip(start).take(count).enumerate() {
                let entry_node = &assets.windows[entry];
                let size = entry_node.size([grid_rect[2], step])?;
                let at = [
                    grid_rect[0]
                        + ((grid_rect[2] - size[0]) * 0.5).trunc()
                        + (index % columns) as f32 * slot_width,
                    grid_rect[1] + (index / columns) as f32 * step,
                    size[0],
                    size[1],
                ];
                let first = painter.drawn.batch.quads.len();
                let first_hover = painter.drawn.tooltips.len();
                painter.node(entry_node, at, "", row)?;
                clip_quads(&mut painter.drawn.batch, first, clip);
                clip_tooltips(&mut painter.drawn.tooltips, first_hover, clip);
            }
            if hidden_rows > 0 {
                let slider = &assets.windows[if side % 2 == 0 {
                    "left_vertical_slider"
                } else {
                    "right_vertical_slider"
                }];
                let mut slider_rect = slider.rect(parent_rect)?;
                if side % 2 == 1 {
                    slider_rect[0] -= slider_rect[2];
                }
                // pairL_land_danzig.png: the committed slider starts below the
                // container's `margin = { top = 10 }`, level with the list.
                slider_rect[1] += grid_rect[1] - parent_rect[1];
                slider_rect[3] = height;
                painter.scrollbar(
                    slider,
                    slider_rect,
                    side,
                    (start / columns) as f32 / hidden_rows as f32,
                )?;
            }
        }
        for row in &bindings.naval_entries {
            let grid = root.path_rect(rect, &row.grid)?;
            let entry = &assets.windows["navalcombat_ship_entry"];
            let at = [
                grid[0] + row.offset[0],
                grid[1] + row.offset[1],
                row.size[0],
                row.size[1],
            ];
            let first = painter.drawn.batch.quads.len();
            painter.node(entry, at, "", &row.values)?;
            clip_quads(
                &mut painter.drawn.batch,
                first,
                // Native UPPER_RIGHT grids center a sprite on their slot edge;
                // that edge is not a horizontal clipping boundary.
                [rect[0], grid[1], rect[2], grid[3]],
            );
        }
        // Some authored backgrounds extend below the root's nominal content height.
        for quad in &drawn.batch.quads {
            drawn.rect[2] = drawn.rect[2].max(quad.rect[0] + quad.rect[2] - drawn.rect[0]);
            drawn.rect[3] = drawn.rect[3].max(quad.rect[1] + quad.rect[3] - drawn.rect[1]);
        }
        if self.tactics_open && matches!(selection, Selection::Land(_)) {
            draw_tactics(
                &mut self.tactics_position,
                self.scroll[4],
                &mut drawn,
                assets,
                &bindings.tactics,
                extent,
                hovers,
            )?;
        }
        Ok(Some(drawn))
    }
}

/// 1415C7D20 places the companion list ten pixels right of the battle window
/// at its y; it does not recentre it.
fn draw_tactics(
    tactics_position: &mut Option<[f32; 2]>,
    scroll: usize,
    drawn: &mut Drawn,
    assets: &Assets,
    menu_values: &tactics::Menu,
    extent: [f32; 2],
    hovers: bool,
) -> io::Result<()> {
    let menu = &assets.windows["tactics_list_view"];
    let size = menu.size(extent)?;
    let rect = drawn.rect;
    let position = *tactics_position.get_or_insert([rect[0] + rect[2] + 10.0, rect[1]]);
    let at = [position[0], position[1], size[0], size[1]];
    let mut popup = Drawn {
        rect: at,
        ..Drawn::default()
    };
    // The popup paints into its own batch, appended after the window's
    // quads: its layers continue the window's push order.
    let layer_base = drawn.batch.quads.len();
    popup.batch.at_layer(window_layer(layer_base));
    let mut values = menu_values.values.clone();
    values.entry("Background".into()).or_default().size = Some(size);
    let mut painter = Painter {
        assets,
        drawn: &mut popup,
        hovers,
        layer_base,
    };
    painter.node(menu, at, "", &values)?;
    let container = menu.path_rect(at, "tactics_list_container")?;
    let grid_path = "tactics_list_container/tactics_list_gridbox";
    let grid = menu.path_rect(at, grid_path)?;
    let grid_node = menu.child(grid_path).expect("authored tactics list");
    let step = installed_content::gui::slot_size(&grid_node.items)?
        .and_then(|(_, height)| match height {
            installed_content::gui::Length::Pixels(height) => Some(height as f32),
            _ => None,
        })
        .ok_or_else(|| invalid("tactics list has no pixel row height"))?;
    let wheel = number(
        &menu
            .child("tactics_list_container")
            .expect("authored container")
            .items,
        "scroll_wheel_factor",
        step,
    )
    .max(1.0);
    let overflow = (menu_values.entries.len() as f32 * step - grid[3]).max(0.0);
    let max_steps = (overflow / wheel).ceil() as usize;
    let scroll = (scroll.min(max_steps) as f32 * wheel).min(overflow);
    let entry = &assets.windows["tactics_list_entry"];
    let entry_size = entry.size([grid[2], step])?;
    for (index, values) in menu_values.entries.iter().enumerate() {
        let y = grid[1] + index as f32 * step - scroll;
        if y + entry_size[1] <= grid[1] || y >= grid[1] + grid[3] {
            continue;
        }
        let first = painter.drawn.batch.quads.len();
        let first_hover = painter.drawn.tooltips.len();
        painter.node(
            entry,
            [grid[0], y, entry_size[0], entry_size[1]],
            "",
            values,
        )?;
        clip_quads(&mut painter.drawn.batch, first, grid);
        clip_tooltips(&mut painter.drawn.tooltips, first_hover, grid);
    }
    if max_steps > 0 {
        let slider = &assets.windows["right_vertical_slider"];
        let mut slider_rect = slider.rect(container)?;
        slider_rect[0] -= slider_rect[2];
        slider_rect[1] += grid[1] - container[1];
        slider_rect[3] = grid[3];
        painter.scrollbar(slider, slider_rect, 4, scroll / overflow)?;
    }
    drawn.tactics_rect = Some(at);
    drawn.tactics_close = popup.close;
    drawn.lists[4] = Some(grid);
    // The foreground popup consumes blank-space hover too; a covered
    // window control must not show its tooltip through it.
    if hovers {
        drawn.tooltips.push(TooltipHit {
            rect: at,
            text: None,
            description: None,
        });
        drawn.tooltips.extend(popup.tooltips);
    }
    drawn.scroll_buttons.extend(popup.scroll_buttons);
    drawn.batch.quads.extend(popup.batch.quads);
    Ok(())
}

#[derive(Default)]
pub struct Drawn {
    pub batch: Batch,
    pub rect: [f32; 4],
    pub close: Option<[f32; 4]>,
    /// Committed and reserve lists per side, then the tactics popup list.
    pub lists: [Option<[f32; 4]>; 5],
    pub scroll_buttons: Vec<([f32; 4], usize, i32)>,
    /// Hover regions in draw order; the last containing one wins.
    tooltips: Vec<TooltipHit>,
    tactics_button: Option<[f32; 4]>,
    tactics_close: Option<[f32; 4]>,
    tactics_rect: Option<[f32; 4]>,
}

/// Bound hover text. The bindings hold it once; each drawn hover region
/// shares it rather than copying the text every frame.
pub(crate) type HoverText = std::sync::Arc<str>;

/// One widget's hover region and its text, in interface pixels.
#[derive(Clone, Debug, PartialEq)]
struct TooltipHit {
    rect: [f32; 4],
    text: Option<HoverText>,
    description: Option<HoverText>,
}

/// Clips the hover regions pushed since `first` to `clip`, as the quads,
/// keeping their order.
fn clip_tooltips(tooltips: &mut Vec<TooltipHit>, first: usize, clip: [f32; 4]) {
    let mut kept = first;
    for index in first..tooltips.len() {
        let rect = tooltips[index].rect;
        let left = rect[0].max(clip[0]);
        let top = rect[1].max(clip[1]);
        let right = (rect[0] + rect[2]).min(clip[0] + clip[2]);
        let bottom = (rect[1] + rect[3]).min(clip[1] + clip[3]);
        if right > left && bottom > top {
            tooltips[index].rect = [left, top, right - left, bottom - top];
            tooltips.swap(kept, index);
            kept += 1;
        }
    }
    tooltips.truncate(kept);
}
impl Drawn {
    fn in_popup(&self, point: [f32; 2]) -> bool {
        self.tactics_rect
            .is_some_and(|rect| layout::contains(rect, point))
    }
    pub fn contains(&self, point: [f32; 2]) -> bool {
        layout::contains(self.rect, point) || self.in_popup(point)
    }
    pub fn closes(&self, point: [f32; 2]) -> bool {
        !self.in_popup(point) && self.close.is_some_and(|rect| layout::contains(rect, point))
    }
    /// The list under `point`; the popup covers the window's lists.
    pub fn list_at(&self, point: [f32; 2]) -> Option<usize> {
        if self.in_popup(point) {
            return self.lists[4]
                .filter(|rect| layout::contains(*rect, point))
                .map(|_| 4);
        }
        self.lists
            .iter()
            .position(|rect| rect.is_some_and(|rect| layout::contains(rect, point)))
    }
    pub fn scroll_action(&self, point: [f32; 2]) -> Option<(usize, i32)> {
        self.scroll_buttons
            .iter()
            .rev()
            .find(|(rect, list, _)| {
                layout::contains(*rect, point) && (*list == 4 || !self.in_popup(point))
            })
            .map(|(_, side, direction)| (*side, *direction))
    }
}

#[derive(Clone, Default)]
pub struct Widget {
    pub text: Option<String>,
    /// Hover text, and the delayed body shown after its longer delay.
    pub tooltip: Option<HoverText>,
    pub tooltip_description: Option<HoverText>,
    /// A dynamic hover builder owns both strings ([`tooltips::set_hover`]):
    /// the window's update (1415C1A60, 1415BF4B0, 1415C0AE0) replaces the
    /// authored `pdx_tooltip` the widget was created with, so the authored
    /// seed never replaces its text or its refusal.
    pub hover_owned: bool,
    pub sprite: Option<String>,
    pub flag: Option<String>,
    pub fill: Option<f32>,
    pub frame: Option<u32>,
    pub rotation: Option<f32>,
    pub opacity: Option<f32>,
    pub hidden: bool,
    pub size: Option<[f32; 2]>,
    pub offset: [f32; 2],
    pub text_width: Option<f32>,
    /// Repeated authored entries, used by reserve, trait and modifier grids.
    pub entry_template: Option<String>,
    pub entries: Vec<Values>,
}
pub type Values = BTreeMap<String, Widget>;
/// The native naval map icon's two progress widgets, using the same authored
/// progress textures and threshold selection as the detail window.
pub(crate) fn naval_bubble_progress(
    assets: &Assets,
    rect: [f32; 4],
    small: bool,
    progress: i64,
    flip: bool,
) -> io::Result<Batch> {
    let root = &assets.windows["naval_combat_mapicon"];
    let name = if small { "progress_small" } else { "progress" };
    let mut values = Values::new();
    for node in &root.children {
        values.insert(
            node.name.clone(),
            Widget {
                hidden: node.name != name,
                ..Widget::default()
            },
        );
    }
    values.insert(
        name.into(),
        Widget {
            fill: Some(if flip {
                1.0 - progress as f32 / 100_000.0
            } else {
                progress as f32 / 100_000.0
            }),
            rotation: Some(if flip { std::f32::consts::PI } else { 0.0 }),
            ..Widget::default()
        },
    );
    let mut drawn = Drawn::default();
    Painter {
        assets,
        drawn: &mut drawn,
        hovers: false,
        layer_base: 0,
    }
    .node(root, rect, "", &values)?;
    Ok(drawn.batch)
}
/// The viewer's inline flag: this marker, then the resolved flag-atlas key,
/// ended by a space or the string end. The game writes 0x12 and a country tag
/// (CCombatantEntry 1415BA240); the viewer resolves the tag first and marks it
/// with a byte the localisation loader never makes a control (142386AD0), so
/// an authored `@` (0x12) inside localised text around it is still refused
/// by name ([`binding::displayable`]), in combat windows and map hovers alike.
pub(crate) const INLINE_FLAG: char = '\u{1f}';
/// CGameBitmapFont::text_icon_lookup (14162E880) reads an icon name as the
/// part before its first `|` and, after it, a frame counted from one: digits
/// only, else the first frame; no digits, or `0`, give frame -1.
fn icon_name(name: &str) -> (&str, i64) {
    let Some((base, digits)) = name.split_once('|') else {
        return (name, 0);
    };
    let mut value: i32 = 0;
    for digit in digits.bytes() {
        if !digit.is_ascii_digit() {
            return (base, 0);
        }
        value = value.wrapping_mul(10).wrapping_add(i32::from(digit - b'0'));
    }
    if value < 0 {
        return (base, 0);
    }
    (base, i64::from(value) - 1)
}
/// The sprite of a text icon and the frame it selects: `GFX_<name>` first,
/// then `<name>` (14162E880). `None` is an icon the content lacks, which
/// the game draws as nothing with no advance (14162EB10, 14162EFE0).
fn text_icon<'a>(assets: &'a Assets, name: &str) -> Option<(Cow<'a, Sprite>, i64)> {
    let (base, frame) = icon_name(name);
    let sprite = assets
        .sprite(&format!("GFX_{base}"))
        .or_else(|| assets.sprite(base))?;
    Some((sprite, frame))
}
/// pairJ_land_5.png: right-aligned "<flag> 169th 'Madison' Division" (117 px
/// of Arial12 text) starts 129 px before its box edge, and "<flag> 900
/// Infantry Division" (101 px) 113 px before it. An attacker's trailing flag
/// starts exactly at the pen after "name ".
const INLINE_FLAG_ADVANCE: f32 = 12.0;
pub(crate) enum Chunk {
    Text(String, [f32; 4]),
    /// A text icon's sprite and frame.
    Icon(Box<Sprite>, u32),
    Flag(String),
}
/// The layer of a window quad pushed after `quads` others: the window draws
/// in push order, one icon ordinal per quad, in the GUI band above every
/// map-icon priority, so the open window covers the icons behind it.
fn window_layer(quads: usize) -> u64 {
    crate::map::sprites::gui_layer(u32::try_from(quads).unwrap_or(u32::MAX), 0)
}

struct Painter<'a> {
    assets: &'a Assets,
    drawn: &'a mut Drawn,
    /// Whether to collect the widgets' hover regions.
    hovers: bool,
    /// The quads the destination batch holds before `drawn`'s: a popup
    /// painted into its own batch and appended stays above them.
    layer_base: usize,
}
impl Painter<'_> {
    /// The layer of the next quad in the destination batch's push order.
    fn next_layer(&self) -> u64 {
        window_layer(self.layer_base.saturating_add(self.drawn.batch.quads.len()))
    }
    fn node(&mut self, node: &Node, rect: [f32; 4], path: &str, values: &Values) -> io::Result<()> {
        for child in &node.children {
            let key = if path.is_empty() {
                child.name.clone()
            } else {
                format!("{path}/{}", child.name)
            };
            let data = values.get(&key);
            if data.is_some_and(|d| d.hidden) {
                continue;
            }
            let mut origin = child.origin(node.content_rect(rect))?;
            if let Some(data) = data {
                origin[0] += data.offset[0];
                origin[1] += data.offset[1];
            }
            if child.kind == "containerwindowtype" {
                let first = self.drawn.batch.quads.len();
                let first_hover = self.drawn.tooltips.len();
                self.node(child, child.rect(node.content_rect(rect))?, &key, values)?;
                if boolean(&child.items, "clipping").map_err(invalid)? == Some(true) {
                    let clip = child.rect(node.content_rect(rect))?;
                    clip_quads(&mut self.drawn.batch, first, clip);
                    clip_tooltips(&mut self.drawn.tooltips, first_hover, clip);
                }
                continue;
            }
            if child.kind == "gridboxtype" {
                if let Some(data) = data {
                    self.grid(child, child.rect(node.content_rect(rect))?, data)?;
                }
                continue;
            }
            self.drawn.batch.at_layer(self.next_layer());
            let sprite = data
                .and_then(|d| d.sprite.as_deref())
                .or_else(|| installed_content::gui::sprite(&child.items));
            if self.hovers
                && let Some(data) =
                    data.filter(|data| data.tooltip.is_some() || data.tooltip_description.is_some())
            {
                self.hover_region(child, rect, origin, sprite, data)?;
            }
            if let Some(flag) = data
                .and_then(|d| d.flag.as_ref())
                .and_then(|key| self.assets.flag(key))
            {
                // 14162F510 offsets the masked-shield quad by (0,-1) on this
                // renderer (-0.5,-0.5 on the other branch) in its y-up vertex
                // space: the flag and overlay land one pixel *below* the
                // authored position. Sub-pixel fits of all six medium flags in
                // pairJ_land_5/6 and pairJ_naval_5 put them 2 px below the old
                // (0,-1) screen shift (UK flag MAE 55.8 -> 6.0).
                let origin = if sprite == Some("GFX_flag_small2") {
                    [origin[0], origin[1] + 1.0]
                } else {
                    origin
                };
                let (texture, uv) = if sprite == Some("GFX_flag_small2") {
                    (flag.texture, flag.uv)
                } else {
                    flag.inline.unwrap_or((flag.texture, flag.uv))
                };
                self.drawn.batch.cell(
                    texture,
                    origin,
                    data.and_then(|d| d.size).unwrap_or([41.0, 26.0]),
                    uv,
                    [1.0; 4],
                );
                // GFX_flag_small2's maskedShieldType draws the authored overlay
                // over the large flag atlas. Inline country markers have no shield.
                if sprite == Some("GFX_flag_small2")
                    && let Some(overlay) = self.assets.sprite("GFX_flag_small2_not_masked")
                {
                    self.drawn.batch.at_layer(self.next_layer());
                    self.drawn
                        .batch
                        .scaled(&overlay.sheet, 0, origin, overlay.size, [1.0; 4]);
                }
            } else if let Some(sprite) = sprite.and_then(|name| self.assets.sprite(name)) {
                let scale = number(&child.items, "scale", 1.0);
                let size = data
                    .and_then(|d| d.size)
                    .unwrap_or(sprite.size.map(|v| v * scale));
                let rotation = data
                    .and_then(|d| d.rotation)
                    .unwrap_or_else(|| number(&child.items, "rotation", 0.0));
                let at = if boolean(&child.items, "centerPosition").map_err(invalid)? == Some(true)
                {
                    [origin[0] - size[0] * 0.5, origin[1] - size[1] * 0.5]
                } else {
                    origin
                };
                if sprite.progress {
                    let (at, rotation) = progress_turn(at, size, rotation);
                    self.progress(&sprite, at, size, rotation, data.and_then(|d| d.fill));
                } else {
                    let frame = data
                        .and_then(|d| d.frame)
                        .unwrap_or(number(&child.items, "frame", 1.0).max(1.0) as u32 - 1);
                    let color = [1.0, 1.0, 1.0, data.and_then(|d| d.opacity).unwrap_or(1.0)];
                    match sprite.border {
                        Some(border) if size != sprite.size && rotation == 0.0 => {
                            self.cornered(&sprite, frame, at, size, border, color)
                        }
                        _ => self.drawn.batch.rotated(
                            &sprite.sheet,
                            frame,
                            at,
                            size,
                            rotation,
                            color,
                        ),
                    }
                }
                if child.name == "close" || child.name == "close_button" {
                    self.drawn.close = Some([at[0], at[1], size[0], size[1]]);
                }
                if child.name == "tactics_list_button" {
                    self.drawn.tactics_button = Some([at[0], at[1], size[0], size[1]]);
                }
            }
            if let Some(font) =
                installed_content::gui::font(&child.items).and_then(|name| self.assets.fonts.get(name))
            {
                // .gui text fields contain developer examples ("999", "Supreme Leader").
                // No unbound dynamic widget is allowed to expose those examples.
                let value = data.and_then(|d| d.text.as_deref()).unwrap_or("—");
                let align = match installed_content::gui::format(&child.items) {
                    Some("center" | "centre") => map_text::Align::Centre,
                    Some("right") => map_text::Align::Right,
                    _ => map_text::Align::Left,
                };
                let width = data.and_then(|d| d.text_width).or(installed_content::gui::number(
                    &child.items,
                    "maxWidth",
                )?
                .map(|v| v as f32));
                let fixed = boolean(&child.items, "fixedsize").map_err(invalid)? == Some(true);
                let value = match width {
                    Some(width) if fixed => {
                        // CInstantTextObject (1422D7A00) replaces an unset box
                        // dimension with 200.
                        let height = installed_content::gui::number(&child.items, "maxHeight")?
                            .filter(|v| *v != 0.0)
                            .map_or(200.0, |v| v as f32);
                        self.fit(font, value, width, height)
                    }
                    _ => value.to_owned(),
                };
                self.text(font, &value, origin, align, width);
            }
        }
        Ok(())
    }
    /// An authored vertical scrollbar over `rect` with its thumb at
    /// `fraction` of the travel. pairL_land_danzig.png, device pixels of the
    /// defender committed slider at (1054,649,18,165): the up button ends 19 px
    /// down, the track glow spans from there to the down button at H - 19, and
    /// the 16 px thumb sits at the track origin plus the authored (-2,-2).
    fn scrollbar(
        &mut self,
        slider: &Node,
        rect: [f32; 4],
        list: usize,
        fraction: f32,
    ) -> io::Result<()> {
        let assets = self.assets;
        let part = |name: &str| -> io::Result<([f32; 2], [f32; 2])> {
            let node = slider
                .child(name)
                .ok_or_else(|| invalid(format!("scrollbar has no {name}")))?;
            let origin = node.origin([0.0; 4])?;
            let size = installed_content::gui::sprite(&node.items)
                .and_then(|key| assets.sprite(key))
                .map_or([0.0; 2], |sprite| sprite.size);
            Ok((origin, size))
        };
        let (decrease, decrease_size) = part("Decrease")?;
        let (increase, _) = part("Increase")?;
        let (track, track_size) = part("Track")?;
        let (_, thumb) = part("Slider")?;
        let top = decrease[1] + decrease_size[1];
        let height = (rect[3] + increase[1] - top).max(0.0);
        let mut values = Values::from([
            (
                "Background".into(),
                Widget {
                    size: Some([rect[2], rect[3]]),
                    ..Widget::default()
                },
            ),
            (
                "Track".into(),
                Widget {
                    size: Some([track_size[0], height]),
                    offset: [0.0, top],
                    ..Widget::default()
                },
            ),
            (
                "Increase".into(),
                Widget {
                    offset: [rect[2], rect[3]],
                    ..Widget::default()
                },
            ),
            (
                "Slider".into(),
                Widget {
                    hidden: true,
                    ..Widget::default()
                },
            ),
        ]);
        self.node(slider, rect, "", &values)?;
        let thumb_only: Values = slider
            .children
            .iter()
            .map(|child| {
                let shown = child.name == "Slider";
                (
                    child.name.clone(),
                    Widget {
                        hidden: !shown,
                        offset: if shown {
                            [
                                track[0],
                                top + (height - thumb[1]).max(0.0) * fraction.clamp(0.0, 1.0),
                            ]
                        } else {
                            [0.0; 2]
                        },
                        ..Widget::default()
                    },
                )
            })
            .collect();
        self.node(slider, rect, "", &thumb_only)?;
        for (name, direction) in [("Decrease", -1), ("Increase", 1)] {
            let node = slider.child(name).expect("checked above");
            let mut at = node.origin(rect)?;
            if let Some(value) = values.remove(name) {
                at[0] += value.offset[0];
                at[1] += value.offset[1];
            }
            if let Some(sprite) =
                installed_content::gui::sprite(&node.items).and_then(|key| assets.sprite(key))
            {
                self.drawn.scroll_buttons.push((
                    [at[0], at[1], sprite.size[0], sprite.size[1]],
                    list,
                    direction,
                ));
            }
        }
        Ok(())
    }
    /// The widget's hover region: its explicit size, else its sprite at the
    /// authored scale, else its text box. A GUI background inherits the
    /// containing window's extent, as transparent column backgrounds do.
    fn hover_region(
        &mut self,
        child: &Node,
        rect: [f32; 4],
        origin: [f32; 2],
        sprite: Option<&str>,
        data: &Widget,
    ) -> io::Result<()> {
        let size = if child.kind == "background" {
            [rect[2], rect[3]]
        } else {
            data.size
                .or_else(|| {
                    sprite
                        .and_then(|name| self.assets.sprite(name))
                        .map(|sprite| {
                            sprite
                                .size
                                .map(|dimension| dimension * number(&child.items, "scale", 1.0))
                        })
                })
                .unwrap_or([
                    number(&child.items, "maxWidth", rect[2]),
                    number(&child.items, "maxHeight", rect[3]),
                ])
        };
        let at = if boolean(&child.items, "centerPosition").map_err(invalid)? == Some(true) {
            [origin[0] - size[0] * 0.5, origin[1] - size[1] * 0.5]
        } else {
            origin
        };
        self.drawn.tooltips.push(TooltipHit {
            rect: [at[0], at[1], size[0], size[1]],
            text: data.tooltip.clone(),
            description: data.tooltip_description.clone(),
        });
        Ok(())
    }
    /// A `corneredTileSpriteType` drawn at another size keeps its `borderSize`
    /// corners and edges at texel size; the centre repeats when `tilingCenter`
    /// is set and stretches otherwise. pairL_land_danzig.png shows both list
    /// scrollbars built this way (`GFX_scrollbar_vertical_bg`, 52-texel ends;
    /// `GFX_scroll_track`, 4-texel glow). UNVERIFIED: whether edges also tile;
    /// every cornered sprite these windows resize is uniform along its edges.
    fn cornered(
        &mut self,
        sprite: &assets::Sprite,
        frame: u32,
        at: [f32; 2],
        size: [f32; 2],
        border: [f32; 2],
        color: [f32; 4],
    ) {
        let texels = sprite.sheet.frame_size();
        let [u0, v0, u1, v1] = sprite.sheet.frame_uv(frame);
        let spans = |axis: usize| {
            cornered_spans(border[axis], texels[axis], size[axis], sprite.tiling_center)
        };
        let (columns, rows) = (spans(0), spans(1));
        for &[y, height, ty, theight] in &rows {
            for &[x, width, tx, twidth] in &columns {
                if width <= 0.0 || height <= 0.0 {
                    continue;
                }
                let u = |t: f32| u0 + (u1 - u0) * t / texels[0].max(1.0);
                let v = |t: f32| v0 + (v1 - v0) * t / texels[1].max(1.0);
                self.drawn.batch.cell(
                    sprite.sheet.texture,
                    [at[0] + x, at[1] + y],
                    [width, height],
                    [u(tx), v(ty), u(tx + twidth), v(ty + theight)],
                    color,
                );
            }
        }
    }
    fn grid(&mut self, node: &Node, rect: [f32; 4], data: &Widget) -> io::Result<()> {
        let Some(entry) = data
            .entry_template
            .as_ref()
            .and_then(|key| self.assets.windows.get(key))
        else {
            return Ok(());
        };
        let size = entry.size([rect[2], rect[3]])?;
        let slot = installed_content::gui::slot_size(&node.items)?
            .map(|(x, y)| {
                let pixels = |v| match v {
                    installed_content::gui::Length::Pixels(v) => v as f32,
                    _ => 0.0,
                };
                [pixels(x), pixels(y)]
            })
            .unwrap_or(size);
        let columns = number(
            &node.items,
            "max_slots_horizontal",
            (rect[2] / slot[0].max(1.0)).floor(),
        )
        .max(1.0) as usize;
        let right =
            string(&node.items, "format").is_some_and(|s| s.to_ascii_lowercase().contains("right"));
        let traits = data.entry_template.as_deref() == Some("unit_leader_trait_entry");
        let pitch = if traits && data.entries.len() > 6 {
            5.0 * slot[0] / data.entries.len() as f32
        } else {
            slot[0]
        };
        for (index, values) in data.entries.iter().enumerate() {
            let column = if traits { index } else { index % columns };
            // 1422CA030 places an UPPER_RIGHT slot at grid width - slot width
            // - slot width * column: the slot, not the entry, meets the edge.
            let x = if right {
                rect[2] - slot[0] - column as f32 * pitch
            } else {
                column as f32 * pitch
            };
            let at = [
                rect[0] + x,
                rect[1]
                    + if traits {
                        -5.0
                    } else {
                        (index / columns) as f32 * slot[1]
                    },
                size[0],
                size[1],
            ];
            self.node(entry, at, "", values)?;
        }
        Ok(())
    }
    fn text(
        &mut self,
        font: &map_text::Font,
        value: &str,
        origin: [f32; 2],
        align: map_text::Align,
        width: Option<f32>,
    ) {
        if !value.contains(['\n', '\u{11}', '\u{13}', INLINE_FLAG]) {
            self.drawn.batch.text(
                &map_text::layout(font, value, origin, align, width),
                font.color(),
            );
            return;
        }
        let mut color = text::Colours::new(font.color());
        for (line, raw) in value.lines().enumerate() {
            let chunks = self.chunks(raw, &mut color);
            let measured: f32 = chunks.iter().map(|c| self.length(font, c)).sum();
            let slack = (width.unwrap_or(0.0) - measured).max(0.0);
            let mut x = origin[0]
                + match align {
                    map_text::Align::Left => 0.0,
                    map_text::Align::Centre => slack * 0.5,
                    map_text::Align::Right => slack,
                };
            let y = origin[1] + line as f32 * font.line_height();
            for chunk in chunks {
                let advance = self.length(font, &chunk);
                let remaining = width.map(|w| (origin[0] + w - x).max(0.0));
                self.drawn.batch.at_layer(self.next_layer());
                match &chunk {
                    Chunk::Icon(sprite, frame) => {
                        if remaining.is_none_or(|w| w >= advance) {
                            self.drawn.batch.scaled(
                                &sprite.sheet,
                                *frame,
                                [x, y],
                                sprite.size,
                                [1.0; 4],
                            );
                        }
                    }
                    Chunk::Flag(key) => {
                        let size = self.assets.inline_flag_size;
                        if remaining.is_none_or(|w| w >= size[0])
                            && let Some((texture, uv)) = self.assets.inline_flag(key)
                        {
                            // The small flag's top is five pixels below the
                            // line top in both Arial12 (pairJ_land_5.png) and
                            // hoi_16mbs (pairJ_naval_5.png ship descriptions).
                            let top = y + 5.0;
                            self.drawn.batch.cell(texture, [x, top], size, uv, [1.0; 4]);
                        }
                    }
                    Chunk::Text(run, color) => {
                        self.drawn.batch.text(
                            &map_text::layout(font, run, [x, y], map_text::Align::Left, remaining),
                            *color,
                        );
                    }
                }
                x += advance;
            }
        }
    }
    /// One line of rich text as the game's text layout reads it
    /// ([`text::chunks`]).
    fn chunks(&self, raw: &str, colours: &mut text::Colours) -> Vec<Chunk> {
        text::chunks(self.assets, raw, colours)
    }
    fn length(&self, font: &map_text::Font, chunk: &Chunk) -> f32 {
        match chunk {
            Chunk::Icon(sprite, _) => sprite.size[0],
            Chunk::Flag(_) => INLINE_FLAG_ADVANCE,
            Chunk::Text(run, _) => map_text::measure(font, run),
        }
    }
    /// The text a `fixedsize` box shows: [`fit`] with this font's metrics.
    fn fit(&self, font: &map_text::Font, value: &str, width: f32, height: f32) -> String {
        use crate::map::names::GlyphMetrics;
        if value.contains(['\n', '\u{11}', '\u{13}']) {
            return value.into();
        }
        let mut color = text::Colours::new(font.color());
        let glyphs = Glyphs {
            // The font loader (142287830) stores a zero height for the space,
            // which is what ends a word.
            glyph: |ch| {
                GlyphMetrics::glyph(font, ch).map(|g| (g.xadvance, ch == ' ' || g.height == 0.0))
            },
            kerning: |a, b| GlyphMetrics::kerning(font, a, b),
            line_height: font.line_height(),
        };
        fit(self.chunks(value, &mut color), width, height, &glyphs).unwrap_or_else(|| value.into())
    }
    fn progress(
        &mut self,
        sprite: &Sprite,
        at: [f32; 2],
        size: [f32; 2],
        rotation: f32,
        fill: Option<f32>,
    ) {
        // An unavailable value leaves the authored frame empty.
        let Some(fill) = fill else { return };
        let fill = fill.clamp(0.0, 1.0);
        // CProgressbarSprite builds its vertex buffer (14234A5E0,
        // progressbarsprite.cpp:163) from (0,0)..(w,h) plus (0,-1) in y-up
        // space on this renderer ((-0.5,-0.5) on the other branch), like the
        // masked flag sprite, whereas ordinary sprites (14234C9D0) add nothing.
        // Every progress bar is therefore drawn one pixel below its layout
        // position, turned with the quad. pairJ_naval_5.png: the naval progress
        // bar, both escape bars and the ship strength/organisation bars.
        let (sin, cos) = rotation.sin_cos();
        let at = [at[0] - sin, at[1] + cos];
        // progress.shader PixelTexture selects texture one at x <= CurrentState,
        // texture two otherwise. Native threshold-edge sampling still needs a
        // same-hour format-22 crop; the blend is not a crossfade.
        let bar = Bar {
            at,
            size,
            rotation,
            vertical: sprite.vertical,
        };
        if let Some(second) = &sprite.second {
            self.progress_segment(second, bar, [fill, 1.0], [fill, 1.0], [0.0, 1.0]);
        }
        if sprite.start_end {
            // progress_startend.shader: TextureOne is three stacked bands. The
            // filled part draws the middle band, the start band over it, then
            // the stop band shifted by 1 - state so its right end meets the
            // fill edge. pairJ_naval_5.png's escaping convoy (state 0.36147)
            // fits this band order at MAE 12.77 against 19.81 for the order a
            // vertically cancelled coordinate would select.
            for (band, u) in [
                ([1.0 / 3.0, 2.0 / 3.0], [0.0, fill]),
                ([0.0, 1.0 / 3.0], [0.0, fill]),
                ([2.0 / 3.0, 1.0], [1.0 - fill, 1.0]),
            ] {
                self.progress_segment(&sprite.sheet, bar, [0.0, fill], u, band);
            }
        } else {
            self.progress_segment(&sprite.sheet, bar, [0.0, fill], [0.0, fill], [0.0, 1.0]);
        }
    }
    /// Draws the `range` fraction of `bar` along its fill axis, sampling the
    /// frame's `u` fraction along that axis and its `band` across it.
    fn progress_segment(
        &mut self,
        sheet: &crate::map::sprites::SpriteSheet,
        bar: Bar,
        range: [f32; 2],
        u: [f32; 2],
        band: [f32; 2],
    ) {
        let Bar {
            at,
            size,
            rotation,
            vertical,
        } = bar;
        let fill = range[1] - range[0];
        if fill <= 0.0 {
            return;
        }
        let mut target = size;
        target[usize::from(vertical)] *= fill;
        let frame = sheet.frame_uv(0);
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let uv = if vertical {
            [
                lerp(frame[0], frame[2], band[0]),
                lerp(frame[1], frame[3], u[0]),
                lerp(frame[0], frame[2], band[1]),
                lerp(frame[1], frame[3], u[1]),
            ]
        } else {
            [
                lerp(frame[0], frame[2], u[0]),
                lerp(frame[1], frame[3], band[0]),
                lerp(frame[0], frame[2], u[1]),
                lerp(frame[1], frame[3], band[1]),
            ]
        };
        // Keep the cropped foreground's centre on the rotated original rectangle.
        let mut delta = [(target[0] - size[0]) * 0.5, (target[1] - size[1]) * 0.5];
        delta[usize::from(vertical)] += size[usize::from(vertical)] * range[0];
        let (sin, cos) = rotation.sin_cos();
        let center = [
            at[0] + size[0] * 0.5 + delta[0] * cos - delta[1] * sin,
            at[1] + size[1] * 0.5 + delta[0] * sin + delta[1] * cos,
        ];
        let first = self.drawn.batch.quads.len();
        self.drawn.batch.cell(
            sheet.texture,
            [center[0] - target[0] * 0.5, center[1] - target[1] * 0.5],
            target,
            uv,
            [1.0; 4],
        );
        self.drawn.batch.quads[first].rotation = rotation;
    }
}

#[derive(Clone, Copy)]
struct Bar {
    at: [f32; 2],
    size: [f32; 2],
    rotation: f32,
    vertical: bool,
}

/// The font operands the native text layout reads.
struct Glyphs<G, K> {
    /// Advance and whether the glyph ends a word; `None` without a glyph.
    glyph: G,
    kerning: K,
    line_height: f32,
}

/// One element of laid-out text: a character or an inline country flag.
#[derive(Clone, Debug, PartialEq)]
enum Unit {
    Char(char),
    Flag(String),
}

/// What a `fixedsize` box shows, as text for [`Painter::text`]; `None` keeps
/// the value unchanged (it fits, or carries a sprite this port does not lay out).
///
/// The box's CInstantTextObject (1422D7D10) lays the text out once through the
/// font (142286770 -> 142285B10) with its `maxWidth`, `maxHeight` and
/// truncation, and draws that result through a second pass (14228C8D0 ->
/// 142285B10) that wraps at `maxWidth` with no height limit and no truncation.
/// pairJ_land_5.png's Arial12 reserve names (maxWidth 74, maxHeight 16) show
/// "884 Infantry ...", "144th Infantr...", "5th Royal Ma...", "16th Royal M...",
/// "22 Dywizja ..." and, because that ellipsis overflows, "Iraqi" over
/// "al-Musha...".
fn fit<G, K>(chunks: Vec<Chunk>, width: f32, height: f32, glyphs: &Glyphs<G, K>) -> Option<String>
where
    G: Fn(char) -> Option<(f32, bool)>,
    K: Fn(char, char) -> f32,
{
    let mut units = Vec::new();
    for chunk in chunks {
        match chunk {
            Chunk::Text(run, _) => units.extend(run.chars().map(Unit::Char)),
            Chunk::Flag(key) => units.push(Unit::Flag(key)),
            Chunk::Icon(..) => return None,
        }
    }
    let laid = lay_out(&units, width, height, true, false, glyphs);
    let shown = lay_out(&laid, width, f32::INFINITY, false, true, glyphs);
    (shown != units).then(|| {
        let mut text = String::new();
        for unit in shown {
            match unit {
                Unit::Char(ch) => text.push(ch),
                Unit::Flag(key) => {
                    text.push(INLINE_FLAG);
                    text.push_str(&key);
                }
            }
        }
        text
    })
}

/// 142285B10. A glyph that does not end a word, or that starts one, and whose
/// advance plus kerning would reach `width` starts a new line (142283E90) while
/// two line heights plus the finished lines stay below `height`; otherwise the
/// layout stops before it. A word as long as its line breaks at that glyph.
/// With `truncate`, a stopped layout ends in an ellipsis (14228DE40): up to
/// three trailing characters become dots until the line's remaining width
/// holds three dot advances plus three pixels, then the missing dots are
/// appended, so the result can still overflow. `newlines` writes each line
/// break, dropping the trailing space the native line width excludes.
fn lay_out<G, K>(
    units: &[Unit],
    width: f32,
    height: f32,
    truncate: bool,
    newlines: bool,
    glyphs: &Glyphs<G, K>,
) -> Vec<Unit>
where
    G: Fn(char) -> Option<(f32, bool)>,
    K: Fn(char, char) -> f32,
{
    let room_for_line = |lines: f32| 2.0 * glyphs.line_height + lines < height;
    let mut out = Vec::new();
    let mut word = Vec::new();
    let (mut line, mut word_width, mut lines) = (0.0_f32, 0.0_f32, 0.0_f32);
    let mut previous = None;
    let mut stopped = false;
    for unit in units {
        let (step, ends_word) = match unit {
            Unit::Char('\n') => {
                if !room_for_line(lines) {
                    stopped = true;
                    break;
                }
                word.push(unit.clone());
                out.append(&mut word);
                (line, word_width, previous) = (0.0, 0.0, None);
                lines += glyphs.line_height;
                continue;
            }
            Unit::Char(ch) => match (glyphs.glyph)(*ch) {
                Some((advance, ends)) => {
                    let kerning = previous.map_or(0.0, |p| (glyphs.kerning)(p, *ch));
                    (advance + kerning, ends)
                }
                None => {
                    word.push(unit.clone());
                    previous = None;
                    continue;
                }
            },
            // An inline flag is a formatting code: no kerning and no word end.
            Unit::Flag(_) => (INLINE_FLAG_ADVANCE, false),
        };
        if (!ends_word || word_width == 0.0) && width <= line + step {
            if !room_for_line(lines) {
                stopped = true;
                break;
            }
            if newlines && line != word_width && out.last() == Some(&Unit::Char(' ')) {
                out.pop();
            }
            if line == word_width {
                out.append(&mut word);
                word_width = 0.0;
            }
            if newlines {
                out.push(Unit::Char('\n'));
            }
            out.append(&mut word);
            line = word_width;
            lines += glyphs.line_height;
        }
        word_width += step;
        line += step;
        word.push(unit.clone());
        if ends_word {
            out.append(&mut word);
            word_width = 0.0;
        }
        previous = match unit {
            Unit::Char(ch) => Some(*ch),
            Unit::Flag(_) => None,
        };
    }
    out.append(&mut word);
    if stopped && truncate && out.len() > 2 {
        let dot = (glyphs.glyph)('.').map_or(0.0, |(advance, _)| advance);
        let mut room = width - line;
        let mut need = (dot + 1.0) * 3.0 - (glyphs.kerning)('.', '.');
        let mut replaced = 0;
        while replaced < 3 && room < need {
            let index = out.len() - 1 - replaced;
            // UNVERIFIED for a flag: the native codes stored behind it.
            let advance = match &out[index] {
                Unit::Char(ch) => (glyphs.glyph)(*ch).map_or(0.0, |(advance, _)| advance),
                Unit::Flag(_) => 0.0,
            };
            room += advance - dot;
            need -= dot;
            out[index] = Unit::Char('.');
            replaced += 1;
        }
        out.extend(std::iter::repeat_n(Unit::Char('.'), 3 - replaced));
    }
    out
}

/// Where a `.gui` quarter-turned progress bar lands, as the unrotated quad and
/// the clockwise turn [`Batch::rotated`] applies about its centre.
///
/// `landcombat.gui` turns the 16x7 `GFX_combat_unit_bar_org/str` bars by
/// `rotation = 1.5708`. pairJ_land_5.png (every committed and reserve entry)
/// shows the game turning them **counter-clockwise**: texture one, which the
/// progress shader keeps at `x <= state`, fills from the bottom. The turned
/// 7x16 rectangle ends at the declared x and spans the declared y from -10 to
/// +6, `[x - h, y + h - w - 1, h, w]` as drawn (committed bars at (1031,681)
/// measure x 1024..1031, y 671..687; reserve bars at (915,852) x 908..915,
/// y 842..858). The returned quad excludes the progress quad's own offset.
/// The native pivot is UNVERIFIED; no other quarter-turned progress bar is
/// drawn by these windows. Half turns (the naval map icon) keep the centre.
fn progress_turn(at: [f32; 2], size: [f32; 2], rotation: f32) -> ([f32; 2], f32) {
    if (rotation - std::f32::consts::FRAC_PI_2).abs() > 1e-3 {
        return (at, rotation);
    }
    let [w, h] = size;
    // Before the progress quad's own one-pixel offset, which this turn carries
    // one pixel to the right.
    let turned = [at[0] - h - 1.0, at[1] + h - w - 1.0];
    let centre = [turned[0] + h * 0.5, turned[1] + w * 0.5];
    ([centre[0] - w * 0.5, centre[1] - h * 0.5], -rotation)
}

/// One axis of a cornered tile: (destination start, length, texel start,
/// texel length) for the leading edge, the centre pieces and the trailing edge.
fn cornered_spans(border: f32, texels: f32, size: f32, tiling: bool) -> Vec<[f32; 4]> {
    let edge = border.min(texels * 0.5).min(size * 0.5);
    let middle = (size - 2.0 * edge).max(0.0);
    let texel_middle = (texels - 2.0 * edge).max(0.0);
    let mut out = vec![[0.0, edge, 0.0, edge]];
    if tiling && texel_middle > 0.0 {
        let mut offset = 0.0;
        while offset < middle {
            let length = texel_middle.min(middle - offset);
            out.push([edge + offset, length, edge, length]);
            offset += length;
        }
    } else {
        out.push([edge, middle, edge, texel_middle]);
    }
    out.push([edge + middle, edge, texels - edge, edge]);
    out
}

fn clip_quads(batch: &mut Batch, first: usize, clip: [f32; 4]) {
    for quad in &mut batch.quads[first..] {
        let [x, y, w, h] = quad.rect;
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let left = x.max(clip[0]);
        let top = y.max(clip[1]);
        let right = (x + w).min(clip[0] + clip[2]);
        let bottom = (y + h).min(clip[1] + clip[3]);
        let [u0, v0, u1, v1] = quad.uv;
        quad.uv = [
            u0 + (u1 - u0) * (left - x) / w,
            v0 + (v1 - v0) * (top - y) / h,
            u0 + (u1 - u0) * (right - x) / w,
            v0 + (v1 - v0) * (bottom - y) / h,
        ];
        quad.rect = [left, top, (right - left).max(0.0), (bottom - top).max(0.0)];
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_combat_window_sorts_above_every_map_icon() {
        use crate::map::{counters, sprites::layer};
        // The highest priority a define can give a map icon.
        let top_icon = layer(i8::MAX, u32::MAX, crate::map::sprites::CHILD_SLOTS - 1);
        assert!(window_layer(0) > top_icon);
        // Window quads keep their push order among themselves.
        assert!(window_layer(1) > window_layer(0));
        assert!(window_layer(usize::MAX) >= window_layer(70_000));
        // Sorted as the sprite pass sorts, the window's quads come last.
        let icon = layer(counters::CounterDefines::VANILLA.land_combat_priority, 9, 3);
        let mut keys = vec![window_layer(3), icon, window_layer(0)];
        keys.sort();
        assert_eq!(keys, [icon, window_layer(0), window_layer(3)]);
    }

    use super::*;

    /// The tactics popup paints into its own batch, which is appended after
    /// the window's quads: with the window's quad count as its layer base,
    /// every popup quad sorts above every window quad, as the sprite pass
    /// sorts them (map::sprites, by layer).
    #[test]
    fn the_tactics_popup_draws_above_the_whole_window() {
        let mut window = Drawn::default();
        for texture in 0..5 {
            window
                .batch
                .at_layer(window_layer(window.batch.quads.len()));
            window
                .batch
                .cell(texture, [0.0, 0.0], [1.0, 1.0], [0.0; 4], [1.0; 4]);
        }
        let layer_base = window.batch.quads.len();
        let mut popup = Drawn::default();
        for texture in 100..103 {
            let layer = window_layer(layer_base + popup.batch.quads.len());
            popup.batch.at_layer(layer);
            popup
                .batch
                .cell(texture, [0.0, 0.0], [1.0, 1.0], [0.0; 4], [1.0; 4]);
        }
        window.batch.quads.extend(popup.batch.quads);
        let mut order: Vec<_> = window.batch.quads.iter().collect();
        order.sort_by_key(|quad| (quad.layer, quad.texture));
        let textures: Vec<_> = order.iter().map(|quad| quad.texture).collect();
        assert_eq!(textures, [0, 1, 2, 3, 4, 100, 101, 102]);
    }

    #[test]
    fn an_open_list_keeps_its_order_until_its_membership_changes() {
        let entry = |unit: u64, name: &str| {
            let mut values = Values::new();
            values.insert(
                "name".into(),
                Widget {
                    text: Some(name.into()),
                    ..Widget::default()
                },
            );
            (unit, values)
        };
        let shown = [entry(13, "old"), entry(6, "old"), entry(11, "old")];
        // Same members in a new array order: shown order, refreshed values.
        let mut next = vec![entry(11, "new"), entry(13, "new"), entry(6, "new")];
        keep_list_order(&mut next, &shown);
        assert_eq!(
            next.iter().map(|(unit, _)| *unit).collect::<Vec<_>>(),
            [13, 6, 11]
        );
        assert!(
            next.iter()
                .all(|(_, values)| values["name"].text.as_deref() == Some("new"))
        );
        // A replaced member, or a different count, rebuilds in array order.
        for mut next in [
            vec![entry(11, "new"), entry(13, "new"), entry(9, "new")],
            vec![entry(11, "new"), entry(13, "new")],
        ] {
            let order: Vec<_> = next.iter().map(|(unit, _)| *unit).collect();
            keep_list_order(&mut next, &shown);
            assert_eq!(
                next.iter().map(|(unit, _)| *unit).collect::<Vec<_>>(),
                order
            );
        }
    }

    #[test]
    fn cornered_tiles_keep_their_borders_and_repeat_a_tiling_centre() {
        // GFX_scrollbar_vertical_bg, 156 texels with 52-texel ends, at the
        // committed list's 165 px: one whole centre tile and 9 more texels.
        assert_eq!(
            cornered_spans(52.0, 156.0, 165.0, true),
            [
                [0.0, 52.0, 0.0, 52.0],
                [52.0, 52.0, 52.0, 52.0],
                [104.0, 9.0, 52.0, 9.0],
                [113.0, 52.0, 104.0, 52.0],
            ]
        );
        // GFX_scroll_track, 12 texels with 4-texel glow, stretched to 127 px.
        assert_eq!(
            cornered_spans(4.0, 12.0, 127.0, false),
            [
                [0.0, 4.0, 0.0, 4.0],
                [4.0, 119.0, 4.0, 4.0],
                [123.0, 4.0, 8.0, 4.0]
            ]
        );
        // A zero border leaves the whole texture as the centre.
        assert_eq!(
            cornered_spans(0.0, 18.0, 18.0, true),
            [
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 18.0, 0.0, 18.0],
                [18.0, 0.0, 18.0, 0.0]
            ]
        );
    }

    #[test]
    fn a_quarter_turned_bar_ends_at_its_declared_x_and_fills_upwards() {
        // pairJ_land_5.png reserve bar declared at (915, 852), 16x7.
        let authored: f32 = "1.5708".parse().unwrap();
        let (at, rotation) = progress_turn([915.0, 852.0], [16.0, 7.0], authored);
        assert_eq!(rotation, -authored);
        // The unrotated quad's centre is the turned rectangle's centre, one
        // pixel left of the drawn 908..915 before the progress quad's offset.
        assert_eq!([at[0] + 8.0, at[1] + 3.5], [910.5, 850.0]);
        // A half turn keeps its position and angle.
        assert_eq!(
            progress_turn([1.0, 2.0], [16.0, 7.0], std::f32::consts::PI),
            ([1.0, 2.0], std::f32::consts::PI)
        );
    }

    #[test]
    fn a_fixed_size_box_truncates_then_wraps_like_the_game() {
        // Arial12 advances and kerning pairs of the pairJ_land_5.png reserve
        // names (maxWidth 74, maxHeight 16, lineHeight 14).
        let glyphs = Glyphs {
            glyph: |ch: char| {
                let advance = match ch {
                    'I' | 'i' | 'j' | 'l' => 2.0,
                    ' ' | '.' | 't' => 3.0,
                    '-' | 'f' | 'r' => 4.0,
                    'k' => 5.0,
                    'D' | 'R' | 'S' => 7.0,
                    'G' | 'M' => 8.0,
                    'w' => 10.0,
                    _ => 6.0,
                };
                Some((advance, ch == ' '))
            },
            kerning: |a: char, b: char| match (a, b) {
                ('y' | 'r', '.') | ('1', '1') => -1.0,
                _ => 0.0,
            },
            line_height: 14.0,
        };
        let shown = |run: &str| {
            let chunks = vec![Chunk::Text(run.into(), [1.0; 4])];
            fit(chunks, 74.0, 16.0, &glyphs)
        };
        for (name, game) in [
            ("884 Infantry Division", "884 Infantry ..."),
            ("144th Infantry Division", "144th Infantr..."),
            ("5th Royal Marines Division", "5th Royal Ma..."),
            ("16th Royal Marines Division", "16th Royal M..."),
            ("22 Dywizja Piechoty Górskiej", "22 Dywizja ..."),
            // The ellipsis overflows the box, so the drawing pass wraps it.
            ("Iraqi al-Mushati al-16r", "Iraqi\nal-Musha..."),
        ] {
            assert_eq!(shown(name).as_deref(), Some(game), "{name}");
        }
        assert_eq!(shown("Short"), None);
        // With room for a second line, the layout wraps instead of cutting.
        let chunks = vec![Chunk::Text("Iraqi al-Mushati al-16r".into(), [1.0; 4])];
        assert_eq!(
            fit(chunks, 74.0, 29.0, &glyphs).as_deref(),
            Some("Iraqi al-Mushati\nal-16r")
        );
        // A flag that would reach the edge stops the layout before it.
        let chunks = vec![
            Chunk::Text("Name that is long enough ".into(), [1.0; 4]),
            Chunk::Flag("SOV".into()),
        ];
        assert_eq!(
            fit(chunks, 130.0, 16.0, &glyphs).as_deref(),
            Some("Name that is long enough...")
        );
    }

    #[test]
    fn clipped_hover_regions_keep_order_and_share_their_text() {
        let text: HoverText = "shared".into();
        let hit = |x: f32| TooltipHit {
            rect: [x, 0.0, 10.0, 10.0],
            text: Some(text.clone()),
            description: None,
        };
        let mut tooltips = vec![hit(-100.0), hit(0.0), hit(50.0), hit(5.0), hit(200.0)];
        clip_tooltips(&mut tooltips, 1, [0.0, 0.0, 60.0, 10.0]);
        // The first region is before `first` and stays untouched.
        let rects: Vec<_> = tooltips.iter().map(|hit| hit.rect).collect();
        assert_eq!(
            rects,
            [
                [-100.0, 0.0, 10.0, 10.0],
                [0.0, 0.0, 10.0, 10.0],
                [50.0, 0.0, 10.0, 10.0],
                [5.0, 0.0, 10.0, 10.0],
            ]
        );
        // Every region points at the one bound string.
        assert_eq!(std::sync::Arc::strong_count(&text), 5);
    }
}
