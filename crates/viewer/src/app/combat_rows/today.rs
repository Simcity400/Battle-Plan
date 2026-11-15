//! Today's combat readers: the combat window, the marker, panel, terrain, infrastructure and raid
//! hovers made over the presented frame of today's pipeline while there is no playhead map (off,
//! refused or stopped at an error), and today's engaged index. Deleted with today's pipeline,
//! whole.

use super::{CombatRows, From, Loading};
use crate::{app::Viewer, document::Document, playback::Current};
use eframe::egui;
use replay_data::{BranchId, Hour};
use replay_frame::frame::DecodedFrame;
use std::{io, sync::Arc};
use viewer_combat::Prepared;
use viewer_foundation::{
    engagement_history::History,
    map::perspective::{Choice, Country, Vision},
    tooltip_text::Tooltip,
};
use viewer_raids::map::raids::tooltips::Key;

impl CombatRows {
    #[allow(clippy::too_many_arguments, reason = "the tooltip's inputs, each from another owner")]
    pub(crate) fn land_marker_today(&mut self, document: &Document, today: &Arc<DecodedFrame>, viewer: u32, id: u64, index: Option<Arc<History>>, nato: bool, wake: impl Fn() + Send + 'static) -> Option<Arc<Tooltip>> {
        self.land_marker_from(From::Today(today), document, viewer, id, index, nato, wake)
    }

    #[allow(clippy::too_many_arguments, reason = "the tooltip's inputs, each from another owner")]
    pub(crate) fn naval_marker_today(&mut self, document: &Document, today: &Arc<DecodedFrame>, viewer: u32, id: u64, index: Option<Arc<History>>, wake: impl Fn() + Send + 'static) -> Option<Arc<Tooltip>> {
        self.naval_marker_from(From::Today(today), document, viewer, id, index, wake)
    }

    pub(crate) fn division_panel_today(&mut self, document: &Document, today: &Arc<DecodedFrame>, base: Arc<Tooltip>, id: u64, wake: impl Fn() + Send + 'static) -> Arc<Tooltip> {
        self.division_panel_from(From::Today(today), document, base, id, wake)
    }

    pub(crate) fn terrain_today(&mut self, document: &Document, today: &Arc<DecodedFrame>, province: u32, vision: &Arc<Vision>, wake: impl Fn() + Send + 'static) -> Option<Arc<Tooltip>> {
        self.terrain_from(From::Today(today), document, province, vision, wake)
    }

    pub(crate) fn infrastructure_today(&mut self, document: &Document, today: &Arc<DecodedFrame>, province: u32, wake: impl Fn() + Send + 'static) -> Result<i32, String> {
        self.infrastructure_from(From::Today(today), document, province, wake)
    }

    /// The raid tooltip, built on its worker from today's frame; none until the first build for
    /// this icon is done.
    #[allow(clippy::too_many_arguments, reason = "the tooltip's inputs, each from another owner")]
    pub(crate) fn raid_today(&mut self, document: &Document, today: &Arc<DecodedFrame>, ours: u32, nato: bool, key: Key, ctx: &egui::Context) -> Option<Arc<Tooltip>> {
        self.raid_asked = true;
        self.raid_loading = Loading::default();
        let text = self.raid_lane.request(Arc::clone(&document.raid_hover), Arc::clone(today), ours, nato, key, ctx);
        self.raid.learn(&self.raid_lane.take_missed());
        text
    }
}

/// The window's frame today: the presented frame (a gap's reason when today's hour is one), with
/// its branch and hour.
fn window_today(document: &Document) -> (BranchId, Hour, Result<Arc<DecodedFrame>, String>) {
    let today = match document.playback.current() {
        Current::Frame(frame) => Ok(Arc::clone(frame)),
        Current::Gap { .. } | Current::Refused { .. } => Err(document.frame().err().map_or_else(String::new, |error| error.to_string())),
    };
    (document.playback.branch(), document.playback.hour(), today)
}

impl Viewer {
    /// The combat window's read for this frame over today's frame, as `combat_prepared` makes it
    /// with no playhead map.
    pub(in crate::app) fn combat_prepared_today(&mut self, document: &Document, perspective: &Choice, engaged: Option<Arc<History>>, ctx: &egui::Context) -> io::Result<Prepared> {
        if let Some(selector) = self.combat_rows.pending.take() {
            match document.frame().and_then(|frame| selector.resolve(frame)) {
                Ok(selection) => {
                    self.combat.open(selection);
                    self.combat.configure_air(self.options.air_details, self.options.air_range, self.options.air_channels);
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        if self.combat.selection.is_none() {
            return Ok(Prepared::Closed);
        }
        let (branch, hour, frame) = window_today(document);
        let wake = ctx.clone();
        let prepared = self.combat.ask(&document.combat, branch, hour, frame, perspective, engaged, self.counter_view.settings.nato, move || wake.request_repaint());
        let missed = self.combat.take_missed();
        self.combat_rows.window.learn(&missed);
        prepared
    }

    /// Today's pipeline's engaged index for `country`, none without the pipeline.
    pub(in crate::app) fn engaged_today(&self, country: &Country) -> io::Result<Option<Arc<History>>> {
        self.hours.as_ref().map(|hours| hours.engaged(country)).transpose().map(|index| index.map(|index| Arc::clone(&index.shared)))
    }
}
