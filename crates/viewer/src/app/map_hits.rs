
use super::{MapHit, unit_family};
use std::collections::HashMap;
use viewer_counter_kit::CounterReport;
use viewer_native_gui::{anchored::Shifts, free_icons};
use viewer_gpu::icon_tracks::IconFamily;

pub(crate) struct Shapes<'a> {
    pub report: &'a CounterReport,
    pub raids: &'a [viewer_raids::map::raids::tooltips::Hit],
    pub occupation: &'a [viewer_map_icons::occupation_icons::Hit],
    pub adjacency: &'a [viewer_map_icons::adjacency_icons::Hit],
    pub looks: Option<&'a free_icons::Looks>,
    pub shifts: Option<&'a Shifts>,
}

fn shift(shifts: Option<&Shifts>, family: IconFamily, layer: u64) -> [f32; 2] {
    shifts
        .and_then(|shifts| shifts.of(family, layer))
        .map_or([0.0; 2], |shift| shift.map(|value| value as f32))
}

pub(crate) fn local(
    shifts: Option<&Shifts>,
    looks: Option<&free_icons::Looks>,
    family: IconFamily,
    layer: u64,
    point: [f32; 2],
) -> Option<([f32; 2], u32)> {
    let shift = shift(shifts, family, layer);
    let point = [0, 1].map(|axis| point[axis] - shift[axis]);
    match looks {
        None => Some((point, 0)),
        Some(looks) => looks.hit(family, layer, point),
    }
}

pub(crate) fn shown(
    shifts: Option<&Shifts>,
    looks: Option<&free_icons::Looks>,
    family: IconFamily,
    layer: u64,
    bounds: [f32; 4],
) -> Option<[f32; 4]> {
    let shift = shift(shifts, family, layer);
    let bounds = [bounds[0] + shift[0], bounds[1] + shift[1], bounds[2], bounds[3]];
    let Some(looks) = looks else {
        return Some(bounds);
    };
    let present = looks.of(family, layer).filter(|present| present.hittable())?;
    let scaled = |axis: usize| present.pivot[axis] + (bounds[axis] - present.pivot[axis]) * present.scale;
    Some([
        scaled(0),
        scaled(1),
        bounds[2] * present.scale,
        bounds[3] * present.scale,
    ])
}

pub(crate) fn raid_at<'a>(
    shapes: &Shapes<'a>,
    point: [f32; 2],
) -> Option<&'a viewer_raids::map::raids::tooltips::Hit> {
    shapes
        .raids
        .iter()
        .filter_map(|hit| {
            let (local, rank) = seen(shapes, IconFamily::Raids, hit.layer, point)?;
            hit.contains(local).then_some(((rank, hit.layer), hit))
        })
        .max_by_key(|(key, _)| *key)
        .map(|(_, hit)| hit)
}

#[derive(Clone, Copy)]
enum Shape {
    Pin(usize),
    Base(usize),
    AirMission(usize),
    Occupation(usize),
    NavalMission(usize),
    NavalRegion(usize),
    Raid(usize),
    Unit(usize),
    TaskForce(usize),
    TaskForcePort(usize),
    NavalMarker(usize),
    Combat(usize),
}

struct Entry {
    bounds: [f32; 4],
    family: IconFamily,
    layer: u64,
    pass: u8,
    order: usize,
    shape: Shape,
}

const SIDE: usize = 24;

const WIDE: usize = SIDE / 3;

pub(crate) struct Index {
    entries: Vec<Entry>,
    cells: Vec<Vec<u32>>,
    at: [f32; 2],
    cell: [f32; 2],
    everywhere: Vec<u32>,
    after_raids: u32,
    adjacency: Vec<(u32, u64)>,
}

fn seen(
    shapes: &Shapes<'_>,
    family: IconFamily,
    layer: u64,
    point: [f32; 2],
) -> Option<([f32; 2], u32)> {
    local(shapes.shifts, shapes.looks, family, layer, point)
}

fn on_screen(
    shapes: &Shapes<'_>,
    family: IconFamily,
    layer: u64,
    bounds: [f32; 4],
) -> Option<[f32; 4]> {
    shown(shapes.shifts, shapes.looks, family, layer, bounds)
}

fn geometry_bounds(hit: Option<&viewer_native_gui::native_gui::HitGeometry>, rect: [f32; 4]) -> Option<[f32; 4]> {
    match hit {
        None => Some(rect),
        Some(hit) => hit.bounds(),
    }
}

impl Index {
    pub(crate) fn of(shapes: &Shapes<'_>) -> Self {
        let report = shapes.report;
        let mut entries: Vec<Entry> = Vec::new();
        let push = |entries: &mut Vec<Entry>, family, layer, pass, order, shape, bounds: Option<[f32; 4]>| {
            entries.push(Entry {
                bounds: bounds
                    .and_then(|bounds| on_screen(shapes, family, layer, bounds))
                    .unwrap_or(NOWHERE),
                family,
                layer,
                pass,
                order,
                shape,
            });
        };
        for (index, hit) in report.faction_pin_hovers.iter().enumerate() {
            push(&mut entries, IconFamily::Pins, hit.layer, 0, index, Shape::Pin(index), Some(hit.rect));
        }
        for (index, hit) in report.base_hits.iter().enumerate() {
            let grid = report.air_grid_hits.get(&index);
            let family = grid.map_or(IconFamily::Bases, |hit| hit.family);
            let pass = if grid.is_some() { 8 } else { 1 };
            push(&mut entries, family, hit.layer, pass, index, Shape::Base(index), Some(hit.rect));
        }
        for (index, hit) in report.air_mission_hits.iter().enumerate() {
            push(&mut entries, IconFamily::Missions, hit.layer, 2, index, Shape::AirMission(index), Some(hit.rect));
        }
        for (index, hit) in shapes.occupation.iter().enumerate() {
            push(&mut entries, IconFamily::Occupation, hit.layer, 3, index, Shape::Occupation(index), hit.bounds());
        }
        for (index, hit) in report.naval_mission_hits.iter().enumerate() {
            push(&mut entries, IconFamily::Missions, hit.layer, 2, index, Shape::NavalMission(index), Some(hit.rect));
        }
        for (index, hit) in report.naval_region_hovers.iter().enumerate() {
            push(&mut entries, IconFamily::Missions, hit.layer, 2, index, Shape::NavalRegion(index), geometry_bounds(hit.hit.as_ref(), hit.rect));
        }
        for (index, hit) in shapes.raids.iter().enumerate() {
            let bounds = [hit.origin[0], hit.origin[1], hit.size[0], hit.size[1]];
            push(&mut entries, IconFamily::Raids, hit.layer, 4, 0, Shape::Raid(index), Some(bounds));
        }
        let after_raids = entries.len() as u32;
        for (index, hit) in report.unit_hits.iter().enumerate() {
            let family = unit_family(report, index, hit);
            let pass = if hit.port { 1 } else { 5 };
            push(&mut entries, family, hit.layer, pass, index, Shape::Unit(index), Some(hit.rect));
        }
        for (index, hover) in report.task_force_hovers.iter().enumerate() {
            let Some(hit) = report.unit_hits.get(hover.unit_hit).filter(|hit| hover.matches(hit)) else {
                continue;
            };
            let family = unit_family(report, hover.unit_hit, hit);
            push(&mut entries, family, hit.layer, 5, hover.unit_hit, Shape::TaskForce(index), geometry_bounds(hover.hit.as_ref(), hover.rect));
        }
        for (index, hover) in report.task_force_port_hovers.iter().enumerate() {
            let Some(hit) = report.unit_hits.get(hover.unit_hit).filter(|hit| hover.matches(hit)) else {
                continue;
            };
            let family = unit_family(report, hover.unit_hit, hit);
            push(&mut entries, family, hit.layer, 1, hover.unit_hit, Shape::TaskForcePort(index), geometry_bounds(hover.hit.as_ref(), hover.rect));
        }
        for (index, hit) in report.naval_marker_hovers.iter().enumerate() {
            push(&mut entries, IconFamily::Combats, hit.layer, 7, hit.order, Shape::NavalMarker(index), Some(hit.rect));
        }
        for (index, hit) in report.combat_hits.iter().enumerate() {
            let pass = if matches!(hit.selection, viewer_engagements::combat_selection::Selection::Land(_)) { 6 } else { 7 };
            push(&mut entries, IconFamily::Combats, hit.layer, pass, hit.order, Shape::Combat(index), Some(hit.rect));
        }
        let adjacency = shapes
            .adjacency
            .iter()
            .enumerate()
            .map(|(index, hit)| (index as u32, hit.layer))
            .collect();
        let mut index = Self {
            entries,
            cells: Vec::new(),
            at: [0.0; 2],
            cell: [1.0; 2],
            everywhere: Vec::new(),
            after_raids,
            adjacency,
        };
        index.bucket();
        index
    }

    fn bucket(&mut self) {
        self.cells = vec![Vec::new(); SIDE * SIDE];
        let placed: Vec<[f32; 4]> = self
            .entries
            .iter()
            .map(|entry| entry.bounds)
            .filter(|bounds| bounds[2] > 0.0 && bounds[3] > 0.0 && bounds[0].is_finite() && bounds[1].is_finite())
            .collect();
        let (mut low, mut high) = ([f32::MAX; 2], [f32::MIN; 2]);
        for bounds in &placed {
            for axis in 0..2 {
                low[axis] = low[axis].min(bounds[axis]);
                high[axis] = high[axis].max(bounds[axis] + bounds[axis + 2]);
            }
        }
        if placed.is_empty() {
            self.everywhere = (0..self.entries.len() as u32).collect();
            return;
        }
        self.at = low;
        self.cell = [0, 1].map(|axis| ((high[axis] - low[axis]) / SIDE as f32).max(f32::MIN_POSITIVE));
        for (at, entry) in self.entries.iter().enumerate() {
            let at = at as u32;
            let bounds = entry.bounds;
            if !(bounds[2] > 0.0 && bounds[3] > 0.0) || !bounds[0].is_finite() || !bounds[1].is_finite() {
                self.everywhere.push(at);
                continue;
            }
            let span = |axis: usize| {
                let first = ((bounds[axis] - self.at[axis]) / self.cell[axis]).floor();
                let last = ((bounds[axis] + bounds[axis + 2] - self.at[axis]) / self.cell[axis]).floor();
                let clamp = |value: f32| value.clamp(0.0, SIDE as f32 - 1.0) as usize;
                clamp(first)..=clamp(last)
            };
            let (columns, rows) = (span(0), span(1));
            if columns.end() - columns.start() >= WIDE || rows.end() - rows.start() >= WIDE {
                self.everywhere.push(at);
                continue;
            }
            for row in rows {
                for column in columns.clone() {
                    self.cells[row * SIDE + column].push(at);
                }
            }
        }
    }

    fn cell_at(&self, point: [f32; 2]) -> Option<usize> {
        let at = |axis: usize| {
            let cell = ((point[axis] - self.at[axis]) / self.cell[axis]).floor();
            (cell >= 0.0 && cell < SIDE as f32).then_some(cell as usize)
        };
        Some(at(1)? * SIDE + at(0)?)
    }

    pub(crate) fn top(
        &self,
        point: [f32; 2],
        shapes: &Shapes<'_>,
        intel: Option<&viewer_raids::map::intel_icons::IntelHit>,
    ) -> Option<MapHit> {
        let mut candidates: Vec<u32> = self
            .cell_at(point)
            .map(|cell| self.cells[cell].as_slice())
            .unwrap_or_default()
            .iter()
            .copied()
            .chain(self.everywhere.iter().copied())
            .collect();
        candidates.sort_unstable();
        self.offered(point, shapes, intel, candidates)
    }

    #[cfg(test)]
    fn top_scanned(
        &self,
        point: [f32; 2],
        shapes: &Shapes<'_>,
        intel: Option<&viewer_raids::map::intel_icons::IntelHit>,
    ) -> Option<MapHit> {
        self.offered(point, shapes, intel, (0..self.entries.len() as u32).collect())
    }

    fn offered(
        &self,
        point: [f32; 2],
        shapes: &Shapes<'_>,
        intel: Option<&viewer_raids::map::intel_icons::IntelHit>,
        candidates: Vec<u32>,
    ) -> Option<MapHit> {
        let mut top: Option<((u32, u64, u8, usize), MapHit)> = None;
        let mut offer = |rank: u32, layer: u64, pass: u8, order: usize, target: MapHit| {
            let key = (rank, layer, pass, order);
            if top.as_ref().is_none_or(|(previous, _)| key >= *previous) {
                top = Some((key, target));
            }
        };
        let mut offered_intel = false;
        let intel_offer = |offer: &mut dyn FnMut(u32, u64, u8, usize, MapHit)| {
            if let Some(hit) = intel
                && let Some((point, rank)) = seen(shapes, IconFamily::Intel, hit.layer, point)
                && hit.contains(point)
            {
                offer(rank, hit.layer, 4, 1, MapHit::Intel);
            }
        };
        for at in candidates {
            if at >= self.after_raids && !offered_intel {
                offered_intel = true;
                intel_offer(&mut offer);
            }
            let entry = &self.entries[at as usize];
            let Some((local, rank)) = seen(shapes, entry.family, entry.layer, point) else {
                continue;
            };
            let Some(target) = entry.target(shapes, local) else {
                continue;
            };
            offer(rank, entry.layer, entry.pass, entry.order, target);
        }
        if !offered_intel {
            intel_offer(&mut offer);
        }
        let top = top.map(|(_, target)| target);
        if top == Some(MapHit::Blocked) {
            return top;
        }
        let mut key = top
            .and_then(|hit| hit_layer(shapes, hit))
            .and_then(|(family, layer)| seen(shapes, family, layer, point).map(|(_, rank)| (rank, layer)));
        let mut top = top;
        for &(index, layer) in &self.adjacency {
            let Some(hit) = shapes.adjacency.get(index as usize) else { continue };
            if let Some((local, rank)) = seen(shapes, IconFamily::AdjacencyRules, layer, point)
                && hit.geometry.contains(local)
                && key.is_none_or(|key| (rank, layer) >= key)
            {
                key = Some((rank, layer));
                top = Some(MapHit::Adjacency(index as usize));
            }
        }
        top
    }
}

const NOWHERE: [f32; 4] = [f32::NAN, f32::NAN, 0.0, 0.0];

impl Entry {
    fn target(&self, shapes: &Shapes<'_>, local: [f32; 2]) -> Option<MapHit> {
        use viewer_native_gui::counter_hovers::task_force_port::Control;
        let report = shapes.report;
        match self.shape {
            Shape::Pin(index) => report.faction_pin_hovers.get(index)?.contains(local).then_some(MapHit::Pin(index)),
            Shape::Base(index) => report.base_hits.get(index)?.contains(local).then_some(MapHit::Base(index)),
            Shape::AirMission(index) => report.air_mission_hits.get(index)?.contains(local).then_some(MapHit::AirMission(index)),
            Shape::Occupation(index) => shapes.occupation.get(index)?.contains(local).then_some(MapHit::Occupation(index)),
            Shape::NavalMission(index) => report.naval_mission_hits.get(index)?.contains(local).then_some(MapHit::NavalMission(index)),
            Shape::NavalRegion(index) => {
                let hit = report.naval_region_hovers.get(index)?;
                hit.contains(local).then(|| if hit.supported { MapHit::NavalRegionHover(index) } else { MapHit::Blocked })
            }
            Shape::Raid(index) => {
                let hit = shapes.raids.get(index)?;
                hit.contains(local).then(|| if hit.supported { MapHit::Raid } else { MapHit::Blocked })
            }
            Shape::Unit(index) => report.unit_hits.get(index)?.contains(local).then_some(MapHit::Unit(index)),
            Shape::TaskForce(index) => {
                let hover = report.task_force_hovers.get(index)?;
                hover.contains(local).then(|| if hover.supported { MapHit::Unit(hover.unit_hit) } else { MapHit::Blocked })
            }
            Shape::TaskForcePort(index) => {
                let hover = report.task_force_port_hovers.get(index)?;
                hover.contains(local).then(|| {
                    if !hover.supported || !hover.has_tooltip() {
                        MapHit::Blocked
                    } else if hover.control == Control::Unbound {
                        MapHit::PortTooltip(index)
                    } else {
                        MapHit::Unit(hover.unit_hit)
                    }
                })
            }
            Shape::NavalMarker(index) => {
                let hit = report.naval_marker_hovers.get(index)?;
                hit.contains(local).then(|| if hit.supported { MapHit::NavalObserver(index) } else { MapHit::Blocked })
            }
            Shape::Combat(index) => {
                let hit = report.combat_hits.get(index)?;
                hit.contains(local).then(|| if hit.supported { MapHit::Combat(index) } else { MapHit::Blocked })
            }
        }
    }
}

pub(crate) fn hit_layer(shapes: &Shapes<'_>, target: MapHit) -> Option<(IconFamily, u64)> {
    let report = shapes.report;
    match target {
        MapHit::Adjacency(index) => shapes.adjacency.get(index).map(|hit| (IconFamily::AdjacencyRules, hit.layer)),
        MapHit::AirMission(index) => report.air_mission_hits.get(index).map(|hit| (IconFamily::Missions, hit.layer)),
        MapHit::NavalMission(index) => report.naval_mission_hits.get(index).map(|hit| (IconFamily::Missions, hit.layer)),
        MapHit::NavalRegionHover(index) => report.naval_region_hovers.get(index).map(|hit| (IconFamily::Missions, hit.layer)),
        MapHit::Raid | MapHit::Intel => None,
        MapHit::Occupation(index) => shapes.occupation.get(index).map(|hit| (IconFamily::Occupation, hit.layer)),
        MapHit::Combat(index) => report.combat_hits.get(index).map(|hit| (IconFamily::Combats, hit.layer)),
        MapHit::NavalObserver(index) => report.naval_marker_hovers.get(index).map(|hit| (IconFamily::Combats, hit.layer)),
        MapHit::PortTooltip(index) => report.task_force_port_hovers.get(index).and_then(|hover| {
            report.unit_hits.get(hover.unit_hit).map(|hit| (unit_family(report, hover.unit_hit, hit), hit.layer))
        }),
        MapHit::Unit(index) => report.unit_hits.get(index).map(|hit| (unit_family(report, index, hit), hit.layer)),
        MapHit::Pin(index) => report.faction_pin_hovers.get(index).map(|hit| (IconFamily::Pins, hit.layer)),
        MapHit::Base(index) => report.base_hits.get(index).map(|hit| {
            (report.air_grid_hits.get(&index).map_or(IconFamily::Bases, |grid| grid.family), hit.layer)
        }),
        MapHit::Blocked => None,
    }
}

#[derive(Default)]
pub(crate) struct Resolved {
    stamp: Option<u64>,
    index: Option<Index>,
    points: HashMap<[u32; 2], Option<MapHit>>,
    counted: [u64; 2],
}

impl Resolved {
    pub(crate) fn at<'a>(
        &mut self,
        stamp: u64,
        point: [f32; 2],
        shapes: &Shapes<'a>,
        intel: impl FnOnce([f32; 2]) -> Option<&'a viewer_raids::map::intel_icons::IntelHit>,
    ) -> Option<MapHit> {
        if self.stamp != Some(stamp) {
            self.note();
            self.stamp = Some(stamp);
            self.index = Some(Index::of(shapes));
            self.points.clear();
        }
        let key = [point[0].to_bits(), point[1].to_bits()];
        self.counted[0] += 1;
        if let Some(&held) = self.points.get(&key) {
            self.counted[1] += 1;
            return held;
        }
        let index = self.index.as_ref().expect("made above");
        let top = index.top(point, shapes, intel(point));
        self.points.insert(key, top);
        top
    }

    fn note(&mut self) {
        let [asked, kept] = std::mem::take(&mut self.counted);
        if asked > 0 {
            viewer_runtime::redraws::note("pointer.asked", asked);
            viewer_runtime::redraws::note("pointer.resolved", asked - kept);
        }
    }

    pub(crate) fn end_frame(&mut self) {
        self.note();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use viewer_engagements::combat_selection;
    use viewer_native_gui::{counter_hovers::{task_force, unit_hits}, sprites::layer};

    fn shapes() -> (CounterReport, Vec<viewer_raids::map::raids::tooltips::Hit>) {
        let mut report = CounterReport::default();
        for step in 0..40u32 {
            let at = |scale: f32| [f32::from(step as u16) * scale, f32::from(step as u16) * scale * 0.5];
            let [x, y] = at(61.0);
            report.faction_pin_hovers.push(viewer_counter_kit::PinHover {
                rect: [x, y, 24.0, 24.0],
                theatre: (0, 0),
                regions: Arc::from([step]),
                layer: layer(5, step, 0),
            });
            report.base_hits.push(unit_hits::Blocker { rect: [x - 8.0, y - 8.0, 40.0, 40.0], layer: layer(3, step, 0) });
            report.unit_hits.push(unit_hits::Hit {
                ours: step % 2 == 0,
                port: step % 3 == 0,
                rect: [x, y, 66.0, 90.0],
                kind: unit_hits::Kind::TaskForce,
                members: Arc::from([u64::from(step)]),
                layer: layer(10, step, 0),
                plate: Some([x + 4.0, y + 4.0, 20.0, 20.0]),
            });
            report.task_force_hovers.push(task_force::Hover {
                control: task_force::Control::Carrier,
                unit_hit: step as usize,
                rect: [x - 16.0, y, 16.0, 16.0],
                hit: None,
                authored: None,
                supported: step % 4 != 0,
                ours: true,
                coarse: false,
                hidden: false,
                members: Arc::from([u64::from(step)]),
            });
            report.combat_hits.push(combat_selection::Hit {
                selection: combat_selection::Selection::Land(u64::from(step)),
                rect: [x, y + 20.0, 32.0, 32.0],
                layer: layer(20, step, 0),
                order: step as usize,
                supported: step % 5 != 0,
            });
            report.naval_marker_hovers.push(combat_selection::Hit {
                selection: combat_selection::Selection::Land(u64::from(step)),
                rect: [x + 10.0, y, 32.0, 32.0],
                layer: layer(20, step, 0),
                order: step as usize,
                supported: true,
            });
            report.air_mission_hits.push(viewer_counter_kit::AirMissionHover {
                region: step,
                part: viewer_counter_kit::AirMissionPart::Supply,
                rect: [x, y - 30.0, 50.0, 50.0],
                layer: layer(8, step, 0),
                text: Arc::default(),
            });
            report.naval_mission_hits.push(viewer_counter_kit::NavalMissionHit {
                region: step,
                mission: step,
                rect: [x + 2.0, y - 28.0, 50.0, 50.0],
                layer: layer(8, step, 0),
                text: None,
            });
        }
        report.base_hits.push(unit_hits::Blocker { rect: [0.0, 0.0, 2560.0, 1440.0], layer: layer(1, 0, 0) });
        let raids = (0..8u32)
            .map(|step| viewer_raids::map::raids::tooltips::Hit {
                origin: [f32::from(step as u16) * 140.0, 300.0],
                size: [48.0, 48.0],
                key: viewer_raids::map::raids::tooltips::Key::Raid { id: step, reference_type: 0, incoming: false },
                province: 0,
                layer: layer(15, step, 0),
                supported: step % 2 == 0,
            })
            .collect();
        (report, raids)
    }

    #[test]
    fn the_cell_index_answers_what_a_scan_of_every_hit_shape_answers() {
        let (report, raids) = shapes();
        let shapes = Shapes { report: &report, raids: &raids, occupation: &[], adjacency: &[], looks: None, shifts: None };
        let index = Index::of(&shapes);
        let mut hit = 0;
        for y in 0..72 {
            for x in 0..128 {
                let point = [x as f32 * 20.0 + 0.5, y as f32 * 20.0 + 0.5];
                let scanned = index.top_scanned(point, &shapes, None);
                assert_eq!(index.top(point, &shapes, None), scanned, "{point:?}");
                hit += usize::from(scanned.is_some());
            }
        }
        assert!(hit > 128, "the sampled points must reach the shapes: {hit}");
    }

    #[test]
    fn the_hit_shapes_move_with_the_windows_a_moved_camera_offset() {
        use viewer_native_gui::anchored::Shifts;
        let (report, raids) = shapes();
        let by = [40, -25];
        let families = [
            IconFamily::Pins, IconFamily::Bases, IconFamily::Raids, IconFamily::Labels,
            IconFamily::Columns, IconFamily::Combats, IconFamily::Missions, IconFamily::Occupation,
            IconFamily::Intel, IconFamily::Resources, IconFamily::AdjacencyRules,
        ];
        let layers: Vec<u64> = report
            .unit_hits
            .iter()
            .map(|hit| hit.layer)
            .chain(report.base_hits.iter().map(|hit| hit.layer))
            .chain(report.combat_hits.iter().map(|hit| hit.layer))
            .chain(report.naval_marker_hovers.iter().map(|hit| hit.layer))
            .chain(report.air_mission_hits.iter().map(|hit| hit.layer))
            .chain(report.naval_mission_hits.iter().map(|hit| hit.layer))
            .chain(report.faction_pin_hovers.iter().map(|hit| hit.layer))
            .chain(report.naval_region_hovers.iter().map(|hit| hit.layer))
            .chain(raids.iter().map(|hit| hit.layer))
            .collect();
        let icons: Vec<(IconFamily, u64, [i32; 2])> = layers
            .iter()
            .flat_map(|layer| families.map(|family| (family, *layer, by)))
            .collect();
        let shifts = Shifts::of_icons(icons);
        let standing = Shapes { report: &report, raids: &raids, occupation: &[], adjacency: &[], looks: None, shifts: None };
        let moved = Shapes { report: &report, raids: &raids, occupation: &[], adjacency: &[], looks: None, shifts: Some(&shifts) };
        let (standing_index, moved_index) = (Index::of(&standing), Index::of(&moved));
        let (mut hit, mut changed) = (0, 0);
        for y in 0..72 {
            for x in 0..128 {
                let point = [x as f32 * 20.0 + 0.5, y as f32 * 20.0 + 0.5];
                let there = [point[0] + by[0] as f32, point[1] + by[1] as f32];
                let was = standing_index.top(point, &standing, None);
                assert_eq!(moved_index.top(there, &moved, None), was, "{point:?} moved to {there:?}");
                hit += usize::from(was.is_some());
                changed += usize::from(moved_index.top(point, &moved, None) != was);
            }
        }
        assert!(hit > 128, "the sampled points must reach the shapes: {hit}");
        assert!(changed > 64, "the icons must leave where they were laid out: {changed} points changed");
    }

    #[test]
    fn the_cell_index_answers_a_scan_for_a_free_eye_s_scaled_icons() {
        use viewer_native_gui::free_icons::{Looks, Present};
        let (report, raids) = shapes();
        let mut looks = Looks::default();
        for (rank, hit) in report.unit_hits.iter().enumerate() {
            looks.show(IconFamily::Bases, hit.layer, Present { pivot: [hit.rect[0], hit.rect[1]], scale: 0.6, alpha: 1.0, rank: rank as u32 });
            looks.show(IconFamily::Columns, hit.layer, Present { pivot: [hit.rect[0], hit.rect[1]], scale: 1.4, alpha: 1.0, rank: rank as u32 });
        }
        for (rank, hit) in report.base_hits.iter().enumerate() {
            let alpha = if rank % 2 == 0 { 1.0 } else { 0.25 };
            looks.show(IconFamily::Bases, hit.layer, Present { pivot: [300.0, 200.0], scale: 0.8, alpha, rank: rank as u32 });
        }
        for (rank, hit) in report.combat_hits.iter().enumerate() {
            looks.show(IconFamily::Combats, hit.layer, Present { pivot: [hit.rect[0], hit.rect[1]], scale: 2.0, alpha: 1.0, rank: rank as u32 });
        }
        let shapes = Shapes { report: &report, raids: &raids, occupation: &[], adjacency: &[], looks: Some(&looks), shifts: None };
        let index = Index::of(&shapes);
        let mut hit = 0;
        for y in 0..72 {
            for x in 0..128 {
                let point = [x as f32 * 20.0 + 0.5, y as f32 * 20.0 + 0.5];
                let scanned = index.top_scanned(point, &shapes, None);
                assert_eq!(index.top(point, &shapes, None), scanned, "{point:?}");
                hit += usize::from(scanned.is_some());
            }
        }
        assert!(hit > 32, "the sampled points must reach the shown icons: {hit}");
    }

    #[test]
    fn a_frame_resolves_each_point_once_and_answers_the_same_as_its_index() {
        let (report, raids) = shapes();
        let shapes = Shapes { report: &report, raids: &raids, occupation: &[], adjacency: &[], looks: None, shifts: None };
        let index = Index::of(&shapes);
        let mut resolved = Resolved::default();
        let points = [[5.5, 5.5], [310.5, 155.5], [1500.5, 700.5], [5.5, 5.5], [310.5, 155.5]];
        for point in points {
            assert_eq!(resolved.at(7, point, &shapes, |_| None), index.top(point, &shapes, None), "{point:?}");
        }
        assert_eq!(resolved.counted, [points.len() as u64, 2], "the two points asked twice were resolved once");
        let other = Shapes { report: &report, raids: &[], occupation: &[], adjacency: &[], looks: None, shifts: None };
        assert_eq!(resolved.at(8, points[0], &other, |_| None), Index::of(&other).top(points[0], &other, None));
        assert_eq!(resolved.counted, [1, 0]);
    }

    #[test]
    fn every_consumer_of_a_hit_reads_the_point_in_the_layout_its_window_stands_in() {
        use viewer_native_gui::{anchored::Shifts, native_gui::HitGeometry};
        use viewer_raids::map::intel_icons::{self, IntelHit};
        let (panned, tracked) = ([40, -25], [-18, 33]);
        let mut report = CounterReport::default();
        let counter = layer(10, 0, 0);
        let members: Arc<[u64]> = Arc::from([7u64]);
        report.unit_hits.push(unit_hits::Hit {
            ours: true,
            port: false,
            rect: [600.0, 400.0, 20.0, 28.0],
            kind: unit_hits::Kind::Division,
            members: Arc::clone(&members),
            layer: counter,
            plate: None,
        });
        report.land_hovers.push(viewer_native_gui::counter_hovers::land::CounterHover {
            control: viewer_native_gui::counter_hovers::land::CounterControl::Normal,
            authored: None,
            rect: [600.0, 400.0, 20.0, 12.0],
            remaining: None,
            ours: true,
            coarse: false,
            hidden: false,
            railway_gun: false,
            members,
            timing: Arc::from([]),
        });
        let raid = viewer_raids::map::raids::tooltips::Hit {
            origin: [900.0, 500.0],
            size: [48.0, 48.0],
            key: viewer_raids::map::raids::tooltips::Key::Raid { id: 3, reference_type: 0, incoming: false },
            province: 0,
            layer: layer(15, 0, 0),
            supported: true,
        };
        let intel = layer(4, 1, 0);
        let intel_hits = [IntelHit {
            shapes: vec![HitGeometry::test_rectangle([1200, 300], [20, 20], [2560, 1440])],
            layer: intel,
            tooltip: Arc::default(),
        }];
        let shifts = Shifts::of_icons([
            (unit_family(&report, 0, &report.unit_hits[0]), counter, panned),
            (IconFamily::Raids, raid.layer, panned),
            (IconFamily::Intel, intel, tracked),
        ]);
        let raids = [raid];
        let shapes = Shapes { report: &report, raids: &raids, occupation: &[], adjacency: &[], looks: None, shifts: Some(&shifts) };
        let family = unit_family(&report, 0, &report.unit_hits[0]);
        let moved = |at: [f32; 2], by: [i32; 2]| [0, 1].map(|axis| at[axis] + by[axis] as f32);
        let point = |rect: [f32; 4], by: [i32; 2]| moved([rect[0] + rect[2] * 0.5, rect[1] + 2.0], by);

        let centre = point(report.unit_hits[0].rect, panned);
        assert_eq!(Index::of(&shapes).top(centre, &shapes, None), Some(MapHit::Unit(0)));
        let (inside, _) = local(Some(&shifts), None, family, counter, centre).expect("the map camera shows every icon");
        assert!(report.land_hovers[0].contains(inside), "the child hover under the drawn counter takes the point");
        assert!(!report.land_hovers[0].contains(centre), "the laid-out shape does not hold the drawn point: the offset must be removed");

        let over_raid = moved([raids[0].origin[0] + 4.0, raids[0].origin[1] + 4.0], panned);
        assert_eq!(Index::of(&shapes).top(over_raid, &shapes, None), Some(MapHit::Raid));
        assert!(raid_at(&shapes, over_raid).is_some(), "the raid the index gave is found again");
        assert!(!raids[0].contains(over_raid), "the laid-out raid shape does not hold the drawn point");

        let over_intel = moved([1205.0, 305.0], tracked);
        let seen = |hit: &IntelHit| local(Some(&shifts), None, IconFamily::Intel, hit.layer, over_intel);
        assert!(intel_icons::hit_seen(&intel_hits, seen).is_some(), "the intel entry under its drawn place takes the point");
        assert!(intel_icons::hit_at(&intel_hits, over_intel).is_none(), "the laid-out entry does not hold the drawn point");
        assert!(intel_icons::hit_seen(&intel_hits, |hit| local(Some(&shifts), None, IconFamily::Intel, hit.layer, moved(over_intel, panned))).is_none(), "another window's offset is not this icon's");

        assert_eq!(
            shown(Some(&shifts), None, family, counter, report.unit_hits[0].rect),
            Some([640.0, 375.0, 20.0, 28.0]),
        );
    }
}
