
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

use std::time::Duration;
use std::time::Instant;
#[cfg(test)]
use viewer_engagements::engaged::Ambiguity;
use viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap as NavySources;

pub type NavySourcesOf = Arc<dyn Fn(&replay_frame::frame::DecodedFrame, u32, u32) -> NavySources + Send + Sync>;

pub type RuleRelations = Arc<
    dyn Fn(
            &replay_frame::frame::DecodedFrame,
            &content_reader_core::adjacency_rules::AdjacencyRule,
            u32,
        ) -> Result<viewer_unit_reads::movement_reads::RuleReason, String>
        + Send
        + Sync,
>;
use viewer_engagements::engagement_history::CombatFlags;
use viewer_engagements::engagement_history::History;
use viewer_native_gui::native_gui::lifecycle;
use viewer_native_gui::native_gui::pointer::Input;
use viewer_native_gui::native_text::RESOLVED_FLAG;
use viewer_foundation::perspective::Choice;
use viewer_foundation::perspective::cached_vision;
use viewer_native_gui::sprites::gui_layer;
use viewer_foundation::visibility::FogContent;
use viewer_foundation::visibility::Relations;
use viewer_native_gui::tooltip_painter::HoverState;
use viewer_native_gui::tooltip_painter::TooltipDefines;
use viewer_native_gui::tooltip_painter::{Explicit, TooltipPosition};
mod air_menu;
mod assets;
pub(crate) mod availability;
mod battle_name;
pub(crate) mod binding;
mod border_marker;
mod commander_tooltips;
pub mod confirmation;
mod content;
mod defines;
pub(crate) mod details;
mod division_icons;
mod division_tooltips;
pub mod find_view;
mod fixed;
mod gui;
mod gui_resources;
mod hq;
mod keyed;
pub(crate) mod land_details;
pub mod land_marker_tooltips;
mod land_terrain;
pub mod minimap;
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
mod naval_region_view;
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
mod state_view;
mod tactics;
mod template_icons;
pub mod terrain_mode;
mod tooltips;
pub use content::{Content, Occupation};
pub use division_tooltips::division_panel;

pub use assets::Assets;
pub use assets::CombatContent;
use binding::Bindings;
use std::{collections::BTreeMap, io, sync::Arc};
#[cfg(test)]
use viewer_engagements::combat_selection::Hit;
use viewer_engagements::combat_selection::Selection;
use viewer_runtime::geometry::contains;
use self::content::Content as StaticContent;
use viewer_native_gui::sprites::Batch;
use viewer_counter_products::map::province_tooltips::air_region::{RosterHistory, Sampler};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selector {
    Id(u64),
    Province(u32),
    AirRegion(u32),
    State(u32),
    NavalRegion(u32),
}

impl Selector {
    pub fn direct(self) -> Option<Selection> {
        match self {
            Self::AirRegion(region) => Some(Selection::AirRegion(region)),
            Self::State(province) => Some(Selection::State(province)),
            Self::NavalRegion(region) => Some(Selection::NavalRegion(region)),
            Self::Id(_) | Self::Province(_) => None,
        }
    }

    pub fn resolve(self, frame: &replay_frame::frame::DecodedFrame) -> io::Result<Selection> {
        if let Some(selection) = self.direct() {
            return Ok(selection);
        }
        use replay_data::rows::tables;
        use viewer_foundation::tables::rows;
        let mut matches = Vec::new();
        for row in rows::<tables::LandCombats>(frame)?.rows() {
            if match self {
                Self::Id(id) => id == row.id,
                Self::Province(id) => id == row.province,
                Self::AirRegion(_) | Self::State(_) | Self::NavalRegion(_) => unreachable!(),
            } {
                matches.push(Selection::Land(row.id));
            }
        }
        for row in rows::<tables::NavalCombats>(frame)?.rows() {
            if match self {
                Self::Id(id) => id == row.id,
                Self::Province(id) => id == row.province,
                Self::AirRegion(_) | Self::State(_) | Self::NavalRegion(_) => unreachable!(),
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
                        && match self {
                            Self::Id(id) => id == row.id,
                            Self::Province(id) => id == row.province,
                            Self::AirRegion(_) | Self::State(_) | Self::NavalRegion(_) => unreachable!(),
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
    air: air_menu::State,
    playing: bool,
    rule_relations: Option<RuleRelations>,
    navy_sources_of: Option<NavySourcesOf>,
    settled: bool,
    pub selection: Option<Selection>,
    pub offset: [f32; 2],
    air_offset: [f32; 2],
    pub dragging: bool,
    pub tactics_open: bool,
    result_details_open: bool,
    tactics_position: Option<[f32; 2]>,
    dragging_tactics: bool,
    hover: HoverState,
    tooltip_from_map: bool,
    cached: Option<Cached>,
    bindings_dirty: bool,
    settings: u64,
    opened: u64,
    asked: Option<Family>,
    loading: bool,
    pending: std::collections::VecDeque<Product>,
    closed: Option<String>,
    retired: Vec<Box<dyn Send>>,
    gui: gui::Windows,
    gui_time: Option<f32>,
    refused: BTreeMap<String, usize>,
}

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

    fn holds_at(&self, now: &Scope) -> bool {
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

type Contents = (Scope, Bindings);

struct Cached {
    at: Scope,
    covered: bool,
    contents: Option<Contents>,
    shown: bool,
    held: Option<String>,
    generation: u64,
    gui_assets: Option<gui_resources::GuiAssets>,
    of: Option<(Family, u64)>,
}

fn combat_flags(
    frame: &replay_frame::frame::DecodedFrame,
    ours: u32,
    engaged: Option<&History>,
) -> io::Result<CombatFlags> {
    use replay_data::rows::tables;
    use viewer_foundation::tables::rows;
    let links: BTreeMap<u32, u32> = rows::<tables::CountryLinks>(frame)?
        .rows()
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let relations = Relations::read(frame, &links);
    CombatFlags::read(
        frame,
        ours,
        &links,
        relations.as_ref().map_err(|reason| *reason),
        engaged,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Update {
    Close,
    Refresh(bool),
    Hold {
        combat: u64,
        undecided: Option<&'static str>,
    },
}

fn update(selection: Selection, flags: &CombatFlags) -> Update {
    use viewer_engagements::engaged::Engaged;
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
        Selection::AirRegion(_) | Selection::State(_) | Selection::NavalRegion(_) => Update::Refresh(true),
    }
}

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

pub type HoverHit<'a> = (&'a str, Option<&'a str>, [f32; 4], Option<u32>);

impl Window {
    pub fn set_modifiers(&mut self, shift: bool, control: bool) {
        self.air.shift = shift;
        self.air.control = control;
    }
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    pub fn set_settled(&mut self, settled: bool) {
        self.settled = settled;
    }

    pub fn take_inputs(&mut self) -> impl Send + 'static {
        (self.rule_relations.take(), self.navy_sources_of.take())
    }

    pub fn holds_inputs(&self) -> bool {
        self.rule_relations.is_some() || self.navy_sources_of.is_some()
    }

    fn kept(&mut self) -> (Option<RuleRelations>, Option<NavySourcesOf>, bool) {
        (self.rule_relations.take(), self.navy_sources_of.take(), self.settled)
    }

    pub fn is_loading(&self) -> bool {
        self.selection.is_some() && self.loading
    }

    pub fn not_shown(&self) -> Option<String> {
        let Some(selection) = self.selection else {
            return Some(self.closed.clone().unwrap_or_else(|| "the window is closed".into()));
        };
        if self.showing() {
            return None;
        }
        let cached = self.cached.as_ref()?;
        Some(cached.held.clone().unwrap_or_else(|| format!("at hour {}, {selection:?}: the viewing country is not engaged in it, so the window is not shown", cached.at.hour)))
    }
    pub fn set_rule_relations(&mut self, relations: RuleRelations) {
        self.rule_relations = Some(relations);
    }

    pub fn set_navy_sources_of(&mut self, of: NavySourcesOf) {
        self.navy_sources_of = Some(of);
    }
    pub fn warnings(&self, artwork: &[String]) -> Vec<String> {
        let Some(cache) = self.cached.as_ref().filter(|_| self.selection.is_some() && !self.loading) else {
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
    pub fn holding(&self) -> bool {
        self.showing() && self.gui.holding()
    }

    fn showing(&self) -> bool {
        !self.loading && self.cached.as_ref().is_some_and(|cached| cached.shown)
    }

    pub fn highlighted(&self) -> Option<&str> {
        self.showing().then(|| self.gui.highlighted()).flatten()
    }

    pub fn gui_active(&self) -> bool {
        self.showing() && self.gui.active()
    }
    pub fn open(&mut self, selection: Selection) {
        if matches!(self.selection, Some(Selection::AirRegion(_))) {
            self.air_offset = self.offset;
        }
        let air_offset = self.air_offset;
        let offset = if matches!(selection, Selection::AirRegion(_)) {
            air_offset
        } else {
            [0.; 2]
        };
        let opened = self.opened.wrapping_add(1);
        let retired = self.let_go();
        let (rule_relations, navy_sources_of, settled) = self.kept();
        let mut air = std::mem::take(&mut self.air);
        air.initial_side = true;
        *self = Self {
            air,
            air_offset,
            offset,
            opened,
            retired,
            rule_relations,
            navy_sources_of,
            settled,
            selection: Some(selection),
            ..Self::default()
        };
    }
    pub fn close(&mut self) {
        if matches!(self.selection, Some(Selection::AirRegion(_))) {
            self.air_offset = self.offset;
        }
        let air_offset = self.air_offset;
        let retired = self.let_go();
        let (rule_relations, navy_sources_of, settled) = self.kept();
        let air = std::mem::take(&mut self.air);
        *self = Self {
            air,
            air_offset,
            opened: self.opened,
            retired,
            rule_relations,
            navy_sources_of,
            settled,
            ..Self::default()
        };
    }

    fn let_go(&mut self) -> Vec<Box<dyn Send>> {
        let (cached, pending) = (self.cached.take(), self.take_pending());
        self.retire(cached);
        self.retire(pending);
        std::mem::take(&mut self.retired)
    }

    fn retire(&mut self, value: Option<impl Send + 'static>) {
        if let Some(value) = value {
            self.retired.push(Box::new(value));
        }
    }

    pub fn take_retired(&mut self) -> Option<impl Send + 'static> {
        (!self.retired.is_empty()).then(|| std::mem::take(&mut self.retired))
    }

    pub fn configure_air(&mut self, details: bool, range: usize, channels: u32) {
        self.air.details = details;
        self.air.range = range;
        self.air.channels = channels;
        self.dirty();
    }

    pub fn click(&mut self, drawn: &Drawn, path: &str, rect: [f32; 4]) -> bool {
        if matches!(self.selection, Some(Selection::AirRegion(_)))
            && let Some((_, action)) = drawn.air_controls.iter().find(|(at, _)| at == path)
        {
            self.air.click(action);
            self.dirty();
            return true;
        }
        if matches!(self.selection, Some(Selection::NavalResult(_)))
            && drawn.result_details_button == Some(rect)
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
        if drawn.tactics_close == Some(rect) {
            self.tactics_open = false;
            return true;
        }
        if drawn.tactics_button == Some(rect) {
            self.tactics_open = !self.tactics_open;
            self.tactics_position = None;
            return true;
        }
        if drawn.close == Some(rect) || drawn.dismiss == Some(rect) {
            self.close();
            return true;
        }
        if let Some((_, target, direction)) =
            drawn.scroll_buttons.iter().find(|(at, _, _)| *at == rect)
        {
            self.scroll(target.clone(), *direction, false);
            return true;
        }
        false
    }
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
                    match hit.anchor {
                        TooltipAnchor::NavalWindow => Some(Explicit::NavalWindow(
                            drawn.expect("combat hover has a drawn window").rect,
                        )),
                        TooltipAnchor::At { x, y, below } => Some(Explicit::At {
                            at: [x as f32, y as f32],
                            below,
                        }),
                        TooltipAnchor::Pointer | TooltipAnchor::Authored(_) => None,
                    },
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
            self.tooltip_from_map = combat_hit.is_none();
            self.hover.anchor = window.zip(point).map(|(window, point)| TooltipPosition {
                point,
                anchor: Some(window),
            });
        }
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
                        anchor: None,
                    }),
                )
            })
    }
    pub fn tooltip_from_map(&self) -> bool {
        self.tooltip_from_map
    }

    pub fn tooltip_repaint_after(
        &self,
        defines: Option<&TooltipDefines>,
        now: Instant,
    ) -> Option<Duration> {
        self.hover.next_change(now, defines?.timing)
    }
    #[allow(clippy::too_many_arguments, reason = "the window's read inputs, each from another owner")]
    #[allow(clippy::too_many_arguments, reason = "the window's read inputs, each from another owner")]
    pub fn prepare(
        &mut self,
        document: &Arc<StaticContent>,
        rosters: &Arc<Sampler>,
        branch: replay_data::BranchId,
        hour: replay_data::Hour,
        frame: Result<Arc<replay_frame::frame::DecodedFrame>, String>,
        perspective: &Choice,
        engaged: Option<Arc<History>>,
        nato: bool,
    ) -> io::Result<Prepared> {
        let family = match self.want(branch, hour, perspective, engaged.as_deref(), nato) {
            Want::Closed => return Ok(Prepared::Closed),
            Want::Kept => return Ok(Prepared::Kept),
            Want::Read(family) => family,
        };
        read(&self.asking_of(document, rosters, family).request(branch, hour, frame, engaged)).map(|fresh| Prepared::Read(Box::new(fresh)))
    }

    pub fn asking(&mut self, document: &Arc<StaticContent>, rosters: &Arc<Sampler>, perspective: &Choice, nato: bool) -> Option<Asking> {
        let family = self.ask(perspective, nato)?;
        Some(self.asking_of(document, rosters, family))
    }

    fn ask(&mut self, perspective: &Choice, nato: bool) -> Option<Family> {
        let selection = self.selection?;
        let family = self.family(selection, perspective, nato);
        self.asked = Some(family.clone());
        Some(family)
    }

    pub fn offer(&mut self, made: Product) {
        let asked = self.asked.clone();
        let (kept, gone): (std::collections::VecDeque<Product>, std::collections::VecDeque<Product>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|held| held.seek == made.seek && held.hour != made.hour && asked.as_ref() == Some(&held.key.0));
        self.pending = kept;
        if !gone.is_empty() {
            self.retire(Some(gone));
        }
        let at = self.pending.partition_point(|held| held.hour < made.hour);
        self.pending.insert(at, made);
    }

    fn take_pending(&mut self) -> Option<std::collections::VecDeque<Product>> {
        (!self.pending.is_empty()).then(|| std::mem::take(&mut self.pending))
    }

    pub fn shown(&mut self, at: Option<OnScreen>) -> io::Result<Prepared> {
        if self.selection.is_none() {
            let pending = self.take_pending();
            self.retire(pending);
            return Ok(Prepared::Closed);
        }
        while let Some(made) = self.pending.pop_front() {
            let ours = self.asked.as_ref() == Some(&made.key.0);
            match at {
                Some(at) if ours && (made.branch, made.seek, made.hour) == (at.branch, at.seek, at.hour) => {
                    let of = Some((made.key.0, made.seek));
                    return made.fresh.map(|fresh| Prepared::Read(Box::new(Fresh { of, ..fresh })));
                }
                Some(at) if ours && (made.branch, made.seek) == (at.branch, at.seek) && made.hour > at.hour => {
                    self.pending.push_front(made);
                    break;
                }
                None if ours => {
                    self.pending.push_front(made);
                    break;
                }
                _ => self.retire(Some(made)),
            }
        }
        let holds = at.is_some_and(|at| {
            self.cached.as_ref().is_some_and(|cached| {
                (cached.at.branch, cached.at.hour) == (at.branch, at.hour)
                    && cached.of.as_ref().is_some_and(|(family, seek)| self.asked.as_ref() == Some(family) && *seek == at.seek)
            })
        });
        Ok(if holds { Prepared::Kept } else { Prepared::Loading })
    }

    fn want(&mut self, branch: replay_data::BranchId, hour: replay_data::Hour, perspective: &Choice, engaged: Option<&History>, nato: bool) -> Want {
        let Some(selection) = self.selection else {
            return Want::Closed;
        };
        let covered = engaged.is_some_and(|index| index.covers(branch, hour));
        if !self.bindings_dirty
            && self.cached.as_ref().is_some_and(|cached| {
                cached.covered == covered
                    && cached.at.matches_request(branch, hour, selection, perspective, self.playing, nato)
            })
        {
            return Want::Kept;
        }
        Want::Read(self.family(selection, perspective, nato))
    }

    fn family(&self, selection: Selection, perspective: &Choice, nato: bool) -> Family {
        Family { selection, perspective: perspective.clone(), playing: self.playing, nato, settings: self.settings, opened: self.opened }
    }

    fn asking_of(&self, document: &Arc<StaticContent>, rosters: &Arc<Sampler>, family: Family) -> Asking {
        Asking { content: Arc::clone(document), family, air: self.air_asking(rosters), details: self.result_details_open, rule_relations: self.rule_relations.clone(), navy_sources_of: self.navy_sources_of.clone() }
    }

    fn air_asking(&self, rosters: &Arc<Sampler>) -> AirAsking {
        AirAsking { state: self.air.clone(), rosters: Arc::clone(rosters) }
    }

    pub fn content_replaced(&mut self) {
        let (cached, pending) = (self.cached.take(), self.take_pending());
        self.retire(cached);
        self.retire(pending);
        self.asked = None;
        self.dirty();
    }

    fn dirty(&mut self) {
        self.bindings_dirty = true;
        self.settings = self.settings.wrapping_add(1);
    }

    #[allow(clippy::too_many_arguments, reason = "the window's layout inputs, each from another owner")]
    pub fn draw(
        &mut self,
        document: &Arc<StaticContent>,
        prepared: Prepared,
        extent: [f32; 2],
        hovers: bool,
        time: f32,
        advance: bool,
        input: Option<Input>,
    ) -> io::Result<Option<Drawn>> {
        let Some(selection) = self.selection else {
            return Ok(None);
        };
        if !self.take_in(document, prepared) || self.loading {
            return Ok(None);
        }
        let Some(cached) = self.cached.as_mut() else {
            return Ok(None);
        };
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
            self.settled,
            input,
        )?;
        self.refused.clone_from(&drawn.refused);
        Ok(Some(drawn))
    }

    pub fn take_in(&mut self, document: &Arc<StaticContent>, prepared: Prepared) -> bool {
        let Some(selection) = self.selection else {
            return false;
        };
        self.loading = matches!(prepared, Prepared::Loading);
        match prepared {
            Prepared::Closed => false,
            Prepared::Kept | Prepared::Loading => true,
            Prepared::Read(fresh) => self.keep(document, selection, *fresh),
        }
    }

    fn keep(&mut self, document: &Arc<StaticContent>, selection: Selection, fresh: Fresh) -> bool {
        let Fresh {
            air,
            now,
            covered,
            read,
            of,
        } = fresh;
        if let Some(air) = air {
            self.air.mission_groups = air.mission_groups;
            if let Some(enemy) = air.enemy {
                self.air.enemy = enemy;
                self.air.initial_side = false;
            }
        }
        if matches!(selection, Selection::NavalResult(_))
            && !self
                .cached
                .as_ref()
                .is_some_and(|cached| cached.at.result_ui_at(&now))
        {
            self.result_details_open = false;
            self.gui = gui::Windows::default();
            self.gui_time = None;
        }
        let cached = match read {
            Read::Close => {
                self.close();
                self.closed = Some(format!("at hour {}, {selection:?}: the viewing country is not engaged in it, so the window closes", now.hour));
                return false;
            }
            Read::Hold { combat, undecided } => {
                let (previous, gui_assets, generation) =
                    self.cached.take().map_or((None, None, 0), |cached| {
                        (cached.contents, cached.gui_assets, cached.generation)
                    });
                let (previous, gone) = match previous {
                    Some(contents) if contents.0.holds_at(&now) => (Some(contents), None),
                    gone => (None, gone),
                };
                self.retire(gone);
                let Some((contents, held)) = hold(previous, &now, combat, undecided) else {
                    self.retire(gui_assets);
                    self.close();
                    self.closed = Some(format!("at hour {}, {selection:?}: never engaged for the viewing country, so the window does not open", now.hour));
                    return false;
                };
                Cached {
                    at: now,
                    covered,
                    shown: false,
                    contents,
                    held,
                    generation,
                    gui_assets,
                    of,
                }
            }
            Read::Refresh {
                shown,
                mut bindings,
                mut gui_assets,
                details,
            } => {
                let generation = self
                    .cached
                    .as_ref()
                    .map_or(1, |cached| cached.generation.wrapping_add(1));
                if matches!(selection, Selection::NavalResult(_)) && details != self.result_details_open {
                    naval_results::show_details(&mut bindings.values, self.result_details_open);
                    gui_assets = gui_resources::GuiAssets::new(document.assets.as_ref(), &bindings);
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
                Cached {
                    at: now.clone(),
                    covered,
                    contents: Some((now, bindings)),
                    shown,
                    held: None,
                    generation,
                    gui_assets: Some(gui_assets),
                    of,
                }
            }
        };
        let replaced = self.cached.replace(cached);
        self.retire(replaced);
        self.bindings_dirty = false;
        true
    }

    pub fn model(&self) -> Option<(replay_data::Hour, String)> {
        let cached = self.cached.as_ref()?;
        Some((
            cached.at.hour,
            format!(
                "shown {} held {:?} contents {:?}",
                cached.shown, cached.held, cached.contents
            ),
        ))
    }
}

struct Request {
    content: Arc<StaticContent>,
    branch: replay_data::BranchId,
    hour: replay_data::Hour,
    frame: Result<Arc<replay_frame::frame::DecodedFrame>, String>,
    selection: Selection,
    perspective: Choice,
    engaged: Option<Arc<History>>,
    nato: bool,
    playing: bool,
    air: AirAsking,
    details: bool,
    navy_sources_of: Option<NavySourcesOf>,
    rule_relations: Option<RuleRelations>,
}

#[derive(Clone)]
struct AirAsking {
    state: air_menu::State,
    rosters: Arc<Sampler>,
}

impl AirAsking {
    fn at(&self, branch: replay_data::BranchId, hour: replay_data::Hour, viewer: u32) -> air_menu::State {
        let pass = RosterHistory::of(Arc::clone(&self.rosters)).visiting_pass(branch, hour, viewer);
        air_menu::State { pass, ..self.state.clone() }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Family {
    selection: Selection,
    perspective: Choice,
    playing: bool,
    nato: bool,
    settings: u64,
    opened: u64,
}

#[derive(Clone)]
pub struct Asking {
    content: Arc<StaticContent>,
    family: Family,
    air: AirAsking,
    details: bool,
    rule_relations: Option<RuleRelations>,
    navy_sources_of: Option<NavySourcesOf>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReadKey(Family);

impl Asking {
    pub fn key(&self) -> ReadKey {
        ReadKey(self.family.clone())
    }

    pub fn read(&self, branch: replay_data::BranchId, hour: replay_data::Hour, seek: u64, frame: Arc<replay_frame::frame::DecodedFrame>, engaged: Option<Arc<History>>) -> Product {
        Product { key: self.key(), branch, hour, seek, fresh: read(&self.request(branch, hour, Ok(frame), engaged)) }
    }

    fn request(&self, branch: replay_data::BranchId, hour: replay_data::Hour, frame: Result<Arc<replay_frame::frame::DecodedFrame>, String>, engaged: Option<Arc<History>>) -> Request {
        let Family { selection, perspective, playing, nato, .. } = &self.family;
        Request { content: Arc::clone(&self.content), branch, hour, frame, selection: *selection, perspective: perspective.clone(), engaged, nato: *nato, playing: *playing, air: self.air.clone(), details: self.details, navy_sources_of: self.navy_sources_of.clone(), rule_relations: self.rule_relations.clone() }
    }
}

#[derive(Debug)]
pub struct Product {
    key: ReadKey,
    branch: replay_data::BranchId,
    hour: replay_data::Hour,
    seek: u64,
    fresh: io::Result<Fresh>,
}

impl Product {
    pub fn hour(&self) -> replay_data::Hour {
        self.hour
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnScreen {
    pub branch: replay_data::BranchId,
    pub hour: replay_data::Hour,
    pub seek: u64,
}

enum Want {
    Closed,
    Kept,
    Read(Family),
}

fn read(request: &Request) -> io::Result<Fresh> {
    let Request { content: document, branch, hour, frame, selection, perspective, engaged, nato, playing, air, details, navy_sources_of, rule_relations } = request;
    let frame = frame.as_deref().map_err(|why| invalid(why.clone()))?;
    let vision = cached_vision(frame, perspective, FogContent::loaded(&document.fog), &document.vision_memo)?;
    let air = match *selection {
        Selection::AirRegion(region) => {
            let mut air = air.at(*branch, *hour, vision.ours());
            let mut enemy = None;
            air.mission_groups = mission_groups(frame);
            if air.initial_side
                && let Ok(side) = air_menu::combat::initial_side(document, frame, &vision, region, &air)
            {
                air.enemy = side;
                air.initial_side = false;
                enemy = Some(side);
            }
            Some((region, air, enemy))
        }
        _ => None,
    };
    let air_read = air.as_ref().map(|(_, air, enemy)| AirRead { mission_groups: air.mission_groups.clone(), enemy: *enemy });
    let now = Scope {
        branch: *branch,
        hour: *hour,
        selection: *selection,
        perspective: perspective.clone(),
        viewer: vision.ours(),
        playing: *playing,
        nato: *nato,
    };
    let flags = if matches!(selection, Selection::AirRegion(_) | Selection::State(_) | Selection::NavalRegion(_)) {
        CombatFlags::default()
    } else {
        combat_flags(frame, vision.ours(), engaged.as_deref())?
    };
    let read = match update(*selection, &flags) {
        Update::Close => Read::Close,
        Update::Hold { combat, undecided } => Read::Hold { combat, undecided },
        Update::Refresh(shown) => {
            let mut bindings = if let Some((region, air, _)) = &air {
                air_menu::bind(document, frame, &vision, *region, air)?
            } else {
                let sources = navy_sources(*selection, navy_sources_of.as_ref(), frame, vision.ours());
                Bindings::read(document, frame, *selection, &vision, &flags, !*playing, *nato, &sources, rule_relations.as_ref())?
            };
            if matches!(selection, Selection::NavalResult(_)) {
                naval_results::show_details(&mut bindings.values, *details);
            }
            let gui_assets = gui_resources::GuiAssets::new(document.assets.as_ref(), &bindings);
            Read::Refresh { shown, bindings, gui_assets, details: *details }
        }
    };
    let covered = engaged.as_ref().is_some_and(|index| index.covers(*branch, *hour));
    Ok(Fresh { air: air_read, now, covered, read, of: None })
}

fn navy_sources(selection: Selection, of: Option<&NavySourcesOf>, frame: &replay_frame::frame::DecodedFrame, ours: u32) -> NavySources {
    match (selection, of) {
        (Selection::NavalRegion(region), Some(of)) => of(frame, ours, region),
        (Selection::NavalRegion(_), None) => Err("the naval dominance sources are derived with the map's content, which this read was not given".into()),
        _ => Ok(BTreeMap::new()),
    }
}

#[derive(Debug)]
struct AirRead {
    mission_groups: Result<[u32; 3], String>,
    enemy: Option<bool>,
}

fn mission_groups(frame: &replay_frame::frame::DecodedFrame) -> Result<[u32; 3], String> {
    use replay_data::rows::*;
    match frame.complete::<tables::SimulationFeatures>() {
        Ok([features]) => {
            let when = |feature, missions| {
                if features.dlc_mask & feature != 0 {
                    missions
                } else {
                    0
                }
            };
            Ok([
                AIR_MENU_FIGHTER_MISSIONS | when(AIR_MENU_RECON_FEATURE, AIR_MENU_RECON_MISSIONS),
                AIR_MENU_NAVAL_MISSIONS | when(AIR_MENU_MINES_FEATURE, AIR_MENU_MINE_MISSIONS),
                AIR_MENU_STRATEGIC_MISSIONS
                    | when(AIR_MENU_LOGISTICS_FEATURE, AIR_MENU_LOGISTICS_MISSIONS)
                    | when(AIR_MENU_SUPPLY_FEATURE, AIR_MENU_SUPPLY_MISSIONS),
            ])
        }
        Ok(_) => Err("air mission filters need one SimulationFeatures row".into()),
        Err(why) => Err(why.to_string()),
    }
}

#[derive(Debug)]
pub enum Prepared {
    Closed,
    Kept,
    Loading,
    Read(Box<Fresh>),
}

#[derive(Debug)]
pub struct Fresh {
    air: Option<AirRead>,
    now: Scope,
    covered: bool,
    read: Read,
    of: Option<(Family, u64)>,
}

enum Read {
    Close,
    Hold {
        combat: u64,
        undecided: Option<&'static str>,
    },
    Refresh {
        shown: bool,
        bindings: Bindings,
        gui_assets: gui_resources::GuiAssets,
        details: bool,
    },
}

impl std::fmt::Debug for Read {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Close => out.write_str("Close"),
            Self::Hold { combat, undecided } => out.debug_struct("Hold").field("combat", combat).field("undecided", undecided).finish(),
            Self::Refresh { shown, bindings, details, .. } => out.debug_struct("Refresh").field("shown", shown).field("bindings", bindings).field("details", details).finish_non_exhaustive(),
        }
    }
}

#[derive(Clone, Default)]
pub struct Drawn {
    air_controls: Vec<(String, air_menu::Action)>,
    pub batch: Batch,
    pub refused: BTreeMap<String, usize>,
    pub rect: [f32; 4],
    areas: Vec<[f32; 4]>,
    pub close: Option<[f32; 4]>,
    dismiss: Option<[f32; 4]>,
    result_details_button: Option<[f32; 4]>,
    lists: Vec<([f32; 4], ScrollTarget)>,
    scroll_buttons: Vec<([f32; 4], ScrollTarget, i32)>,
    tooltips: Vec<TooltipHit>,
    tactics_button: Option<[f32; 4]>,
    tactics_close: Option<[f32; 4]>,
    tactics_rect: Option<[f32; 4]>,
    pub sounds: Vec<viewer_native_gui::native_gui::pointer::Played>,
    pub clicks: Vec<(String, [f32; 4], usize)>,
}
impl Drawn {
    pub fn missing(&self) -> Option<String> {
        if !self.batch.quads.is_empty() {
            return None;
        }
        let refused: Vec<&str> = self.refused.keys().map(String::as_str).collect();
        Some(match refused.as_slice() {
            [] => "the window drew no quads".to_owned(),
            refused => format!("the window drew no quads: {}", refused.join("; ")),
        })
    }

    pub fn add_hovers(&mut self, other: &Drawn) {
        self.tooltips.extend(other.tooltips.iter().cloned());
    }

    fn again(&self) -> Self {
        Self {
            sounds: Vec::new(),
            clicks: Vec::new(),
            ..self.clone()
        }
    }
}

#[derive(Clone)]
pub struct ScrollTarget {
    popup: bool,
    path: String,
    vertical: bool,
}

pub(crate) type HoverText = Arc<str>;

#[derive(Clone, Debug, PartialEq)]
struct TooltipHit {
    rect: [f32; 4],
    text: Option<HoverText>,
    description: Option<HoverText>,
    shortcut: Option<HoverText>,
    anchor: TooltipAnchor,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TooltipAnchor {
    #[default]
    Pointer,
    NavalWindow,
    Authored(content_reader_core::gui::native::TooltipAnchor),
    At { x: i32, y: i32, below: bool },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Footprint(Vec<[f32; 4]>);

impl Footprint {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        self.0.iter().any(|rect| contains(*rect, point))
    }
}

impl Drawn {
    fn in_popup(&self, point: [f32; 2]) -> bool {
        self.tactics_rect.is_some_and(|rect| contains(rect, point))
    }
    pub fn contains(&self, point: [f32; 2]) -> bool {
        contains(self.rect, point)
            || self.areas.iter().any(|rect| contains(*rect, point))
            || self.in_popup(point)
    }
    pub fn footprint(&self) -> Footprint {
        Footprint(std::iter::once(self.rect).chain(self.areas.iter().copied()).chain(self.tactics_rect).collect())
    }
    pub fn closes(&self, point: [f32; 2]) -> bool {
        !self.in_popup(point)
            && (self.close.is_some_and(|rect| contains(rect, point))
                || self.dismiss.is_some_and(|rect| contains(rect, point)))
    }
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

#[derive(Clone, Debug, Default)]
pub struct Widget {
    pub text: Option<String>,
    pub text_scroll_lines: Option<bool>,
    pub tooltip: Option<HoverText>,
    pub tooltip_description: Option<HoverText>,
    pub tooltip_shortcut: Option<HoverText>,
    pub tooltip_anchor: TooltipAnchor,
    pub hover_owned: bool,
    pub sprite: Option<String>,
    pub retain_sprite: bool,
    pub flag: Option<String>,
    pub fill: Option<f32>,
    pub progress: Option<i32>,
    pub frame: Option<u32>,
    pub rotation: Option<f32>,
    pub stretch: Option<f32>,
    pub scale: Option<f32>,
    pub caret: Option<usize>,
    pub opacity: Option<f32>,
    pub color: Option<[f32; 4]>,
    pub enabled: Option<bool>,
    pub hidden: bool,
    pub size: Option<[f32; 2]>,
    pub offset: [f32; 2],
    pub entry_template: Option<String>,
    pub entry_templates: BTreeMap<usize, String>,
    pub entries: Vec<Values>,
}
pub type Values = BTreeMap<String, Widget>;
pub(crate) const INLINE_FLAG: char = RESOLVED_FLAG;
pub fn window_layer(quads: usize) -> u64 {
    gui_layer(u32::try_from(quads).unwrap_or(u32::MAX), 0)
}

#[cfg(test)]
mod capture_scope_tests;
#[cfg(test)]
mod tests {
    #[test]
    fn the_window_updates_only_while_its_combat_is_engaged() {
        use std::collections::BTreeMap;
        use super::{Selection, Update, update};
        use viewer_engagements::engaged::Engaged;
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
            of: None,
        });
        let drawn = Drawn {
            result_details_button: Some([5., 5., 20., 20.]),
            ..Default::default()
        };
        assert!(window.click(&drawn, "", [5., 5., 20., 20.]));
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
        assert!(window.click(&drawn, "", [5., 5., 20., 20.]));
        assert!(!window.result_details_open);
        window.open(Selection::NavalResult(7));
        assert!(!window.result_details_open && window.cached.is_none());
    }

    #[test]
    fn a_held_window_keeps_only_its_own_branch_and_viewers_contents() {
        use super::{Bindings, hold};
        let read = || Some((scope(0, 10, 1), Bindings::default()));
        let kept = |now: &super::Scope, undecided| {
            hold(read(), now, 2, undecided)
                .map(|(contents, note)| (contents.map(|(read, _)| read.hour), note))
        };
        assert_eq!(kept(&scope(0, 12, 1), None), Some((Some(10), None)));
        assert_eq!(kept(&scope(1, 12, 1), None), None);
        assert_eq!(kept(&scope(0, 12, 7), None), None);
        let mut fogless = scope(0, 12, 1);
        fogless.perspective.fog = false;
        assert_eq!(kept(&fogless, None), None);
        assert_eq!(kept(&scope(0, 9, 1), None), None);
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
    fn the_window_reads_again_only_for_another_request_and_a_changed_setting_reads_as_another_window() {
        let mut window = Window::default();
        window.open(Selection::Naval(2));
        window.cached = Some(Cached {
            at: scope(0, 10, 1),
            covered: false,
            contents: None,
            shown: false,
            held: None,
            generation: 1,
            gui_assets: None,
            of: None,
        });
        let want = |window: &mut Window, hour| window.want(0, hour, &Choice::default(), None, false);
        assert!(matches!(want(&mut window, 10), Want::Kept), "the contents of this request are held");
        let Want::Read(later) = want(&mut window, 11) else {
            panic!("another hour reads again");
        };
        window.dirty();
        let Want::Read(changed) = want(&mut window, 10) else {
            panic!("a changed setting reads again at the same hour");
        };
        assert_ne!(later, changed, "a read under the old setting is not taken in");
        window.close();
        assert!(matches!(want(&mut window, 10), Want::Closed));
    }

    fn at(hour: i32) -> Option<OnScreen> {
        Some(OnScreen { branch: 0, hour, seek: 0 })
    }

    fn made_at(window: &mut Window, hour: i32, seek: u64, nato: bool) -> Product {
        let family = window.ask(&Choice::default(), nato).expect("an open window asks");
        let fresh = Fresh { air: None, now: scope(0, hour, 1), covered: false, read: Read::Hold { combat: 2, undecided: Some("its engaged flag is unknown") }, of: None };
        Product { key: ReadKey(family), branch: 0, hour, seek, fresh: Ok(fresh) }
    }

    fn made(window: &mut Window, hour: i32) -> Product {
        made_at(window, hour, 0, false)
    }

    fn shown_on(window: &mut Window, made: Option<Product>, on: Option<OnScreen>) -> Option<i32> {
        if let Some(made) = made {
            window.offer(made);
        }
        match window.shown(on).expect("no read failed") {
            Prepared::Read(fresh) => {
                let replaced = window.cached.replace(Cached { at: fresh.now.clone(), covered: false, contents: None, shown: false, held: None, generation: 1, gui_assets: None, of: fresh.of.clone() });
                window.retire(replaced);
                Some(fresh.now.hour)
            }
            Prepared::Kept => window.model().map(|(hour, _)| hour),
            Prepared::Loading => None,
            Prepared::Closed => panic!("the window is open"),
        }
    }

    fn shown_hour(window: &mut Window, made: Option<Product>, hour: i32) -> Option<i32> {
        shown_on(window, made, at(hour))
    }

    #[test]
    fn a_read_of_an_older_hour_is_never_shown_over_a_newer_map() {
        let mut window = Window::default();
        window.open(Selection::Naval(2));
        let ten = made(&mut window, 10);
        assert_eq!(shown_hour(&mut window, Some(ten), 10), Some(10), "the read of the hour on screen is shown");
        assert_eq!(shown_hour(&mut window, None, 10), Some(10), "and held while the map stands at its hour");
        assert_eq!(shown_hour(&mut window, None, 11), None, "the map moved on: loading, not hour 10's read");
        let late = made(&mut window, 10);
        assert_eq!(shown_hour(&mut window, Some(late), 11), None, "hour 10's read, late, is never shown over hour 11");
        assert!(window.pending.is_empty(), "and is let go: the map never comes back to it");
        let eleven = made(&mut window, 11);
        assert_eq!(shown_hour(&mut window, Some(eleven), 11), Some(11), "hour 11's read is");
        let stale = made(&mut window, 12);
        window.dirty();
        let _ = window.ask(&Choice::default(), false);
        assert_eq!(shown_hour(&mut window, Some(stale), 12), None, "a read under a setting since changed is not shown");
    }

    #[test]
    fn a_read_of_an_hour_the_map_has_not_reached_waits_for_it_and_is_shown_there() {
        let mut window = Window::default();
        window.open(Selection::Naval(2));
        let ten = made(&mut window, 10);
        assert_eq!(shown_hour(&mut window, Some(ten), 10), Some(10));
        let eleven = made(&mut window, 11);
        assert_eq!(shown_hour(&mut window, Some(eleven), 10), Some(10), "the map still shows hour 10: hour 11's read is not shown early");
        assert_eq!(shown_hour(&mut window, None, 10), Some(10), "nor on the next frame at hour 10");
        assert_eq!(shown_hour(&mut window, None, 11), Some(11), "the map reaches hour 11: the read held for it is shown");
        let (twelve, thirteen) = (made(&mut window, 12), made(&mut window, 13));
        window.offer(twelve);
        window.offer(thirteen);
        assert_eq!(shown_hour(&mut window, None, 11), Some(11), "two reads ahead of the map: neither is shown early");
        assert_eq!(shown_hour(&mut window, None, 12), Some(12), "hour 13's read did not replace hour 12's, which the map reaches first");
        assert_eq!(shown_hour(&mut window, None, 13), Some(13), "and hour 13's is still held for its hour");
    }

    #[test]
    fn reads_offered_while_the_display_stands_still_do_not_pile_up() {
        let mut window = Window::default();
        window.open(Selection::Naval(2));
        assert_eq!(shown_hour(&mut window, None, 10), None);
        for _ in 0..100 {
            let read = made(&mut window, 11);
            window.offer(read);
        }
        assert_eq!(window.pending.len(), 1, "one read of hour 11");
        for nato in (0..100).map(|at| at % 2 == 1) {
            let read = made_at(&mut window, 12, 0, nato);
            window.offer(read);
        }
        assert_eq!(window.pending.len(), 1, "only the newest asking's read: hour 11's went with its asking");
        assert_eq!(shown_hour(&mut window, None, 12), Some(12));
    }

    #[test]
    fn opening_and_closing_keep_what_the_window_was_given_once() {
        let mut window = Window::default();
        window.set_settled(true);
        window.set_navy_sources_of(Arc::new(|_: &replay_frame::frame::DecodedFrame, _, _| Ok(BTreeMap::new())));
        window.open(Selection::Naval(2));
        window.open(Selection::NavalRegion(4));
        window.close();
        window.open(Selection::State(9));
        assert!(window.settled, "still drawing saved pictures");
        assert!(window.navy_sources_of.is_some(), "still holding the content's provider");
    }

    #[test]
    fn a_window_that_draws_nothing_says_why() {
        let mut window = Window::default();
        window.open(Selection::Land(7));
        window.cached = Some(Cached { at: scope(0, 10, 1), covered: false, contents: None, shown: false, held: None, generation: 1, gui_assets: None, of: None });
        assert!(window.not_shown().is_some_and(|why| why.contains("hour 10") && why.contains("Land(7)") && why.contains("not engaged")), "{:?}", window.not_shown());
        window.cached.as_mut().unwrap().held = Some("Naval combat 2: at hour 10 its engaged flag is unknown".into());
        assert_eq!(window.not_shown().as_deref(), Some("Naval combat 2: at hour 10 its engaged flag is unknown"));
        window.close();
        assert!(window.not_shown().is_some(), "closed, it draws nothing");
    }

    #[test]
    fn a_naval_region_read_derives_its_sources_from_its_own_rows_and_viewer() {
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let of: NavySourcesOf = {
            let asked = Arc::clone(&asked);
            Arc::new(move |frame: &replay_frame::frame::DecodedFrame, ours, region| {
                asked.lock().unwrap().push((frame.hour, ours, region));
                Err(format!("the sources of hour {} for slot {ours} in region {region}", frame.hour))
            })
        };
        let frame = viewer_foundation::test_support::complete_frame(12, |table| replay_data::rows::sample_extended_table(table, 0));
        assert_eq!(navy_sources(Selection::NavalRegion(7), Some(&of), &frame, 3), Err("the sources of hour 12 for slot 3 in region 7".into()));
        assert_eq!(*asked.lock().unwrap(), [(12, 3, 7)], "derived once, from the read's own hour, viewer and region");
        assert_eq!(navy_sources(Selection::Land(1), Some(&of), &frame, 3), Ok(BTreeMap::new()), "another window derives none");
        assert_eq!(asked.lock().unwrap().len(), 1);
        assert!(navy_sources(Selection::NavalRegion(7), None, &frame, 3).is_err_and(|why| why.contains("not given")), "refused by name without the map's content");
    }

    #[test]
    fn a_loading_window_reports_nothing_of_the_hour_before() {
        let mut window = Window::default();
        window.open(Selection::Naval(2));
        window.cached = Some(Cached { at: scope(0, 10, 1), covered: false, contents: None, shown: false, held: Some("hour 10's combat is held".into()), generation: 1, gui_assets: None, of: None });
        window.refused.insert("hour 10's drawing refused a control".into(), 1);
        assert_eq!(window.warnings(&[]).len(), 2, "showing hour 10, its warnings are reported");
        window.loading = true;
        assert!(window.warnings(&["the combat artwork".into()]).is_empty(), "loading hour 11, none of hour 10's");
    }

    #[test]
    fn a_changed_asking_at_the_same_hour_is_loading_never_the_contents_of_the_one_before() {
        let mut window = Window::default();
        window.open(Selection::Naval(2));
        let ten = made(&mut window, 10);
        assert_eq!(shown_hour(&mut window, Some(ten), 10), Some(10));
        let _ = window.ask(&Choice::default(), true);
        assert_eq!(shown_hour(&mut window, None, 10), None, "NATO naming changed, paused at hour 10: loading, not the old contents");
        let named = made_at(&mut window, 10, 0, true);
        assert_eq!(shown_hour(&mut window, Some(named), 10), Some(10), "the new asking's read is shown");
        assert_eq!(shown_hour(&mut window, None, 10), Some(10), "and held under it");
        let (pending, late) = (made_at(&mut window, 10, 0, true), made_at(&mut window, 10, 0, true));
        window.offer(pending);
        window.content_replaced();
        assert_eq!(shown_hour(&mut window, None, 10), None, "the map installed again: neither its contents nor a read pending from before");
        assert_eq!(shown_hour(&mut window, Some(late), 10), None, "nor a read made from the content before, handed after");
    }

    #[test]
    fn a_read_or_contents_of_an_older_seek_are_never_shown_at_the_same_hour() {
        let mut window = Window::default();
        window.open(Selection::Naval(2));
        let on = |seek| Some(OnScreen { branch: 0, hour: 20, seek });
        let before = made_at(&mut window, 20, 1, false);
        assert_eq!(shown_on(&mut window, Some(before), on(1)), Some(20));
        assert_eq!(shown_on(&mut window, None, on(3)), None, "sought away and back to hour 20: the contents of the seek before are not shown");
        let late = made_at(&mut window, 20, 1, false);
        assert_eq!(shown_on(&mut window, Some(late), on(3)), None, "nor a read of hour 20 made before the seek");
        let after = made_at(&mut window, 20, 3, false);
        assert_eq!(shown_on(&mut window, Some(after), on(3)), Some(20), "the read of this seek is shown");
        assert_eq!(shown_on(&mut window, None, on(3)), Some(20), "and held under it");
    }

    #[test]
    fn the_window_frees_no_read_and_no_contents_itself() {
        let mut window = Window::default();
        window.open(Selection::Naval(2));
        let (ten, eleven) = (made(&mut window, 10), made(&mut window, 11));
        window.offer(ten);
        window.offer(eleven);
        assert!(window.retired.is_empty(), "reads ahead of the map wait for their hours");
        assert_eq!(shown_hour(&mut window, None, 11), Some(11));
        assert_eq!(window.retired.len(), 1, "the read of the hour the map passed");
        let twelve = made(&mut window, 12);
        assert_eq!(shown_hour(&mut window, Some(twelve), 12), Some(12));
        assert_eq!(window.retired.len(), 2, "and the contents the newer read replaced");
        let late = made(&mut window, 12);
        assert_eq!(shown_hour(&mut window, Some(late), 13), None);
        assert_eq!(window.retired.len(), 3, "a read the map passed");
        assert!(window.take_retired().is_some());
        assert!(window.take_retired().is_none(), "taken once");
        let pending = made(&mut window, 14);
        window.offer(pending);
        window.open(Selection::Naval(3));
        assert_eq!(window.retired.len(), 2, "opened on another combat: the contents and the read it held");
        let read = made(&mut window, 15);
        assert_eq!(shown_hour(&mut window, Some(read), 15), Some(15));
        let orphan = made(&mut window, 16);
        window.close();
        assert_eq!(window.retired.len(), 3, "closed: the contents it held");
        let _ = window.take_retired();
        window.offer(orphan);
        assert_eq!(window.shown(at(16)).ok().map(|prepared| matches!(prepared, Prepared::Closed)), Some(true));
        assert_eq!(window.retired.len(), 1, "a read offered to a closed window");
    }

    #[test]
    fn a_window_opened_mid_play_is_loading_until_the_read_of_the_hour_on_screen() {
        let mut window = Window::default();
        window.open(Selection::Naval(2));
        let before = made(&mut window, 7);
        window.open(Selection::Naval(2));
        let _ = window.ask(&Choice::default(), false);
        assert_eq!(shown_hour(&mut window, Some(before), 7), None, "a read asked by the window before it opened again is not this one's");
        assert_eq!(shown_hour(&mut window, None, 7), None, "loading until its first read");
        assert_eq!(shown_hour(&mut window, None, 8), None, "still loading as the map moves on");
        let eight = made(&mut window, 8);
        assert_eq!(shown_hour(&mut window, Some(eight), 8), Some(8), "then the read of the hour on screen");
    }

    fn roster_recording() -> Arc<replay::campaign::Replay> {
        use replay_capture_spool::spool::Record;
        use replay_data::identity::{CaptureMode, Identity, Roster, SampleBoundary, SessionStart, Verification};
        use replay_data::rows::{AirDailyRosterRow, CampaignRow, CountryLinkRow, TableId, TableRows};
        use replay_frame::branches::{BranchReason, BranchStart, Parentage};
        use replay_frame::frame::{DecodedFrame, FrameEncoder, Metrics, TableView};
        let identity = Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified { launcher: Roster::default(), memory: Roster::default() },
        };
        let mut records = vec![
            Record::SessionStart(SessionStart { id: 1, process_id: 1, process_start_time: 1, qpc_frequency: 10_000_000, sample_boundary: SampleBoundary::EndOfHour, capture_mode: CaptureMode::Full, identity }),
            Record::BranchStart(BranchStart { id: 1, session: 1, loaded_hour: Some(19), first_hour: 20, parent: Parentage::Unknown, reason: BranchReason::SessionStart }),
        ];
        let mut encoder = FrameEncoder::new();
        for hour in 20..=50 {
            let mut tables: [TableView; TableId::ALL.len()] = std::array::from_fn(|_| TableView::Unavailable { reason: "outside the roster fixture".into() });
            for rows in [
                TableRows::Campaign(vec![CampaignRow { id: b"campaign".to_vec() }]),
                TableRows::CountryLinks(vec![CountryLinkRow { id: 7, canonical_slot: 5 }]),
                TableRows::AirDailyRoster(vec![AirDailyRosterRow { country: 7, eligible: true }]),
            ] {
                let index = rows.table().index();
                tables[index] = TableView::Complete(rows);
            }
            let frame = DecodedFrame { branch: 1, hour, tables: tables.map(Into::into), metrics: Metrics::default() };
            records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
        }
        Arc::new(replay::campaign::Replay::from_records(recorder_derive::writers(), b"campaign".to_vec(), &[records]).unwrap())
    }

    #[test]
    fn an_air_read_takes_the_daily_pass_of_its_own_hour_not_the_hour_it_was_asked_at() {
        let rosters = Arc::new(Sampler::today(roster_recording(), None));
        let mut window = Window::default();
        window.open(Selection::AirRegion(3));
        let asked = window.air_asking(&rosters);
        assert_eq!(asked.at(1, 47, 5).pass, Ok(Some(24)), "hour 47 is under the pass of hour 24");
        assert_eq!(asked.at(1, 48, 5).pass, Ok(Some(48)), "hour 48 is under its own day's pass");
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
