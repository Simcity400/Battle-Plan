
use super::{Document, InterfaceSound, Viewer, read_click};
use eframe::egui;
use replay_data::{BranchId, Hour};
use replay_frame::frame::DecodedFrame;
use std::sync::Arc;
use viewer_counters::selection_sounds;
use viewer_engagements::combat_selection::Selection as Window;
use viewer_native_gui::counter_hovers::unit_hits::{Hit, Kind};
use viewer_runtime::unit_handle::Unit;
use viewer_raids::map::raids::offers::TooltipTarget;
use viewer_map_modes::map_mode::{MapMode, Stock};

#[derive(Clone, Debug)]
pub(super) enum Click {
    NavalMission { viewer: u32, region: u32, mission: u32, rect: [f32; 4], layer: u64, order: usize },
    AirGrid { members: Arc<Vec<u64>>, shift: bool },
    AirBase { viewer: u32, province: u32, kind: i32, shift: bool },
    NavalBase { viewer: u32, province: u32, rect: [f32; 4], layer: u64 },
    AirMission { viewer: u32, region: u32, mission: usize, shift: bool },
    Raid { viewer: u32, province: u32, targets: Arc<[TooltipTarget]> },
    Province { province: u32, mode: MapMode },
    Observe { viewer: u32, province: u32, mode: MapMode, ground: bool, shift: bool },
    BoxAir { members: Vec<u64>, shift: bool },
    Sounds { viewer: u32, added: Vec<Unit>, fleet: bool, order: usize },
    AirOrder { region: Option<u32>, wings: Arc<Vec<u64>>, shift: bool, order: usize },
    Box(Box<super::selection_input::HeldBox>),
    Model(super::selection_input::HeldModel),
}

impl Click {
    fn name(&self) -> &'static str {
        match self {
            Self::NavalMission { .. } => "the naval mission's members",
            Self::AirGrid { .. } => "the air grid's selection",
            Self::AirBase { .. } => "the air base's selection",
            Self::NavalBase { .. } => "the naval base's task forces",
            Self::AirMission { .. } => "the air mission's selection",
            Self::Raid { .. } => "the raid targets' details",
            Self::Province { .. } => "the clicked province's state",
            Self::Observe { .. } => "the Ctrl+click's country",
            Self::BoxAir { .. } => "the box's air wings",
            Self::Sounds { .. } => "the selection's sounds",
            Self::AirOrder { .. } => "the air order's answer",
            Self::Box(_) => "the box's units",
            Self::Model(_) => "the unit model's selection",
        }
    }
}

#[derive(Debug)]
pub(super) struct Held {
    click: Click,
    at: Option<(BranchId, Hour)>,
}

#[must_use]
enum Answer {
    Done,
    Loading,
}

impl Viewer {
    pub(super) fn click_hour(&self) -> Option<(BranchId, Hour)> {
        super::live_of(&self.playhead_map).and_then(|side| side.branch()).zip(self.overlay_hour)
    }

    pub(super) fn clickable(&self, document: &Document) -> super::head_rows::Clickable {
        use viewer_counters::counters::Shown;
        let defines = &document.content.counters.defines;
        let on_screen = self.overlay_placed.as_ref().zip(self.overlay_view.as_ref()).map(|(camera, view)| viewer_playback::overlay::shown(camera, None, view, defines));
        let shown = [self.overlay_asked, on_screen].into_iter().flatten().fold(super::head_rows::Clickable::GROUND.shown, |held, more| Shown {
            units: held.units || more.units,
            bases: held.bases || more.bases,
            land_combats: held.land_combats || more.land_combats,
            naval_missions: held.naval_missions || more.naval_missions,
        });
        let strategic_air = matches!(self.shown_mode(), MapMode::Stock(Stock::StrategicAir));
        let figures = super::live_of(&self.playhead_map).is_some_and(|side| !side.products().pick_divisions.is_empty());
        let raids = self.overlay_hits.raids.iter().any(|hit| matches!(hit.key, viewer_raids::map::raids::tooltips::Key::Offer(_)));
        super::head_rows::Clickable { shown, air: shown.bases || strategic_air || !self.air_selection.wings().is_empty(), divisions: shown.units || figures, raids }
    }

    pub(super) fn viewing(&mut self, ours: Option<u32>, what: &str) -> Option<u32> {
        if ours.is_none() {
            self.selection_input.refuse(format!("{what}: the click came while the viewing country was loading"));
        }
        ours
    }

    pub(super) fn click(&mut self, document: &Document, click: Click, frame: Option<&DecodedFrame>, ctx: &egui::Context) {
        if self.held_clicks.is_empty() && matches!(self.answer(document, &click, frame, ctx), Answer::Done) {
            return;
        }
        self.hold(click);
    }

    fn hold(&mut self, click: Click) {
        if super::live_of(&self.playhead_map).is_none() {
            self.selection_input.refuse(format!("{}: the map holds no rows to answer it", click.name()));
            return;
        }
        self.held_clicks.push_back(Held { click, at: self.head_rows.clicked_at() });
    }

    pub(super) fn answer_held(&mut self, document: &Document, frame: Option<&DecodedFrame>, ctx: &egui::Context) {
        if self.held_clicks.is_empty() {
            return;
        }
        let now = self.head_rows.clicked_at();
        let open = super::live_of(&self.playhead_map).is_some();
        while let Some(held) = self.held_clicks.pop_front() {
            self.head_rows.click_at(held.at);
            let answer = if open { self.answer(document, &held.click, frame, ctx) } else { Answer::Loading };
            match answer {
                Answer::Done => {}
                Answer::Loading if open && held.at == now => {
                    self.held_clicks.push_front(held);
                    break;
                }
                Answer::Loading => self.selection_input.refuse(format!("{}: the map left the hour it was clicked at before that hour's rows were read", held.click.name())),
            }
        }
        self.head_rows.click_at(now);
        for held in &self.held_clicks {
            self.overlay_loading.push(format!("{}: loading the rows of the hour it was clicked at", held.click.name()));
        }
        if !self.held_clicks.is_empty() {
            ctx.request_repaint();
        }
    }

    fn answer(&mut self, document: &Document, click: &Click, frame: Option<&DecodedFrame>, ctx: &egui::Context) -> Answer {
        let head = super::head_rows::head(super::live_of(&self.playhead_map));
        match click {
            &Click::NavalMission { viewer, region, mission, rect, layer, order } => {
                let Some(read) = self.head_rows.clicks(head, frame, |frame| viewer_counters::naval_selection::mission_members(frame, viewer, region, mission)) else {
                    return Answer::Loading;
                };
                let Some(members) = read_click(&mut self.overlay_loading, click.name(), Some(read)) else { return Answer::Done };
                let selection = Hit { plate: None, ours: true, port: false, rect, layer, kind: Kind::TaskForce, members: members.into() };
                let before = Arc::clone(self.unit_selection.units());
                self.unit_selection.click(Some(&selection), false);
                self.ground_selection = None;
                self.air_selection.clear();
                self.fleet_selection.clear();
                self.counter_view.selected = Arc::clone(self.unit_selection.units());
                self.selection_added(document, &before, Some(viewer), frame, order);
            }
            Click::AirGrid { members, shift } => {
                let shift = *shift;
                let Some(read) = self.head_rows.clicks(head, frame, |frame| self.air_selection.grid_click(frame, members, shift)) else {
                    return Answer::Loading;
                };
                if read_click(&mut self.overlay_loading, click.name(), Some(read)).unwrap_or(false) {
                    self.unit_selection.clear();
                    if !shift {
                        self.ground_selection = None;
                    }
                    self.counter_view.selected = Arc::clone(self.unit_selection.units());
                }
            }
            &Click::AirBase { viewer, province, kind, shift } => {
                let Some(read) = self.head_rows.clicks(head, frame, |frame| viewer_counters::air_selection::base_target(frame, viewer, province, kind)) else {
                    return Answer::Loading;
                };
                let Some(target) = read_click(&mut self.overlay_loading, "the air base's target", Some(read)).flatten() else { return Answer::Done };
                self.choose_map_mode(MapMode::Stock(Stock::StrategicAir));
                let head = super::head_rows::head(super::live_of(&self.playhead_map));
                let Some(read) = self.head_rows.clicks(head, frame, |frame| self.air_selection.base_click(frame, target.clone(), shift)) else {
                    return Answer::Loading;
                };
                read_click(&mut self.overlay_loading, click.name(), Some(read));
                self.unit_selection.clear();
                if !shift {
                    self.ground_selection = None;
                }
                self.counter_view.selected = Arc::clone(self.unit_selection.units());
            }
            &Click::NavalBase { viewer, province, rect, layer } => {
                let Some(read) = self.head_rows.clicks(head, frame, |frame| viewer_counters::naval_selection::base_members(frame, viewer, province)) else {
                    return Answer::Loading;
                };
                let Some(members) = read_click(&mut self.overlay_loading, click.name(), Some(read)) else { return Answer::Done };
                if members.is_empty() {
                    self.choose_map_mode(MapMode::Stock(Stock::StrategicNavy));
                } else {
                    let selection = Hit { plate: None, ours: true, port: false, rect, layer, kind: Kind::TaskForce, members: members.into() };
                    self.unit_selection.click(Some(&selection), false);
                    self.ground_selection = None;
                    self.air_selection.clear();
                    self.fleet_selection.clear();
                    self.counter_view.selected = Arc::clone(self.unit_selection.units());
                }
            }
            &Click::AirMission { viewer, region, mission, shift } => {
                let Some(read) = self.head_rows.clicks(head, frame, |frame| viewer_counters::air_selection::mission_members(frame, viewer, region, mission)) else {
                    return Answer::Loading;
                };
                let Some(members) = read_click(&mut self.overlay_loading, "the air mission's members", Some(read)) else { return Answer::Done };
                let Some(read) = self.head_rows.clicks(head, frame, |frame| self.air_selection.mission_click(frame, &members, shift)) else {
                    return Answer::Loading;
                };
                if read_click(&mut self.overlay_loading, click.name(), Some(read)).unwrap_or(false) {
                    self.unit_selection.clear();
                    if !shift {
                        self.ground_selection = None;
                    }
                    self.counter_view.selected = Arc::clone(self.unit_selection.units());
                }
            }
            Click::Raid { viewer, province, targets } => {
                let content = Arc::clone(&document.raid_hover);
                let Some(details) = self.head_rows.clicks(head, frame, |frame| viewer_raids::map::raids::tooltips::details(&content, frame, *viewer, targets)) else {
                    return Answer::Loading;
                };
                self.counter_view.raid_details = Some((*province, Arc::new(details)));
                self.ground_selection = None;
                self.unit_selection.clear();
                self.air_selection.clear();
                self.fleet_selection.clear();
                self.counter_view.selected = Arc::clone(self.unit_selection.units());
            }
            Click::Province { province, mode } => return self.answer_province(document, frame, *province, mode),
            Click::Observe { viewer, province, mode, ground, shift } => {
                let kind = document.content.counters.kinds.get(province).copied();
                let Some(read) = self.head_rows.clicks(head, frame, |frame| super::observe::ctrl_click_target(frame, mode, *province, kind, *viewer)) else {
                    return Answer::Loading;
                };
                match read {
                    Err(why) => self.overlay_loading.push(format!("{}: {why}", click.name())),
                    Ok(Some(target)) => self.observe(target),
                    Ok(None) => {
                        if matches!(mode, MapMode::Stock(Stock::Diplomacy))
                            && *ground
                            && let Some(country) = self.counter_report.perspective.province_controller(*province)
                        {
                            self.diplomatic_selected = Some(country);
                        }
                        if *ground {
                            self.click_units(None, *shift, ctx);
                            self.click(document, Click::Province { province: *province, mode: mode.clone() }, frame, ctx);
                        }
                    }
                }
            }
            Click::BoxAir { members, shift } => {
                let shift = *shift;
                let Some(read) = self.head_rows.clicks(head, frame, |frame| self.air_selection.box_click(frame, members, shift)) else {
                    return Answer::Loading;
                };
                if read_click(&mut self.overlay_loading, click.name(), Some(read)).unwrap_or(false) {
                    if !shift {
                        self.unit_selection.clear();
                        self.fleet_selection.clear();
                        self.ground_selection = None;
                    }
                    self.counter_view.selected = Arc::clone(self.unit_selection.units());
                    self.counter_view.fleets_selected = Arc::clone(self.fleet_selection.fleets());
                }
            }
            Click::Sounds { viewer, added, fleet, order } => return self.answer_sounds(document, added, *fleet, *viewer, frame, *order),
            Click::AirOrder { region, wings, shift, order } => return self.answer_air_order(document, *region, wings, *shift, *order),
            Click::Box(held) => {
                return match self.answer_box(document, held, frame, ctx) {
                    Some(()) => Answer::Done,
                    None => Answer::Loading,
                };
            }
            Click::Model(held) => {
                return match self.answer_model(document, held, frame, ctx) {
                    Some(()) => Answer::Done,
                    None => Answer::Loading,
                };
            }
        }
        ctx.request_repaint();
        Answer::Done
    }

    fn answer_province(&mut self, document: &Document, frame: Option<&DecodedFrame>, province: u32, mode: &MapMode) -> Answer {
        use super::ground_click::{Ground, Outline, Picked, Selected, has_state, naval_region, route, selects, state_provinces};
        let head = super::head_rows::head(super::live_of(&self.playhead_map));
        let Some(read) = self.head_rows.clicks(head, frame, |frame| has_state(frame, province)) else { return Answer::Loading };
        let Some(state) = read_click(&mut self.overlay_loading, "the clicked province's state", Some(read)) else { return Answer::Done };
        let counters = &document.content.counters;
        let region = counters.region(province);
        let picked = Picked {
            province,
            kind: counters.kinds.get(&province).copied(),
            state,
            region,
            naval: region.map_or(Ok(false), |region| naval_region(region, counters.region_provinces(region).iter().map(|member| counters.kinds.get(member).copied()))),
        };
        let selection = match selects(mode, &picked) {
            Some(Selected::Region(region)) => Some(Outline { provinces: counters.region_provinces(region).into(), state: false }),
            Some(Selected::State(province)) => {
                let Some(read) = self.head_rows.clicks(head, frame, |frame| state_provinces(frame, province)) else { return Answer::Loading };
                read_click(&mut self.overlay_loading, "the clicked state's provinces", Some(read)).map(|provinces| Outline { provinces: provinces.into(), state: true })
            }
            None => None,
        };
        self.ground_selection = selection;
        let open = |combat: &mut super::combat::Window, window: Window| {
            if combat.selection != Some(window) {
                combat.open(window);
            }
        };
        match route(mode, &picked) {
            Some(Ground::AirRegion(region)) => open(&mut self.combat, Window::AirRegion(region)),
            Some(Ground::NavalRegion(region)) => open(&mut self.combat, Window::NavalRegion(region)),
            Some(Ground::State(province)) => open(&mut self.combat, Window::State(province)),
            Some(Ground::Close) => {
                if matches!(self.combat.selection, Some(Window::State(_) | Window::NavalRegion(_))) {
                    self.combat.close();
                }
            }
            Some(Ground::Refused(why)) => self.overlay_loading.push(format!("the ground click: {why}")),
            None => {}
        }
        Answer::Done
    }

    fn answer_sounds(&mut self, document: &Document, added: &[Unit], fleet: bool, viewer: u32, frame: Option<&DecodedFrame>, order: usize) -> Answer {
        let Some(audio) = document.audio.as_ref() else { return Answer::Done };
        let mut queued = selection_sounds::Queued::default();
        if !added.is_empty() {
            let sub_units = &document.content.unit_models.sub_units;
            let mask = |name: &str| sub_units.subunit(name).map(|sub_unit| sub_unit.category);
            let head = super::head_rows::head(super::live_of(&self.playhead_map));
            let Some(read) = self.head_rows.clicks(head, frame, |frame| selection_sounds::queued(frame, added, viewer, &mask)) else {
                return Answer::Loading;
            };
            let Some(read) = read_click(&mut self.overlay_loading, "the selection's sounds", Some(read)) else { return Answer::Done };
            queued = read;
        }
        if fleet && !queued.effects.contains(&selection_sounds::FLEET) {
            queued.effects.push(selection_sounds::FLEET);
        }
        if queued.effects.is_empty() && queued.voice.is_none() && queued.refused.is_empty() {
            return Answer::Done;
        }
        let known = |effect: &str| audio.content.catalog.effect(effect).is_some();
        let cool_down = audio.content.defines.as_ref().map(|defines| f64::from(defines.voice_cool_down));
        let now = self.started.elapsed().as_secs_f64();
        let sounds = self.selection_input.sounds(now, queued, &known, cool_down, order);
        self.interface_sounds.extend(sounds);
        Answer::Done
    }

    fn answer_air_order(&mut self, document: &Document, region: Option<u32>, wings: &Arc<Vec<u64>>, shift: bool, order: usize) -> Answer {
        use viewer_counters::air_orders;
        let head = super::head_rows::head(super::live_of(&self.playhead_map));
        let reads = document.icons.raids.movement_reads();
        let content = &document.counter_content;
        let read = self.head_rows.clicks(head, None, |frame| match region {
            None => air_orders::base_right_click(frame, wings),
            Some(region) => air_orders::region_right_click(frame, wings, region, shift, || {
                viewer_counter_products::map::province_tooltips::assignment_checks(content, reads, frame, region, wings)
            }),
        });
        let Some(response) = read else { return Answer::Loading };
        self.interface_sounds.extend(response.sounds.iter().map(|effect| InterfaceSound { effect: Arc::from(*effect), click: None, order }));
        self.air_orders.answered(response);
        Answer::Done
    }

    pub(super) fn selection_sounds(&mut self, document: &Document, added: &[Unit], fleet: bool, ours: Option<u32>, frame: Option<&DecodedFrame>, order: usize) {
        if document.audio.is_none() || (added.is_empty() && !fleet) {
            return;
        }
        let Some(viewer) = ours.or(added.is_empty().then_some(0)) else { return };
        let click = Click::Sounds { viewer, added: added.to_vec(), fleet, order };
        if self.held_clicks.is_empty() && matches!(self.answer_sounds(document, added, fleet, viewer, frame, order), Answer::Done) {
            return;
        }
        self.hold(click);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Config, MapHit, Options, Viewer};
    use std::sync::{Arc, Mutex};
    use viewer_native_gui::{counter_hovers::unit_hits, sprites::layer};
    use viewer_raids::map::raids::tooltips::{Hit, Key};

    #[test]
    fn a_click_before_the_installed_layout_is_drawn_hits_the_icons_on_screen() {
        let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(Mutex::new(None)), None, None);
        viewer.overlay_hits.raids.push(Hit { origin: [100.0, 100.0], size: [40.0, 40.0], key: Key::Offer(Arc::from([])), province: 9, layer: layer(15, 0, 0), supported: true });
        viewer.counter_report.base_hits.push(unit_hits::Blocker { rect: [300.0, 100.0, 40.0, 40.0], layer: layer(3, 0, 0) });
        viewer.hits_stamp += 1;
        assert_eq!(viewer.map_hit([110.0, 110.0]), Some(MapHit::Raid));
        assert_eq!(viewer.raid_hit([110.0, 110.0]).map(|hit| hit.province), Some(9), "the bubble the index found is the one its click reads");
        assert_eq!(viewer.map_hit([310.0, 110.0]), Some(MapHit::Base(0)));
        assert_eq!(viewer.map_hit([200.0, 110.0]), None);
    }
}
