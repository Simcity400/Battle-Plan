//! Game-authored land and naval combat windows, shared by playback and PNG export.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::time::Duration;
use std::time::Instant;
#[cfg(test)]
use viewer_foundation::engaged::Ambiguity;
use viewer_foundation::engagement_history::CombatFlags;
use viewer_foundation::engagement_history::History;
use viewer_foundation::map::native_gui::lifecycle;
use viewer_foundation::map::native_text::RESOLVED_FLAG;
use viewer_foundation::map::perspective::Choice;
use viewer_foundation::map::perspective::cached_vision;
use viewer_foundation::map::sprites::gui_layer;
use viewer_foundation::map::visibility::FogContent;
use viewer_foundation::map::visibility::Relations;
use viewer_foundation::tooltip_painter::HoverState;
use viewer_foundation::tooltip_painter::TooltipDefines;
use viewer_foundation::tooltip_painter::TooltipPosition;
mod assets;
pub(crate) mod availability;
mod battle_name;
pub(crate) mod binding;
mod border_marker;
mod commander_tooltips;
mod content;
mod defines;
pub(crate) mod details;
mod division_icons;
mod division_tooltips;
mod fixed;
mod gui;
mod gui_resources;
mod hq;
mod keyed;
pub(crate) mod land_details;
pub mod land_marker_tooltips;
mod land_terrain;
pub mod modifier_text;
mod naval_air_groups;
mod naval_air_tooltips;
mod naval_anti_air;
mod naval_details;
mod naval_environment;
mod naval_escape;
pub mod naval_labels;
pub mod naval_marker_tooltips;
mod naval_positioning;
mod naval_results;
mod naval_screening;
mod naval_ship_tooltips;
mod naval_summary_tooltips;
mod naval_surface_detection;
pub(crate) mod naval_terrain;
mod naval_trees;
mod naval_variants;
pub mod picker;
mod presentation;
mod tactics;
mod template_icons;
pub mod terrain_mode;
mod tooltips;
pub use content::Content;
pub use division_tooltips::{DivisionPanelCache, division_panel};

pub use assets::Assets;
pub use assets::CombatContent;
use binding::Bindings;
use std::{collections::BTreeMap, io, sync::Arc};
#[cfg(test)]
use viewer_foundation::combat_selection::Hit;
use viewer_foundation::combat_selection::Selection;
use viewer_foundation::map::gui_resources::contains;
use {self::content::Content as StaticContent, viewer_foundation::map::sprites::Batch};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selector {
    Id(u64),
    Province(u32),
}

impl Selector {
    pub fn resolve(self, frame: &replay_frame::frame::DecodedFrame) -> io::Result<Selection> {
        use replay_data::rows::tables;
        use viewer_foundation::map::tables::rows;
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
            // Map icons also open completed results (readiness zero), not just pending ones.
            let results = rows::<tables::NavalResults>(frame)?;
            let newest = results
                .rows()
                .iter()
                .filter(|row| {
                    row.result_hour <= frame.hour
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

/// An open land window refills its lists only when membership changes.
/// If the existing entries match the committed or reserve array count and each
/// unit remains in the combat and in that array, refresh values and preserve order.
/// Otherwise clear the grid and rebuild in array order. Construction and an
/// interface-setting change force a rebuild; a newly opened window starts empty.
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
    /// Viewer playback pause state.
    playing: bool,
    pub selection: Option<Selection>,
    pub offset: [f32; 2],
    pub dragging: bool,
    /// The possible-tactics popup: window presentation state,
    /// never recorded.
    pub tactics_open: bool,
    /// retained-result detail panels are local UI state.
    result_details_open: bool,
    tactics_position: Option<[f32; 2]>,
    dragging_tactics: bool,
    hover: HoverState,
    cached: Option<Cached>,
    gui: gui::Windows,
    gui_time: Option<f32>,
    refused: BTreeMap<String, usize>,
}

/// Where a window's contents were read: the branch and hour, the combat, the
/// perspective choice and the viewing country it resolved to there.
#[derive(Clone, Debug, PartialEq)]
struct Scope {
    branch: replay_data::BranchId,
    hour: i32,
    selection: Selection,
    perspective: Choice,
    viewer: u32,
    playing: bool,
    nato: bool,
}

impl Scope {
    fn matches_request(
        &self,
        branch: replay_data::BranchId,
        hour: i32,
        selection: Selection,
        perspective: &Choice,
        playing: bool,
        nato: bool,
    ) -> bool {
        self.branch == branch
            && self.hour == hour
            && self.selection == selection
            && self.perspective == *perspective
            && self.playing == playing
            && self.nato == nato
    }

    /// Whether contents read at `self` are the window's last refresh at
    /// `now`: the same combat on the same branch, seen by the same viewing
    /// country under the same choice, no later than `now`. Another branch's
    /// contents, or another resolved viewer's, are not this window's.
    fn holds_at(&self, now: &Scope) -> bool {
        // Only land tactics have scripted unit-bonus icons, and land windows
        // refresh at `now`. A local icon or playback toggle does not end a naval hold.
        self.branch == now.branch
            && self.selection == now.selection
            && self.perspective == now.perspective
            && self.viewer == now.viewer
            && self.hour <= now.hour
    }

    fn result_ui_at(&self, now: &Scope) -> bool {
        self.branch == now.branch
            && self.selection == now.selection
            && self.perspective == now.perspective
            && self.viewer == now.viewer
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
    /// combat selection/refresh serial; holds keep the bound generation.
    generation: u64,
    gui_assets: Option<gui_resources::GuiAssets>,
}

/// The hour's engaged flags for the viewer, of every land and
/// naval combat.
fn combat_flags(
    frame: &replay_frame::frame::DecodedFrame,
    ours: u32,
    engaged: Option<&History>,
) -> io::Result<CombatFlags> {
    use replay_data::rows::tables;
    use viewer_foundation::map::tables::rows;
    let links: BTreeMap<u32, u32> = rows::<tables::CountryLinks>(frame)?
        .rows()
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let relations = Relations::read(frame, &links);
    // A load-rebuilt flag's provenance is named with the hour's marker lines
    // (CounterReport::combat_limits), which cover this combat too.
    CombatFlags::read(
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
    Close,
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

/// An active land window hides when neither side is engaged; an undecided
/// flag keeps it open while the marker reports the gap. A naval window opens
/// and refreshes only while engaged. Other hours hide its retained contents.
fn update(selection: Selection, flags: &CombatFlags) -> Update {
    use viewer_foundation::engaged::Engaged;
    let flag = |flags: &BTreeMap<u64, Engaged>, id| flags.get(&id).copied();
    match selection {
        Selection::Land(id) => Update::Refresh(flag(&flags.land, id) != Some(Engaged::No)),
        Selection::Naval(combat) => {
            let undecided = match flag(&flags.naval, combat) {
                Some(Engaged::Yes) => return Update::Refresh(true),
                Some(Engaged::No) => return Update::Close,
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
    /// The speed-above-3 night picture depends on whether playback is paused.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }
    /// What the open window cannot show exactly: its bindings' gaps and the
    /// combat artwork's. Built only for diagnostics and exports; empty when no
    /// window is open.
    pub fn warnings(&self, artwork: &[String]) -> Vec<String> {
        let Some(cache) = self.cached.as_ref().filter(|_| self.selection.is_some()) else {
            return Vec::new();
        };
        let mut warnings: Vec<String> = cache.held.iter().cloned().collect();
        if let Some((_, bindings)) = &cache.contents {
            warnings.extend(bindings.warnings.iter().cloned());
            for (reason, &count) in bindings.refused() {
                warnings.push(match count {
                    1 => reason.clone(),
                    count => format!("{reason} ({count} times)"),
                });
            }
        }
        warnings.extend(artwork.iter().cloned());
        warnings.extend(
            self.refused
                .iter()
                .map(|(reason, count)| format!("{reason} ({count} times)")),
        );
        warnings
    }
    pub fn gui_active(&self) -> bool {
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
        if matches!(self.selection, Some(Selection::NavalResult(_)))
            && drawn
                .result_details_button
                .is_some_and(|rect| contains(rect, point))
        {
            let Some(cache) = self.cached.as_mut() else {
                return false;
            };
            let Some((_, bindings)) = cache.contents.as_mut() else {
                return false;
            };
            if bindings
                .values
                .get("open_details")
                .is_some_and(|widget| widget.enabled == Some(false))
            {
                return true;
            }
            self.result_details_open = !self.result_details_open;
            naval_results::show_details(&mut bindings.values, self.result_details_open);
            cache.generation = cache.generation.wrapping_add(1);
            return true;
        }
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
    pub fn scroll(&mut self, target: ScrollTarget, direction: i32, wheel: bool) {
        self.gui.scroll(target, direction, wheel);
    }
    /// The topmost combat hover, otherwise the map's, on the native tooltip clock.
    pub fn tooltip(
        &mut self,
        drawn: Option<&Drawn>,
        defines: Option<&TooltipDefines>,
        point: Option<[f32; 2]>,
        map: Option<HoverHit<'_>>,
        now: Instant,
    ) -> Option<(&str, TooltipPosition)> {
        let defines = defines?;
        let combat_hit = point.and_then(|point| {
            drawn?
                .tooltips
                .iter()
                .rev()
                .find(|hit| contains(hit.rect, point))
        });
        let hit = combat_hit
            .map(|hit| {
                (
                    hit.text.as_deref().unwrap_or_default(),
                    hit.description.as_deref(),
                    hit.shortcut.as_deref(),
                    hit.rect,
                    None,
                    (hit.anchor == TooltipAnchor::NavalWindow)
                        .then(|| drawn.expect("combat hover has a drawn window").rect),
                )
            })
            .or_else(|| {
                map.map(|(text, body, rect, province)| (text, body, None, rect, province, None))
            });
        self.hover
            .bind_province(hit.and_then(|(_, _, _, _, province, _)| province));
        if let Some((text, delayed, shortcut, _, _, window)) = hit
            && (!text.is_empty()
                || delayed.is_some_and(|text| !text.is_empty())
                || shortcut.is_some_and(|text| !text.is_empty()))
        {
            self.hover.anchor = window.zip(point).map(|(window, point)| TooltipPosition {
                point,
                naval_window: Some(window),
            });
        }
        // Explicit anchors retain their position while the hide timer runs.
        let anchor = self.hover.anchor;
        self.hover
            .shown(
                hit.map(|(text, delayed, shortcut, rect, _, _)| (text, delayed, shortcut, rect)),
                point,
                now,
                defines.timing,
            )
            .map(|(text, point)| {
                (
                    text,
                    anchor.unwrap_or(TooltipPosition {
                        point,
                        naval_window: None,
                    }),
                )
            })
    }
    /// When the tooltip next changes with a still pointer, for a repaint.
    pub fn tooltip_repaint_after(
        &self,
        defines: Option<&TooltipDefines>,
        now: Instant,
    ) -> Option<Duration> {
        self.hover.next_change(now, defines?.timing)
    }
    /// Lays the open window out on a frame of `extent` interface pixels; the
    /// batch and every hit rectangle are in interface pixels too
    /// ([`viewer_foundation::map::ui_scale`]). `engaged` is the window's engaged-flag
    /// index; without it (a dump) the flag is what a load at this hour
    /// rebuilds. Only a draw with `hovers` collects the hover regions
    /// [`Self::tooltip`] needs, so a frame that lays the window out
    /// twice (once for its hit tests) does the hover work once.
    /// Hit preparation does not advance; drawing ticks once after input.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        document: &Arc<StaticContent>,
        branch: replay_data::BranchId,
        hour: replay_data::Hour,
        frame: io::Result<&replay_frame::frame::DecodedFrame>,
        extent: [f32; 2],
        perspective: &Choice,
        engaged: Option<&History>,
        nato: bool,
        hovers: bool,
        time: f32,
        advance: bool,
    ) -> io::Result<Option<Drawn>> {
        let Some(selection) = self.selection else {
            return Ok(None);
        };
        let covered = engaged.is_some_and(|index| index.covers(branch, hour));
        if !self.cached.as_ref().is_some_and(|cached| {
            cached.covered == covered
                && cached.at.matches_request(
                    branch,
                    hour,
                    selection,
                    perspective,
                    self.playing,
                    nato,
                )
        }) {
            let frame = frame?;
            let vision = cached_vision(
                frame,
                perspective,
                FogContent::loaded(&document.fog),
                &document.vision_memo,
            )?;
            let now = Scope {
                branch,
                hour,
                selection,
                perspective: perspective.clone(),
                viewer: vision.ours(),
                playing: self.playing,
                nato,
            };
            if matches!(selection, Selection::NavalResult(_)) {
                let same = self
                    .cached
                    .as_ref()
                    .is_some_and(|cached| cached.at.result_ui_at(&now));
                if !same {
                    self.result_details_open = false;
                    self.gui = gui::Windows::default();
                    self.gui_time = None;
                }
            }
            let flags = combat_flags(frame, vision.ours(), engaged)?;
            let cached = match update(selection, &flags) {
                Update::Close => {
                    self.close();
                    return Ok(None);
                }
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
                        shown: false,
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
                    let mut bindings = Bindings::read(
                        document,
                        frame,
                        selection,
                        &vision,
                        &flags,
                        !self.playing,
                        nato,
                    )?;
                    if matches!(selection, Selection::NavalResult(_)) {
                        naval_results::show_details(&mut bindings.values, self.result_details_open);
                    }
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
                        gui_resources::GuiAssets::new(document.assets.as_ref(), &bindings);
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
        let assets = document.assets.as_ref();
        let windows = document.window_cache.at(extent.map(|value| value as u32))?;
        let inventory = cached.gui_assets.as_mut().expect("refreshed with bindings");
        inventory.refresh_portraits(assets, time, document.lazy.version());
        let resources = gui_resources::GuiResources::new(
            assets,
            &windows,
            &document.localisation,
            &document.small_flags,
            &document.large_flags,
            inventory,
        );
        let tick = lifecycle::Tick {
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
    pub refused: BTreeMap<String, usize>,
    pub rect: [f32; 4],
    pub close: Option<[f32; 4]>,
    dismiss: Option<[f32; 4]>,
    result_details_button: Option<[f32; 4]>,
    result_details_rects: Vec<[f32; 4]>,
    lists: Vec<([f32; 4], ScrollTarget)>,
    scroll_buttons: Vec<([f32; 4], ScrollTarget, i32)>,
    /// Hover regions in draw order; the last containing one wins.
    tooltips: Vec<TooltipHit>,
    tactics_button: Option<[f32; 4]>,
    tactics_close: Option<[f32; 4]>,
    tactics_rect: Option<[f32; 4]>,
}
impl Drawn {
    /// Adds another window's hover regions above this window's, as a later window paints over
    /// it.
    pub fn add_hovers(&mut self, other: &Drawn) {
        self.tooltips.extend(other.tooltips.iter().cloned());
    }
}

#[derive(Clone)]
pub struct ScrollTarget {
    popup: bool,
    path: String,
    vertical: bool,
}

/// Bound hover text. The bindings hold it once; each drawn hover region
/// shares it rather than copying the text every frame.
pub(crate) type HoverText = Arc<str>;

/// One widget's hover region and its text, in interface pixels.
#[derive(Clone, Debug, PartialEq)]
struct TooltipHit {
    rect: [f32; 4],
    text: Option<HoverText>,
    description: Option<HoverText>,
    shortcut: Option<HoverText>,
    anchor: TooltipAnchor,
}

/// Producer-specific placement; ship groups anchor beside the naval window.
/// Pride, sunk ships and air entries keep their own default placement.
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
        contains(self.rect, point)
            || self.in_popup(point)
            || self
                .result_details_rects
                .iter()
                .any(|rect| contains(*rect, point))
    }
    pub fn closes(&self, point: [f32; 2]) -> bool {
        !self.in_popup(point)
            && (self.close.is_some_and(|rect| contains(rect, point))
                || self.dismiss.is_some_and(|rect| contains(rect, point)))
    }
    /// The list under `point`; the popup covers the window's lists.
    pub fn list_at(&self, point: [f32; 2]) -> Option<ScrollTarget> {
        self.lists
            .iter()
            .rev()
            .find(|(rect, target)| target.popup == self.in_popup(point) && contains(*rect, point))
            .map(|(_, target)| target.clone())
    }
    pub fn scroll_action(&self, point: [f32; 2]) -> Option<(ScrollTarget, i32)> {
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
    /// the game overrides the text scrollbar's step from its newline count.
    pub text_scroll_lines: Option<bool>,
    /// Hover text, and the delayed body shown after its longer delay.
    pub tooltip: Option<HoverText>,
    pub tooltip_description: Option<HoverText>,
    /// Native shortcut tail, appended after the immediate and delayed strings.
    pub tooltip_shortcut: Option<HoverText>,
    pub tooltip_anchor: TooltipAnchor,
    /// A dynamic hover builder owns both strings ([`tooltips::set_hover`]):
    /// the window's update replaces the
    /// authored `pdx_tooltip` the widget was created with, so the authored
    /// seed never replaces its text or its refusal.
    pub hover_owned: bool,
    pub sprite: Option<String>,
    /// the game leaves the existing drawable untouched when its picture path is absent.
    pub retain_sprite: bool,
    pub flag: Option<String>,
    pub fill: Option<f32>,
    pub progress: Option<i32>,
    pub frame: Option<u32>,
    pub rotation: Option<f32>,
    pub opacity: Option<f32>,
    pub color: Option<[f32; 4]>,
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
/// Native text presentation resolves the complete key. The game writes 0x12
/// and a country tag; the viewer resolves the tag first and marks it with a
/// byte the localisation loader never makes a control, so
/// an authored `@` (0x12) inside localised text around it is still refused
/// by name ([`viewer_foundation::tooltip_text::displayable`]), in combat windows and map hovers alike.
pub(crate) const INLINE_FLAG: char = RESOLVED_FLAG;
/// The layer of a window quad pushed after `quads` others: the window draws
/// in push order, one icon ordinal per quad, in the GUI band above every
/// map-icon priority, so the open window covers the icons behind it.
pub fn window_layer(quads: usize) -> u64 {
    gui_layer(u32::try_from(quads).unwrap_or(u32::MAX), 0)
}

#[cfg(test)]
mod capture_scope_tests;
#[cfg(test)]
mod tests {
    /// A land window hides for an unengaged combat and refreshes on an undecided flag; a naval
    /// one refreshes only while engaged and holds otherwise, reporting an undecided or absent
    /// combat.
    #[test]
    fn the_window_updates_only_while_its_combat_is_engaged() {
        use std::collections::BTreeMap;
        use {
            super::{Selection, Update, update},
            viewer_foundation::engaged::Engaged,
        };
        let flags = |flag: Engaged| CombatFlags {
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
            Engaged::Ambiguous(Ambiguity::Scope),
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
            Update::Refresh(_) | Update::Close => None,
        };
        assert_eq!(
            update(Selection::Naval(2), &flags(Engaged::No)),
            Update::Close
        );
        assert_eq!(
            held(Selection::Naval(2), Engaged::Unknown),
            Some((2, Some("its engaged flag is unknown")))
        );
        assert_eq!(
            held(Selection::Naval(2), Engaged::Ambiguous(Ambiguity::Scope)),
            Some((2, Some(Ambiguity::Scope.reason())))
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

    #[test]
    fn the_selector_opens_completed_and_pending_retained_naval_results() {
        use replay_data::rows::{NavalResultRow, TableId, TableRows};
        use replay_frame::frame::{DecodedFrame, TableView};
        let mut frame = DecodedFrame {
            branch: 0,
            hour: 100,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "missing".into(),
                }
                .into()
            }),
            metrics: Default::default(),
        };
        let result = |id, result_hour, readiness| NavalResultRow {
            id,
            province: 362,
            result_hour,
            readiness,
            ..Default::default()
        };
        *frame.tables[TableId::NavalResults.index()] =
            TableView::Complete(TableRows::NavalResults(vec![
                result(7, 90, 0),
                result(8, 80, 1),
                result(9, 120, 0),
            ]));
        let resolved = |selector: Selector| selector.resolve(&frame).ok();
        assert_eq!(resolved(Selector::Id(7)), Some(Selection::NavalResult(7)));
        assert_eq!(resolved(Selector::Id(8)), Some(Selection::NavalResult(8)));
        assert_eq!(
            resolved(Selector::Province(362)),
            Some(Selection::NavalResult(7))
        );
        assert_eq!(resolved(Selector::Id(9)), None);
    }

    fn scope(branch: u64, hour: i32, viewer: u32) -> super::Scope {
        super::Scope {
            branch,
            hour,
            selection: super::Selection::Naval(2),
            perspective: Choice::default(),
            viewer,
            playing: false,
            nato: false,
        }
    }

    #[test]
    fn retained_result_details_toggle_both_panels_and_reset_on_reopen() {
        let mut window = Window::default();
        window.open(Selection::NavalResult(7));
        let mut at = scope(0, 10, 1);
        at.selection = Selection::NavalResult(7);
        let mut bindings = Bindings::default();
        naval_results::show_details(&mut bindings.values, false);
        window.cached = Some(Cached {
            at: at.clone(),
            covered: false,
            contents: Some((at.clone(), bindings)),
            shown: true,
            held: None,
            generation: 1,
            gui_assets: None,
        });
        let drawn = Drawn {
            result_details_button: Some([5., 5., 20., 20.]),
            ..Default::default()
        };
        assert!(window.handle_click(&drawn, [10., 10.]));
        assert!(window.result_details_open);
        let cached = window.cached.as_ref().unwrap();
        assert_eq!(cached.generation, 2);
        let values = &cached.contents.as_ref().unwrap().1.values;
        assert!(!values["attacker_details"].hidden && !values["defender_details"].hidden);
        let mut later = at.clone();
        later.hour = 12;
        later.playing = true;
        assert!(
            at.result_ui_at(&later),
            "playing keeps the local result details"
        );
        for changed in [
            Scope {
                branch: 1,
                ..later.clone()
            },
            Scope {
                viewer: 2,
                ..later.clone()
            },
            Scope {
                hour: 9,
                ..later.clone()
            },
            Scope {
                selection: Selection::NavalResult(8),
                ..later
            },
        ] {
            assert!(!at.result_ui_at(&changed));
        }
        assert!(window.handle_click(&drawn, [10., 10.]));
        assert!(!window.result_details_open);
        window.open(Selection::NavalResult(7));
        assert!(!window.result_details_open && window.cached.is_none());
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
    fn viewer_nato_changes_the_combat_refresh_key_without_ending_a_naval_hold() {
        let read = scope(0, 10, 1);
        let mut toggled = read.clone();
        toggled.nato = true;
        let matches = |now: &Scope| {
            read.matches_request(
                now.branch,
                now.hour,
                now.selection,
                &now.perspective,
                now.playing,
                now.nato,
            )
        };
        assert!(matches(&read));
        assert!(
            !matches(&toggled),
            "land tactic bindings must refresh at the same hour"
        );
        toggled.hour = 12;
        let kept = hold(Some((read, Bindings::default())), &toggled, 2, None)
            .expect("a viewer icon toggle keeps a disengaged naval window");
        let (at, _) = kept.0.unwrap();
        assert_eq!(
            at.hour, 10,
            "held simulation operands stay at their read hour"
        );
        assert!(kept.1.is_none());
    }

    #[test]
    fn a_playback_toggle_refreshes_the_key_without_ending_a_naval_hold() {
        let read = scope(0, 10, 1);
        let mut toggled = read.clone();
        toggled.playing = true;
        assert!(!read.matches_request(
            toggled.branch,
            toggled.hour,
            toggled.selection,
            &toggled.perspective,
            toggled.playing,
            toggled.nato,
        ));
        toggled.hour = 12;
        let (contents, note) = hold(Some((read, Bindings::default())), &toggled, 2, None)
            .expect("a playback toggle keeps a disengaged naval window");
        assert_eq!(contents.map(|(at, _)| at.hour), Some(10));
        assert!(note.is_none());
    }

    use super::*;

    /// The tactics popup paints into its own batch, which is appended after
    /// the window's quads: with the window's quad count as its layer base,
    /// every popup quad sorts above every window quad, as the sprite pass
    /// sorts them (viewer_foundation::map::sprites, by layer).
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

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    pub use super::{
        assets::WINDOW_TEMPLATES,
        binding::{Bindings, sprite as bind_sprite},
        gui::Windows as GuiWindows,
        gui_resources::{GuiAssets as CombatGuiAssets, GuiResources as CombatGuiResources},
        tactics::NULL_PICTURE,
        tooltips::bind_authored,
    };
}
