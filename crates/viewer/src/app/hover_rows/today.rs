//! Today's hovers: made over the presented frame of today's pipeline while there is no playhead
//! map (off, refused or stopped at an error). Deleted with today's pipeline, whole.

use super::{Caches, Family, HoverRows, Job, Key, Loading};
use crate::{
    app::{MapHit, Viewer, hover_rows, land_hover_matches_unit, picker, prepared_naval_tooltip},
    document::Document,
};
use eframe::egui;
use replay_frame::frame::DecodedFrame;
use std::sync::Arc;
use viewer_foundation::{
    combat_selection::Selection,
    map::{free_icons, province_pick::pick},
    tooltip_text::Tooltip,
};

impl HoverRows {
    /// The newest text made for `key` over today's frame `frame`; a text is asked of the
    /// family's worker when the key or the frame changed.
    pub(crate) fn ask_today(&mut self, family: Family, frame: &Arc<DecodedFrame>, key: Key, make: impl FnOnce(&Caches) -> Job) -> Option<Arc<Tooltip>> {
        self.timed_ask(family, |hovers| hovers.post_today(family, frame, key, make).0)
    }

    /// As [`Self::ask_today`], waiting for the text over `frame`: the capture's.
    pub(crate) fn wait_today(&mut self, family: Family, frame: &Arc<DecodedFrame>, key: Key, make: impl FnOnce(&Caches) -> Job) -> Option<Arc<Tooltip>> {
        self.wait_today_until(family, frame, key, make, None)
    }

    /// [`Self::wait_today`] within `limit`, refused by name when the text is not made.
    #[cfg(test)]
    pub(crate) fn wait_today_within(&mut self, family: Family, frame: &Arc<DecodedFrame>, key: Key, make: impl FnOnce(&Caches) -> Job, limit: std::time::Duration) -> Result<Arc<Tooltip>, String> {
        self.wait_today_until(family, frame, key, make, Some(std::time::Instant::now() + limit)).ok_or_else(|| format!("the {} hover's worker made no text over today's frame within {limit:?}", family.name()))
    }

    fn wait_today_until(&mut self, family: Family, frame: &Arc<DecodedFrame>, key: Key, make: impl FnOnce(&Caches) -> Job, deadline: Option<std::time::Instant>) -> Option<Arc<Tooltip>> {
        let (text, at, posted) = self.post_today(family, frame, key.clone(), make);
        let hover = &mut self.hovers[family as usize];
        let Some(worker) = hover.worker.as_ref().filter(|_| posted || text.is_none()) else { return text };
        let made = worker.wait(|made| made.key == key && made.frame == at, deadline)?;
        let text = made.text.clone();
        hover.keep(made);
        text
    }

    fn post_today(&mut self, family: Family, frame: &Arc<DecodedFrame>, key: Key, make: impl FnOnce(&Caches) -> Job) -> (Option<Arc<Tooltip>>, usize, bool) {
        let hover = &mut self.hovers[family as usize];
        hover.asked = true;
        hover.take();
        hover.loading = Loading::default();
        self.post(family, Arc::clone(frame), false, key, make)
    }
}

impl Viewer {
    /// The map's tooltip as `map_tooltip` makes it, from today's hour and frame.
    pub(in crate::app) fn map_tooltip_today(
        &mut self,
        document: &Document,
        point: Option<[f32; 2]>,
        extent: [f32; 2],
        ctx: &egui::Context,
    ) -> Option<(Arc<Tooltip>, [f32; 4], Option<u32>)> {
        let point = point?;
        let target = self.map_hit(point);
        // An intel entry takes the hover even when its text is empty: the province's is not
        // asked for.
        if matches!(target, Some(MapHit::Intel)) {
            let hit = self
                .prepared_overlay
                .as_ref()
                .and_then(|map| map.intel_at(point, self.looks.as_ref()))?;
            let layer = Some((free_icons::IconFamily::Intel, hit.layer));
            let rect = self.shown_rect(layer, hit.bounds());
            let empty = hit.tooltip.title.is_empty()
                && hit.tooltip.body.is_empty()
                && hit.tooltip.refused.is_empty();
            return (!empty).then(|| (Arc::clone(&hit.tooltip), rect, None));
        }
        let layer = target.and_then(|target| self.hit_layer(target));
        // The pointer in the hit icon's own layout, as its hover shapes are laid out.
        let local = self.local_point(layer, point);
        if let Some(MapHit::Occupation(index)) = target {
            let hit = self
                .prepared_overlay
                .as_ref()?
                .occupation_hits()
                .get(index)?;
            let rect = self.shown_rect(layer, hit.bounds()?);
            let crate::playback::Current::Frame(frame) = document.playback.current() else {
                return None;
            };
            use viewer_world::map::occupation_icons::Hover;
            let text = match &hit.hover {
                Hover::Modifier { state, token } => {
                    let result = (|| {
                        let content = document.icons.occupation.as_ref().map_err(Clone::clone)?;
                        content
                            .definitions
                            .find(token)?
                            .ok_or_else(|| format!("active definition {token} is absent"))
                    })();
                    // The same catalogue chose and drew the badge; keep its exact definition.
                    match result {
                        Ok(definition) => {
                            let inputs = self.hover_inputs(document, self.overlay_tables.as_ref()?.3.vision(), ctx);
                            let key = inputs.key(hover_rows::What::Occupation { state: *state, token: token.clone() }, [0, 0]);
                            self.hover_rows.ask_today(hover_rows::Family::Occupation, frame, key, |_| inputs.occupation(*state, definition.clone()))?
                        }
                        Err(why) => {
                            let mut text = Tooltip::default();
                            text.refused
                                .insert(format!("occupation badge {token}: {why}"), 1);
                            Arc::new(text)
                        }
                    }
                }
                Hover::Resistance { state } => {
                    let province = document
                        .icons
                        .raids
                        .trigger_content
                        .state_provinces
                        .get(state)
                        .and_then(|members| members.first())
                        .copied();
                    let Some(province) = province else {
                        let mut text = Tooltip::default();
                        text.refused.insert(format!("occupation marker state {state}: no member province supplies the resistance hover"), 1);
                        return Some((Arc::new(text), rect, None));
                    };
                    let inputs = self.hover_inputs(document, self.overlay_tables.as_ref()?.3.vision(), ctx);
                    let (mode, selected, rosters) = (viewer_counter_products::map::province_tooltips::ModeHover::OccupationMarker, self.unit_selection.ordered_units(), document.playback.roster_sampler());
                    let key = inputs.key(hover_rows::What::Province { province, mode: mode.clone() }, [hover_rows::id(selected), hover_rows::id(&rosters)]);
                    self.hover_rows.ask_today(hover_rows::Family::Province, frame, key, |caches| inputs.province(caches, province, mode, Arc::clone(selected), rosters))?
                }
            };
            return Some((text, rect, None));
        }
        if let Some((text, rect)) = prepared_naval_tooltip(&self.counter_report, target) {
            return Some((text, self.shown_rect(layer, rect), None));
        }
        if let Some(MapHit::Base(index)) = target
            && let Some(hit) = self.counter_report.air_grid_hits.get(&index)
        {
            return Some((
                Arc::clone(&hit.text),
                self.shown_rect(layer, self.counter_report.base_hits[index].rect),
                None,
            ));
        }
        if let Some(MapHit::Base(index)) = target
            && let Some(&(province, kind)) = self.counter_report.air_base_hits.get(&index)
            && let crate::playback::Current::Frame(frame) = document.playback.current()
        {
            let (inputs, wings) = (self.hover_inputs(document, self.overlay_tables.as_ref()?.3.vision(), ctx), self.air_selection.wings());
            let key = inputs.key(hover_rows::What::AirBase { province, kind }, [hover_rows::id(wings), 0]);
            let text = self.hover_rows.ask_today(hover_rows::Family::Counters, frame, key, |_| inputs.air_base(province, kind, Arc::clone(wings)))?;
            return Some((
                text,
                self.shown_rect(layer, self.counter_report.base_hits[index].rect),
                None,
            ));
        }
        if let Some(MapHit::AirMission(index)) = target {
            let hit = self.counter_report.air_mission_hits.get(index)?;
            return Some((
                Arc::clone(&hit.text),
                self.shown_rect(layer, hit.rect),
                None,
            ));
        }
        if let Some(MapHit::NavalRegionHover(index)) = target {
            use viewer_counters::map::counters::NavalRegionalControl;
            let hit = self.counter_report.naval_region_hovers.get(index)?;
            let crate::playback::Current::Frame(frame) = document.playback.current() else {
                return None;
            };
            let inputs = self.hover_inputs(document, self.overlay_tables.as_ref()?.3.vision(), ctx);
            let text = match hit.control {
                NavalRegionalControl::Mines | NavalRegionalControl::Accidents => {
                    let mines = matches!(hit.control, NavalRegionalControl::Mines);
                    let key = inputs.key(if mines { hover_rows::What::Mines(hit.region) } else { hover_rows::What::Accidents(hit.region) }, [0, 0]);
                    self.hover_rows.ask_today(hover_rows::Family::Naval, frame, key, |_| inputs.naval(hit.region, mines))?
                }
                _ => {
                    let mut text = Tooltip::default();
                    text.refuse(
                        "regional Navy control hover",
                        "drawn control has no prepared hover text".into(),
                    );
                    Arc::new(text)
                }
            };
            return Some((text, self.shown_rect(layer, hit.rect), None));
        }
        if matches!(target, Some(MapHit::Raid))
            && let Some(hit) = self.raid_hit(point).cloned()
        {
            let crate::playback::Current::Frame(frame) = document.playback.current() else {
                return None;
            };
            let ours = self.overlay_tables.as_ref()?.3.ours();
            let text = self.combat_rows.raid_today(document, frame, ours, self.counter_view.settings.nato, hit.key.clone(), ctx)?;
            let rect = [hit.origin[0], hit.origin[1], hit.size[0], hit.size[1]];
            let layer = Some((free_icons::IconFamily::Raids, hit.layer));
            return Some((text, self.shown_rect(layer, rect), None));
        }
        let land_hit = match target {
            Some(MapHit::Combat(index)) => {
                let hit = &self.counter_report.combat_hits[index];
                match hit.selection {
                    Selection::Land(id) => Some((id, hit.rect)),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some((id, rect)) = land_hit
            && let crate::playback::Current::Frame(frame) = document.playback.current()
        {
            let viewer = self.overlay_tables.as_ref()?.3.ours();
            let index = match self
                .hours
                .as_ref()
                .map(|hours| hours.engaged(&self.perspective.country))
                .transpose()
            {
                Ok(index) => index,
                Err(why) => {
                    let mut text = Tooltip::default();
                    text.refused
                        .insert(format!("land marker {id} engaged index: {why}"), 1);
                    return Some((Arc::new(text), self.shown_rect(layer, rect), None));
                }
            };
            let text = self.combat_rows.land_marker_today(document, frame, viewer, id, index.as_ref().map(|index| Arc::clone(&index.shared)), self.counter_view.settings.nato, { let ctx = ctx.clone(); move || ctx.request_repaint() })?;
            return Some((text, self.shown_rect(layer, rect), None));
        }
        let naval_hit = match target {
            Some(MapHit::Combat(index)) => Some(&self.counter_report.combat_hits[index]),
            Some(MapHit::NavalObserver(index)) => {
                Some(&self.counter_report.naval_marker_hovers[index])
            }
            _ => None,
        }
        .and_then(|hit| match hit.selection {
            Selection::Naval(id) => Some((id, hit.rect)),
            _ => None,
        });
        if let Some((id, rect)) = naval_hit
            && let crate::playback::Current::Frame(frame) = document.playback.current()
        {
            let viewer = self.overlay_tables.as_ref()?.3.ours();
            let index = match self
                .hours
                .as_ref()
                .map(|hours| hours.engaged(&self.perspective.country))
                .transpose()
            {
                Ok(index) => index,
                Err(why) => {
                    let mut text = Tooltip::default();
                    text.refused
                        .insert(format!("naval marker {id} engaged index: {why}"), 1);
                    return Some((Arc::new(text), self.shown_rect(layer, rect), None));
                }
            };
            let text = self.combat_rows.naval_marker_today(document, frame, viewer, id, index.as_ref().map(|index| Arc::clone(&index.shared)), { let ctx = ctx.clone(); move || ctx.request_repaint() })?;
            return Some((text, self.shown_rect(layer, rect), None));
        }
        if let Some(hit) = self.counter_report.land_hovers.iter().rev().find(|hit| {
            hit.contains(local)
                && matches!(target, Some(MapHit::Unit(index))
                if land_hover_matches_unit(hit, &self.counter_report.unit_hits[index]))
        }) && let crate::playback::Current::Frame(frame) = document.playback.current()
        {
            let inputs = self.hover_inputs(document, self.overlay_tables.as_ref()?.3.vision(), ctx);
            let key = inputs.key(hover_rows::What::Land(hit.clone()), [0, 0]);
            let text = self.hover_rows.ask_today(hover_rows::Family::Counters, frame, key, |caches| inputs.land(caches, hit.clone()))?;
            let text = if hit.ours
                && !hit.hidden
                && !hit.railway_gun
                && hit.members.len() == 1
                && hit.control
                    == viewer_foundation::map::counter_hovers::land::CounterControl::Normal
            {
                self.combat_rows.division_panel_today(document, frame, text, hit.members[0], { let ctx = ctx.clone(); move || ctx.request_repaint() })
            } else {
                text
            };
            return Some((text, self.shown_rect(layer, hit.rect), None));
        }
        if let Some(hit) = self
            .counter_report
            .task_force_hovers
            .iter()
            .rev()
            .find(|hit| {
                hit.contains(local)
                    && matches!(target, Some(MapHit::Unit(index))
                if index == hit.unit_hit && hit.matches(&self.counter_report.unit_hits[index]))
            })
            && let crate::playback::Current::Frame(frame) = document.playback.current()
        {
            if !hit.supported {
                return None;
            }
            let inputs = self.hover_inputs(document, self.overlay_tables.as_ref()?.3.vision(), ctx);
            let key = inputs.key(hover_rows::What::TaskForce(hit.clone()), [0, 0]);
            let text = self.hover_rows.ask_today(hover_rows::Family::Counters, frame, key, |caches| inputs.task_force(caches, hit.clone()))?;
            return Some((text, self.shown_rect(layer, hit.rect), None));
        }
        if let Some((_, hit)) = self.counter_report.task_force_port_hovers.iter().enumerate().rev()
            .find(|(hover_index, hit)| hit.contains(local) && hit.has_tooltip()
                && self.counter_report.unit_hits.get(hit.unit_hit).is_some_and(|unit| hit.matches(unit))
                && (matches!(target, Some(MapHit::Unit(index)) if index == hit.unit_hit)
                    || matches!(target, Some(MapHit::PortTooltip(index)) if index == *hover_index)))
            && let crate::playback::Current::Frame(frame) = document.playback.current()
        {
            if !hit.supported { return None; }
            let inputs = self.hover_inputs(document, self.overlay_tables.as_ref()?.3.vision(), ctx);
            let key = inputs.key(hover_rows::What::Port(hit.clone()), [0, 0]);
            let text = self.hover_rows.ask_today(hover_rows::Family::Counters, frame, key, |caches| inputs.port(caches, hit.clone()))?;
            return Some((text, self.shown_rect(layer, hit.rect), None));
        }
        if target.is_some() {
            return None;
        }
        let crate::playback::Current::Frame(frame) = document.playback.current() else {
            return None;
        };
        let view = self.last_view.as_ref()?;
        if self.counter_view.visibility.trade_routes
            && self.uploaded.orders.as_ref().is_some_and(|orders| {
                viewer_foundation::map::ribbon_content::trade_route_hit(
                    &orders.trade_route_hovers,
                    view.camera.eye(),
                    view.camera
                        .ray(point.map(|v| v * view.ui_scale), view.viewport),
                    document.map.width as f32,
                )
            })
        {
            let mut text = Tooltip::default();
            text.refused.insert("trade-route hover: transfer rows, delivery efficiency and naval-dominance explanation are not implemented".to_owned(), 1);
            return Some((Arc::new(text), [0.0, 0.0, extent[0], extent[1]], None));
        }
        let province = match pick(
            &view.camera,
            point.map(|v| v * view.ui_scale),
            view.viewport,
            &document.map,
            document.assets.heightmap_texels(),
        ) {
            Ok(province) => province?,
            Err(reason) => {
                let mut text = Tooltip::default();
                text.refused.insert(reason, 1);
                return Some((Arc::new(text), [0.0, 0.0, extent[0], extent[1]], None));
            }
        };
        let vision = self.overlay_tables.as_ref()?.3.vision();
        if *self.shown_mode()
            == viewer_world::map::map_mode::MapMode::Stock(
                viewer_world::map::map_mode::Stock::Terrain,
            )
        {
            let text = self.combat_rows.terrain_today(document, frame, province, vision, { let ctx = ctx.clone(); move || ctx.request_repaint() })?;
            return Some((text, [0.0, 0.0, extent[0], extent[1]], Some(province)));
        }
        let mode = self.shown_mode().clone();
        let mut mode_hover = picker::mode_hover(&mode, document.map_modes.defines.opinion_thresholds(), || self.combat_rows.infrastructure_today(document, frame, province, { let ctx = ctx.clone(); move || ctx.request_repaint() }));
        if let viewer_counter_products::map::province_tooltips::ModeHover::StrategicAir(selected) =
            &mut mode_hover
        {
            *selected = Arc::clone(self.air_selection.wings());
        }
        if let viewer_counter_products::map::province_tooltips::ModeHover::StrategicNavy(sources) =
            &mut mode_hover
        {
            *sources = self.overlay_tables.as_ref()?.3.naval_dominance_sources();
        }
        let inputs = self.hover_inputs(document, vision, ctx);
        let (selected, rosters) = (self.unit_selection.ordered_units(), document.playback.roster_sampler());
        let key = inputs.key(hover_rows::What::Province { province, mode: mode_hover.clone() }, [hover_rows::id(selected), hover_rows::id(&rosters)]);
        let text = self.hover_rows.ask_today(hover_rows::Family::Province, frame, key, |caches| inputs.province(caches, province, mode_hover, Arc::clone(selected), rosters))?;
        Some((text, [0.0, 0.0, extent[0], extent[1]], Some(province)))
    }
}
