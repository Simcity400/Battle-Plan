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
use viewer_world::map::scene::SceneInput;
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
    Arc<hours::Shared<map_display_data::buildings::Buildings>>,
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

/// Cheap refresh and size checks precede the snow content the window uploads.
fn same_snow_content(
    had: &viewer_world::map::weather::SnowMud,
    snow: &viewer_world::map::weather::SnowMud,
) -> bool {
    had.refresh == snow.refresh
        && had.width == snow.width
        && had.height == snow.height
        && had.province_snow == snow.province_snow
        && had.texels == snow.texels
        && had.unknown == snow.unknown
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
                    document.playback.map_binding(),
                )?,
                None => hours::Pipeline::with_map_binding(
                    Arc::clone(&document.content),
                    replay,
                    document.playback.map_binding(),
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
        // Playing, request one pool of hours from the playhead; paused, a step
        // either way is already built, except on a dashboard map, which only
        // seeks. Do not turn cold-start latency into a recording-wide window.
        let (ahead, behind) = if self.playing {
            (
                self.hours.as_ref().expect("started above").playback_ahead(),
                0,
            )
        } else if self.dashboard_map {
            (0, 0)
        } else {
            (2, 2)
        };
        let ahead = self
            .options
            .playback_cost_end
            .map_or(ahead, |end| ahead.min(end.saturating_sub(playhead.hour)));
        let wait = exact_capture_wait(
            self.capture.as_ref().map(|capture| capture.remaining),
            profiling(),
        );
        let activation = grid_activation(
            hours::page_grid(),
            self.playing,
            self.dashboard_map,
            matches!(
                self.map_mode,
                viewer_world::map::map_mode::MapMode::Scripted(_)
            ),
            self.capture.is_some(),
            wait,
            self.profile_playback.wall_active(),
            self.speed,
        );
        let count = u32::try_from(ahead.saturating_add(1)).unwrap_or(0);
        let lead = build_lead(self.speed, self.build_latency_ms);
        let ahead = match activation {
            Ok(stride) => i32::try_from(u64::from(lead) + u64::from(stride) * u64::from(count))
                .unwrap_or(i32::MAX),
            Err(_) => ahead,
        };
        let shown = shown_held(
            self.shown
                .as_ref()
                .map(|shown| (shown.epoch, shown.branch, shown.hour)),
            self.epoch,
            self.animation_seek,
        );
        let grid = activation.ok().map(|stride| {
            let (installed, usable) = grid_feedback(
                &self.ready,
                |built| built.layers.is_ok(),
                shown,
                playhead.branch,
            );
            hours::Grid {
                cadence: hours::Cadence {
                    sequence: 0,
                    stride,
                    count,
                    anchor: None,
                    lead,
                },
                installed,
                usable,
            }
        });
        let want = hours::Want {
            branch: playhead.branch,
            hour: playhead.hour,
            ahead,
            behind,
            playing: self.playing,
            perspective: self.perspective.clone(),
            map_mode: self.map_mode.clone(),
            diplomatic_selected: self.diplomatic_selected,
            show_allied_plans: self.graphics.show_allied_plans,
            counters: self.counter_view.clone(),
            view,
            have: held_hours(self.ready.keys().copied(), shown, playhead.branch),
            received: 0,
            grid,
        };
        // Everything this frame lets go of, freed off the frame path at its end.
        let mut retired = Retired::default();
        let pipeline = self.hours.as_mut().expect("started above");
        pipeline.note_grid(activation, playhead.hour);
        let epoch = pipeline.want(want)?;
        let new_epoch = epoch != self.epoch;
        if new_epoch {
            self.epoch = epoch;
            retired.push(std::mem::take(&mut self.ready));
        }
        if new_epoch || !self.playing {
            self.held_back = None;
        }
        if self.dropper.is_none() {
            self.dropper = Some(Arc::new(Dropper::new()?));
        }
        let target = (playhead.branch, playhead.hour);
        poll_presentation(&mut NativePresentation {
            viewer: self,
            document,
            render: state,
            laps,
            epoch,
            target,
            behind,
            ahead,
            wait,
            fraction: playhead.fraction as f32,
            retired,
            passed: BTreeMap::new(),
        })?;
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
        let _cost = replay_frame::cpu::scope("viewer", Some("upload"));
        let hours::Built {
            epoch,
            branch,
            hour,
            view,
            current,
            viewing,
            layers,
            laps: built_laps,
            child_laps,
            wall,
            waits: _,
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
                    scene.apply(
                        &state.device,
                        &state.queue,
                        SceneInput::MissingMapTextEvents {
                            at: (branch, hour),
                            seek,
                            reason: format!("map text presented hour failed: {error}"),
                        },
                    )?;
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
                        viewing: viewing.as_ref(),
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
        // Presentation history starts again with the trains and the weather.
        if continuity == Continuity::Reset {
            self.day_night = Default::default();
            self.counter_view
                .presentation
                .movement()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clear();
        }
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
            held,
        } = layers;
        retired.push(std::mem::replace(&mut self.uploaded.products, held));
        self.music = music;
        self.battles = battles;
        let scene = self
            .scene
            .as_mut()
            .ok_or_else(|| std::io::Error::other("the map scene was not created"))?;
        laps.lap("apply_start");
        let uploaded = &mut self.uploaded;
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
                viewing: viewing.as_ref(),
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
            let replaced = scene
                .apply(
                    &state.device,
                    &state.queue,
                    SceneInput::SetTrains {
                        assets: document.trains.clone(),
                        plan: trains,
                        continuity,
                    },
                )?
                .into_trains();
            retired.push(replaced);
        }
        laps.lap("trains_upload");
        if keep_movers {
            retired.push(aircraft);
        } else {
            let replaced = scene
                .apply(
                    &state.device,
                    &state.queue,
                    SceneInput::SetAircraft {
                        assets: document.aircraft.clone(),
                        plan: aircraft,
                        continuity,
                    },
                )?
                .into_aircraft();
            retired.push(replaced);
        }
        laps.lap("aircraft_upload");
        if !keep_movers {
            match unit_restart {
                UnitRestart::Keep => {}
                UnitRestart::ViewingCountry => {
                    scene.apply(
                        &state.device,
                        &state.queue,
                        SceneInput::RestartUnitModels(Some(units.owned())),
                    )?;
                }
                UnitRestart::All => {
                    scene.apply(
                        &state.device,
                        &state.queue,
                        SceneInput::RestartUnitModels(None),
                    )?;
                }
            }
            if unit_restart == UnitRestart::Keep && !same_units {
                if let Some(previous) = &self.unit_models {
                    units.inherit_convoys(previous);
                }
            } else {
                units.reset_convoys();
            }
        }
        // The unit field installs with the hour's other layers rather than on a later frame.
        let unit_field = (!keep_movers).then(|| units.field(&document.unit_models));
        let model_warnings = units.report.warnings();
        retired.push(self.unit_models.replace(units));
        // The frame uploads the plan once, with this hour's air orders appended.
        if uploaded
            .orders
            .as_ref()
            .is_none_or(|had| !Arc::ptr_eq(had, &orders))
        {
            retired.push(uploaded.orders.replace(Arc::clone(&orders)));
        }
        let plan_warnings = orders.report.warnings();
        retired.push(orders);
        laps.lap("orders_upload");
        if keep_movers {
            retired.push(effects);
        } else {
            let replaced = scene
                .apply(
                    &state.device,
                    &state.queue,
                    SceneInput::SetPreparedWeatherEffects {
                        assets: document.weather_effects.clone(),
                        prepared: effects,
                        continuity,
                    },
                )?
                .into_weather_effects();
            retired.push(replaced);
        }
        laps.lap("effects_upload");
        // Equal snapshots preserve local refresh history; discontinuities reset it.
        let same_snow = uploaded
            .snow_mud
            .as_ref()
            .is_some_and(|had| Arc::ptr_eq(had, &snow_mud) || same_snow_content(had, &snow_mud));
        let snow_changed = !same_snow || continuity == Continuity::Reset;
        // fog of war: hours whose fog did not change share the grid on the GPU.
        let fog_changed = uploaded
            .intel
            .as_ref()
            .is_none_or(|had| !Arc::ptr_eq(had, &intel));
        let passed = skipped.as_ref().map(|hours| {
            hours
                .iter()
                .map(|(hour, _)| &hour.value)
                .collect::<Vec<_>>()
        });
        let installs_units = unit_field.is_some();
        let rails_changed = uploaded.rails != Some(rails.id);
        // The plan installs with the hour's air orders as no hover highlights them; a hover the
        // frame finds uploads them again.
        let highlighted = overlay.air_ranges.shown(None).highlighted_bases.clone();
        let plan = uploaded.orders.clone();
        // borders: the window has no demilitarised-zone override, so the zone is the
        // recorded one.
        scene.apply(
            &state.device,
            &state.queue,
            SceneInput::SetHourLayers(viewer_world::map::scene::HourLayers {
                buildings: (uploaded.buildings != Some(buildings.id)).then(|| {
                    (
                        &document.map_buildings,
                        buildings.value.clone(),
                        passed.as_deref(),
                    )
                }),
                political: (uploaded.political != Some(political.id))
                    .then_some(&political.layers.regions),
                border_plans: (uploaded.borders != Some(borders.id))
                    .then_some((&borders.classes, &borders.highlighted)),
                snow_mud: snow_changed.then_some((&snow_mud, continuity)),
                fog: fog_changed.then_some((&intel.texels, intel.fog)),
                unit_models: unit_field
                    .map(|field| (&document.unit_models, field, self.options.window_dump)),
                rails: rails_changed.then(|| {
                    let map_products::landscape::RailProduct(mesh, bridges, _) =
                        rails.value.as_ref();
                    (
                        &document.rail_assets,
                        mesh,
                        rails.geometry.as_ref(),
                        &document.bridges,
                        bridges.clone(),
                    )
                }),
                names: Some(&names),
                orders: plan.as_deref().map(|plan| {
                    (
                        &document.orders,
                        plan,
                        &*overlay.air_orders,
                        highlighted.as_slice(),
                    )
                }),
            }),
        )?;
        if let Some(plan) = plan {
            uploaded.air_order_sources = Some((plan, Arc::clone(&overlay.air_orders), highlighted));
        }
        uploaded.rails = Some(rails.id);
        uploaded.buildings = Some(buildings.id);
        uploaded.political = Some(political.id);
        uploaded.borders = Some(borders.id);
        if snow_changed {
            uploaded.snow_mud = Some(Arc::clone(&snow_mud));
        }
        if fog_changed {
            uploaded.intel = Some(Arc::clone(&intel));
        }
        if installs_units {
            self.unit_field_uploaded = true;
            // The field placed the raid units at the hour's start.
            self.raid_tick = Some(0.0);
        }
        laps.lap("hour_layers");
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
                scene.apply(
                    &state.device,
                    &state.queue,
                    SceneInput::PresentHours {
                        at: (branch, hour),
                        seek,
                        hours: presented_events(
                            skipped.iter().flatten().map(|(_, events)| events),
                            &presented,
                            continuity,
                        )
                        .into_iter()
                        .map(|(impacts, map_text)| (&**impacts, &**map_text))
                        .collect(),
                        continuity,
                        anchor: &|province: u32| document.content.counters.anchor(province, 0),
                        heights,
                        gui: &document.content.icons.counters.gui,
                    },
                )?;
            }
            crate::playback::Current::Gap { reason, .. } => {
                scene.apply(
                    &state.device,
                    &state.queue,
                    SceneInput::MissingMapTextEvents {
                        at: (branch, hour),
                        seek,
                        reason: format!("map text presented hour is unavailable: {reason}"),
                    },
                )?;
            }
            crate::playback::Current::Refused { reason, .. } => {
                scene.apply(
                    &state.device,
                    &state.queue,
                    SceneInput::MissingMapTextEvents {
                        at: (branch, hour),
                        seek,
                        reason: format!("map text presented hour cannot be shown: {reason}"),
                    },
                )?;
            }
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

type HourKey = (replay_data::BranchId, replay_data::Hour);

trait HourPresentation {
    fn completed(&self) -> &hours::Completions;
    fn install(&mut self, arrivals: Vec<hours::Built>) -> std::io::Result<()>;
    fn retire(&mut self, result: std::io::Result<()>) -> std::io::Result<()>;
}

fn poll_presentation(presentation: &mut impl HourPresentation) -> std::io::Result<()> {
    let arrivals = presentation.completed().take();
    let installed = arrivals.and_then(|arrivals| presentation.install(arrivals));
    presentation.retire(installed)
}

fn retain_ready<T>(
    ready: &mut BTreeMap<HourKey, T>,
    target: HourKey,
    window: (i32, i32),
    fallback: Option<HourKey>,
) -> BTreeMap<HourKey, T> {
    let (keep, gone) = std::mem::take(ready)
        .into_iter()
        .partition(|(key, _)| keeps_ready(*key, target, window, fallback));
    *ready = keep;
    gone
}

fn select_ready<T>(
    ready: &mut BTreeMap<HourKey, T>,
    target: HourKey,
    shown: Option<HourKey>,
    seek: bool,
    playing: bool,
    fallback: Option<HourKey>,
    passed: &mut BTreeMap<HourKey, T>,
) -> Option<T> {
    if !seek && shown == Some(target) {
        return None;
    }
    if let Some(next) = ready.remove(&target) {
        return Some(next);
    }
    if playing
        && let Some(key) =
            fallback.filter(|key| shown.is_none_or(|shown| shown.0 != key.0 || shown.1 < key.1))
    {
        let next = ready.remove(&key);
        let later = ready.split_off(&key);
        passed.extend(std::mem::replace(ready, later));
        return next;
    }
    None
}

struct NativePresentation<'a> {
    viewer: &'a mut Viewer,
    document: &'a mut Document,
    render: &'a eframe::egui_wgpu::RenderState,
    laps: &'a mut Laps,
    epoch: u64,
    target: HourKey,
    behind: i32,
    ahead: i32,
    wait: bool,
    fraction: f32,
    retired: Retired,
    passed: BTreeMap<HourKey, hours::Built>,
}
impl HourPresentation for NativePresentation<'_> {
    fn completed(&self) -> &hours::Completions {
        self.viewer
            .hours
            .as_ref()
            .expect("started above")
            .completed()
    }
    fn install(&mut self, arrivals: Vec<hours::Built>) -> std::io::Result<()> {
        let viewer = &mut *self.viewer;
        let document = &mut *self.document;
        let state = self.render;
        let laps = &mut *self.laps;
        let retired = &mut self.retired;
        let passed = &mut self.passed;
        let (epoch, target, behind, ahead, wait) =
            (self.epoch, self.target, self.behind, self.ahead, self.wait);
        let shown_hour = viewer
            .shown
            .as_ref()
            .filter(|shown| shown.epoch == epoch && shown.branch == target.0)
            .map(|shown| shown.hour);
        for built in arrivals {
            feed_raids(&viewer.raid_lane, &built);
            crate::perf_log::map_hour(viewer.perf_view.as_deref(), &built, target.1);
            if built.epoch == epoch {
                match &built.layers {
                    Ok(_) => {
                        viewer.build_latency_ms = next_latency(viewer.build_latency_ms, built.wall)
                    }
                    // A newer hour the clock passed that cannot be shown holds the map back.
                    Err(error)
                        if viewer.playing
                            && built.branch == target.0
                            && built.hour <= target.1
                            && shown_hour.is_none_or(|shown| shown < built.hour)
                            && viewer
                                .held_back
                                .as_ref()
                                .is_none_or(|(hour, _)| *hour <= built.hour) =>
                    {
                        viewer.held_back =
                            Some((built.hour, format!("hour {} failed: {error}", built.hour)));
                    }
                    Err(_) => {}
                }
                if let Some(old) = viewer.ready.insert((built.branch, built.hour), built) {
                    retired.push(old);
                }
            } else {
                retired.push(built);
            }
        }
        let fallback = viewer
            .playing
            .then(|| latest_built_key(&viewer.ready, target, |built| built.layers.is_ok()))
            .flatten();
        *passed = retain_ready(&mut viewer.ready, target, (behind, ahead), fallback);
        laps.lap("hours_poll");
        let shown_at = viewer
            .shown
            .as_ref()
            .filter(|shown| shown.epoch == epoch)
            .map(|shown| (shown.branch, shown.hour));
        let mut next = select_ready(
            &mut viewer.ready,
            target,
            shown_at,
            viewer.animation_seek,
            viewer.playing && !wait,
            fallback,
            passed,
        );
        if next.is_none() && wait && (viewer.animation_seek || shown_at != Some(target)) {
            let completed = viewer.hours.as_ref().expect("started above").completed();
            next = Some(wait_for_hour(
                &viewer.raid_lane,
                (target, epoch),
                || completed.wait(),
                &mut viewer.ready,
                |built| retired.push(built),
            )?);
        }
        match next {
            Some(built) => {
                // The hour on screen of any epoch: a perspective change starts
                // a new build epoch, but the scene's building history does not
                // depend on the viewing country, so it continues across it.
                let on_screen = viewer
                    .shown
                    .as_ref()
                    .map(|shown| (shown.branch, shown.hour));
                // Hours the builders passed over to keep up with the clock
                // continue the history, also on the catch-up after a pause; a
                // seek or a gap in the recording does not.
                let passes_unbuilt = !viewer.animation_seek
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
                        let passed_hour = viewer.ready.get(&key).or_else(|| passed.get(&key))?;
                        let layers = passed_hour.layers.as_ref().ok();
                        Some(layers.map(|layers| {
                            (
                                Arc::clone(&layers.buildings),
                                (Arc::clone(&layers.impacts), Arc::clone(&layers.map_text)),
                            )
                        }))
                    },
                );
                let (shown_epoch, hour, usable) = (built.epoch, built.hour, built.layers.is_ok());
                let started = std::time::Instant::now();
                let applied = viewer.apply_hour(state, document, built, skipped, retired, laps);
                crate::perf_log::map_upload(
                    viewer.perf_view.as_deref(),
                    shown_epoch,
                    hour,
                    target.1,
                    started.elapsed().as_secs_f64() * 1000.0,
                    usable && applied.is_ok(),
                );
                if usable
                    && applied.is_ok()
                    && viewer
                        .held_back
                        .as_ref()
                        .is_some_and(|(failed, _)| *failed <= hour)
                {
                    viewer.held_back = None;
                }
                applied
            }
            None => Ok(()),
        }
    }
    fn retire(&mut self, result: std::io::Result<()>) -> std::io::Result<()> {
        if !self.passed.is_empty() {
            self.retired.push(std::mem::take(&mut self.passed));
        }
        finish_hour_poll(
            &mut self.document.playback,
            self.viewer
                .dropper
                .as_ref()
                .expect("created before polling"),
            std::mem::take(&mut self.retired),
            result,
        )?;
        self.document.hour_fraction = if (
            self.document.playback.branch(),
            self.document.playback.hour(),
        ) == self.target
        {
            self.fraction
        } else {
            0.0
        };
        Ok(())
    }
}

fn finish_hour_poll<T>(
    playback: &mut crate::playback::Playback,
    dropper: &Dropper,
    mut retired: Retired,
    result: std::io::Result<T>,
) -> std::io::Result<T> {
    if let Some(source) = playback.release_source() {
        retired.push(source);
    }
    if !retired.is_empty() {
        dropper.drop_later(retired);
    }
    result
}

fn shown_held(
    shown: Option<(u64, replay_data::BranchId, i32)>,
    epoch: u64,
    seeking: bool,
) -> Option<(replay_data::BranchId, i32)> {
    shown
        .filter(|&(shown_epoch, ..)| shown_epoch == epoch && !seeking)
        .map(|(_, branch, hour)| (branch, hour))
}

fn held_hours(
    ready: impl Iterator<Item = (replay_data::BranchId, i32)>,
    shown: Option<(replay_data::BranchId, i32)>,
    branch: replay_data::BranchId,
) -> Vec<i32> {
    ready
        .chain(shown)
        .filter(|(at, _)| *at == branch)
        .map(|(_, hour)| hour)
        .collect()
}

fn keeps_ready(
    key: (replay_data::BranchId, i32),
    playhead: (replay_data::BranchId, i32),
    (behind, ahead): (i32, i32),
    fallback: Option<(replay_data::BranchId, i32)>,
) -> bool {
    let (branch, hour) = key;
    branch == playhead.0
        && ((playhead.1.saturating_sub(behind)..=playhead.1.saturating_add(ahead)).contains(&hour)
            || fallback == Some(key))
}

pub(crate) fn exact_capture_wait(remaining: Option<u32>, profiling: bool) -> bool {
    remaining.is_some_and(|remaining| !profiling || remaining == 0)
}

pub(crate) fn grid_activation(
    switch: bool,
    playing: bool,
    dashboard_map: bool,
    scripted: bool,
    capture: bool,
    exact_wait: bool,
    wall_sweep: bool,
    speed: f64,
) -> Result<u32, &'static str> {
    if !switch && !dashboard_map {
        return Err("switch-off");
    }
    if !playing {
        return Err("paused");
    }
    if scripted {
        return Err("scripted-map-mode");
    }
    if exact_wait || (capture && !wall_sweep) {
        return Err("capture");
    }
    hours::grid_stride(speed).ok_or("below-finest-stride")
}

/// Hours the clock passes while a target is built, from the measured build
/// latency; nothing is assumed before the first hour is measured.
pub(crate) fn build_lead(speed: f64, latency_ms: Option<f64>) -> u32 {
    let hours = speed * latency_ms.unwrap_or(0.0) / 1000.0;
    if hours.is_finite() && hours > 0.0 {
        hours.ceil().min(f64::from(u32::MAX)) as u32
    } else {
        0
    }
}

/// The build latency estimate after one more measured hour.
pub(crate) fn next_latency(estimate: Option<f64>, wall_ms: f64) -> Option<f64> {
    if !wall_ms.is_finite() || wall_ms <= 0.0 {
        return estimate;
    }
    // A quarter weight follows a change in load within a few hours.
    Some(estimate.map_or(wall_ms, |old| old + (wall_ms - old) * 0.25))
}

fn grid_feedback<T>(
    ready: &BTreeMap<(replay_data::BranchId, i32), T>,
    usable: impl Fn(&T) -> bool,
    shown: Option<(replay_data::BranchId, i32)>,
    branch: replay_data::BranchId,
) -> (Option<i32>, Vec<i32>) {
    let installed = shown.filter(|(at, _)| *at == branch).map(|(_, hour)| hour);
    let usable = ready
        .iter()
        .filter(|((at, _), built)| *at == branch && usable(built))
        .map(|((_, hour), _)| *hour)
        .collect();
    (installed, usable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_world::map::strike_effects::{Impact, Impacts};

    fn check_poll_cleanup(poll: impl FnOnce() -> std::io::Result<()>) -> std::io::Result<()> {
        use crate::playback::{Current, Playback};
        use std::sync::mpsc;

        struct ReportDrop(mpsc::Sender<std::thread::ThreadId>);
        impl Drop for ReportDrop {
            fn drop(&mut self) {
                let _ = self.0.send(std::thread::current().id());
            }
        }

        let mut playback = Playback::new(crate::playback::tests::fixture()).unwrap();
        let Current::Frame(shown) = playback.current().clone() else {
            panic!("the initial hour is recorded");
        };
        assert!(playback.holds_source());
        let at = (playback.branch(), playback.hour());
        let dropper = Dropper::new().unwrap();
        let (freed, dropped) = mpsc::channel();
        let mut retired = Retired::default();
        retired.push(ReportDrop(freed));
        let result = poll();
        let result = finish_hour_poll(&mut playback, &dropper, retired, result);
        assert!(!playback.holds_source());
        assert_eq!((playback.branch(), playback.hour()), at);
        assert!(matches!(
            playback.current(),
            Current::Frame(current) if Arc::ptr_eq(current, &shown)
        ));
        assert_ne!(
            dropped
                .recv_timeout(crate::map::test_support::TEST_HANG_GUARD)
                .unwrap(),
            std::thread::current().id(),
            "cleanup must hand its retirement to the bounded dropper",
        );
        result
    }

    #[test]
    fn a_disconnected_first_poll_releases_the_source_and_preserves_its_original_error() {
        let error = check_poll_cleanup(|| {
            let (send, receive) = std::sync::mpsc::channel::<hours::Built>();
            drop(send);
            hours::poll_results(|| receive.try_recv()).map(|_| ())
        })
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(error.to_string(), "the hour builders stopped");
    }

    #[test]
    fn a_successful_empty_first_poll_releases_the_source_without_changing_the_shown_hour() {
        check_poll_cleanup(|| {
            let (_send, receive) = std::sync::mpsc::channel::<hours::Built>();
            let built = hours::poll_results(|| receive.try_recv())?;
            assert!(built.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn an_exact_wait_failure_releases_the_source_and_preserves_its_original_error() {
        let error = check_poll_cleanup(|| {
            let (_send, receive) = std::sync::mpsc::channel::<hours::Built>();
            assert!(hours::poll_results(|| receive.try_recv())?.is_empty());
            let mut original = Some(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the exact hour is unavailable",
            ));
            wait_for_hour::<hours::Built>(
                &crate::raid_lane::RaidLane::default(),
                ((1, 137), 0),
                || Err(original.take().expect("the failed wait is not retried")),
                &mut BTreeMap::new(),
                |_| panic!("the failed wait delivered nothing"),
            )
            .map(|_| ())
        })
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "the exact hour is unavailable");
    }

    #[test]
    fn an_apply_failure_after_polling_releases_the_source_and_preserves_its_original_error() {
        let error = check_poll_cleanup(|| {
            let (_send, receive) = std::sync::mpsc::channel::<hours::Built>();
            assert!(hours::poll_results(|| receive.try_recv())?.is_empty());
            Err(std::io::Error::from_raw_os_error(5))
        })
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(5));
    }

    #[test]
    fn snow_content_comparison_keeps_its_value_in_either_order() {
        use viewer_world::map::weather::{Refresh, SnowMud};
        let snow = || SnowMud {
            width: 2,
            height: 1,
            texels: vec![1, 2, 3, 4, 5, 6, 7, 8].into(),
            unknown: vec![0, 1].into(),
            province_snow: vec![None, Some(3)].into(),
            post_factors: None,
            build: Default::default(),
            summary: Default::default(),
            refresh: Some(Refresh {
                interval: 25_000,
                identity: 7,
            }),
        };
        let original = |had: &SnowMud, snow: &SnowMud| {
            had.width == snow.width
                && had.height == snow.height
                && had.texels == snow.texels
                && had.unknown == snow.unknown
                && had.province_snow == snow.province_snow
                && had.refresh == snow.refresh
        };
        let base = snow();
        let changes: [fn(&mut SnowMud); 7] = [
            |_| {},
            |snow| snow.width = 3,
            |snow| snow.height = 2,
            |snow| snow.texels[7] = 0,
            |snow| snow.unknown[1] = 0,
            |snow| snow.province_snow[1] = None,
            |snow| snow.refresh = None,
        ];
        for (index, change) in changes.into_iter().enumerate() {
            let mut other = snow();
            change(&mut other);
            assert_eq!(
                same_snow_content(&base, &other),
                original(&base, &other),
                "change {index}"
            );
        }
        // Report-only fields stay outside the comparison, as before.
        let mut reported = snow();
        reported.summary.with_snow = 4;
        reported.build = std::time::Duration::from_millis(9);
        assert!(same_snow_content(&base, &reported));
    }

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
            (
                0,
                1,
                60827943,
                "shown_ns=0 shown_branch=1 shown_hour=60827943",
            ),
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
            for &hour in arrivals {
                self.ready.insert((1, hour), hour as u64);
            }
            let Some(key) = super::super::latest_built_key(&self.ready, (1, playhead), |_| true)
                .filter(|key| self.shown.is_none_or(|shown| key.1 > shown.1))
            else {
                return;
            };
            let skipped = super::super::skipped_hours(self.shown, key, true, |hour| {
                self.ready.get(&(1, hour)).map(|id| Some(*id))
            });
            let events: Vec<_> = skipped.unwrap_or_default().into_iter().map(hour).collect();
            let presented = hour(self.ready[&key]);
            let continuity = if self.shown.is_some() {
                Continuity::Continue
            } else {
                Continuity::Reset
            };
            for events in presented_events(events.iter(), &presented, continuity) {
                for impact in &events.impacts {
                    self.bytes.extend_from_slice(&impact.id.to_le_bytes());
                }
            }
            self.installed.push(key.1);
            self.shown = Some(key);
            self.ready.retain(|key, _| key.1 > self.shown.unwrap().1);
        }
    }

    struct TestPresentation {
        completed: hours::Completions,
        target: HourKey,
        playback: crate::playback::Playback,
        ready: BTreeMap<HourKey, crate::playback::Current>,
        passed: BTreeMap<HourKey, crate::playback::Current>,
        retired: Retired,
        dropper: Dropper,
        installed: Vec<i32>,
        retired_poll: bool,
    }
    impl HourPresentation for TestPresentation {
        fn completed(&self) -> &hours::Completions {
            &self.completed
        }
        fn install(&mut self, arrivals: Vec<hours::Built>) -> std::io::Result<()> {
            assert!(arrivals.is_empty(), "the usable hour was already in ready");
            let fallback = latest_built_key(&self.ready, self.target, |frame| {
                matches!(frame, crate::playback::Current::Frame(_))
            });
            self.passed = retain_ready(&mut self.ready, self.target, (0, 2), fallback);
            let shown = Some((self.playback.branch(), self.playback.hour()));
            if let Some(current) = select_ready(
                &mut self.ready,
                self.target,
                shown,
                false,
                true,
                fallback,
                &mut self.passed,
            ) {
                let crate::playback::Current::Frame(frame) = &current else {
                    panic!("recorded ready frame")
                };
                let (branch, hour) = (frame.branch, frame.hour);
                self.retired
                    .push(self.playback.present(branch, hour, current));
                self.installed.push(hour);
            }
            Ok(())
        }
        fn retire(&mut self, result: std::io::Result<()>) -> std::io::Result<()> {
            self.retired.push(std::mem::take(&mut self.passed));
            self.retired_poll = true;
            finish_hour_poll(
                &mut self.playback,
                &self.dropper,
                std::mem::take(&mut self.retired),
                result,
            )
        }
    }

    #[test]
    fn a_ready_hour_runs_the_production_take_install_retire_sequence() {
        use crate::playback::{Current, Playback};
        for target in [101, 102] {
            let (_send, receive) = std::sync::mpsc::sync_channel(1);
            let playback = Playback::new(crate::playback::tests::fixture()).unwrap();
            let replay::campaign::SeekResult::Frame(next) =
                playback.replay().seek(1, 101).unwrap()
            else {
                panic!("recorded ready hour")
            };
            let ready = BTreeMap::from([
                ((1, 100), playback.current().clone()),
                ((1, 101), Current::Frame(Arc::from(next))),
            ]);
            let mut presentation = TestPresentation {
                completed: hours::Completions::for_test(receive),
                target: (1, target),
                playback,
                ready,
                passed: BTreeMap::new(),
                retired: Retired::default(),
                dropper: Dropper::new().unwrap(),
                installed: Vec::new(),
                retired_poll: false,
            };
            poll_presentation(&mut presentation).unwrap();
            assert_eq!(presentation.installed, [101]);
            assert_eq!(
                (presentation.playback.branch(), presentation.playback.hour()),
                (1, 101)
            );
            assert!(presentation.ready.is_empty() && presentation.passed.is_empty());
            assert!(presentation.retired_poll && !presentation.playback.holds_source());
            let Current::Frame(frame) = presentation.playback.current() else {
                panic!("installed ready frame")
            };
            assert_eq!(
                frame
                    .complete::<replay_data::rows::tables::Clock>()
                    .unwrap()[0]
                    .hour,
                101
            );
        }
    }

    #[test]
    fn coordinator_ready_poll_permutations_preserve_installs_and_retained_event_bytes() {
        let mut baseline = None;
        for first in [[10, 11, 12], [12, 10, 11], [11, 12, 10]] {
            for second in [[13, 14, 15], [15, 13, 14], [14, 15, 13]] {
                let mut state = PollHistory::default();
                state.poll(10, &first);
                state.poll(14, &second);
                state.poll(15, &[]);
                assert_eq!(state.installed, [10, 14, 15]);
                let value = (state.installed, state.bytes);
                if let Some(baseline) = &baseline {
                    assert_eq!(&value, baseline);
                } else {
                    baseline = Some(value);
                }
            }
        }
    }

    #[test]
    fn coordinator_different_poll_availability_has_valid_not_identical_sequences() {
        let mut earlier = PollHistory::default();
        earlier.poll(11, &[10]);
        earlier.poll(11, &[11]);
        let mut later = PollHistory::default();
        later.poll(11, &[11]);
        later.poll(11, &[10]);
        assert_eq!(earlier.installed, [10, 11]);
        assert_eq!(later.installed, [11]);
        for state in [&earlier, &later] {
            assert!(state.installed.windows(2).all(|w| w[0] < w[1]));
            assert!(state.installed.iter().all(|hour| *hour <= 11));
            // Replay the same installed sequence and same skipped-built inputs
            // through the unchanged old presentation helpers, not arrival order.
            let mut old = PollHistory::default();
            for &hour in &state.installed {
                old.poll(11, &[hour]);
            }
            assert_eq!(state.bytes, old.bytes);
        }
    }

    #[test]
    fn coordinator_retained_oracle_includes_skipped_built_not_just_installed_sequence() {
        let mut with_skipped = PollHistory::default();
        with_skipped.poll(10, &[10]);
        with_skipped.poll(12, &[12, 11]);
        let mut without_skipped = PollHistory::default();
        without_skipped.poll(10, &[10]);
        without_skipped.poll(12, &[12]);
        assert_eq!(with_skipped.installed, without_skipped.installed);
        assert_ne!(with_skipped.bytes, without_skipped.bytes);
        assert_eq!(
            with_skipped.bytes,
            [10_u64, 11, 12]
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            without_skipped.bytes,
            [10_u64, 12]
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect::<Vec<_>>()
        );
        // Matching both installed sequence and skipped-built set is invariant
        // under within-poll delivery permutation, byte for byte.
        let mut old = PollHistory::default();
        old.poll(10, &[10]);
        old.poll(12, &[11, 12]);
        assert_eq!(with_skipped.bytes, old.bytes);
    }

    #[test]
    fn the_lead_is_where_the_clock_will_be_when_a_measured_build_lands() {
        assert_eq!(build_lead(1_440.0, None), 0);
        assert_eq!(build_lead(1_440.0, Some(100.0)), 144);
        assert_eq!(build_lead(1_440.0, Some(100.1)), 145);
        assert_eq!(build_lead(0.0, Some(100.0)), 0);
        assert_eq!(build_lead(f64::NAN, Some(100.0)), 0);
        assert_eq!(next_latency(None, 0.0), None);
        assert_eq!(next_latency(None, 80.0), Some(80.0));
        assert_eq!(next_latency(Some(80.0), 160.0), Some(100.0));
        assert_eq!(next_latency(Some(80.0), f64::NAN), Some(80.0));
    }

    #[test]
    fn the_page_grid_is_off_with_its_switch_and_outside_fast_unscripted_play() {
        for playing in [false, true] {
            for capture in [false, true] {
                for speed in [24.0, 720.0, 1_440.0] {
                    assert_eq!(
                        grid_activation(false, playing, false, false, capture, false, true, speed),
                        Err("switch-off")
                    );
                }
            }
        }
        let on = |playing, dashboard, scripted, capture, wall, speed| {
            grid_activation(
                true, playing, dashboard, scripted, capture, false, wall, speed,
            )
        };
        assert_eq!(on(true, false, false, false, false, 1_440.0), Ok(40));
        assert_eq!(on(true, false, false, false, false, 720.0), Ok(20));
        assert_eq!(
            on(true, false, false, false, false, 719.0),
            Err("below-finest-stride")
        );
        assert_eq!(
            on(false, false, false, false, false, 1_440.0),
            Err("paused")
        );
        assert_eq!(on(true, true, false, false, false, 1_440.0), Ok(40));
        assert_eq!(
            grid_activation(false, true, true, false, false, false, false, 1_440.0),
            Ok(40)
        );
        assert_eq!(
            grid_activation(false, false, true, false, false, false, false, 1_440.0),
            Err("paused")
        );
        assert_eq!(
            on(true, false, true, false, false, 1_440.0),
            Err("scripted-map-mode")
        );
        assert_eq!(on(true, false, false, true, false, 1_440.0), Err("capture"));
        assert_eq!(on(true, false, false, true, true, 1_440.0), Ok(40));
        for profiling in [false, true] {
            for remaining in [None, Some(0), Some(1)] {
                let wait = exact_capture_wait(remaining, profiling);
                assert_eq!(wait, remaining.is_some_and(|n| !profiling || n == 0));
                assert_eq!(
                    grid_activation(
                        true,
                        true,
                        false,
                        false,
                        remaining.is_some(),
                        wait,
                        true,
                        720.0
                    ),
                    if wait { Err("capture") } else { Ok(20) },
                );
            }
        }
    }

    #[test]
    fn a_polls_held_hours_partition_and_grid_feedback_keep_todays_cuts() {
        let ready: BTreeMap<(u64, i32), bool> = [
            ((1, 118), true),
            ((1, 119), true),
            ((1, 120), false),
            ((1, 122), true),
            ((2, 121), true),
        ]
        .into();
        let fallback_at = |ready: &BTreeMap<(u64, i32), bool>, hour| {
            super::super::latest_built_key(ready, (1, hour), |usable| *usable)
        };
        let kept = |ready: &BTreeMap<(u64, i32), bool>, playhead, window, fallback| -> Vec<_> {
            ready
                .keys()
                .copied()
                .filter(|key| keeps_ready(*key, playhead, window, fallback))
                .collect()
        };
        let fallback = fallback_at(&ready, 121);
        assert_eq!(fallback, Some((1, 119)));
        assert_eq!(
            kept(&ready, (1, 121), (0, 1), fallback),
            [(1, 119), (1, 122)]
        );
        assert_eq!(
            kept(&ready, (1, 120), (2, 2), None),
            [(1, 118), (1, 119), (1, 120), (1, 122)]
        );
        let failed: BTreeMap<(u64, i32), bool> = [((1, 120), false), ((1, 122), true)].into();
        let fallback = fallback_at(&failed, 121);
        assert_eq!(fallback, None);
        assert_eq!(kept(&failed, (1, 121), (0, 1), fallback), [(1, 122)]);
        let mut unavailable = failed.clone();
        unavailable.remove(&(1, 120));
        for (shown, ready, held) in [
            ((3, 1, 80), &failed, vec![120, 122, 80]),
            ((3, 1, 120), &unavailable, vec![122, 120]),
        ] {
            let displayed = shown_held(Some(shown), 3, false);
            assert_eq!(displayed, Some((1, shown.2)));
            assert_eq!(held_hours(ready.keys().copied(), displayed, 1), held);
            assert_eq!(
                grid_feedback(ready, |usable| *usable, displayed, 1),
                (Some(shown.2), vec![122])
            );
        }
        for (epoch, seeking) in [(4, false), (3, true)] {
            let shown = shown_held(Some((3, 1, 120)), epoch, seeking);
            assert_eq!(shown, None);
            assert_eq!(held_hours(failed.keys().copied(), shown, 1), [120, 122]);
            assert_eq!(
                grid_feedback(&failed, |usable| *usable, shown, 1),
                (None, vec![122])
            );
        }
        assert_eq!(shown_held(None, 3, false), None);
        let shown = shown_held(Some((3, 1, 117)), 3, false);
        assert_eq!(
            held_hours(ready.keys().copied(), shown, 1),
            [118, 119, 120, 122, 117]
        );
        assert_eq!(held_hours(ready.keys().copied(), shown, 2), [121]);
        assert_eq!(
            grid_feedback(&ready, |usable| *usable, shown, 1),
            (Some(117), vec![118, 119, 122])
        );
        assert_eq!(
            grid_feedback(&ready, |usable| *usable, shown, 2),
            (None, vec![121])
        );
    }
}
