
#[cfg(test)]
mod tests;

use super::{
    Viewer,
    hover_rows::{Head, Loading},
};
use crate::{
    document::Document,
    playhead_host::views::AtHead,
};
use eframe::egui;
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use std::{
    io,
    sync::{Arc, Weak},
};
use viewer_runtime::reads::Reads;
use viewer_combat::{Prepared, Selector, division_panel, land_marker_tooltips, naval_marker_tooltips, terrain_mode};
use viewer_engagements::engagement_history::History;
use viewer_foundation::{perspective::Vision, tooltip_text::Tooltip};
use viewer_raids::map::raids::tooltips::{Key, lane::Lane};

#[derive(Clone, Debug)]
pub(crate) struct Hovered {
    target: [u64; 3],
    exact: u64,
    frame: Weak<DecodedFrame>,
}

impl PartialEq for Hovered {
    fn eq(&self, other: &Self) -> bool {
        self.target == other.target && self.exact == other.exact && Weak::ptr_eq(&self.frame, &other.frame)
    }
}

pub(crate) struct Hover<T> {
    name: &'static str,
    at: AtHead,
    reads: Option<Reads<Hovered, T>>,
    loading: Loading,
    asked: bool,
    times: Option<Vec<u64>>,
}

impl<T: Clone + Send + 'static> Hover<T> {
    fn new(name: &'static str) -> Self {
        Self { name, at: AtHead::learning(name), reads: None, loading: Loading::default(), asked: false, times: None }
    }

    fn ask(&mut self, views: Head<'_>, (target, exact): ([u64; 3], impl FnOnce(&DecodedFrame) -> u64), build: impl FnOnce(&DecodedFrame) -> T + Send + 'static, wake: impl Fn() + Send + 'static) -> Option<T> {
        self.asked = true;
        let frame = {
            let missed = self.reads.as_mut().map(Reads::take_missed).unwrap_or_default();
            if !missed.is_empty() {
                self.at.learn(&missed);
                self.loading.learned(&missed);
            }
            let Some(rows) = self.at.frame(views) else {
                self.loading.no_rows();
                return self.latest(target);
            };
            self.loading.joined();
            rows
        };
        let key = Hovered { target, exact: exact(&frame), frame: Arc::downgrade(&frame) };
        let (name, timing) = (self.name, self.times.is_some());
        let reads = self.reads.get_or_insert_with(|| {
            let reads = Reads::new(name);
            if timing {
                reads.time();
            }
            reads
        });
        reads.ask(key, move || build(&frame), wake);
        self.latest(target)
    }

    fn latest(&self, target: [u64; 3]) -> Option<T> {
        self.reads.as_ref()?.latest(|done| done.target == target).map(|(_, done)| done)
    }

    fn end_frame(&mut self) {
        if !std::mem::take(&mut self.asked) {
            self.loading = Loading::default();
            if let Some(reads) = self.reads.take()
                && let Some(times) = &mut self.times
            {
                times.extend(reads.times());
            }
        }
        self.at.end_frame();
    }

    fn time(&mut self) {
        self.times.get_or_insert_with(Vec::new);
        if let Some(reads) = &self.reads {
            reads.time();
        }
    }

    fn times(&self) -> Vec<u64> {
        let mut times = self.times.clone().unwrap_or_default();
        times.extend(self.reads.as_ref().map(Reads::times).unwrap_or_default());
        times
    }
}

pub(crate) struct CombatRows {
    window_loading: Option<String>,
    window_asked: bool,
    pub pending: Option<Selector>,
    pub(crate) selector_refused: Option<String>,
    pub(crate) selector_waits: u32,
    selector: AtHead,
    land_marker: Hover<Arc<Tooltip>>,
    naval_marker: Hover<Arc<Tooltip>>,
    division_panel: Hover<Arc<Tooltip>>,
    extended: Option<(Arc<Tooltip>, Arc<Tooltip>, Arc<Tooltip>)>,
    terrain: Hover<Arc<Tooltip>>,
    infrastructure: Hover<Result<i32, String>>,
    raid: AtHead,
    raid_lane: Lane,
    raid_asked: bool,
    raid_loading: Loading,
    raid_shown: Option<(Key, u32, bool, Arc<Tooltip>)>,
    engaged: Option<Arc<History>>,
}

impl Default for CombatRows {
    fn default() -> Self {
        Self {
            window_loading: None,
            window_asked: false,
            pending: None,
            selector_refused: None,
            selector_waits: 0,
            selector: AtHead::new("combat selector", &[TableId::LandCombats, TableId::NavalCombats, TableId::NavalResults], crate::playhead_host::views::Pace::Step),
            land_marker: Hover::new("land marker hover"),
            naval_marker: Hover::new("naval marker hover"),
            division_panel: Hover::new("division panel"),
            extended: None,
            terrain: Hover::new("terrain hover"),
            infrastructure: Hover::new("infrastructure hover"),
            raid: AtHead::learning("raid tooltip"),
            raid_lane: Lane::default(),
            raid_asked: false,
            raid_loading: Loading::default(),
            raid_shown: None,
            engaged: None,
        }
    }
}

impl CombatRows {
    pub(crate) fn end_frame(&mut self) {
        for at in [&mut self.selector, &mut self.raid] {
            at.end_frame();
        }
        if !std::mem::take(&mut self.window_asked) {
            self.window_loading = None;
        }
        for hover in [&mut self.land_marker, &mut self.naval_marker, &mut self.division_panel, &mut self.terrain] {
            hover.end_frame();
        }
        self.infrastructure.end_frame();
        if self.division_panel.reads.is_none() {
            self.extended = None;
        }
        if !std::mem::take(&mut self.raid_asked) {
            self.raid_lane = Lane::default();
            self.raid_loading = Loading::default();
            self.raid_shown = None;
        }
    }

    pub(crate) fn loading(&self) -> Vec<(String, usize)> {
        let hovers = [&self.land_marker, &self.naval_marker, &self.division_panel, &self.terrain].into_iter().filter_map(|hover| hover.loading.reason(&format!("the {}", hover.name)));
        let more = [self.window_loading.as_ref().map(|state| format!("the combat window: loading: {state}")), self.infrastructure.loading.reason("the infrastructure hover"), self.raid_loading.reason("the raid tooltip")];
        hovers.chain(more.into_iter().flatten()).map(|reason| (reason, 1)).collect()
    }

    pub(crate) fn learned(&mut self, name: &str, tables: &[TableId]) {
        let at = match name {
            "land_marker" => &mut self.land_marker.at,
            "naval_marker" => &mut self.naval_marker.at,
            "division_panel" => &mut self.division_panel.at,
            "terrain" => &mut self.terrain.at,
            "infrastructure" => &mut self.infrastructure.at,
            _ => return,
        };
        at.learn(tables);
    }

    pub(crate) fn time(&mut self) {
        for hover in [&mut self.land_marker, &mut self.naval_marker, &mut self.division_panel, &mut self.terrain] {
            hover.time();
        }
        self.infrastructure.time();
    }

    pub(crate) fn times(&self) -> [(&'static str, Vec<u64>); 5] {
        [
            ("land_marker", self.land_marker.times()),
            ("naval_marker", self.naval_marker.times()),
            ("division_panel", self.division_panel.times()),
            ("terrain", self.terrain.times()),
            ("infrastructure", self.infrastructure.times()),
        ]
    }

    #[expect(clippy::too_many_arguments, reason = "the tooltip's inputs, each from another owner")]
    pub(crate) fn land_marker(&mut self, head: Head<'_>, document: &Document, viewer: u32, id: u64, index: Option<Arc<History>>, nato: bool, wake: impl Fn() + Send + 'static) -> Option<Arc<Tooltip>> {
        self.land_marker_from(head, document, viewer, id, index, nato, wake)
    }

    #[expect(clippy::too_many_arguments, reason = "the tooltip's inputs, each from another owner")]
    fn land_marker_from(&mut self, from: Head<'_>, document: &Document, viewer: u32, id: u64, index: Option<Arc<History>>, nato: bool, wake: impl Fn() + Send + 'static) -> Option<Arc<Tooltip>> {
        let (content, covering) = (Arc::clone(&document.combat), index.clone());
        self.land_marker.ask(from, ([u64::from(viewer), id, u64::from(nato)], |frame| covered(covering.as_deref(), frame)), move |frame| Arc::new(land_marker_tooltips::build(&content, frame, viewer, id, index.as_deref(), nato)), wake)
    }

    pub(crate) fn naval_marker(&mut self, head: Head<'_>, document: &Document, viewer: u32, id: u64, index: Option<Arc<History>>, wake: impl Fn() + Send + 'static) -> Option<Arc<Tooltip>> {
        self.naval_marker_from(head, document, viewer, id, index, wake)
    }

    fn naval_marker_from(&mut self, from: Head<'_>, document: &Document, viewer: u32, id: u64, index: Option<Arc<History>>, wake: impl Fn() + Send + 'static) -> Option<Arc<Tooltip>> {
        let (content, covering) = (Arc::clone(&document.combat), index.clone());
        self.naval_marker.ask(from, ([u64::from(viewer), id, 0], |frame| covered(covering.as_deref(), frame)), move |frame| Arc::new(naval_marker_tooltips::build(&content, frame, viewer, id, index.as_deref())), wake)
    }

    pub(crate) fn division_panel(&mut self, head: Head<'_>, document: &Document, base: Arc<Tooltip>, id: u64, wake: impl Fn() + Send + 'static) -> Arc<Tooltip> {
        self.division_panel_from(head, document, base, id, wake)
    }

    fn division_panel_from(&mut self, from: Head<'_>, document: &Document, base: Arc<Tooltip>, id: u64, wake: impl Fn() + Send + 'static) -> Arc<Tooltip> {
        let content = Arc::clone(&document.combat);
        let Some(panel) = self.division_panel.ask(from, ([id, 0, 0], |_| 0), move |frame| Arc::new(division_panel(&content, frame, id)), wake) else {
            return base;
        };
        if let Some((held_base, held_panel, text)) = &self.extended
            && Arc::ptr_eq(held_base, &base)
            && Arc::ptr_eq(held_panel, &panel)
        {
            return Arc::clone(text);
        }
        let text = Arc::new(extend(&base, &panel));
        self.extended = Some((base, panel, Arc::clone(&text)));
        text
    }

    pub(crate) fn terrain(&mut self, head: Head<'_>, document: &Document, province: u32, vision: &Arc<Vision>, wake: impl Fn() + Send + 'static) -> Option<Arc<Tooltip>> {
        self.terrain_from(head, document, province, vision, wake)
    }

    fn terrain_from(&mut self, from: Head<'_>, document: &Document, province: u32, vision: &Arc<Vision>, wake: impl Fn() + Send + 'static) -> Option<Arc<Tooltip>> {
        let (content, vision) = (Arc::clone(&document.combat), Arc::clone(vision));
        self.terrain.ask(from, ([u64::from(province), u64::from(vision.ours()), 0], |_| 0), move |frame| Arc::new(terrain_mode::hover(&content, frame, province, &vision)), wake)
    }

    pub(crate) fn infrastructure(&mut self, head: Head<'_>, document: &Document, province: u32, wake: impl Fn() + Send + 'static) -> Result<i32, String> {
        self.infrastructure_from(head, document, province, wake)
    }

    fn infrastructure_from(&mut self, from: Head<'_>, document: &Document, province: u32, wake: impl Fn() + Send + 'static) -> Result<i32, String> {
        let modes = Arc::clone(&document.map_modes);
        let level = self.infrastructure.ask(from, ([u64::from(province), 0, 0], |_| 0), move |frame| modes.infrastructure.hover_level(frame, province), wake);
        level.unwrap_or_else(|| {
            Err(match self.infrastructure.loading.state() {
                Some(loading) => format!("the infrastructure level of province {province} is loading: {loading}"),
                None => format!("the infrastructure level of province {province} is being read"),
            })
        })
    }

    pub(crate) fn raid(&mut self, views: Head<'_>, document: &Document, ours: u32, nato: bool, key: Key, ctx: &egui::Context) -> Option<Arc<Tooltip>> {
        self.raid_asked = true;
        let missed = self.raid_lane.take_missed();
        if !missed.is_empty() {
            self.raid.learn(&missed);
            self.raid_loading.learned(&missed);
        }
        let Some(frame) = self.raid.frame(views) else {
            self.raid_loading.no_rows();
            return self.raid_shown.as_ref().filter(|(held, viewer, naming, _)| *held == key && *viewer == ours && *naming == nato).map(|(.., text)| Arc::clone(text));
        };
        self.raid_loading.joined();
        let text = self.raid_lane.request(Arc::clone(&document.raid_hover), frame, ours, nato, key.clone(), ctx);
        if let Some(text) = &text {
            self.raid_shown = Some((key, ours, nato, Arc::clone(text)));
        }
        text
    }
}

fn covered(index: Option<&History>, frame: &DecodedFrame) -> u64 {
    u64::from(index.is_some_and(|index| index.covers(frame.branch, frame.hour)))
}

fn extend(base: &Tooltip, panel: &Tooltip) -> Tooltip {
    let mut text = base.clone();
    text.line(true, 2, "division panel", Ok(panel.body.clone()));
    for (reason, count) in &panel.refused {
        *text.refused.entry(reason.clone()).or_default() += count;
    }
    text
}

impl Viewer {
    pub(super) fn combat_on_screen(&self) -> Option<viewer_combat::OnScreen> {
        let side = super::live_of(&self.playhead_map)?;
        let views = side.views();
        let hour = views.shown_through().or(side.applied())?;
        Some(viewer_combat::OnScreen { branch: side.branch()?, hour, seek: views.seek() })
    }

    pub(super) fn combat_prepared(&mut self) -> io::Result<Prepared> {
        if let Some(selector) = self.combat_rows.pending.take() {
            let resolved = match selector.direct() {
                Some(selection) => Some(Ok(selection)),
                None => self.rows_head().and_then(|views| {
                    let at = &mut self.combat_rows.selector;
                    at.frame(&views).and_then(|rows| at.discover(|| selector.resolve(&rows)))
                }),
            };
            match resolved {
                None => self.combat_rows.pending = Some(selector),
                Some(Ok(selection)) => {
                    self.combat.open(selection);
                    self.combat.configure_air(self.options.air_details, self.options.air_range, self.options.air_channels);
                }
                Some(Err(error)) => {
                    self.combat_rows.selector_refused = Some(error.to_string());
                    self.error = Some(error.to_string());
                }
            }
        }
        let on_screen = self.combat_on_screen();
        let prepared = self.combat.shown(on_screen);
        if let Some(retired) = self.combat.take_retired() {
            self.retire(retired);
        }
        let prepared = prepared?;
        if matches!(prepared, Prepared::Closed) {
            return Ok(prepared);
        }
        self.combat_rows.window_asked = true;
        let waiting = match super::live_of(&self.playhead_map).map(|side| side.products()) {
            None => "the map's side".to_owned(),
            Some(products) if !products.combat_loading.is_empty() => products.combat_loading.join("; "),
            Some(_) => format!("its read of hour {}", on_screen.map_or_else(|| "none yet".to_owned(), |at| at.hour.to_string())),
        };
        self.combat_rows.window_loading = matches!(prepared, Prepared::Loading).then_some(waiting);
        Ok(prepared)
    }

    pub(super) fn layer_history(&self) -> Option<Arc<History>> {
        let side = self.open_map()?;
        side.products().engaged.clone().or_else(|| self.combat_rows.engaged.clone())
    }

    pub(super) fn layer_engaged(&mut self) -> Option<Arc<History>> {
        let history = self.layer_history();
        self.combat_rows.engaged.clone_from(&history);
        history
    }
}
