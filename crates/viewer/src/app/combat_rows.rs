//! The combat window (land, naval and air region), the land and naval marker tooltips, the
//! division panel, the terrain and infrastructure hovers and the raid tooltip read rows at the
//! playback head while the playhead's map is open. Each learns its tables by reading, asks at most
//! four times a second while playing, and is closed by a frame that did not ask it. While its rows
//! do not hold a table it reads, it shows what it showed and says it is loading, by name. The
//! window's read and the hovers are made on workers (a read takes up to hundreds of milliseconds
//! late in the war); the frame thread only asks and takes results in, and shows what it holds
//! until a newer one is there.

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
use viewer_combat::{Prepared, Selector, division_panel, land_marker_tooltips, naval_marker_tooltips, reads::Reads, terrain_mode};
use viewer_foundation::{
    engagement_history::History,
    map::perspective::{Choice, Vision},
    tooltip_text::Tooltip,
};
use viewer_raids::map::raids::tooltips::{Key, lane::Lane};

/// A hover's read: what it is of (a combat and viewing country, a division, a province) and the
/// settings it shows, and the frame it read, known by identity, which keeps none of its rows alive.
#[derive(Clone, Debug)]
pub(crate) struct Hovered {
    target: [u64; 3],
    /// Inputs that make another text of the same hover (the engaged index's coverage).
    exact: u64,
    frame: Weak<DecodedFrame>,
}

impl PartialEq for Hovered {
    fn eq(&self, other: &Self) -> bool {
        self.target == other.target && self.exact == other.exact && Weak::ptr_eq(&self.frame, &other.frame)
    }
}

/// A hover made on its worker, which ends with the hover so it holds no frame.
pub(crate) struct Hover<T> {
    name: &'static str,
    at: AtHead,
    reads: Option<Reads<Hovered, T>>,
    loading: Loading,
    asked: bool,
    /// Each build's nanoseconds on its workers, while timing (the proof tool).
    times: Option<Vec<u64>>,
}

impl<T: Clone + Send + 'static> Hover<T> {
    fn new(name: &'static str) -> Self {
        Self { name, at: AtHead::learning(name), reads: None, loading: Loading::default(), asked: false, times: None }
    }

    /// The hover's newest result for `target` at any hour, while the one over the newest rows is
    /// made; none before the first. Rows at the head that lack a table its build read make no
    /// result: the table is learned and the hover is loading until it joins.
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

/// Each reader's rows at the head with what it made from them.
pub(crate) struct CombatRows {
    window: AtHead,
    window_loading: Loading,
    window_asked: bool,
    /// The combat named at load (`--combat`), opened on the first frame that resolves it.
    pub pending: Option<Selector>,
    selector: AtHead,
    land_marker: Hover<Arc<Tooltip>>,
    naval_marker: Hover<Arc<Tooltip>>,
    /// The division panel's own part; the land counter's tooltip it extends, the panel, and the
    /// two together, kept while both stay.
    division_panel: Hover<Arc<Tooltip>>,
    extended: Option<(Arc<Tooltip>, Arc<Tooltip>, Arc<Tooltip>)>,
    terrain: Hover<Arc<Tooltip>>,
    infrastructure: Hover<Result<i32, String>>,
    raid: AtHead,
    /// The raid tooltip's worker, ended with its hover so it holds no frame.
    raid_lane: Lane,
    raid_asked: bool,
    raid_loading: Loading,
    /// The raid text last shown, for its icon, viewer and naming, kept while its rows load.
    raid_shown: Option<(Key, u32, bool, Arc<Tooltip>)>,
    /// The open map's engaged index, kept while its layers compose.
    engaged: Option<Arc<History>>,
}

impl Default for CombatRows {
    fn default() -> Self {
        Self {
            window: AtHead::learning("combat window"),
            window_loading: Loading::default(),
            window_asked: false,
            pending: None,
            selector: AtHead::learning("combat selector"),
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
    /// Closes every reader a frame did not ask.
    pub(crate) fn end_frame(&mut self) {
        for at in [&mut self.window, &mut self.selector, &mut self.raid] {
            at.end_frame();
        }
        if !std::mem::take(&mut self.window_asked) {
            self.window_loading = Loading::default();
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

    /// Every open reader's loading state, named for the window's refusals.
    pub(crate) fn loading(&self) -> Vec<(String, usize)> {
        let hovers = [&self.land_marker, &self.naval_marker, &self.division_panel, &self.terrain].into_iter().filter_map(|hover| hover.loading.reason(&format!("the {}", hover.name)));
        let more = [self.window_loading.reason("the combat window"), self.infrastructure.loading.reason("the infrastructure hover"), self.raid_loading.reason("the raid tooltip")];
        hovers.chain(more.into_iter().flatten()).map(|reason| (reason, 1)).collect()
    }

    /// The window's rows at the head; none, loading by name, while they do not hold what it reads.
    pub(crate) fn window_rows(&mut self, views: Head<'_>) -> Option<Arc<DecodedFrame>> {
        self.window_asked = true;
        let rows = self.window.frame(views);
        match rows {
            Some(_) => self.window_loading.joined(),
            None => self.window_loading.no_rows(),
        }
        rows
    }

    /// Gives reader `name` the tables a run learned before, so it opens with them (the proof tool).
    pub(crate) fn learned(&mut self, name: &str, tables: &[TableId]) {
        let at = match name {
            "window" => &mut self.window,
            "land_marker" => &mut self.land_marker.at,
            "naval_marker" => &mut self.naval_marker.at,
            "division_panel" => &mut self.division_panel.at,
            "terrain" => &mut self.terrain.at,
            "infrastructure" => &mut self.infrastructure.at,
            _ => return,
        };
        at.learn(tables);
    }

    /// Times the hovers' builds on their workers from now on (the proof tool).
    pub(crate) fn time(&mut self) {
        for hover in [&mut self.land_marker, &mut self.naval_marker, &mut self.division_panel, &mut self.terrain] {
            hover.time();
        }
        self.infrastructure.time();
    }

    /// Each hover build's nanoseconds on its worker, by reader name.
    pub(crate) fn times(&self) -> [(&'static str, Vec<u64>); 5] {
        [
            ("land_marker", self.land_marker.times()),
            ("naval_marker", self.naval_marker.times()),
            ("division_panel", self.division_panel.times()),
            ("terrain", self.terrain.times()),
            ("infrastructure", self.infrastructure.times()),
        ]
    }

    /// Declares the tables the window's worker read undeclared; the window is loading until they
    /// join.
    pub(crate) fn window_missed(&mut self, missed: &[TableId]) {
        if !missed.is_empty() {
            self.window.learn(missed);
            self.window_loading.learned(missed);
        }
    }

    #[allow(clippy::too_many_arguments, reason = "the tooltip's inputs, each from another owner")]
    pub(crate) fn land_marker(&mut self, head: Head<'_>, document: &Document, viewer: u32, id: u64, index: Option<Arc<History>>, nato: bool, wake: impl Fn() + Send + 'static) -> Option<Arc<Tooltip>> {
        self.land_marker_from(head, document, viewer, id, index, nato, wake)
    }

    #[allow(clippy::too_many_arguments, reason = "the tooltip's inputs, each from another owner")]
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

    /// The land counter's tooltip `base` with the division panel of its one division `id`, once
    /// the panel is made; `base` alone before.
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

    /// The infrastructure mode's level at `province`; refused by name until the first read.
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

    /// The raid tooltip, built on its worker from rows at the head; none until the first build for
    /// this icon is done. While its rows load, the text last shown for the icon.
    #[allow(clippy::too_many_arguments, reason = "the tooltip's inputs, each from another owner")]
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

/// Whether the engaged index covers the hour of `frame`, which a marker's text depends on.
fn covered(index: Option<&History>, frame: &DecodedFrame) -> u64 {
    u64::from(index.is_some_and(|index| index.covers(frame.branch, frame.hour)))
}

/// The land counter's tooltip with the division panel's lines and refusals added.
fn extend(base: &Tooltip, panel: &Tooltip) -> Tooltip {
    let mut text = base.clone();
    text.line(true, 2, "division panel", Ok(panel.body.clone()));
    for (reason, count) in &panel.refused {
        *text.refused.entry(reason.clone()).or_default() += count;
    }
    text
}

#[cfg(test)]
impl CombatRows {
    pub(crate) fn window(&mut self) -> &mut AtHead {
        &mut self.window
    }

    /// What the window is loading, none when it is not.
    pub(crate) fn window_loading(&self) -> Option<&str> {
        self.window_loading.state()
    }

    /// The readers open now, and the marker workers running.
    pub(crate) fn open(&self) -> (usize, usize) {
        let readers = [&self.window, &self.selector, &self.land_marker.at, &self.naval_marker.at, &self.division_panel.at, &self.terrain.at, &self.infrastructure.at, &self.raid].into_iter().filter(|at| at.is_open()).count();
        let workers = [&self.land_marker, &self.naval_marker, &self.division_panel, &self.terrain].into_iter().filter(|hover| hover.reads.is_some()).count() + usize::from(self.infrastructure.reads.is_some());
        (readers, workers)
    }
}

impl Viewer {
    /// The combat window's read for this frame, asked of its worker over rows at the head of the
    /// open map; the combat named at load is opened first, once its rows resolve it. Nothing is
    /// asked while no window is open; while the window's rows load it keeps what it shows.
    pub(super) fn combat_prepared(&mut self, document: &Document, perspective: &Choice, engaged: Option<Arc<History>>, ctx: &egui::Context) -> io::Result<Prepared> {
        let Some(views) = self.rows_head() else { return Ok(Prepared::Kept) };
        if let Some(selector) = self.combat_rows.pending.take() {
            let at = &mut self.combat_rows.selector;
            let resolved = at.frame(&views).and_then(|rows| at.discover(|| selector.resolve(&rows)));
            match resolved {
                None => self.combat_rows.pending = Some(selector),
                Some(Ok(selection)) => {
                    self.combat.open(selection);
                    self.combat.configure_air(self.options.air_details, self.options.air_range, self.options.air_channels);
                }
                Some(Err(error)) => self.error = Some(error.to_string()),
            }
        }
        if self.combat.selection.is_none() {
            return Ok(Prepared::Closed);
        }
        let Some(rows) = self.combat_rows.window_rows(&views) else { return Ok(Prepared::Kept) };
        let (branch, hour) = (rows.branch, rows.hour);
        let wake = ctx.clone();
        let prepared = self.combat.ask(&document.combat, branch, hour, Ok(rows), perspective, engaged, self.counter_view.settings.nato, move || wake.request_repaint());
        let missed = self.combat.take_missed();
        self.combat_rows.window_missed(&missed);
        prepared
    }

    /// The engaged index the open map's engagements layer reads, the one held while its layers
    /// compose; none with no map open. What the engaged byte check reads, without keeping it.
    pub(super) fn layer_history(&self) -> Option<Arc<History>> {
        let side = self.open_map()?;
        side.products().engaged.clone().or_else(|| self.combat_rows.engaged.clone())
    }

    /// [`Self::layer_history`], kept for the frames its layers compose: the index the marker
    /// hovers and the combat window read.
    pub(super) fn layer_engaged(&mut self) -> Option<Arc<History>> {
        let history = self.layer_history();
        self.combat_rows.engaged.clone_from(&history);
        history
    }
}
