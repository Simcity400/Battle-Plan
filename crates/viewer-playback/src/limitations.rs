#[cfg(test)]
mod tests;

use crate::map_open::MapSide;
use viewer_layer_core::playhead_host::{details, mirror::{Cell, Mirror, Mirrors}};
use viewer_layer_core::{layers::Layer, vision::Vision};
use viewer_layers_forces::{aircraft::Aircraft, orders::Orders, units::Units};
use viewer_layers_infrastructure::{intel::Intel, strikes::Strikes, trains::Trains};
use viewer_layers_political::labels::Labels;
use replay_columnar::manifest::Status;
use replay_data::rows::{Field, NativeValue, Opaque, Recorded, ShipCounterInputs, TableId, WeatherDisplay};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    time::Duration,
};
use viewer_foundation::tables::{
    DISPLAY_NOT_RECORDED,
    DISPLAY_UNAVAILABLE,
    Limitations,
    RowFacts,
    Seen,
    Statuses,
    table_limitations,
};

pub const NOT_GIVEN: &str = "Map limitations: not given at the playback head: ";

struct Head<'a> {
    side: &'a MapSide,
    hour: replay_data::Hour,
    wait: Duration,
}

impl Statuses for Head<'_> {
    fn seen(&self, table: TableId) -> Seen {
        details::seen(self.side.entry(), table, self.hour, self.wait)
    }
}

fn column(table: TableId, field: &str) -> Option<usize> {
    (0..).map_while(|column| table.field_name(column).map(|name| (column, name))).find(|&(_, name)| name == field).map(|(column, _)| column)
}

fn rows_of(mirrors: &Mirrors, table: TableId) -> Option<&Mirror> {
    mirrors.get(table).filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })) && mirror.rows().next().is_some())
}

fn columns_of<'m, const N: usize>(mirrors: &'m Mirrors, table: TableId, fields: [&str; N], line: &str, missing: &mut Vec<String>) -> Option<(&'m Mirror, [usize; N])> {
    let rows = rows_of(mirrors, table)?;
    let mut columns = [0; N];
    for (at, field) in fields.into_iter().enumerate() {
        match column(table, field).filter(|&column| rows.holds(column)) {
            Some(column) => columns[at] = column,
            None => {
                missing.push(format!("{NOT_GIVEN}{line}: no drawn layer reads {table:?}.{field}"));
                return None;
            }
        }
    }
    Some((rows, columns))
}

fn decoded<T: NativeValue>(cell: Cell<&[u8]>, format: u16) -> Option<T> {
    let field = match cell {
        Cell::Value(bytes) => Field::Opaque(Opaque::Encoded(bytes, format)),
        _ => Field::NotRecorded,
    };
    T::from_field(field).ok()
}

fn facts(mirrors: &Mirrors) -> (RowFacts, Vec<String>) {
    let mut missing = Vec::new();
    let count = |(rows, [column]): (&Mirror, [usize; 1])| rows.rows().filter(|&slot| rows.raw_cell(column, slot).is_some_and(|value| value != 0)).count();
    let unreadable_naval_bases = columns_of(mirrors, TableId::NavalBases, ["countries_unreadable"], "Field Naval base access lists", &mut missing).map_or(0, count);
    let unreadable_templates = columns_of(mirrors, TableId::DivisionTemplateIcons, ["model_unreadable"], "Field Division template model inputs", &mut missing).map_or(0, count);
    let ship_counters = columns_of(mirrors, TableId::ShipModels, ["counter"], "Field ShipModels formation", &mut missing).map(|(rows, [column])| rows.rows().all(|slot| decoded::<Option<ShipCounterInputs>>(rows.list_cell(column, slot), rows.format()).flatten().is_some()));
    let weather = columns_of(mirrors, TableId::WeatherRegions, ["light_rain", "heavy_rain", "snow", "blizzard", "sandstorm"], "Field weather particle presentation", &mut missing)
        .is_some_and(|(rows, columns)| rows.rows().any(|slot| columns.iter().any(|&column| rows.raw_cell(column, slot).is_some_and(|value| value != 0))));
    let mut display_missing = Vec::new();
    let weather_display = columns_of(mirrors, TableId::WeatherProvinces, ["display"], "Field WeatherProvinces.display", &mut display_missing).map(|(rows, [column])| {
        match rows.slot(0).map(|slot| decoded::<Recorded<Option<WeatherDisplay>>>(rows.list_cell(column, slot), rows.format())) {
            Some(Some(Recorded::Value(Some(_)))) => None,
            Some(Some(Recorded::NotRecorded)) => Some(DISPLAY_NOT_RECORDED),
            _ => Some(DISPLAY_UNAVAILABLE),
        }
    });
    if !display_missing.is_empty() {
        missing.extend(display_missing);
        missing.push(format!("{NOT_GIVEN}Field Winter grading: no drawn layer reads WeatherProvinces.display, which decides whether it is stated"));
    }
    let facts = RowFacts {
        unreadable_naval_bases,
        divisions: rows_of(mirrors, TableId::Divisions).is_some(),
        task_forces: rows_of(mirrors, TableId::TaskForces).is_some(),
        unreadable_templates,
        ship_counters,
        weather,
        weather_display,
    };
    (facts, missing)
}

fn drawn<'a, T: 'static>(side: &MapSide, layers: &'a [Box<dyn Layer>], name: &str) -> Option<&'a T> {
    if !side.draws(name) {
        return None;
    }
    layers.iter().find(|layer| layer.name() == name)?.any().downcast_ref::<T>()
}

fn raid_notes(side: &MapSide) -> Option<std::sync::Arc<Vec<String>>> {
    let layers = side.parts().map(|(_, layers)| layers)?;
    let ours = drawn::<Vision>(side, layers, "vision").and_then(Vision::ours)?;
    viewer_layers_political::scripts::notes(side, ours)
}

fn made(side: &MapSide, head: &Head<'_>, raids: &[String]) -> Vec<String> {
    let Some((mirrors, layers)) = side.parts() else { return table_limitations(head, &RowFacts::default()).into_lines() };
    let (facts, missing) = facts(mirrors);
    let mut lines = table_limitations(head, &facts).into_lines();
    lines.extend(missing);
    if let Some(trains) = drawn::<Trains>(side, layers, "trains") {
        lines.extend(trains.notes(head));
    }
    if let Some(aircraft) = drawn::<Aircraft>(side, layers, "aircraft") {
        lines.extend(viewer_layers_forces::aircraft::notes(aircraft, head));
    }
    if drawn::<Units>(side, layers, "units").is_some_and(Units::drawn) {
        let mut out = Limitations::default();
        out.note_point_lights();
        lines.extend(out.into_lines());
    }
    if let Some(strikes) = drawn::<Strikes>(side, layers, "strikes") {
        lines.extend_from_slice(strikes.notes());
    }
    let vision = drawn::<Vision>(side, layers, "vision");
    let notes = vision.and_then(Vision::notes);
    if let Some(vision) = vision {
        match notes {
            Some(notes) => lines.extend_from_slice(notes.perspective()),
            None => lines.push(format!("{NOT_GIVEN}Viewing country: the vision layer refuses the hour: {}", vision.refused().unwrap_or_default())),
        }
    }
    if let Some(orders) = drawn::<Orders>(side, layers, "orders") {
        lines.extend_from_slice(orders.notes());
    }
    lines.extend_from_slice(raids);
    if let Some(notes) = notes {
        lines.extend(notes.sight(head));
        if notes.fog_drawn() {
            match drawn::<Intel>(side, layers, "intel") {
                Some(intel) => lines.extend(notes.fog_of_war(intel.map().2)),
                None => lines.push(format!("{NOT_GIVEN}Fog of war: the map cells it covers are counted by the intel layer, which is not drawn")),
            }
        }
        lines.extend_from_slice(notes.truncated());
    }
    if let Some(labels) = drawn::<Labels>(side, layers, "labels") {
        lines.extend_from_slice(labels.notes());
    }
    lines
}

pub fn lines(side: &MapSide) -> Option<Vec<String>> {
    let hour = side.applied()?;
    let raids = raid_notes(side);
    let mut lines = made(side, &Head { side, hour, wait: Duration::ZERO }, raids.as_deref().map_or(&[], Vec::as_slice));
    lines.extend(side.refusals().map(|(name, why)| format!("Layer {name} is refused: {why}")));
    Some(lines)
}

#[derive(Debug, Default)]
pub struct Compared {
    pub entries: u64,
    pub differing: u64,
    pub missing: Vec<String>,
    pub extra: Vec<String>,
    pub matched: u64,
    pub reordered: u64,
    pub apart: Vec<TableId>,
    pub not_given: Vec<String>,
    pub raids_apart: usize,
    pub refused_layers: usize,
    pub not_registered: BTreeMap<String, usize>,
    pub first: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Tables,
    Layers(&'static [&'static str]),
    Rows(TableId),
    Scripts,
}

const NOTES: &[(&str, Owner)] = &[
    ("Field Naval base access lists:", Owner::Rows(TableId::NavalBases)),
    ("Field Counter presentation:", Owner::Rows(TableId::Divisions)),
    ("Field Naval counter presentation:", Owner::Rows(TableId::TaskForces)),
    ("Field Division template model inputs:", Owner::Rows(TableId::DivisionTemplateIcons)),
    ("Field ShipModels formation:", Owner::Rows(TableId::ShipModels)),
    ("Field weather particle presentation:", Owner::Rows(TableId::WeatherRegions)),
    ("Field WeatherProvinces.display:", Owner::Rows(TableId::WeatherProvinces)),
    ("Field Winter grading:", Owner::Rows(TableId::WeatherProvinces)),
    ("Field Nuclear impact effects:", Owner::Layers(&["strikes"])),
    ("Viewing country ", Owner::Layers(&["vision"])),
    ("Fog of war on, but ", Owner::Layers(&["vision"])),
    ("Field UnitIntel (perspective):", Owner::Layers(&["vision"])),
    ("Field CountryIntel (perspective):", Owner::Layers(&["vision"])),
    ("Fog of war: ", Owner::Layers(&["vision", "intel"])),
    ("Field ArmyOrderPlans (", Owner::Layers(&["orders"])),
    ("Field Raids (", Owner::Scripts),
    ("Field ProvinceNames.name:", Owner::Layers(&["labels"])),
    ("Field Capital stars:", Owner::Layers(&["labels"])),
];

pub fn owner(line: &str, by_status: &[(&'static [&'static str], Vec<String>)]) -> Option<Owner> {
    if let Some((owners, _)) = by_status.iter().find(|(_, lines)| lines.iter().any(|held| held == line)) {
        return Some(Owner::Layers(owners));
    }
    if let Some(&(_, owner)) = NOTES.iter().find(|(start, _)| line.starts_with(start)) {
        return Some(owner);
    }
    line.starts_with("Table ").then_some(Owner::Tables)
}

fn by_registered<'t>(today: impl Iterator<Item = &'t String>, by_status: &[(&'static [&'static str], Vec<String>)], registered: impl Fn(&str) -> bool, held: impl Fn(TableId) -> bool) -> (Vec<&'t String>, BTreeMap<String, usize>) {
    let mut apart = BTreeMap::new();
    let compared = today
        .filter(|line| {
            let missing = match owner(line, by_status) {
                Some(Owner::Layers(owners)) => owners.iter().find(|owner| !registered(owner)).map(|owner| (*owner).to_owned()),
                Some(Owner::Rows(table)) => (!held(table)).then(|| format!("readers of {table:?}")),
                Some(Owner::Tables | Owner::Scripts) | None => None,
            };
            match missing {
                Some(owner) => {
                    *apart.entry(owner).or_default() += 1;
                    false
                }
                None => true,
            }
        })
        .collect();
    (compared, apart)
}

pub fn status_notes(statuses: &dyn Statuses) -> Vec<(&'static [&'static str], Vec<String>)> {
    let (mut trains, mut aircraft, mut lights, mut sight) = (Limitations::default(), Limitations::default(), Limitations::default(), Limitations::default());
    trains.note_trains(statuses);
    aircraft.note_aircraft(statuses);
    lights.note_point_lights();
    sight.note_fog_on_sight(statuses);
    const TRAINS: &[&str] = &["trains"];
    const AIRCRAFT: &[&str] = &["aircraft"];
    const UNITS: &[&str] = &["units"];
    const VISION: &[&str] = &["vision"];
    vec![(TRAINS, trains.into_lines()), (AIRCRAFT, aircraft.into_lines()), (UNITS, lights.into_lines()), (VISION, sight.into_lines())]
}

fn by_text(ours: &[String], theirs: &[&String]) -> (Vec<String>, Vec<String>, u64, u64) {
    fn count<'l>(lines: impl Iterator<Item = &'l str>) -> HashMap<&'l str, usize> {
        let mut held = HashMap::new();
        lines.for_each(|line| *held.entry(line).or_default() += 1);
        held
    }
    fn take(held: &mut HashMap<&str, usize>, line: &str) -> bool {
        held.get_mut(line).filter(|left| **left > 0).map(|left| *left -= 1).is_some()
    }
    let mut left = count(ours.iter().map(String::as_str));
    let (mut missing, mut matched_theirs) = (Vec::new(), Vec::new());
    for line in theirs {
        if take(&mut left, line) {
            matched_theirs.push(line.as_str());
        } else {
            missing.push((*line).clone());
        }
    }
    let mut wanted = count(matched_theirs.iter().copied());
    let (mut extra, mut matched_ours) = (Vec::new(), Vec::new());
    for line in ours {
        if take(&mut wanted, line) {
            matched_ours.push(line.as_str());
        } else {
            extra.push(line.clone());
        }
    }
    let reordered = matched_ours.iter().zip(&matched_theirs).filter(|(ours, theirs)| ours != theirs).count() as u64;
    (missing, extra, matched_ours.len() as u64, reordered)
}

pub fn differ(side: &MapSide, every: &replay_frame::frame::DecodedFrame, today: &[String]) -> Compared {
    let Some(hour) = side.applied() else { return Compared { entries: 1, differing: 1, first: Some("the head has reached no hour".into()), ..Compared::default() } };
    let head = Head { side, hour, wait: details::REPORT_WAIT };
    let raids = raid_notes(side);
    let raids = raids.as_deref().map(Vec::as_slice);
    let apart: Vec<TableId> = TableId::ALL.into_iter().filter(|&table| matches!(head.seen(table), Seen::NotRecorded(_)) && !matches!(every.seen(table), Seen::NotRecorded(_))).collect();
    let counted_apart = |line: &str| apart.iter().any(|table| line.starts_with(&format!("Table {table:?} ")));
    let (mut ours, mut not_given, mut stood) = (Vec::new(), Vec::new(), Vec::new());
    for line in made(side, &head, raids.unwrap_or_default()) {
        match line.strip_prefix(NOT_GIVEN) {
            Some(rest) => {
                stood.push(format!("{}:", rest.split(": ").next().unwrap_or(rest)));
                not_given.push(line);
            }
            None if !counted_apart(&line) => ours.push(line),
            None => {}
        }
    }
    if raids.is_none() {
        stood.extend(["Field Raids (arrows):".to_owned(), "Field Raids (unit):".to_owned()]);
    }
    let raids_apart = if raids.is_none() { today.iter().filter(|line| line.starts_with("Field Raids (")).count() } else { 0 };
    let layers = side.parts().map(|(_, layers)| layers).unwrap_or_default();
    let registered = |name: &str| match name {
        "units" => drawn::<Units>(side, layers, "units").is_some_and(Units::drawn),
        "trains" => drawn::<Trains>(side, layers, "trains").is_some_and(Trains::shown),
        "aircraft" => drawn::<Aircraft>(side, layers, "aircraft").is_some_and(Aircraft::drawn),
        name => side.draws(name),
    };
    let held = |table: TableId| side.parts().is_some_and(|(mirrors, _)| mirrors.get(table).is_some());
    let (theirs, not_registered) = by_registered(today.iter().filter(|line| !counted_apart(line)), &status_notes(&head), registered, held);
    let theirs: Vec<&String> = theirs.into_iter().filter(|line| !stood.iter().any(|prefix| line.starts_with(prefix.as_str()))).collect();
    let (missing, extra, matched, reordered) = by_text(&ours, &theirs);
    let first = missing.first().map(|line| format!("at {hour} today's line is not at the head: {line:?}")).or_else(|| extra.first().map(|line| format!("at {hour} the head's line is not today's: {line:?}")));
    let differing = (missing.len() + extra.len()) as u64;
    Compared { entries: matched + differing, differing, missing, extra, matched, reordered, apart, not_given, raids_apart, refused_layers: side.refusals().count(), not_registered, first }
}

pub fn differ_composed(side: &MapSide) -> io::Result<(u64, u64, Option<String>)> {
    let Some((mirrors, layers)) = side.parts() else { return Ok((0, 0, None)) };
    let (mut entries, mut differing, mut first) = (0, 0, None);
    for (at, layer) in layers.iter().enumerate() {
        if !side.draws(layer.name()) {
            continue;
        }
        let upstream = viewer_layer_core::layers::Upstream::new(&layers[..at]);
        let (ours, fresh) = if let Some(vision) = layer.any().downcast_ref::<Vision>() {
            let mut fresh = vision.fresh();
            fresh.compose_with(mirrors, &upstream)?;
            (format!("{:?}", vision.notes()), format!("{:?}", fresh.notes()))
        } else if let Some(strikes) = layer.any().downcast_ref::<Strikes>() {
            let mut fresh = strikes.fresh();
            fresh.compose_with(mirrors, &upstream)?;
            fresh.settle_with(mirrors, &upstream, viewer_layer_core::layers::Settling::Whole)?;
            (format!("{:?}", strikes.notes()), format!("{:?}", fresh.notes()))
        } else if let Some(labels) = layer.any().downcast_ref::<Labels>() {
            let mut fresh = labels.fresh();
            fresh.compose_with(mirrors, &upstream)?;
            (format!("{:?}", labels.notes()), format!("{:?}", fresh.notes()))
        } else {
            continue;
        };
        entries += 1;
        if ours != fresh {
            differing += 1;
            first.get_or_insert_with(|| format!("{}: stepped {ours}, composed anew {fresh}", layer.name()));
        }
    }
    Ok((entries, differing, first))
}
