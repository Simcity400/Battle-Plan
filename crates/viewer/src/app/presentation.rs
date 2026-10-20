use super::{
    Laps, OverlayKey, Playhead, ShownHour, Viewer, feed_raids, latest_built_key, present_raids,
    skipped_hours, wait_for_hour,
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use viewer_foundation::map::presentation::Continuity;
use viewer_foundation::map::presentation::UnitRestart;
use viewer_foundation::map::presentation::continuity as presentation_continuity;
use viewer_foundation::map::presentation::crosses_gap;
use viewer_foundation::map::presentation::unit_restart as presentation_unit_restart;
use viewer_foundation::profile_writer::profile_print;
use viewer_foundation::profile_writer::profiling;
use viewer_foundation::retirement::Dropper;
use viewer_foundation::retirement::Retired;
use {
    crate::{document::Document, hours},
    viewer_foundation::map::camera::{Camera, MapSize},
};

/// One built hour's events: its nuclear impacts and its map events for the
/// viewing country.
type HourEvents = (
    Arc<viewer_world::map::strike_effects::Impacts>,
    Arc<viewer_world::map::animated_map_text::events::Prepared>,
);

/// What a forward skip carries from a built hour it passed: its buildings,
/// whose history continues, and its events, whose effects and text start.
type SkippedHour = (
    Arc<hours::Shared<viewer_world::map::map_buildings::PreparedBuildings>>,
    HourEvents,
);

/// The hours whose events a presented hour starts, oldest first: continuing
/// playback also starts those of the built hours a forward skip passed; a
/// reset (a seek, branch, gap or viewing-country change) starts only the
/// presented hour's own.
fn presented_events<'a, T>(
    skipped: impl Iterator<Item = &'a T>,
    presented: &'a T,
    continuity: Continuity,
) -> Vec<&'a T> {
    skipped
        .filter(|_| continuity == Continuity::Continue)
        .chain([presented])
        .collect()
}

impl Viewer {
    // hours
    /// Whether the map can show `hour` of `branch` on the next frame: it is on
    /// screen or built and waiting.
    pub(super) fn hour_ready(&self, branch: replay_data::BranchId, hour: i32) -> bool {
        self.ready.contains_key(&(branch, hour))
            || self.shown.as_ref().is_some_and(|shown| {
                shown.epoch == self.epoch && (shown.branch, shown.hour) == (branch, hour)
            })
    }

    /// Keeps the hour builders told where the playhead is and uploads the
    /// newest hour they finished that the window should show.
    ///
    /// Nothing here builds an hour: the political fill, railways, buildings,
    /// trains, aircraft, unit models, battle plan, weather effects, snow and
    /// mud, fog, limitations, names and icons all arrive built
    /// (`crate::hours`), and the frame that shows an hour only uploads it, all
    /// of its layers at once so no frame mixes two hours. Until the playhead's
    /// hour is built the scene keeps the hour it has; during playback it takes
    /// the newest built hour on the way. A capture waits for the hour it names.
    pub(super) fn refresh_hour(
        &mut self,
        state: &eframe::egui_wgpu::RenderState,
        document: &mut Document,
        laps: &mut Laps,
    ) -> Result<(), std::io::Error> {
        laps.lap("before_hour");
        // The hour builders lay counters and icons out with the map mode's visibility.
        self.counter_view.set_mode(&self.map_mode);
        if self.scene.is_none() {
            return Err(std::io::Error::other("the map scene was not created"));
        }
        // Static geometry arrives with the scene from the loading thread
        // (`load_scene`); only hour data uploads per frame from here.
        laps.lap("static_geometry");
        let replay = document.playback.replay_handle();
        if self
            .hours
            .as_ref()
            .is_none_or(|hours| !hours.serves(&document.content, &replay))
        {
            self.hours = Some(match &self.shared_resources {
                Some(resources) if self.dashboard_map => {
                    hours::Pipeline::shared(Arc::clone(&document.content), replay, resources)?
                }
                Some(resources) => hours::Pipeline::campaign(
                    Arc::clone(&document.content),
                    replay,
                    resources,
                    document.playback.checkpoint_source(),
                )?,
                None => hours::Pipeline::with_checkpoint_source(
                    Arc::clone(&document.content),
                    replay,
                    document.playback.checkpoint_source(),
                )?,
            });
            if let (Some(hours), Some(index)) = (&self.hours, self.engaged_carry.take()) {
                hours.adopt_engaged(index);
            }
            self.epoch = 0;
            self.ready.clear();
            self.shown = None;
            self.raid_lane = Default::default();
        }
        let playhead = self.playhead.unwrap_or(Playhead {
            branch: document.playback.branch(),
            hour: document.playback.hour(),
            fraction: f64::from(document.hour_fraction),
        });
        let view = self.last_view.unwrap_or_else(|| {
            let map_size = MapSize::of(&document.map);
            let defines = document.map_defines;
            let (viewport, camera) = match self.capture.as_ref().and_then(|capture| capture.pinned)
            {
                Some((size, camera)) => {
                    let camera = camera.map_or_else(
                        || Camera::fit(map_size, defines, size),
                        |placement| placement.on(map_size, defines),
                    );
                    (size, camera)
                }
                None => {
                    let size = [1280, 720];
                    (
                        size,
                        self.camera.map_or_else(
                            || Camera::fit(map_size, defines, size),
                            |placement| placement.on(map_size, defines),
                        ),
                    )
                }
            };
            hours::View {
                camera,
                viewport,
                // The first frame lays the icons out again at its own scale.
                ui_scale: 1.0,
            }
        });
        // Playing, request one pool of consecutive hours from the playhead;
        // paused, a step either way is already built. Do not turn cold-start
        // latency into a recording-wide read-ahead window.
        let (ahead, behind) = if self.dashboard_map {
            (0, 0)
        } else if self.playing {
            (
                self.hours.as_ref().expect("started above").playback_ahead(),
                0,
            )
        } else {
            (2, 2)
        };
        let want = hours::Want {
            branch: playhead.branch,
            hour: playhead.hour,
            ahead,
            behind,
            playing: self.playing,
            perspective: self.perspective.clone(),
            map_mode: self.map_mode.clone(),
            show_allied_plans: self.graphics.show_allied_plans,
            counters: self.counter_view.clone(),
            view,
            // Not the hour on screen after a seek, which may be to that hour.
            have: self
                .ready
                .keys()
                .chain(
                    self.shown
                        .as_ref()
                        .filter(|shown| shown.epoch == self.epoch && !self.animation_seek)
                        .map(|shown| (shown.branch, shown.hour))
                        .iter(),
                )
                .filter(|(branch, _)| *branch == playhead.branch)
                .map(|(_, hour)| *hour)
                .collect(),
            received: 0,
        };
        // Everything this frame lets go of, freed off the frame path at its end.
        let mut retired = Retired::default();
        let pipeline = self.hours.as_mut().expect("started above");
        let epoch = pipeline.want(want)?;
        if epoch != self.epoch {
            self.epoch = epoch;
            retired.push(std::mem::take(&mut self.ready));
        }
        if self.dropper.is_none() {
            self.dropper = Some(Arc::new(Dropper::new()?));
        }
        for built in pipeline.take()? {
            feed_raids(&self.raid_lane, &built);
            if built.epoch == epoch {
                if let Some(old) = self.ready.insert((built.branch, built.hour), built) {
                    retired.push(old);
                }
            } else {
                retired.push(built);
            }
        }
        let fallback = self
            .playing
            .then(|| {
                latest_built_key(&self.ready, (playhead.branch, playhead.hour), |built| {
                    built.layers.is_ok()
                })
            })
            .flatten();
        let (keep, gone): (BTreeMap<_, _>, BTreeMap<_, _>) = std::mem::take(&mut self.ready)
            .into_iter()
            .partition(|((branch, hour), _)| {
                *branch == playhead.branch
                    && ((playhead.hour.saturating_sub(behind)
                        ..=playhead.hour.saturating_add(ahead))
                        .contains(hour)
                        || fallback == Some((*branch, *hour)))
            });
        self.ready = keep;
        let mut passed = gone;
        laps.lap("hours_poll");
        let target = (playhead.branch, playhead.hour);
        let shown_at = self
            .shown
            .as_ref()
            .filter(|shown| shown.epoch == epoch)
            .map(|shown| (shown.branch, shown.hour));
        let mut next = None;
        // A seek to the hour on screen takes it again, rebuilt (see `have`),
        // to start a fresh animation history. The fallback below still moves
        // only forward from the hour on screen.
        if self.animation_seek || shown_at != Some(target) {
            next = self.ready.remove(&target);
            // A capture names its hour, and waits for it.
            let wait = self
                .capture
                .as_ref()
                .is_some_and(|capture| !profiling() || capture.remaining == 0);
            if next.is_none() && wait {
                let pipeline = self.hours.as_ref().expect("started above");
                next = Some(wait_for_hour(
                    &self.raid_lane,
                    (target, epoch),
                    || pipeline.wait(),
                    &mut self.ready,
                    |built| retired.push(built),
                )?);
            }
            if next.is_none() && self.playing {
                // The newest hour on the way from the one on screen.
                let on_the_way = fallback
                    .filter(|key| shown_at.is_none_or(|shown| shown.0 != key.0 || shown.1 < key.1));
                if let Some(key) = on_the_way {
                    next = self.ready.remove(&key);
                    let later = self.ready.split_off(&key);
                    passed.extend(std::mem::replace(&mut self.ready, later));
                }
            }
        }
        let applied = match next {
            Some(built) => {
                // The hour on screen of any epoch: a perspective change starts
                // a new build epoch, but the scene's building history does not
                // depend on the viewing country, so it continues across it.
                let on_screen = self.shown.as_ref().map(|shown| (shown.branch, shown.hour));
                // Hours the builders passed over to keep up with the clock
                // continue the history, also on the catch-up after a pause; a
                // seek or a gap in the recording does not.
                let passes_unbuilt = !self.animation_seek
                    && on_screen.is_some_and(|(_, from)| {
                        !crosses_gap(
                            &document.playback.replay().gaps,
                            built.branch,
                            from,
                            built.hour,
                        )
                    });
                let skipped = skipped_hours(
                    on_screen,
                    (built.branch, built.hour),
                    passes_unbuilt,
                    |hour| {
                        let key = (built.branch, hour);
                        let passed_hour = self.ready.get(&key).or_else(|| passed.get(&key))?;
                        let layers = passed_hour.layers.as_ref().ok();
                        Some(layers.map(|layers| {
                            (
                                Arc::clone(&layers.buildings),
                                (Arc::clone(&layers.impacts), Arc::clone(&layers.map_text)),
                            )
                        }))
                    },
                );
                self.apply_hour(state, document, built, skipped, &mut retired, laps)
            }
            None => Ok(()),
        };
        if !passed.is_empty() {
            retired.push(passed);
        }
        if let Some(dropper) = &self.dropper
            && !retired.is_empty()
        {
            dropper.drop_later(retired);
        }
        applied?;
        // The frame sits past the playhead's hour only when it is the hour on
        // screen.
        document.hour_fraction = if (document.playback.branch(), document.playback.hour()) == target
        {
            playhead.fraction as f32
        } else {
            0.0
        };
        if let Some(error) = self
            .shown
            .as_ref()
            .filter(|shown| shown.epoch == epoch)
            .and_then(|shown| shown.error.clone())
        {
            return Err(std::io::Error::other(error));
        }
        Ok(())
    }

    /// Uploads one built hour and makes it the hour on screen.
    fn apply_hour(
        &mut self,
        state: &eframe::egui_wgpu::RenderState,
        document: &mut Document,
        built: hours::Built,
        skipped: Option<Vec<SkippedHour>>,
        retired: &mut Retired,
        laps: &mut Laps,
    ) -> Result<(), std::io::Error> {
        let hours::Built {
            epoch,
            branch,
            hour,
            view,
            current,
            layers,
            laps: built_laps,
            wall,
        } = built;
        if let Some(opened) = self.opened.take()
            && profiling()
        {
            profile_print(format!(
                "PROFILE open first_hour {:.1} ms",
                opened.elapsed().as_secs_f64() * 1000.0
            ));
        }
        // The raid lane gets the hour's own frame with the hour, cache hits
        // too: it may have pruned the frame since the builders delivered it.
        let raid_frame = current.clone();
        retired.push(document.playback.present(branch, hour, current));
        laps.lap("present");
        if profiling() {
            profile_print(format!(
                "PROFILE built frame {} hour {hour} wall {wall:.3} parts {}",
                self.profile_frame,
                built_laps
                    .iter()
                    .map(|(name, ms)| format!("{name}={ms:.2}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        laps.lap("built_print");
        // The map events' consumption follows the hour last presented, built
        // or failed, and whether this is an explicit seek.
        let seek = self.animation_seek;
        let layers = match layers {
            Ok(layers) => layers,
            Err(error) => {
                if let Some(scene) = self.scene.as_mut() {
                    scene.missing_map_text_events(
                        (branch, hour),
                        seek,
                        format!("map text presented hour failed: {error}"),
                    );
                }
                self.animation_seek = false;
                present_raids(
                    &self.raid_lane,
                    Continuity::Continue,
                    crate::raid_lane::Presented {
                        content: Some(&document.content),
                        perspective: &self.perspective,
                        at: (branch, hour),
                        playing: self.playing,
                        current: &raid_frame,
                    },
                );
                retired.push(self.shown.replace(ShownHour {
                    epoch,
                    branch,
                    hour,
                    error: Some(error),
                    political_warnings: Vec::new(),
                    plan_warnings: Vec::new(),
                    building_warnings: Vec::new(),
                    model_warnings: Vec::new(),
                    political_summary: (0, 0),
                    political_build: Duration::ZERO,
                    limitations: Default::default(),
                    snow: Default::default(),
                    header: Default::default(),
                    borders: None,
                }));
                return Ok(());
            }
        };
        let previous = self
            .shown
            .as_ref()
            .filter(|shown| shown.error.is_none())
            .map(|shown| (shown.branch, shown.hour, shown.header.viewing.as_deref()));
        let next = (branch, hour, layers.header.viewing.as_deref());
        let continuity = presentation_continuity(
            previous,
            next,
            self.animation_seek,
            &document.playback.replay().gaps,
        );
        let unit_restart = presentation_unit_restart(previous, next, self.animation_seek);
        // Audio follows the same continuity: a reset, or recreated units,
        // ends what sounds and does not count present events as fired.
        if continuity == Continuity::Reset || unit_restart != UnitRestart::Keep {
            self.audio_reset = true;
        }
        let hours::Layers {
            political,
            borders,
            rails,
            buildings,
            building_warnings,
            trains,
            aircraft,
            units,
            orders,
            effects,
            impacts,
            map_text,
            music,
            battles,
            intel,
            snow_mud,
            limitations,
            header,
            names,
            overlay,
            overlay_tables,
        } = layers;
        self.music = music;
        self.battles = battles;
        let scene = self
            .scene
            .as_mut()
            .ok_or_else(|| std::io::Error::other("the map scene was not created"))?;
        laps.lap("apply_start");
        let uploaded = &mut self.uploaded;
        if uploaded.rails != Some(rails.id) {
            let (mesh, bridges) = &rails.value;
            scene.set_railways(&state.device, &state.queue, &document.rail_assets, mesh)?;
            scene.set_bridges(
                &state.device,
                &state.queue,
                &document.bridges,
                bridges.clone(),
            )?;
            uploaded.rails = Some(rails.id);
        }
        laps.lap("rails_upload");
        if uploaded.buildings != Some(buildings.id) {
            let skipped = skipped.as_ref().map(|hours| {
                hours
                    .iter()
                    .map(|(hour, _)| &hour.value)
                    .collect::<Vec<_>>()
            });
            scene.set_map_buildings(
                &state.device,
                &state.queue,
                &document.map_buildings,
                buildings.value.clone(),
                skipped.as_deref(),
            )?;
            uploaded.buildings = Some(buildings.id);
        }
        laps.lap("buildings_upload");
        // The raid offer cycle is presentation history too: it starts again
        // exactly when the trains and the weather do.
        present_raids(
            &self.raid_lane,
            continuity,
            crate::raid_lane::Presented {
                content: Some(&document.content),
                perspective: &self.perspective,
                at: (branch, hour),
                playing: self.playing,
                current: &raid_frame,
            },
        );
        let same_units = self
            .unit_models
            .as_ref()
            .is_some_and(|previous| Arc::ptr_eq(previous, &units));
        let keep_movers =
            same_units && continuity == Continuity::Continue && unit_restart == UnitRestart::Keep;
        if keep_movers {
            retired.push(trains);
        } else {
            let replaced = scene.set_trains(document.trains.clone(), trains, continuity)?;
            retired.push(replaced);
        }
        laps.lap("trains_upload");
        if keep_movers {
            retired.push(aircraft);
        } else {
            let replaced = scene.set_aircraft(document.aircraft.clone(), aircraft)?;
            retired.push(replaced);
        }
        laps.lap("aircraft_upload");
        if !keep_movers {
            match unit_restart {
                UnitRestart::Keep => {}
                UnitRestart::ViewingCountry => scene.restart_unit_models(Some(units.owned())),
                UnitRestart::All => scene.restart_unit_models(None),
            }
            if unit_restart == UnitRestart::Keep && !same_units {
                if let Some(previous) = &self.unit_models {
                    units.inherit_convoys(previous);
                }
            } else {
                units.reset_convoys();
            }
            self.unit_field_uploaded = false;
        }
        let model_warnings = units.report.warnings();
        retired.push(self.unit_models.replace(units));
        if uploaded
            .orders
            .as_ref()
            .is_none_or(|had| !Arc::ptr_eq(had, &orders))
        {
            scene.set_orders(&state.device, &state.queue, &document.orders, &orders)?;
            retired.push(uploaded.orders.replace(Arc::clone(&orders)));
        }
        let plan_warnings = orders.report.warnings();
        retired.push(orders);
        laps.lap("orders_upload");
        if keep_movers {
            retired.push(effects);
        } else {
            let replaced = scene.set_prepared_weather_effects(
                &state.device,
                &state.queue,
                document.weather_effects.clone(),
                effects,
                continuity,
            )?;
            retired.push(replaced);
        }
        laps.lap("effects_upload");
        if uploaded.political != Some(political.id) {
            scene.set_political(&state.device, &state.queue, &political.layers.regions)?;
            uploaded.political = Some(political.id);
        }
        // borders: the window has no demilitarised-zone override, so the zone is the
        // recorded one.
        if uploaded.borders != Some(borders.id) {
            scene.set_border_plans(&state.device, &borders.classes, &borders.highlighted)?;
            uploaded.borders = Some(borders.id);
        }
        laps.lap("border_classes");
        // Equal snapshots preserve local refresh history; discontinuities reset it.
        let same_snow = uploaded.snow_mud.as_ref().is_some_and(|had| {
            Arc::ptr_eq(had, &snow_mud)
                || had.width == snow_mud.width
                    && had.height == snow_mud.height
                    && had.texels == snow_mud.texels
                    && had.province_snow == snow_mud.province_snow
                    && had.refresh == snow_mud.refresh
        });
        if !same_snow || continuity == Continuity::Reset {
            scene.set_snow_mud(&state.device, &state.queue, &snow_mud, continuity)?;
            uploaded.snow_mud = Some(Arc::clone(&snow_mud));
        }
        laps.lap("snow_upload");
        // fog of war: hours whose fog did not change share the grid on the GPU.
        if uploaded
            .intel
            .as_ref()
            .is_none_or(|had| !Arc::ptr_eq(had, &intel))
        {
            scene.set_fog_of_war(&state.queue, &intel.texels, intel.fog)?;
            uploaded.intel = Some(Arc::clone(&intel));
        }
        laps.lap("fog_upload");
        retired.push(self.placements.replace(names));
        self.placements_at = Some((branch, hour, self.map_mode.clone()));
        let counter_view = overlay.counter_view().clone();
        retired.push(self.prepared_overlay.replace(overlay));
        // The hours' nuclear impacts start their effects and their map events,
        // interpreted on the hour builders, become floating text and
        // explosions for the viewing country.
        let heights = viewer_foundation::map::province_map::HeightField {
            width: document.assets.heightmap.width,
            height: document.assets.heightmap.height,
            values: document.assets.heightmap_texels(),
        };
        match &raid_frame {
            _ if keep_movers => retired.push((impacts, map_text)),
            crate::playback::Current::Frame(_) => {
                let presented = (impacts, map_text);
                scene.present_hours(
                    (branch, hour),
                    seek,
                    presented_events(
                        skipped.iter().flatten().map(|(_, events)| events),
                        &presented,
                        continuity,
                    )
                    .into_iter()
                    .map(|(impacts, map_text)| (&**impacts, &**map_text)),
                    continuity,
                    |province| document.content.counters.anchor(province, 0),
                    heights,
                    &document.content.icons.counters.gui,
                );
            }
            crate::playback::Current::Gap { reason, .. } => scene.missing_map_text_events(
                (branch, hour),
                seek,
                format!("map text presented hour is unavailable: {reason}"),
            ),
        }
        retired.push(self.overlay_tables.replace((
            branch,
            hour,
            self.perspective.clone(),
            overlay_tables,
        )));
        self.overlay_at = Some(OverlayKey {
            occupation: counter_view.occupation,
            branch,
            hour,
            camera: view.camera,
            pixels: view.viewport,
            ui_scale: view.ui_scale,
            perspective: self.perspective.clone(),
            counter_settings: counter_view.settings,
            selected: counter_view.selected,
            air_selected: counter_view.air_selected,
            air_bases: counter_view.air_bases,
            fleets_selected: counter_view.fleets_selected,
            hovered: counter_view.hovered,
            visibility: counter_view.visibility,
            operatives: counter_view.operatives,
        });
        self.overlay_fraction = None;
        let shown = ShownHour {
            epoch,
            branch,
            hour,
            error: None,
            political_warnings: political.layers.warnings().to_vec(),
            plan_warnings,
            building_warnings,
            model_warnings,
            political_summary: political.layers.summary(),
            political_build: political.layers.build,
            limitations,
            header,
            borders: Some((Arc::clone(&political), Arc::clone(&borders))),
            snow: (
                snow_mud.summary,
                [snow_mud.width, snow_mud.height],
                snow_mud.build,
            ),
        };
        self.animation_seek = false;
        retired.push(self.shown.replace(shown));
        // A refused render can leave the previous hour's province list unread.
        // Supersede it only after every fallible upload succeeded; selection and
        // map-icon pools remain owned by this view, and unconsumed forces survive.
        self.counter_view
            .presentation
            .schedule()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancel_pending();
        retired.push((political, borders, rails, buildings, snow_mud, intel));
        laps.lap("apply_end");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_world::map::strike_effects::{Impact, Impacts};

    fn hour(id: u64) -> Impacts {
        Impacts {
            impacts: vec![Impact {
                id,
                province: 1,
                effect: viewer_world::map::strike_effects::Effect::Nuclear,
                position: [0.0; 3],
                bounds: [0.0; 4],
            }],
            refused: Vec::new(),
        }
    }

    #[test]
    fn only_continuing_playback_starts_the_events_of_skipped_hours() {
        let skipped = [hour(11)];
        let presented = hour(12);
        let ids = |continuity| {
            presented_events(skipped.iter(), &presented, continuity)
                .into_iter()
                .map(|impacts| impacts.impacts[0].id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(Continuity::Continue), [11, 12]);
        // A forward seek from 10 to 12 does not replay the strike at 11.
        assert_eq!(ids(Continuity::Reset), [12]);
        assert_eq!(
            presented_events(std::iter::empty(), &presented, Continuity::Continue).len(),
            1
        );
    }
}
