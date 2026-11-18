//! The map's limitation lines at the playback head, in today's order and words: each table's status
//! from the recording's block pages (no table is opened for them), an unavailable table's reason as
//! the recording words it, the tables' row notes from the values the mirrors hold, then each drawn
//! layer's own notes (made where the value is: vision's on its worker, the battle plan's with the
//! plan, the strikes', the labels' and the raid arrows' with their products), and each registered
//! layer's refusal by name. A layer hidden or refused gives no note of its own.
//!
//! A reason's words are a fact of the recording whether or not any layer declares its table: they
//! are read from the strings the table's restore and parts list, on a thread of their own, once
//! for a table and reason id, and kept while the recording's pages are held.

#[cfg(test)]
mod tests;

use super::{Entry, MapSide, details, lock, mirror::{Cell, Mirror, Mirrors}};
use crate::layers::{Layer, aircraft::Aircraft, intel::Intel, labels::Labels, orders::Orders, strikes::Strikes, trains::Trains, units::Units, vision::Vision};
use playhead::pages::Pages;
use replay_columnar::{manifest::Status, unit::reader};
use replay_data::rows::{Field, NativeValue, Opaque, Recorded, ShipCounterInputs, TableId, WeatherDisplay};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, Condvar, Mutex, Weak},
    time::{Duration, Instant},
};
use viewer_foundation::map::tables::{DISPLAY_NOT_RECORDED, DISPLAY_UNAVAILABLE, Limitations, RowFacts, Seen, Statuses, table_limitations};

/// How a line the map cannot give at the head starts: the rest names the line it stands for and why.
pub const NOT_GIVEN: &str = "Map limitations: not given at the playback head: ";

/// What the line says while a reason's words are still being read.
pub const BEING_READ: &str = "its reason is being read from the recording";

/// What a report says of a reason whose words it waited [`REPORT_WAIT`] for in vain.
const NOT_IN_TIME: &str = "its reason could not be read in time";

/// How long a report (the detail report, the proof's comparison) waits for a reason's words.
const REPORT_WAIT: Duration = Duration::from_secs(60);

/// A reason's words as read from the recording.
enum Text {
    Words(String),
    /// The recording names no text for the id where the table's strings are.
    Unheld(u32),
    Failed(String),
}

#[derive(Default)]
struct Reason {
    text: Mutex<Option<Text>>,
    read: Condvar,
}

impl Reason {
    fn set(&self, read: io::Result<Option<Vec<u8>>>, id: u32) {
        let text = match read {
            Ok(Some(bytes)) => Text::Words(String::from_utf8_lossy(&bytes).into_owned()),
            Ok(None) => Text::Unheld(id),
            Err(error) => Text::Failed(error.to_string()),
        };
        *lock(&self.text) = Some(text);
        self.read.notify_all();
    }

    /// The line's words, waiting up to `wait` for them: still unread, [`BEING_READ`] when it may
    /// not wait, [`NOT_IN_TIME`] when it waited.
    fn words(&self, wait: Duration) -> String {
        let deadline = Instant::now() + wait;
        let mut text = lock(&self.text);
        while text.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            text = self.read.wait_timeout(text, left).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
        match &*text {
            None if wait.is_zero() => BEING_READ.to_owned(),
            None => NOT_IN_TIME.to_owned(),
            Some(Text::Words(words)) => words.clone(),
            Some(Text::Unheld(id)) => format!("the recording holds no text for its reason (string {id})"),
            Some(Text::Failed(error)) => format!("its reason cannot be read from the recording ({error})"),
        }
    }
}

type Held = (Weak<dyn Pages>, HashMap<(TableId, u32), Arc<Reason>>);

/// Every reason asked of each recording's pages while they are held.
static REASONS: Mutex<Vec<Held>> = Mutex::new(Vec::new());

/// Reason `id` of `table`'s unavailable status, seen in block `block` of run `run`: the one asked
/// before, or a new one whose read has just started.
fn reason(pages: &Arc<dyn Pages>, table: TableId, id: u32, (run, block): (usize, usize)) -> Arc<Reason> {
    let mut held = lock(&REASONS);
    held.retain(|(pages, _)| pages.strong_count() > 0);
    let at = match held.iter().position(|(of, _)| std::ptr::addr_eq(of.as_ptr(), Arc::as_ptr(pages))) {
        Some(at) => at,
        None => {
            held.push((Arc::downgrade(pages), HashMap::new()));
            held.len() - 1
        }
    };
    if let Some(reason) = held[at].1.get(&(table, id)) {
        return Arc::clone(reason);
    }
    let reason = Arc::new(Reason::default());
    held[at].1.insert((table, id), Arc::clone(&reason));
    drop(held);
    let (pages, slot) = (Arc::clone(pages), Arc::clone(&reason));
    let spawned = std::thread::Builder::new().name("limitation reason".into()).spawn(move || {
        viewer_foundation::worker_priority::background_priority();
        slot.set(read(&*pages, run, block, table.index(), id), id);
    });
    if let Err(error) = spawned {
        reason.set(Err(error), id);
    }
    reason
}

/// String `id`'s bytes where `table`'s strings are at block `block` of run `run`: its last restore
/// before the block (or the run's first), then its parts since, through the block. A string's id
/// names one text in the whole recording, so the first place naming it gives it.
fn read(pages: &dyn Pages, run: usize, block: usize, table: usize, id: u32) -> io::Result<Option<Vec<u8>>> {
    let held = pages.runs().get(run).ok_or_else(|| io::Error::other(format!("no run {run}")))?;
    let kinds = pages.kinds(table).ok_or_else(|| io::Error::other(format!("table {table} is withheld")))?;
    let source = pages.extents(run);
    let block = block.min(held.pages.len().saturating_sub(1));
    let entry = |at: usize| held.pages.get(at).and_then(|page| page.tables.get(table));
    let restored = (0..block)
        .rev()
        .find_map(|at| entry(at)?.restore.as_ref().map(|restore| (at + 1, restore)))
        .or_else(|| (0..=block).rev().find_map(|at| entry(at)?.first.as_ref().map(|first| (at, first))));
    let from = match restored {
        Some((from, restore)) => {
            let (_, meta) = reader::restore(source, restore, kinds, &[])?;
            if let Some((_, text)) = meta.strings.into_iter().find(|(named, _)| *named == id) {
                return Ok(Some(text));
            }
            from
        }
        None => 0,
    };
    for page in held.pages.get(from..=block).unwrap_or_default() {
        let Some(entry) = page.tables.get(table) else { continue };
        let mut prefix = None;
        for (at, part) in entry.parts.iter().enumerate() {
            let last = entry.parts.get(at + 1).map_or(page.hours - 1, |next| next.first - 1);
            let part = reader::fetch(source, part, last, &[], prefix.as_ref())?;
            if let Some((_, text)) = part.strings().iter().find(|(named, _)| *named == id) {
                return Ok(Some(text.clone()));
            }
            prefix = Some(part.prefix());
        }
    }
    Ok(None)
}

/// What a report says of reason `id` of `table`, seen at `place` (run, block): its words, by name
/// that the recording holds none, or, after waiting [`REPORT_WAIT`], that they were not read in time.
pub(super) fn words(entry: &Entry, table: TableId, id: u32, place: (usize, usize)) -> String {
    reason(entry.pages(), table, id, place).words(REPORT_WAIT)
}

/// The statuses at the hour the map's mirrors are at.
struct Head<'a> {
    side: &'a MapSide,
    hour: replay_data::Hour,
    /// How long a reason's words are waited for: none on the frame thread; the proof waits.
    wait: Duration,
}

impl Statuses for Head<'_> {
    fn seen(&self, table: TableId) -> Seen {
        let entry = self.side.entry();
        match details::status(entry, entry.playhead().timeline(), table, self.hour) {
            Ok(Some((Status::Complete, _))) => Seen::Complete,
            Ok(Some((Status::Truncated { missing }, _))) => Seen::Truncated(missing),
            Ok(Some((Status::Unavailable { reason: id }, place))) => Seen::Unavailable(reason(entry.pages(), table, id, place).words(self.wait)),
            Ok(Some((Status::NotRecorded, _))) | Ok(None) => Seen::NotRecorded(entry.capture.get().copied()),
            Err(error) => Seen::Unavailable(format!("its status cannot be read at the head ({error})")),
        }
    }
}

/// A column of `table` by its field name.
fn column(table: TableId, field: &str) -> Option<usize> {
    (0..).map_while(|column| table.field_name(column).map(|name| (column, name))).find(|&(_, name)| name == field).map(|(column, _)| column)
}

/// The rows a mirror holds at the head: none unless its table is recorded whole or in part, or
/// when it holds no row.
fn rows_of(mirrors: &Mirrors, table: TableId) -> Option<&Mirror> {
    mirrors.get(table).filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })) && mirror.rows().next().is_some())
}

/// `table`'s rows with the columns of `fields`, when it holds rows; a line in `missing` naming
/// `line` when it holds rows but no drawn layer reads one of the columns.
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

/// An opaque cell decoded as the frame of rows decodes it.
fn decoded<T: NativeValue>(cell: Cell<&[u8]>) -> Option<T> {
    let field = match cell {
        Cell::Value(bytes) => Field::Opaque(Opaque::Encoded(bytes, replay_data::FORMAT_VERSION)),
        _ => Field::NotRecorded,
    };
    T::from_field(field).ok()
}

/// The tables' row facts from the values the mirrors hold for the drawn layers, and a line for
/// each fact whose column no drawn layer reads while its table holds rows. A table no layer reads
/// gives no row note: its status line says what is known of it.
fn facts(mirrors: &Mirrors) -> (RowFacts, Vec<String>) {
    let mut missing = Vec::new();
    let count = |(rows, [column]): (&Mirror, [usize; 1])| rows.rows().filter(|&slot| rows.raw_cell(column, slot).is_some_and(|value| value != 0)).count();
    let unreadable_naval_bases = columns_of(mirrors, TableId::NavalBases, ["countries_unreadable"], "Field Naval base access lists", &mut missing).map_or(0, count);
    let unreadable_templates = columns_of(mirrors, TableId::DivisionTemplateIcons, ["model_unreadable"], "Field Division template model inputs", &mut missing).map_or(0, count);
    let ship_counters = columns_of(mirrors, TableId::ShipModels, ["counter"], "Field ShipModels formation", &mut missing).map(|(rows, [column])| rows.rows().all(|slot| decoded::<Option<ShipCounterInputs>>(rows.list_cell(column, slot)).flatten().is_some()));
    let weather = columns_of(mirrors, TableId::WeatherRegions, ["light_rain", "heavy_rain", "snow", "blizzard", "sandstorm"], "Field weather particle presentation", &mut missing)
        .is_some_and(|(rows, columns)| rows.rows().any(|slot| columns.iter().any(|&column| rows.raw_cell(column, slot).is_some_and(|value| value != 0))));
    let mut display_missing = Vec::new();
    let weather_display = columns_of(mirrors, TableId::WeatherProvinces, ["display"], "Field WeatherProvinces.display", &mut display_missing).map(|(rows, [column])| {
        match rows.slot(0).map(|slot| decoded::<Recorded<Option<WeatherDisplay>>>(rows.list_cell(column, slot))) {
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

/// Layer `name` as `T`, while it draws.
fn drawn<'a, T: 'static>(side: &MapSide, layers: &'a [Box<dyn Layer>], name: &str) -> Option<&'a T> {
    if !side.draws(name) {
        return None;
    }
    layers.iter().find(|layer| layer.name() == name)?.any().downcast_ref::<T>()
}

/// The lines made at the head in today's order, without the layers' refusals: the tables' notes,
/// then the trains', the aircraft's, the point lights', the strikes', the viewing country's, the
/// battle plan's, the raid arrows' (`raids`, the scripts reader's), fog and sight, and the labels'.
fn made(side: &MapSide, head: &Head<'_>, raids: &[String]) -> Vec<String> {
    let Some((mirrors, layers)) = side.parts() else { return table_limitations(head, &RowFacts::default()).into_lines() };
    let (facts, missing) = facts(mirrors);
    let mut lines = table_limitations(head, &facts).into_lines();
    lines.extend(missing);
    if let Some(trains) = drawn::<Trains>(side, layers, "trains") {
        lines.extend(trains.notes(head));
    }
    if let Some(aircraft) = drawn::<Aircraft>(side, layers, "aircraft") {
        lines.extend(aircraft.notes(head));
    }
    // The entity animation and its lights are the unit models'.
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

/// The lines at the head: the tables' notes and the drawn layers' own (`raids` is the scripts
/// reader's raid arrows), then every refused layer's reason; none before the head has reached an
/// hour.
pub fn lines(side: &MapSide, raids: &[String]) -> Option<Vec<String>> {
    let hour = side.applied()?;
    let mut lines = made(side, &Head { side, hour, wait: Duration::ZERO }, raids);
    lines.extend(side.refusals().map(|(name, why)| format!("Layer {name} is refused: {why}")));
    Some(lines)
}

/// The whole list at the head against today's of the same hour, line by line by their text.
#[derive(Debug, Default)]
pub struct Compared {
    /// Lines on either side: matched, today's missing at the head, and the head's not today's.
    pub entries: u64,
    /// Lines on one side only.
    pub differing: u64,
    /// Today's lines the head does not give, and the head's lines today does not, in their order.
    pub missing: Vec<String>,
    pub extra: Vec<String>,
    /// Lines given on both sides, and how many of them stand in another place at the head.
    pub matched: u64,
    pub reordered: u64,
    /// Tables counted apart: at the head no recorded status, today one (the tables the dev unit
    /// files do not hold).
    pub apart: Vec<TableId>,
    /// The lines the map names as not given, each in place of today's line it names.
    pub not_given: Vec<String>,
    /// Today's raid arrow lines counted apart because no scripts reader gave the map's.
    pub raids_apart: usize,
    pub refused_layers: usize,
    /// Today's lines whose owner this map does not register, by owner: not compared.
    pub not_registered: BTreeMap<String, usize>,
    pub first: Option<String>,
}

/// Who gives a line of today's list at the head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The tables' status notes, read from the recording's pages whatever is drawn: always compared.
    Tables,
    /// A layer's own note: given while every one of them is registered.
    Layers(&'static [&'static str]),
    /// A note of a table's rows: given while a drawn layer reads the table, so the mirrors hold it.
    Rows(TableId),
    /// The scripts reader's raid lines, counted apart when it gave none.
    Scripts,
}

/// Today's notes by how their line starts, beside the tables' (`Table …`) and the notes that read
/// only statuses ([`status_notes`]).
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

/// The owner of a line of today's list; none for a wording this table does not know.
/// `by_status` are the lines the notes that read only statuses give at the head, with their owners.
pub fn owner(line: &str, by_status: &[(&'static [&'static str], Vec<String>)]) -> Option<Owner> {
    if let Some((owners, _)) = by_status.iter().find(|(_, lines)| lines.iter().any(|held| held == line)) {
        return Some(Owner::Layers(owners));
    }
    if let Some(&(_, owner)) = NOTES.iter().find(|(start, _)| line.starts_with(start)) {
        return Some(owner);
    }
    line.starts_with("Table ").then_some(Owner::Tables)
}

/// `today`'s lines a map compares, and by owner those it cannot give: a layer it does not
/// register (`registered`), or a table no drawn layer reads (`held`). A line of no known owner is
/// compared.
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

/// The lines of the notes that read only the tables' statuses, as `statuses` give them, by owner.
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

/// Two lists by their lines' text, each line matched once: today's lines the head lacks, the
/// head's lines today lacks, the lines matched, and how many of those stand in another place.
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

/// The whole list at the head (every line but the layers' refusals) against `today`'s lines of the
/// same hour, by their text, the order counted apart; `every` is today's frame of every table, for
/// the tables counted apart; `raids` the scripts reader's raid arrow lines, none when no reader
/// gave any. Today's lines the map cannot give (a layer it does not register, a table no drawn
/// layer reads) are counted apart by owner. A reason's words are waited for here, off the frame
/// thread.
pub fn differ(side: &MapSide, every: &replay_frame::frame::DecodedFrame, today: &[String], raids: Option<&[String]>) -> Compared {
    let Some(hour) = side.applied() else { return Compared { entries: 1, differing: 1, first: Some("the head has reached no hour".into()), ..Compared::default() } };
    let head = Head { side, hour, wait: REPORT_WAIT };
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
    let registered = |name: &str| side.names().contains(&name);
    let held = |table: TableId| side.parts().is_some_and(|(mirrors, _)| mirrors.get(table).is_some());
    let (theirs, not_registered) = by_registered(today.iter().filter(|line| !counted_apart(line)), &status_notes(&head), registered, held);
    let theirs: Vec<&String> = theirs.into_iter().filter(|line| !stood.iter().any(|prefix| line.starts_with(prefix.as_str()))).collect();
    let (missing, extra, matched, reordered) = by_text(&ours, &theirs);
    let first = missing.first().map(|line| format!("at {hour} today's line is not at the head: {line:?}")).or_else(|| extra.first().map(|line| format!("at {hour} the head's line is not today's: {line:?}")));
    let differing = (missing.len() + extra.len()) as u64;
    Compared { entries: matched + differing, differing, missing, extra, matched, reordered, apart, not_given, raids_apart, refused_layers: side.refusals().count(), not_registered, first }
}

/// The drawn layers' own notes against the same layers composed anew from the mirrors: vision's
/// (made on its worker), the strikes' and the labels'. The battle plan's and the raid arrows' are
/// made with their products from rows at the head, which a layer composed anew builds the same way;
/// the trains', aircraft's and point lights' read only statuses. Entries compared, differing, and
/// the first difference.
pub fn differ_composed(side: &MapSide) -> io::Result<(u64, u64, Option<String>)> {
    let Some((mirrors, layers)) = side.parts() else { return Ok((0, 0, None)) };
    let (mut entries, mut differing, mut first) = (0, 0, None);
    for (at, layer) in layers.iter().enumerate() {
        if !side.draws(layer.name()) {
            continue;
        }
        let upstream = crate::layers::Upstream::new(&layers[..at]);
        let (ours, fresh) = if let Some(vision) = layer.any().downcast_ref::<Vision>() {
            let mut fresh = vision.fresh();
            fresh.compose_with(mirrors, &upstream)?;
            (format!("{:?}", vision.notes()), format!("{:?}", fresh.notes()))
        } else if let Some(strikes) = layer.any().downcast_ref::<Strikes>() {
            let mut fresh = strikes.fresh();
            fresh.compose_with(mirrors, &upstream)?;
            fresh.settle_with(mirrors, &upstream, &mut crate::layers::Budget::unlimited())?;
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
