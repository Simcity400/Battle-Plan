//! Game-authored land and naval combat windows, shared by playback and PNG export.
mod assets;
mod battle_name;
mod binding;
mod border_marker;
mod commander_tooltips;
mod defines;
mod details;
mod division_icons;
mod division_tooltips;
pub(crate) mod engaged;
mod fixed;
mod gui;
mod gui_resources;
mod hq;
pub(crate) mod icon_frames;
mod keyed;
pub(crate) mod land_details;
pub(crate) mod land_marker_tooltips;
mod land_terrain;
pub(crate) mod modifier_text;
mod naval_air_groups;
mod naval_air_tooltips;
mod naval_anti_air;
pub(crate) mod naval_check;
mod naval_details;
mod naval_environment;
mod naval_escape;
pub(crate) mod naval_marker_tooltips;
mod naval_positioning;
mod naval_screening;
mod naval_ship_tooltips;
pub(crate) mod naval_sides;
mod naval_summary_tooltips;
mod naval_surface_detection;
mod naval_terrain;
mod presentation;
pub(crate) mod progress;
mod reinforcement_speed;
mod tactics;
mod template_icons;
pub(crate) mod text;
pub(crate) mod texture;
pub(crate) mod tooltip_painter;
mod tooltips;

use crate::{document::Document, map::sprites::Batch};
pub use assets::Assets;
pub(crate) use assets::CombatContent;
pub(crate) use battle_name::battle_name_candidates;
use binding::Bindings;
/// The one screen-rectangle hit test; the map's counter hits share it.
pub(crate) fn contains(rect: [f32; 4], point: [f32; 2]) -> bool {
    point[0] >= rect[0]
        && point[1] >= rect[1]
        && point[0] < rect[0] + rect[2]
        && point[1] < rect[1] + rect[3]
}
use std::{collections::BTreeMap, io};

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
        contains(self.rect, point)
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
    /// Viewer playback state for the in-game idler PAUSED virtual (140DC3DB0).
    playing: bool,
    pub selection: Option<Selection>,
    pub offset: [f32; 2],
    pub dragging: bool,
    /// The possible-tactics popup (1415C7D20): window presentation state,
    /// never recorded.
    pub tactics_open: bool,
    tactics_position: Option<[f32; 2]>,
    dragging_tactics: bool,
    hover: tooltip_painter::HoverState,
    cached: Option<Cached>,
    gui: gui::Windows,
    gui_time: Option<f32>,
    refused: BTreeMap<String, usize>,
}

/// Where a window's contents were read: the branch and hour, the combat, the
/// perspective choice and the viewing country it resolved to there.
#[derive(Clone, Debug, PartialEq)]
struct Scope {
    branch: replay::BranchId,
    hour: i32,
    selection: Selection,
    perspective: crate::map::perspective::Choice,
    viewer: u32,
    playing: bool,
}

impl Scope {
    /// Whether contents read at `self` are the window's last refresh at
    /// `now`: the same combat on the same branch, seen by the same viewing
    /// country under the same choice, no later than `now`. Another branch's
    /// contents, or another resolved viewer's, are not this window's.
    fn holds_at(&self, now: &Scope) -> bool {
        self.branch == now.branch
            && self.selection == now.selection
            && self.perspective == now.perspective
            && self.viewer == now.viewer
            && self.playing == now.playing
            && self.hour <= now.hour
    }
}

/// A window's contents and where they were read.
type Contents = (Scope, Bindings);

/// The window's state at one (branch, hour, selection, perspective).
struct Cached {
    /// The hour drawn, and its resolved viewer.
    at: Scope,
    /// Whether the engaged index covered that hour.
    covered: bool,
    /// The contents and where they were read: `at` itself after a refresh, an
    /// earlier hour while held. `None` when the window never refreshed.
    contents: Option<Contents>,
    /// Whether the window shows.
    shown: bool,
    /// Why the window holds contents it may not show natively, reported.
    held: Option<String>,
    /// SetCombat/refresh serial; holds keep the bound generation (1415CA230).
    generation: u64,
    gui_assets: Option<gui_resources::GuiAssets>,
}

/// The hour's engaged flags (1413CD880) for the viewer, of every land and
/// naval combat.
fn combat_flags(
    frame: &replay::frame::DecodedFrame,
    ours: u32,
    engaged: Option<&crate::engaged_index::EngagedIndex>,
) -> io::Result<crate::engaged_index::CombatFlags> {
    use crate::map::tables::rows;
    use replay::rows::tables;
    let links: BTreeMap<u32, u32> = rows::<tables::CountryLinks>(frame)?
        .rows()
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let relations = crate::map::visibility::Relations::read(frame, &links);
    // A load-rebuilt flag's provenance is named with the hour's marker lines
    // (CounterReport::combat_limits), which cover this combat too.
    crate::engaged_index::CombatFlags::read(
        frame,
        ours,
        &links,
        relations.as_ref().map_err(|reason| *reason),
        engaged,
    )
}

/// What the window does at an hour, from its combat's flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Update {
    /// Rebuild its contents; `false` hides it.
    Refresh(bool),
    /// Keep the contents of the last hour it refreshed, or do not open.
    /// `undecided` says why the hour cannot tell whether the combat is
    /// engaged, which is reported.
    Hold {
        combat: u64,
        undecided: Option<&'static str>,
    },
}

/// 1415C8950 hides an active land window when 1413CD880 finds neither side
/// engaged; an undecided land flag keeps the window, whose marker reports the
/// gap. A naval window opens only for an engaged combat (1418E3B40) and
/// 1417DA2F0 refreshes it only while the combat stays engaged, both on
/// 1413CD880's positive answer, so every other hour holds the last engaged
/// contents (owner decision, 2026-09-27): a disengaged one silently, an
/// undecided or absent one reported.
fn update(selection: Selection, flags: &crate::engaged_index::CombatFlags) -> Update {
    use engaged::Engaged;
    let flag = |flags: &BTreeMap<u64, Engaged>, id| flags.get(&id).copied();
    match selection {
        Selection::Land(id) => Update::Refresh(flag(&flags.land, id) != Some(Engaged::No)),
        Selection::Naval(combat) => {
            let undecided = match flag(&flags.naval, combat) {
                Some(Engaged::Yes) => return Update::Refresh(true),
                Some(Engaged::No) => None,
                Some(Engaged::Ambiguous(cause)) => Some(cause.reason()),
                Some(Engaged::Unknown) => Some("its engaged flag is unknown"),
                None => Some("it is absent"),
            };
            Update::Hold { combat, undecided }
        }
        Selection::NavalResult(_) => Update::Refresh(true),
    }
}

/// The contents a holding window keeps at `now` and the note it reports;
/// `None` when it has nothing to keep and nothing to report, so it closes.
/// Only contents read under the same [`Scope::holds_at`] are kept.
fn hold(
    previous: Option<Contents>,
    now: &Scope,
    combat: u64,
    undecided: Option<&'static str>,
) -> Option<(Option<Contents>, Option<String>)> {
    let contents = previous.filter(|(read, _)| read.holds_at(now));
    if contents.is_none() && undecided.is_none() {
        return None;
    }
    let note = undecided.map(|reason| match &contents {
        Some((read, _)) => format!(
            "Naval combat {combat}: at hour {} {reason}; the window keeps the contents it last \
             refreshed while engaged, at hour {}.",
            now.hour, read.hour
        ),
        None => format!(
            "Naval combat {combat}: at hour {} {reason}; the window opens only while the combat \
             is engaged, so it shows nothing.",
            now.hour
        ),
    });
    Some((contents, note))
}

/// A map hover: its title, delayed body, screen rectangle and province.
pub type HoverHit<'a> = (&'a str, Option<&'a str>, [f32; 4], Option<u32>);

impl Window {
    /// The native speed-above-3 night picture asks whether the UI is paused
    /// (140B387D0 -> CInGameIdler slot +0x2E8, 140DC3DB0).
    pub(crate) fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }
    /// What the open window cannot show exactly: its bindings' gaps and the
    /// combat artwork's. Built on request, never per frame; empty when no
    /// window is open.
    pub fn warnings(&self, document: &Document) -> Vec<String> {
        let Some(cache) = self.cached.as_ref().filter(|_| self.selection.is_some()) else {
            return Vec::new();
        };
        let mut warnings: Vec<String> = cache.held.iter().cloned().collect();
        if let Some((_, bindings)) = &cache.contents {
            warnings.extend(bindings.warnings.iter().cloned());
        }
        warnings.extend(document.icons.combat.warnings.iter().cloned());
        warnings.extend(
            self.refused
                .iter()
                .map(|(reason, count)| format!("{reason} ({count} times)")),
        );
        warnings
    }
    pub(crate) fn gui_active(&self) -> bool {
        self.cached.as_ref().is_some_and(|cached| cached.shown) && self.gui.active()
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
            .is_some_and(|rect| contains(rect, point))
        {
            self.tactics_open = false;
            return true;
        }
        if drawn.tactics_rect.is_some_and(|rect| contains(rect, point)) {
            return true;
        }
        if drawn
            .tactics_button
            .is_some_and(|rect| contains(rect, point))
        {
            self.tactics_open = !self.tactics_open;
            self.tactics_position = None;
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
            contains(rect, point)
        };
        if drawn.tactics_rect.is_some_and(title) {
            self.dragging = true;
            self.dragging_tactics = true;
        } else if !drawn.tactics_rect.is_some_and(|rect| contains(rect, point)) && title(drawn.rect)
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
    pub(crate) fn scroll(&mut self, target: ScrollTarget, direction: i32, wheel: bool) {
        self.gui.scroll(target, direction, wheel);
    }
    /// The topmost combat hover, otherwise the map's, on the native tooltip clock.
    pub(crate) fn tooltip(
        &mut self,
        drawn: Option<&Drawn>,
        assets: &Assets,
        point: Option<[f32; 2]>,
        map: Option<HoverHit<'_>>,
    ) -> Option<(&str, tooltip_painter::TooltipPosition)> {
        let defines = assets.tooltip_defines.as_ref().ok()?;
        let hit = point
            .and_then(|point| {
                drawn?
                    .tooltips
                    .iter()
                    .rev()
                    .find(|hit| contains(hit.rect, point))
            })
            .map(|hit| {
                (
                    hit.text.as_deref().unwrap_or_default(),
                    hit.description.as_deref(),
                    hit.rect,
                    None,
                    (hit.anchor == TooltipAnchor::NavalWindow)
                        .then(|| drawn.expect("combat hover has a drawn window").rect),
                )
            })
            .or_else(|| map.map(|(text, body, rect, province)| (text, body, rect, province, None)));
        self.hover
            .bind_province(hit.and_then(|(_, _, _, province, _)| province));
        if let Some((text, delayed, _, _, window)) = hit
            && (!text.is_empty() || delayed.is_some_and(|text| !text.is_empty()))
        {
            self.hover.anchor =
                window
                    .zip(point)
                    .map(|(window, point)| tooltip_painter::TooltipPosition {
                        point,
                        naval_window: Some(window),
                    });
        }
        // Explicit anchors retain their position while the hide timer runs (1402A5CFF).
        let anchor = self.hover.anchor;
        self.hover
            .shown(
                hit.map(|(text, delayed, rect, _, _)| (text, delayed, rect)),
                point,
                std::time::Instant::now(),
                defines.timing,
            )
            .map(|(text, point)| {
                (
                    text,
                    anchor.unwrap_or(tooltip_painter::TooltipPosition {
                        point,
                        naval_window: None,
                    }),
                )
            })
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
    /// [`Self::tooltip`] needs, so a frame that lays the window out
    /// twice (once for its hit tests) does the hover work once.
    /// Hit preparation does not advance; drawing ticks once after input (142247750).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        document: &Document,
        extent: [f32; 2],
        perspective: &crate::map::perspective::Choice,
        engaged: Option<&crate::engaged_index::EngagedIndex>,
        hovers: bool,
        time: f32,
        advance: bool,
    ) -> io::Result<Option<Drawn>> {
        let Some(selection) = self.selection else {
            return Ok(None);
        };
        let branch = document.playback.branch();
        let hour = document.playback.hour();
        let covered = engaged.is_some_and(|index| index.covers(branch, hour));
        if !self.cached.as_ref().is_some_and(|cached| {
            (
                cached.at.branch,
                cached.at.hour,
                cached.at.selection,
                cached.covered,
            ) == (branch, hour, selection, covered)
                && cached.at.perspective == *perspective
                && cached.at.playing == self.playing
        }) {
            let vision = document.vision(perspective)?;
            let now = Scope {
                branch,
                hour,
                selection,
                perspective: perspective.clone(),
                viewer: vision.ours(),
                playing: self.playing,
            };
            let flags = combat_flags(document.frame()?, vision.ours(), engaged)?;
            let cached = match update(selection, &flags) {
                Update::Hold { combat, undecided } => {
                    let (previous, gui_assets, generation) =
                        self.cached.take().map_or((None, None, 0), |cached| {
                            (cached.contents, cached.gui_assets, cached.generation)
                        });
                    let Some((contents, held)) = hold(previous, &now, combat, undecided) else {
                        // Never engaged for this viewer on this branch: it does not open.
                        self.close();
                        return Ok(None);
                    };
                    Cached {
                        at: now,
                        covered,
                        shown: contents.is_some(),
                        contents,
                        held,
                        generation,
                        gui_assets,
                    }
                }
                Update::Refresh(shown) => {
                    let generation = self
                        .cached
                        .as_ref()
                        .map_or(1, |cached| cached.generation.wrapping_add(1));
                    let mut bindings =
                        Bindings::read(document, selection, &vision, &flags, !self.playing)?;
                    if let Some((read, previous)) = self
                        .cached
                        .as_ref()
                        .and_then(|cached| cached.contents.as_ref())
                        && read.selection == selection
                    {
                        for (next, shown) in bindings.entries.iter_mut().zip(&previous.entries) {
                            keep_list_order(next, shown);
                        }
                    }
                    let gui_assets =
                        gui_resources::GuiAssets::new(&document.icons.combat, &bindings);
                    Cached {
                        at: now.clone(),
                        covered,
                        contents: Some((now, bindings)),
                        shown,
                        held: None,
                        generation,
                        gui_assets: Some(gui_assets),
                    }
                }
            };
            self.cached = Some(cached);
        }
        let cached = self.cached.as_mut().expect("bound above");
        let Some((_, bindings)) = cached.contents.as_ref().filter(|_| cached.shown) else {
            self.gui_time = None;
            self.refused.clear();
            return Ok(None);
        };
        let assets = &document.icons.combat;
        let counters = &document.icons.counters;
        let windows = counters.windows_at(extent.map(|value| value as u32))?;
        let inventory = cached.gui_assets.as_mut().expect("refreshed with bindings");
        inventory.refresh_portraits(assets, time);
        let resources = gui_resources::GuiResources::new(
            assets,
            &windows,
            &document.names.localisation,
            &counters.flags,
            &counters.large_flags,
            inventory,
        );
        let tick = crate::map::native_gui::lifecycle::Tick {
            time,
            dt: self
                .gui_time
                .map_or(0.0, |previous| (time - previous).max(0.0)),
        };
        if advance {
            self.gui_time = Some(time);
        }
        let drawn = self.gui.draw(
            &windows,
            &resources,
            bindings,
            cached.generation,
            selection,
            self.offset,
            self.tactics_open,
            &mut self.tactics_position,
            extent,
            hovers,
            tick,
            advance,
        )?;
        self.refused.clone_from(&drawn.refused);
        Ok(Some(drawn))
    }
}

#[derive(Default)]
pub struct Drawn {
    pub batch: Batch,
    refused: BTreeMap<String, usize>,
    pub rect: [f32; 4],
    pub close: Option<[f32; 4]>,
    lists: Vec<([f32; 4], ScrollTarget)>,
    scroll_buttons: Vec<([f32; 4], ScrollTarget, i32)>,
    /// Hover regions in draw order; the last containing one wins.
    tooltips: Vec<TooltipHit>,
    tactics_button: Option<[f32; 4]>,
    tactics_close: Option<[f32; 4]>,
    tactics_rect: Option<[f32; 4]>,
}

#[derive(Clone)]
pub(crate) struct ScrollTarget {
    popup: bool,
    path: String,
    vertical: bool,
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
    anchor: TooltipAnchor,
}

/// Producer-specific placement; only CFexShipIcon's group branch supplies a
/// naval window anchor (1417D5260). Pride, sunk ships and air keep their own default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TooltipAnchor {
    #[default]
    Pointer,
    NavalWindow,
}

impl Drawn {
    fn in_popup(&self, point: [f32; 2]) -> bool {
        self.tactics_rect.is_some_and(|rect| contains(rect, point))
    }
    pub fn contains(&self, point: [f32; 2]) -> bool {
        contains(self.rect, point) || self.in_popup(point)
    }
    pub fn closes(&self, point: [f32; 2]) -> bool {
        !self.in_popup(point) && self.close.is_some_and(|rect| contains(rect, point))
    }
    /// The list under `point`; the popup covers the window's lists.
    pub(crate) fn list_at(&self, point: [f32; 2]) -> Option<ScrollTarget> {
        self.lists
            .iter()
            .rev()
            .find(|(rect, target)| target.popup == self.in_popup(point) && contains(*rect, point))
            .map(|(_, target)| target.clone())
    }
    pub(crate) fn scroll_action(&self, point: [f32; 2]) -> Option<(ScrollTarget, i32)> {
        self.scroll_buttons
            .iter()
            .rev()
            .find(|(rect, target, _)| {
                contains(*rect, point) && target.popup == self.in_popup(point)
            })
            .map(|(_, target, direction)| (target.clone(), *direction))
    }
}

#[derive(Clone, Default)]
pub struct Widget {
    pub text: Option<String>,
    /// Hover text, and the delayed body shown after its longer delay.
    pub tooltip: Option<HoverText>,
    pub tooltip_description: Option<HoverText>,
    pub tooltip_anchor: TooltipAnchor,
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
    /// Runtime button state; absent restores the GUI's authored enabled state.
    pub enabled: Option<bool>,
    pub hidden: bool,
    pub size: Option<[f32; 2]>,
    pub offset: [f32; 2],
    /// Repeated authored entries, used by reserve, trait and modifier grids.
    pub entry_template: Option<String>,
    /// Sparse per-entry template overrides, such as the native exile badge.
    pub entry_templates: BTreeMap<usize, String>,
    pub entries: Vec<Values>,
}
pub type Values = BTreeMap<String, Widget>;
/// The viewer's inline flag: this marker, then the resolved flag-atlas key.
/// Native text presentation resolves its complete key. The game writes 0x12 and a country tag
/// (CCombatantEntry 1415BA240); the viewer resolves the tag first and marks it
/// with a byte the localisation loader never makes a control (142386AD0), so
/// an authored `@` (0x12) inside localised text around it is still refused
/// by name ([`binding::displayable`]), in combat windows and map hovers alike.
pub(crate) const INLINE_FLAG: char = crate::map::native_text::RESOLVED_FLAG;
/// The layer of a window quad pushed after `quads` others: the window draws
/// in push order, one icon ordinal per quad, in the GUI band above every
/// map-icon priority, so the open window covers the icons behind it.
fn window_layer(quads: usize) -> u64 {
    crate::map::sprites::gui_layer(u32::try_from(quads).unwrap_or(u32::MAX), 0)
}

#[cfg(test)]
mod tests {
    /// A land window hides for an unengaged combat and refreshes on an undecided flag; a naval
    /// one refreshes only while engaged and holds otherwise, reporting an undecided or absent
    /// combat.
    #[test]
    fn the_window_updates_only_while_its_combat_is_engaged() {
        use super::{Selection, Update, engaged::Engaged, update};
        use std::collections::BTreeMap;
        let flags = |flag: Engaged| crate::engaged_index::CombatFlags {
            land: BTreeMap::from([(1, flag)]),
            naval: BTreeMap::from([(2, flag)]),
            ..Default::default()
        };
        assert_eq!(
            update(Selection::Land(1), &flags(Engaged::No)),
            Update::Refresh(false)
        );
        for flag in [
            Engaged::Yes,
            Engaged::Unknown,
            Engaged::Ambiguous(engaged::Ambiguity::Scope),
        ] {
            assert_eq!(
                update(Selection::Land(1), &flags(flag)),
                Update::Refresh(true)
            );
        }
        assert_eq!(
            update(Selection::Naval(2), &flags(Engaged::Yes)),
            Update::Refresh(true)
        );
        let held = |selection, flag| match update(selection, &flags(flag)) {
            Update::Hold { combat, undecided } => Some((combat, undecided)),
            Update::Refresh(_) => None,
        };
        assert_eq!(held(Selection::Naval(2), Engaged::No), Some((2, None)));
        assert_eq!(
            held(Selection::Naval(2), Engaged::Unknown),
            Some((2, Some("its engaged flag is unknown")))
        );
        assert_eq!(
            held(
                Selection::Naval(2),
                Engaged::Ambiguous(engaged::Ambiguity::Scope)
            ),
            Some((2, Some(engaged::Ambiguity::Scope.reason())))
        );
        // A combat the hour no longer holds is not engaged either.
        assert_eq!(
            held(Selection::Naval(3), Engaged::Yes),
            Some((3, Some("it is absent")))
        );
        assert_eq!(
            update(Selection::NavalResult(4), &flags(Engaged::No)),
            Update::Refresh(true)
        );
    }

    fn scope(branch: u64, hour: i32, viewer: u32) -> super::Scope {
        super::Scope {
            branch,
            hour,
            selection: super::Selection::Naval(2),
            perspective: crate::map::perspective::Choice::default(),
            viewer,
            playing: false,
        }
    }

    /// Held contents are kept only for the branch, combat, choice and resolved viewing
    /// country they were read under, and never from a later hour; an undecided hour reports
    /// what it kept, and one with nothing to keep still reports and shows nothing.
    #[test]
    fn a_held_window_keeps_only_its_own_branch_and_viewers_contents() {
        use super::{Bindings, hold};
        let read = || Some((scope(0, 10, 1), Bindings::default()));
        let kept = |now: &super::Scope, undecided| {
            hold(read(), now, 2, undecided)
                .map(|(contents, note)| (contents.map(|(read, _)| read.hour), note))
        };
        // A disengaged hour keeps them silently.
        assert_eq!(kept(&scope(0, 12, 1), None), Some((Some(10), None)));
        // Another branch holding the same combat handle, or the recorded player resolving to
        // another country under the same choice: not this window's contents, so it closes.
        assert_eq!(kept(&scope(1, 12, 1), None), None);
        assert_eq!(kept(&scope(0, 12, 7), None), None);
        let mut fogless = scope(0, 12, 1);
        fogless.perspective.fog = false;
        assert_eq!(kept(&fogless, None), None);
        // A seek back before the read.
        assert_eq!(kept(&scope(0, 9, 1), None), None);
        // An undecided flag keeps and reports; without contents it only reports.
        assert_eq!(
            kept(&scope(0, 12, 1), Some("its engaged flag is unknown")),
            Some((
                Some(10),
                Some(
                    "Naval combat 2: at hour 12 its engaged flag is unknown; the window keeps the \
                     contents it last refreshed while engaged, at hour 10."
                        .into()
                )
            ))
        );
        assert_eq!(
            kept(&scope(1, 12, 1), Some("it is absent")),
            Some((
                None,
                Some(
                    "Naval combat 2: at hour 12 it is absent; the window opens only while the \
                     combat is engaged, so it shows nothing."
                        .into()
                )
            ))
        );
        assert!(hold(None, &scope(0, 12, 1), 2, None).is_none());
    }

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
}
