use super::{
    Laps, OverlayKey, Playhead, ShownHour, Viewer, feed_raids, latest_built_key, present_raids,
    skipped_hours, wait_for_hour,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use viewer_foundation::map::presentation::Continuity;
use viewer_foundation::map::presentation::UnitRestart;
use viewer_foundation::map::presentation::continuity as presentation_continuity;
use viewer_foundation::map::presentation::crosses_gap;
use viewer_foundation::map::presentation::unit_restart as presentation_unit_restart;
use viewer_foundation::profile_writer::profile_print;
use viewer_foundation::profile_writer::profiling;
use viewer_foundation::profiling::BuilderSpan;
use viewer_foundation::retirement::Dropper;
use viewer_foundation::retirement::Retired;
use {
    crate::{document::Document, hours},
    viewer_foundation::map::camera::{Camera, MapSize},
};

/// CPU-side presentation point of a successfully submitted map frame, not a
/// GPU completion or scanout timestamp. Sampling never follows the log writer.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ShownStamp {
    ns: u128,
    branch: replay_data::BranchId,
    hour: replay_data::Hour,
}

impl std::fmt::Display for ShownStamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "shown_ns={} shown_branch={} shown_hour={}",
            self.ns, self.branch, self.hour
        )
    }
}

/// PROFILE-only monotonic time and the installed document, never the playhead.
/// The independent process-local origin is shared by every map in this process.
pub(super) fn shown_stamp(
    enabled: bool,
    shown: impl FnOnce() -> (replay_data::BranchId, replay_data::Hour),
) -> Option<ShownStamp> {
    shown_stamp_with(enabled, shown, || {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN.get_or_init(Instant::now).elapsed().as_nanos()
    })
}

fn shown_stamp_with(
    enabled: bool,
    shown: impl FnOnce() -> (replay_data::BranchId, replay_data::Hour),
    clock: impl FnOnce() -> u128,
) -> Option<ShownStamp> {
    enabled.then(|| {
        let ns = clock();
        let (branch, hour) = shown();
        ShownStamp { ns, branch, hour }
    })
}

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

/// Child scopes keep exact zeros and unavailable CPU samples, without the
/// frame collector's short-span filter. Nested scopes are inclusive.
fn child_lap_lines(laps: &[BuilderSpan]) -> (String, String) {
    let wall = laps
        .iter()
        .map(|span| format!("{}:{:.3}", span.name, span.ms))
        .collect::<Vec<_>>()
        .join(",");
    let cpu = laps
        .iter()
        .map(|span| match span.cpu_ms {
            Some(ms) => format!("{}:{ms:.3}", span.name),
            None => format!("{}:unavailable", span.name),
        })
        .collect::<Vec<_>>()
        .join(",");
    (wall, cpu)
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
        self.hours
            .as_ref()
            .expect("started above")
            .set_device(&state.device);
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
            child_laps,
            wall,
            trace: _,
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
            let (children, child_cpu) = child_lap_lines(&child_laps);
            profile_print(format!(
                "PROFILE built frame {} hour {hour} wall {wall:.3} parts {} cpu={} children={children} child_cpu={child_cpu}",
                self.profile_frame,
                built_laps
                    .iter()
                    .map(|(name, timing)| format!("{name}={:.2}", timing.ms))
                    .collect::<Vec<_>>()
                    .join(" "),
                built_laps
                    .iter()
                    .filter(|(name, _)| *name != "decode")
                    .map(|(name, timing)| match timing.cpu_ms {
                        Some(ms) => format!("{name}:{ms:.3}"),
                        None => format!("{name}:unavailable"),
                    })
                    .collect::<Vec<_>>()
                    .join(",")
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
            {
                let _rail_span = viewer_foundation::profiling::span("rail_geometry");
                let geometry = viewer_world::map::railways::PreparedRailGeometry::for_install(
                    &state.device,
                    mesh,
                    rails.geometry.as_ref(),
                )?;
                let replaced = scene.set_prepared_railways(
                    &state.device,
                    &state.queue,
                    &document.rail_assets,
                    geometry,
                )?;
                retired.push(replaced);
            }
            {
                let _bridges_span = viewer_foundation::profiling::span("bridges_install");
                scene.set_bridges(
                    &state.device,
                    &state.queue,
                    &document.bridges,
                    bridges.clone(),
                )?;
            }
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
            let _political_span = viewer_foundation::profiling::span("political_show");
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

    #[test]
    fn shown_stamps_preserve_wall_time_and_installed_identity_only_when_enabled() {
        assert_eq!(
            shown_stamp_with(
                false,
                || panic!("disabled stamp read the shown document"),
                || panic!("disabled stamp read the clock"),
            ),
            None
        );
        let samples = [
            (0, 1, 60827943, "shown_ns=0 shown_branch=1 shown_hour=60827943"),
            (
                16_666_667,
                1,
                60827943,
                "shown_ns=16666667 shown_branch=1 shown_hour=60827943",
            ),
            (
                1_333_333_333,
                9,
                60828400,
                "shown_ns=1333333333 shown_branch=9 shown_hour=60828400",
            ),
        ];
        for (ns, branch, hour, expected) in samples {
            let stamp = shown_stamp_with(true, || (branch, hour), || ns).unwrap();
            assert_eq!(stamp.ns, ns);
            assert_eq!((stamp.branch, stamp.hour), (branch, hour));
            assert_eq!(stamp.to_string(), expected);
        }
    }

    #[test]
    fn builder_child_lines_keep_zero_short_and_unavailable_samples() {
        let laps = [
            BuilderSpan {
                name: "fog.intel_build",
                ms: 0.0,
                cpu_ms: Some(0.0),
            },
            BuilderSpan {
                name: "fog.intel_lookup",
                ms: 0.012,
                cpu_ms: None,
            },
        ];
        assert_eq!(
            child_lap_lines(&laps),
            (
                "fog.intel_build:0.000,fog.intel_lookup:0.012".into(),
                "fog.intel_build:0.000,fog.intel_lookup:unavailable".into(),
            )
        );
        assert_eq!(child_lap_lines(&[]), (String::new(), String::new()));
    }

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

    #[derive(Default)]
    struct PollHistory {
        ready: BTreeMap<(u64, i32), u64>,
        shown: Option<(u64, i32)>,
        installed: Vec<i32>,
        bytes: Vec<u8>,
    }
    impl PollHistory {
        fn poll(&mut self, playhead: i32, arrivals: &[i32]) {
            for &hour in arrivals { self.ready.insert((1, hour), hour as u64); }
            let Some(key) = super::super::latest_built_key(&self.ready, (1, playhead), |_| true)
                .filter(|key| self.shown.is_none_or(|shown| key.1 > shown.1)) else { return };
            let skipped = super::super::skipped_hours(self.shown, key, true, |hour| {
                self.ready.get(&(1, hour)).map(|id| Some(*id))
            });
            let events: Vec<_> = skipped.unwrap_or_default().into_iter().map(hour).collect();
            let presented = hour(self.ready[&key]);
            let continuity = if self.shown.is_some() { Continuity::Continue } else { Continuity::Reset };
            for events in presented_events(events.iter(), &presented, continuity) {
                for impact in &events.impacts { self.bytes.extend_from_slice(&impact.id.to_le_bytes()); }
            }
            self.installed.push(key.1);
            self.shown = Some(key);
            self.ready.retain(|key, _| key.1 > self.shown.unwrap().1);
        }
    }

    #[test]
    fn coordinator_ready_poll_permutations_preserve_installs_and_retained_event_bytes() {
        let mut baseline = None;
        for first in [[10, 11, 12], [12, 10, 11], [11, 12, 10]] {
            for second in [[13, 14, 15], [15, 13, 14], [14, 15, 13]] {
                let mut state = PollHistory::default();
                state.poll(10, &first); state.poll(14, &second); state.poll(15, &[]);
                assert_eq!(state.installed, [10, 14, 15]);
                let value = (state.installed, state.bytes);
                if let Some(baseline) = &baseline { assert_eq!(&value, baseline); }
                else { baseline = Some(value); }
            }
        }
    }

    #[test]
    fn coordinator_different_poll_availability_has_valid_not_identical_sequences() {
        let mut earlier = PollHistory::default();
        earlier.poll(11, &[10]); earlier.poll(11, &[11]);
        let mut later = PollHistory::default();
        later.poll(11, &[11]); later.poll(11, &[10]);
        assert_eq!(earlier.installed, [10, 11]); assert_eq!(later.installed, [11]);
        for state in [&earlier, &later] {
            assert!(state.installed.windows(2).all(|w| w[0] < w[1]));
            assert!(state.installed.iter().all(|hour| *hour <= 11));
            // Replay the same installed sequence and same skipped-built inputs
            // through the unchanged old presentation helpers, not arrival order.
            let mut old = PollHistory::default();
            for &hour in &state.installed { old.poll(11, &[hour]); }
            assert_eq!(state.bytes, old.bytes);
        }
    }

    #[test]
    fn coordinator_retained_oracle_includes_skipped_built_not_just_installed_sequence() {
        let mut with_skipped = PollHistory::default();
        with_skipped.poll(10, &[10]); with_skipped.poll(12, &[12, 11]);
        let mut without_skipped = PollHistory::default();
        without_skipped.poll(10, &[10]); without_skipped.poll(12, &[12]);
        assert_eq!(with_skipped.installed, without_skipped.installed);
        assert_ne!(with_skipped.bytes, without_skipped.bytes);
        assert_eq!(with_skipped.bytes, [10_u64, 11, 12].into_iter().flat_map(u64::to_le_bytes).collect::<Vec<_>>());
        assert_eq!(without_skipped.bytes, [10_u64, 12].into_iter().flat_map(u64::to_le_bytes).collect::<Vec<_>>());
        // Matching both installed sequence and skipped-built set is invariant
        // under within-poll delivery permutation, byte for byte.
        let mut old = PollHistory::default();
        old.poll(10, &[10]); old.poll(12, &[11, 12]);
        assert_eq!(with_skipped.bytes, old.bytes);
    }

}
