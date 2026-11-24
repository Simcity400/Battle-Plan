use super::MapHit;
use eframe::egui;
use crate::document::Document;
use replay_data::rows::{Recorded, tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};
use viewer_counters::{box_selection, selection_sounds};
use viewer_native_gui::counter_hovers::unit_hits::Kind;
use viewer_runtime::unit_handle::Unit;
use super::unit_pick;
use viewer_playback::cursors::MapCursor;
use viewer_layers_forces::units::PickDivision;
use viewer_unit_models::model_pick::{Figures, Shape};
use viewer_models::{ModelLibrary, ModelPass};
use viewer_unit_history::unit_history::{History, LandFigure};

#[derive(Debug, Default)]
pub(crate) struct Theatres(BTreeMap<Unit, Result<u64, &'static str>>);

impl Theatres {
    pub(crate) fn read(frame: &DecodedFrame, units: &[Unit]) -> Self {
        let wants = |gun: bool| units.iter().any(|unit| matches!((unit, gun), (Unit::Division(_), false) | (Unit::RailwayGun(_), true)));
        let divisions = wants(false).then(|| frame.complete::<tables::Divisions>().ok());
        let guns = wants(true).then(|| frame.complete::<tables::RailwayGunCounterInputs>().ok());
        let mut out = BTreeMap::new();
        for &unit in units {
            let found = match unit {
                Unit::Division(handle) => divisions.flatten().map(|rows| rows.binary_search_by_key(&handle, |row| row.id).map(|at| &rows[at].theatre)),
                Unit::RailwayGun(handle) => guns.flatten().map(|rows| rows.binary_search_by_key(&handle, |row| row.gun).map(|at| &rows[at].theatre)),
                Unit::TaskForce(_) => continue,
            };
            let theatre = match found {
                None => Err(box_selection::THEATRE_UNREAD),
                Some(Err(_)) => Err(box_selection::THEATRE_ABSENT),
                Some(Ok(theatre)) => theatre_of(theatre),
            };
            out.insert(unit, theatre);
        }
        Self(out)
    }

    pub(crate) fn of(&self, unit: Unit) -> Result<u64, &'static str> {
        self.0.get(&unit).copied().unwrap_or(Err(box_selection::THEATRE_UNREAD))
    }
}

fn theatre_of(theatre: &Recorded<Option<u64>>) -> Result<u64, &'static str> {
    match theatre {
        Recorded::Value(Some(handle)) => Ok(*handle),
        Recorded::Value(None) => Err(box_selection::THEATRE_REFUSED),
        Recorded::NotRecorded => Err(box_selection::THEATRE_NOT_RECORDED),
    }
}

pub(crate) type ClientOrder = (Result<Arc<[u64]>, &'static str>, Option<u32>);

pub(crate) fn recorded_unit_order(frame: &DecodedFrame) -> ClientOrder {
    let list = match frame.complete::<tables::LandUnitOrder>() {
        Ok([row]) => match &row.units {
            Recorded::Value(Some(units)) => Ok(Arc::from(units.as_slice())),
            Recorded::Value(None) => Err(unit_pick::ORDER_REFUSED),
            Recorded::NotRecorded => Err(unit_pick::ORDER_NOT_RECORDED),
        },
        Ok(_) | Err(_) => Err(unit_pick::ORDER_NOT_RECORDED),
    };
    let links: Option<BTreeMap<u32, u32>> = frame
        .complete::<tables::CountryLinks>()
        .ok()
        .map(|rows| rows.iter().map(|row| (row.id, row.canonical_slot)).collect());
    let client = links.and_then(|links| viewer_foundation::fold_tables::perspective(frame, &links));
    (list, client)
}

#[derive(Clone, Debug)]
pub(crate) struct HeldBox {
    candidates: Vec<([f32; 4], box_selection::Member, Vec<u64>, bool)>,
    area: [f32; 4],
    order: Vec<box_selection::Member>,
    shift: bool,
    selected: Option<box_selection::Member>,
    first: Option<Unit>,
    ours: Option<u32>,
    sounds: usize,
}

impl HeldBox {
    pub(crate) fn units(&self) -> Vec<Unit> {
        use box_selection::Member;
        let mut units: Vec<Unit> = self
            .candidates
            .iter()
            .filter_map(|(_, member, members, _)| match member {
                Member::Unit(kind @ (Kind::Division | Kind::RailwayGun)) => Some(members.iter().map(|&handle| kind.unit(handle))),
                _ => None,
            })
            .flatten()
            .chain(self.first)
            .collect();
        units.sort_unstable();
        units.dedup();
        units
    }
}

#[derive(Clone, Debug)]
pub(crate) struct HeldModel {
    picked: u64,
    mates: Vec<Unit>,
    ours_unit: bool,
    shift: bool,
    ours: Option<u32>,
    sounds: usize,
}

impl HeldModel {
    pub(crate) fn units(&self, anchor: Option<Unit>) -> Vec<Unit> {
        let mut units: Vec<Unit> = self.mates.iter().copied().chain([Unit::Division(self.picked)]).chain(anchor).collect();
        units.sort_unstable();
        units.dedup();
        units
    }
}

#[derive(Default)]
pub(crate) struct SelectionInput {
    pub press: Option<BoxPress>,
    shapes: HashMap<usize, Option<Arc<Shape>>>,
    library: usize,
    figures: Option<Figures>,
    recent: selection_sounds::Recent,
    voice: selection_sounds::VoiceGate,
    refused: BTreeMap<String, usize>,
    pub order: super::order_cursor::Held,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Picked {
    pub candidates: Vec<PickDivision>,
    pub divisions: HashMap<u32, Vec<PickDivision>>,
    pub unordered: &'static str,
}

impl SelectionInput {
    pub fn dragging(&self) -> bool {
        self.press.is_some_and(|press| box_selection::dragging(press.from_frame, press.at_frame))
    }

    pub fn refuse(&mut self, reason: impl Into<String>) {
        *self.refused.entry(reason.into()).or_default() += 1;
    }

    pub fn note(&mut self, reason: &str) {
        self.refused.entry(reason.to_owned()).or_insert(1);
    }

    pub fn warnings(&self) -> impl Iterator<Item = String> + '_ {
        self.refused.iter().map(|(reason, count)| format!("Map selection: {reason} ({count})"))
    }

    #[expect(clippy::too_many_arguments, reason = "one pick's whole input, read where it is held")]
    pub fn pick(
        &mut self,
        history: &History,
        divisions: &HashMap<u32, Vec<PickDivision>>,
        library: &ModelLibrary,
        pass: Option<&ModelPass>,
        eye: [f32; 3],
        ray: [f32; 3],
        width: f32,
        fallback: Option<u32>,
        order: unit_pick::UnitOrder<'_>,
        ours: Option<u32>,
        after: Option<Unit>,
    ) -> Result<Option<Picked>, &'static str> {
        let identity = std::ptr::from_ref(library) as usize;
        if self.library != identity {
            self.library = identity;
            self.shapes.clear();
        }
        let mut slot = self.figures.take();
        let figures = Figures::current(&mut slot, history.land_stamp(), identity, width, || history.land_figures(), |mesh| {
            self.shape(library, mesh).and_then(|shape| shape.reach())
        });
        if figures.is_empty() {
            self.figures = slot;
            return Ok(None);
        }
        let mut met = |figure: &LandFigure| {
            let mut instance = figure.instance;
            let Some(shape) = self.shape(library, figure.mesh) else {
                return false;
            };
            let joints = match &*shape {
                Shape::Triangles { .. } => Vec::new(),
                Shape::Bounds { .. } => match pass.and_then(|pass| pass.root_joints(figure.mesh, &instance)) {
                    None => return false,
                    Some(Ok(joints)) => joints,
                    Some(Err(why)) => {
                        self.note(&format!("unit model pick: {why}"));
                        return false;
                    }
                },
            };
            instance.position[0] = viewer_gpu::unit_pose::unwrap(instance.position[0], eye[0], width);
            shape.hit(instance, &joints, eye, ray)
        };
        let hits: Vec<(u32, u32)> = figures.along(eye, ray).into_iter().filter(|figure| met(figure)).map(|figure| (figure.province, figure.order)).collect();
        self.figures = slot;
        let unit_pick::Walk { mut candidates, mut unordered } = unit_pick::walk(unit_pick::behind(&hits, divisions, false)?, order, ours, after)?;
        if candidates.is_empty()
            && let Some(province) = fallback
        {
            let own: Vec<(u32, u32)> = hits.iter().copied().filter(|&(at, _)| at == province).collect();
            candidates = unit_pick::behind(&own, divisions, true)?;
            unordered = unit_pick::PROVINCE_ORDER;
        }
        if candidates.is_empty() {
            return Ok(None);
        }
        let provinces: std::collections::BTreeSet<u32> = candidates
            .iter()
            .filter_map(|candidate| divisions.iter().find(|(_, list)| list.iter().any(|d| d.id == candidate.id)).map(|(&p, _)| p))
            .collect();
        let divisions = provinces.into_iter().filter_map(|province| Some((province, divisions.get(&province)?.clone()))).collect();
        Ok(Some(Picked { candidates, divisions, unordered }))
    }

    fn shape(&mut self, library: &ModelLibrary, mesh: usize) -> Option<Arc<Shape>> {
        if let Some(shape) = self.shapes.get(&mesh) {
            return shape.clone();
        }
        let shape = match (library.meshes.get(mesh), library.layers()) {
            (None, _) => Err(format!("unit model mesh {mesh} is not in the library")),
            (Some(model), None) => Err(format!("unit model {} was read through no layers", model.name)),
            (Some(model), Some(layers)) => Shape::of(layers, model),
        };
        let shape = shape
            .map_err(|why| self.refuse(format!("unit model pick: {why}")))
            .ok();
        self.shapes.insert(mesh, shape.clone());
        shape
    }

    pub fn sounds(
        &mut self,
        now: f64,
        queued: selection_sounds::Queued,
        known: &dyn Fn(&str) -> bool,
        cool_down: Option<f64>,
        order: usize,
    ) -> Vec<super::InterfaceSound> {
        let sound = |effect: &str| super::InterfaceSound { effect: Arc::from(effect), click: None, order };
        let mut sounds: Vec<_> = self
            .recent
            .admit(now, queued.effects.iter().copied().filter(|effect| known(effect)))
            .into_iter()
            .map(sound)
            .collect();
        match queued.voice {
            Some(Ok(voice)) => {
                if let Some(cool_down) = cool_down
                    && known(&voice)
                    && self.voice.admit(now, cool_down)
                {
                    sounds.push(sound(&voice));
                }
            }
            Some(Err(why)) => self.refuse(why),
            None => {}
        }
        for why in queued.refused {
            self.refuse(why);
        }
        sounds
    }
}

pub(crate) const OPERATIVES_BOX_WITHHELD: &str =
    "box selection in the Operatives mode: the operative icons the game selects first are not drawn";

#[derive(Clone, Copy, Debug)]
pub(crate) struct BoxPress {
    pub from: egui::Pos2,
    pub at: egui::Pos2,
    pub from_frame: [f32; 2],
    pub at_frame: [f32; 2],
}

impl BoxPress {
    pub fn new(pos: egui::Pos2, frame: [f32; 2]) -> Self {
        Self { from: pos, at: pos, from_frame: frame, at_frame: frame }
    }

    pub fn moved(&mut self, pos: egui::Pos2, frame: [f32; 2]) {
        (self.at, self.at_frame) = (pos, frame);
    }
}

fn kind_of(unit: Unit) -> Kind {
    match unit {
        Unit::Division(_) => Kind::Division,
        Unit::RailwayGun(_) => Kind::RailwayGun,
        Unit::TaskForce(_) => Kind::TaskForce,
    }
}

impl super::Viewer {
    pub(super) fn model_at(&mut self, document: &Document, point: [f32; 2], click: bool) -> Result<Option<Picked>, &'static str> {
        use viewer_map_modes::map_mode::{MapMode, Stock};
        let Some(scene) = self.scene.as_ref() else { return Ok(None) };
        let strategic = matches!(self.shown_mode(), MapMode::Stock(Stock::StrategicAir | Stock::StrategicNavy));
        let Some(view) = self.last_view.as_ref() else { return Ok(None) };
        let screen = point.map(|value| value * view.ui_scale);
        let fallback = (!click && !strategic)
            .then(|| {
                super::pick(&view.camera, screen, view.viewport, &document.map, document.assets.heightmap_texels())
                    .ok()
                    .flatten()
            })
            .flatten();
        let (eye, ray) = (view.camera.eye(), view.camera.ray(screen, view.viewport));
        let Some(side) = self.playhead_map.as_ref().filter(|side| side.on_screen().is_some()) else { return Ok(None) };
        let (divisions, ours) = (&side.products().pick_divisions, side.products().ours);
        let library = document.content.unit_models.library();
        let pass = scene.unit_pass();
        let units = self.unit_selection.units();
        let after = (click && units.len() == 1 && self.air_selection.wings().is_empty() && self.fleet_selection.fleets().is_empty())
            .then(|| units.first().copied())
            .flatten();
        let head = super::head_rows::head(super::live_of(&self.playhead_map));
        let (list, client) = self.head_rows.unit_order(head).unwrap_or((Err(unit_pick::ORDER_LOADING), None));
        let order = unit_pick::order_for(list.as_deref().map_err(|why| *why), client, ours);
        self.selection_input.pick(&scene.unit_history, divisions, &library, pass, eye, ray, document.map.width as f32, fallback, order, ours, after)
    }

    #[expect(clippy::too_many_arguments, reason = "a click's whole context, as its siblings take it")]
    pub(super) fn click_model(
        &mut self,
        document: &Document,
        picked: Picked,
        ours: Option<u32>,
        frame: Option<&DecodedFrame>,
        shift: bool,
        ctx: &egui::Context,
        order: usize,
    ) {
        use viewer_map_modes::map_mode::{MapMode, Stock};
        let units = Arc::clone(self.unit_selection.units());
        let settled = unit_pick::click(&picked.candidates, &picked.divisions, ours, &|id| units.contains(&Unit::Division(id)), picked.unordered);
        let click = match settled {
            Ok(click) => click,
            Err(why) => {
                self.selection_input.refuse(why);
                return;
            }
        };
        if matches!(self.shown_mode(), MapMode::Stock(Stock::Infrastructure | Stock::Raids)) && !(shift && click.selected) {
            let anything = !units.is_empty() || !self.air_selection.wings().is_empty() || !self.fleet_selection.fleets().is_empty();
            if shift && !click.ours && anything {
                return;
            }
            if !shift
                && (self.unit_selection.clear() | self.air_selection.clear() | self.fleet_selection.clear())
            {
                self.counter_view.selected = Arc::clone(self.unit_selection.units());
                self.counter_view.fleets_selected = Arc::clone(self.fleet_selection.fleets());
                ctx.request_repaint();
            }
            self.select_province(document, frame, click.province, ctx);
            return;
        }
        let mates: Vec<Unit> = click.mates.iter().map(|&handle| Unit::Division(handle)).collect();
        let held = HeldModel { picked: click.picked, mates, ours_unit: click.ours, shift, ours, sounds: order };
        self.click(document, super::held_clicks::Click::Model(held), frame, ctx);
    }

    pub(super) fn answer_model(&mut self, document: &Document, held: &HeldModel, frame: Option<&DecodedFrame>, ctx: &egui::Context) -> Option<()> {
        let head = super::head_rows::head(super::live_of(&self.playhead_map));
        let units = held.units(self.unit_selection.ordered_units().first().copied());
        let theatres = self.head_rows.clicks(head, frame, |frame| Theatres::read(frame, &units))?;
        let before = Arc::clone(self.unit_selection.units());
        let (mut changed, withheld) =
            self.unit_selection.model_click(Unit::Division(held.picked), &held.mates, held.ours_unit, held.shift, &|unit| theatres.of(unit));
        if let Some(why) = withheld {
            self.selection_input.refuse(why);
        }
        if !held.shift {
            changed |= self.air_selection.clear() | self.fleet_selection.clear() | self.ground_selection.take().is_some();
        }
        if changed {
            self.counter_view.selected = Arc::clone(self.unit_selection.units());
            self.counter_view.fleets_selected = Arc::clone(self.fleet_selection.fleets());
            ctx.request_repaint();
        }
        self.selection_added(document, &before, held.ours, frame, held.sounds);
        Some(())
    }

    #[expect(clippy::too_many_arguments, reason = "a click's whole context, as its siblings take it")]
    pub(super) fn select_box(
        &mut self,
        document: &Document,
        from: [f32; 2],
        to: [f32; 2],
        ours: Option<u32>,
        frame: Option<&DecodedFrame>,
        modifiers: egui::Modifiers,
        ctx: &egui::Context,
        order: usize,
    ) {
        use box_selection::{Member, Mode};
        use viewer_map_modes::map_mode::{MapMode, Stock};
        let (shift, ctrl) = (modifiers.shift, modifiers.ctrl);
        let mode = match self.shown_mode() {
            MapMode::Stock(Stock::StrategicAir) => Mode::StrategicAir,
            MapMode::Stock(Stock::StrategicNavy) => Mode::StrategicNavy,
            MapMode::Stock(Stock::Operatives) => Mode::Operatives,
            _ => Mode::Other,
        };
        if mode == Mode::Operatives {
            self.selection_input.refuse(OPERATIVES_BOX_WITHHELD);
            return;
        }
        let first = self.unit_selection.ordered_units().first().copied();
        let selected = first
            .map(|unit| Member::Unit(kind_of(unit)))
            .or_else(|| (!self.air_selection.wings().is_empty()).then_some(Member::AirWing))
            .or_else(|| (!self.fleet_selection.fleets().is_empty()).then_some(Member::Fleet));
        let report = &self.counter_report;
        let mut candidates: Vec<([f32; 4], Member, Vec<u64>, bool)> = report
            .unit_hits
            .iter()
            .enumerate()
            .map(|(index, hit)| (self.shown_rect(self.hit_layer(MapHit::Unit(index)), hit.rect), Member::Unit(hit.kind), hit.members.to_vec(), hit.ours))
            .collect();
        if mode == Mode::StrategicAir {
            candidates.extend(report.air_grid_hits.iter().filter_map(|(&index, hover)| {
                Some((self.shown_rect(self.hit_layer(MapHit::Base(index)), report.base_hits.get(index)?.rect), Member::AirWing, hover.members.as_deref()?.to_vec(), true))
            }));
        }
        let held = HeldBox {
            candidates,
            area: box_selection::rect(from, to),
            order: box_selection::order(mode, ctrl),
            shift,
            selected,
            first,
            ours,
            sounds: order,
        };
        self.click(document, super::held_clicks::Click::Box(Box::new(held)), frame, ctx);
    }

    pub(super) fn answer_box(&mut self, document: &Document, held: &HeldBox, frame: Option<&DecodedFrame>, ctx: &egui::Context) -> Option<()> {
        use box_selection::{Candidate, Member};
        let units = held.units();
        let theatres = if units.is_empty() {
            Theatres::default()
        } else {
            let head = super::head_rows::head(super::live_of(&self.playhead_map));
            self.head_rows.clicks(head, frame, |frame| Theatres::read(frame, &units))?
        };
        let shift = held.shift;
        let taken = {
            let (units, wings) = (self.unit_selection.units(), self.air_selection.wings());
            let admitted = |handle: u64| match held.selected {
                Some(Member::Unit(kind)) => units.contains(&kind.unit(handle)),
                Some(Member::AirWing) => wings.contains(&handle),
                _ => false,
            };
            let candidates: Vec<Candidate<'_>> =
                held.candidates.iter().map(|(rect, member, members, ours)| Candidate { rect: *rect, member: *member, members, ours: *ours }).collect();
            box_selection::take(
                &candidates,
                held.area,
                &held.order,
                shift,
                held.selected.map(|member| (member, &admitted as &dyn Fn(u64) -> bool)),
                held.first,
                &|unit| theatres.of(unit),
            )
        };
        if let Some(why) = taken.withheld {
            self.selection_input.refuse(why);
        }
        let before = Arc::clone(self.unit_selection.units());
        let changed = match taken.member {
            Some(Member::Unit(kind)) => {
                let mut changed = self.unit_selection.select_box(taken.members.iter().map(|&handle| kind.unit(handle)), shift);
                if !shift {
                    changed |= self.air_selection.clear() | self.fleet_selection.clear() | self.ground_selection.take().is_some();
                }
                changed
            }
            Some(Member::AirWing) => {
                self.click(document, super::held_clicks::Click::BoxAir { members: taken.members.clone(), shift }, frame, ctx);
                false
            }
            _ if !shift => {
                self.unit_selection.clear() | self.air_selection.clear() | self.fleet_selection.clear() | self.ground_selection.take().is_some()
            }
            _ => false,
        };
        if changed {
            self.counter_view.selected = Arc::clone(self.unit_selection.units());
            self.counter_view.fleets_selected = Arc::clone(self.fleet_selection.fleets());
            ctx.request_repaint();
        }
        self.selection_added(document, &before, held.ours, frame, held.sounds);
        Some(())
    }

    pub(super) fn selection_added(
        &mut self,
        document: &Document,
        before: &BTreeSet<Unit>,
        ours: Option<u32>,
        frame: Option<&DecodedFrame>,
        order: usize,
    ) {
        let added: Vec<Unit> = self
            .unit_selection
            .ordered_units()
            .iter()
            .filter(|unit| !before.contains(unit))
            .copied()
            .collect();
        self.selection_sounds(document, &added, false, ours, frame, order);
    }

    pub(super) fn map_cursor(&mut self, document: &Document, point: Option<[f32; 2]>, over_window: bool, shift: bool) -> MapCursor {
        let first = self.unit_selection.ordered_units().first().copied();
        let selected = match first {
            Some(Unit::TaskForce(_)) => Some(Selected::TaskForce),
            Some(_) => Some(Selected::Other),
            None => (!self.fleet_selection.fleets().is_empty()).then_some(Selected::Other),
        };
        let pressed = self.selection_input.press.is_some();
        let target = match point {
            None => Target::Interface,
            Some(_) if over_window => Target::Interface,
            Some(point) if self.map_hit(point).is_some() => Target::Interface,
            Some(point) if selected.is_some() && !pressed => match self.model_at(document, point, false) {
                Ok(Some(picked)) => match (unit_pick::roster_owner(&picked.candidates), self.shown_vision()) {
                    (Some(owner), Ok(vision)) => Target::Unit {
                        ours: vision.ours() == owner,
                        war: vision.at_war_proven(owner).ok(),
                        class: Kind::Division,
                    },
                    _ => Target::Unit { ours: false, war: None, class: Kind::Division },
                },
                Ok(None) if self.air_selection.wings().is_empty() && !self.selection_input.dragging() => {
                    Target::Ground(self.ground_cursor(document, point, shift))
                }
                Ok(None) => Target::Ground(MapCursor::Normal),
                Err(_) => Target::Unit { ours: false, war: None, class: Kind::Division },
            },
            Some(_) => Target::Ground(MapCursor::Normal),
        };
        resolve(CursorInputs {
            ping: self.ping_cursor(),
            air_wings: !self.air_selection.wings().is_empty(),
            dragging: self.selection_input.dragging(),
            pressed,
            selected,
            target,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Selected {
    TaskForce,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Interface,
    Unit { ours: bool, war: Option<bool>, class: Kind },
    Ground(MapCursor),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CursorInputs {
    pub ping: Option<MapCursor>,
    pub air_wings: bool,
    pub dragging: bool,
    pub pressed: bool,
    pub selected: Option<Selected>,
    pub target: Target,
}

pub(crate) fn resolve(inputs: CursorInputs) -> MapCursor {
    if inputs.target == Target::Interface {
        return MapCursor::Normal;
    }
    if let Some(ping) = inputs.ping {
        return ping;
    }
    if inputs.air_wings {
        return MapCursor::AssignAir;
    }
    if inputs.dragging {
        return MapCursor::DragSelect;
    }
    let (Some(selected), false) = (inputs.selected, inputs.pressed) else {
        return MapCursor::Normal;
    };
    match inputs.target {
        Target::Ground(cursor) => cursor,
        Target::Unit { ours: true, .. } | Target::Unit { war: None, .. } | Target::Interface => MapCursor::Normal,
        Target::Unit { war: Some(false), .. } => MapCursor::NoMove,
        Target::Unit { war: Some(true), class, .. } => match (selected, class) {
            (Selected::TaskForce, Kind::TaskForce) | (Selected::Other, Kind::Division) => MapCursor::AttackMove,
            _ => MapCursor::NoMove,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor(air_wings: bool, dragging: bool, pressed: bool, selected: Option<Selected>, target: Target) -> MapCursor {
        resolve(CursorInputs { ping: None, air_wings, dragging, pressed, selected, target })
    }

    fn unit(ours: bool, war: Option<bool>, class: Kind) -> Target {
        Target::Unit { ours, war, class }
    }

    #[test]
    fn the_cursor_follows_the_games_chain() {
        let land = Some(Selected::Other);
        let naval = Some(Selected::TaskForce);
        let enemy = unit(false, Some(true), Kind::Division);
        assert_eq!(cursor(true, true, true, land, enemy), MapCursor::AssignAir);
        assert_eq!(cursor(false, true, true, land, enemy), MapCursor::DragSelect, "a drag beats the selection");
        assert_eq!(cursor(false, false, true, land, enemy), MapCursor::Normal, "a held press shows the arrow");
        assert_eq!(cursor(false, false, false, None, enemy), MapCursor::Normal);
        assert_eq!(cursor(false, false, false, land, unit(true, Some(false), Kind::Division)), MapCursor::Normal);
        assert_eq!(cursor(false, false, false, land, unit(false, Some(false), Kind::Division)), MapCursor::NoMove);
        assert_eq!(cursor(false, false, false, land, enemy), MapCursor::AttackMove);
        assert_eq!(cursor(false, false, false, land, unit(false, Some(true), Kind::RailwayGun)), MapCursor::NoMove);
        assert_eq!(cursor(false, false, false, land, unit(false, Some(true), Kind::TaskForce)), MapCursor::NoMove);
        assert_eq!(cursor(false, false, false, naval, enemy), MapCursor::NoMove);
        assert_eq!(cursor(false, false, false, naval, unit(false, Some(true), Kind::TaskForce)), MapCursor::AttackMove);
        assert_eq!(cursor(false, false, false, land, Target::Ground(MapCursor::Normal)), MapCursor::Normal);
        assert_eq!(cursor(false, false, false, land, Target::Ground(MapCursor::ChainMove)), MapCursor::ChainMove);
        assert_eq!(cursor(false, false, true, land, Target::Ground(MapCursor::FriendlyMove)), MapCursor::Normal);
        assert_eq!(cursor(false, true, false, land, Target::Ground(MapCursor::FriendlyMove)), MapCursor::DragSelect);
        assert_eq!(cursor(true, true, false, land, Target::Interface), MapCursor::Normal);
        assert_eq!(cursor(false, false, false, land, unit(false, None, Kind::Division)), MapCursor::Normal, "unsettled");
        let pinging = |ping, target| resolve(CursorInputs { ping: Some(ping), air_wings: true, dragging: true, pressed: true, selected: land, target });
        assert_eq!(pinging(MapCursor::PingOffensive, enemy), MapCursor::PingOffensive, "ping mode beats air wings and a drag");
        assert_eq!(pinging(MapCursor::PingDefensive, Target::Ground(MapCursor::ChainMove)), MapCursor::PingDefensive);
        assert_eq!(pinging(MapCursor::PingOffensive, Target::Interface), MapCursor::Normal);
    }

    #[test]
    fn theatres_come_from_the_hours_units_and_each_gap_is_named() {
        use replay_data::rows::{DivisionRow, RailwayGunCounterInputRow, TableRows};
        use viewer_foundation::test_support::{test_frame, test_put};
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::Divisions(vec![
                DivisionRow { id: 1, theatre: Recorded::Value(Some(0)), ..Default::default() },
                DivisionRow { id: 2, theatre: Recorded::Value(Some(9)), ..Default::default() },
                DivisionRow { id: 3, theatre: Recorded::Value(None), ..Default::default() },
                DivisionRow { id: 4, theatre: Recorded::NotRecorded, ..Default::default() },
            ]),
        );
        test_put(&mut frame, TableRows::RailwayGunCounterInputs(vec![RailwayGunCounterInputRow { gun: 7, theatre: Recorded::Value(Some(9)), ..Default::default() }]));
        let units = [1, 2, 3, 4, 5].map(Unit::Division);
        let theatres = Theatres::read(&frame, &[&units[..], &[Unit::RailwayGun(7), Unit::TaskForce(8)]].concat());
        assert_eq!(theatres.of(Unit::Division(1)), Ok(0), "no theatre is its own answer");
        assert_eq!(theatres.of(Unit::Division(2)), Ok(9));
        assert_eq!(theatres.of(Unit::RailwayGun(7)), Ok(9));
        assert_eq!(theatres.of(Unit::Division(3)), Err(box_selection::THEATRE_REFUSED));
        assert_eq!(theatres.of(Unit::Division(4)), Err(box_selection::THEATRE_NOT_RECORDED));
        assert_eq!(theatres.of(Unit::Division(5)), Err(box_selection::THEATRE_ABSENT));
        assert_eq!(theatres.of(Unit::TaskForce(8)), Err(box_selection::THEATRE_UNREAD));
    }

    #[test]
    fn the_unit_order_is_the_clients_recorded_list_and_its_absence_is_named() {
        use replay_data::rows::{CountryLinkRow, LandUnitOrderRow, LocalPlayerRow, TableRows};
        use viewer_foundation::test_support::{test_frame, test_put};
        let mut frame = test_frame(0);
        test_put(&mut frame, TableRows::CountryLinks(vec![CountryLinkRow { id: 40, canonical_slot: 7 }]));
        test_put(&mut frame, TableRows::LocalPlayer(vec![LocalPlayerRow { country: 40, observer_country: 0 }]));
        test_put(&mut frame, TableRows::LandUnitOrder(vec![LandUnitOrderRow { units: Recorded::Value(Some(vec![5, 3])) }]));
        let (list, client) = recorded_unit_order(&frame);
        assert_eq!((list.as_deref(), client), (Ok(&[5, 3][..]), Some(7)));
        test_put(&mut frame, TableRows::LandUnitOrder(vec![LandUnitOrderRow { units: Recorded::Value(None) }]));
        assert_eq!(recorded_unit_order(&frame).0.unwrap_err(), unit_pick::ORDER_REFUSED);
        test_put(&mut frame, TableRows::LandUnitOrder(vec![LandUnitOrderRow { units: Recorded::NotRecorded }]));
        assert_eq!(recorded_unit_order(&frame).0.unwrap_err(), unit_pick::ORDER_NOT_RECORDED);
        let older = test_frame(0);
        assert_eq!(recorded_unit_order(&older), (Err(unit_pick::ORDER_NOT_RECORDED), None), "a recording without the table orders nothing");
    }

    #[test]
    fn sounds_play_known_effects_once_and_refuse_what_they_cannot_choose() {
        let mut input = SelectionInput::default();
        let queued = selection_sounds::Queued {
            effects: vec!["select_general", "select_army"],
            voice: Some(Ok("GER_infantry_idle".to_owned())),
            refused: vec!["a refusal".to_owned()],
        };
        let known = |effect: &str| effect != "select_general";
        let played: Vec<_> = input.sounds(1.0, queued, &known, Some(2.8), 3).into_iter().map(|sound| (sound.effect.to_string(), sound.order)).collect();
        assert_eq!(played, [("select_army".to_owned(), 3), ("GER_infantry_idle".to_owned(), 3)]);
        let again = selection_sounds::Queued {
            effects: vec!["select_army"],
            voice: Some(Err(selection_sounds::COMBAT_VOICE_WITHHELD)),
            refused: Vec::new(),
        };
        assert!(input.sounds(1.2, again, &known, Some(2.8), 4).is_empty());
        assert_eq!(input.warnings().count(), 2);
    }
}
