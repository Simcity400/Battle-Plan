//! Nuclear impacts from the mirrors: the impacts listed at the head, located on their provinces'
//! standstill anchors (today's per-hour product), and the impacts every step brought, in birth
//! order, to start their effects.
//!
//! NuclearStrikes is a log: an impact is appended once, in the hour it lands, and forgotten an
//! hour later, so at 24 hours a step nearly every impact is appended and forgotten inside a step.
//! Each step hands every row it appended with its cells, and every impact of every hour the head
//! passed starts its effect on the frame its step is applied.

use super::{Budget, Carried, Dirty, Gpu, Inputs, Layer, Num, View};
use crate::playhead_host::mirror::{Mirror, Mirrors, UNREAD_REASON};
use playhead::{Step, changes::Changes};
use replay_columnar::{manifest::Status, store::Value};
use replay_data::rows::TableId;
use std::{io, sync::Arc};
use viewer_world::map::strike_effects::{self, Impact, Impacts};

#[derive(Clone, Copy)]
struct Columns {
    province: Num<u32>,
    nuke_type: Num<u32>,
}

/// Where a slot's row stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placed {
    Empty,
    /// Live, with no standstill anchor or painted box.
    Unplaced,
    At(usize),
}

/// A province's standstill unit-stack anchor and admission box, where its impact effects stand.
pub type Place = Arc<dyn Fn(u32) -> Option<([f32; 3], [f32; 4])> + Send + Sync>;

/// The strikes layer's arrays and what keeps them.
pub struct Strikes {
    place: Place,
    columns: Option<Columns>,
    /// Per slot of NuclearStrikes, its impact's place in `impacts`.
    placed: Vec<Placed>,
    impacts: Vec<Option<Impact>>,
    unplaced: usize,
    /// Why the table is not whole at the head, in today's words.
    refused: Option<String>,
    /// The impacts the steps applied since the last [`Strikes::take_events`], in birth order.
    events: Vec<Impact>,
    /// The impacts of those steps that have no place.
    unplaced_events: usize,
    dirty: Dirty,
    whole: bool,
    status: Option<Option<Status>>,
    frame: u64,
    /// Rows marked, and events collected, since it was made.
    marked: [usize; 2],
    /// A compose happened: the next upload starts the head's impacts after clearing every slot.
    reset: bool,
    /// The limitation notes of the refusals at the head, and the refusal and unplaced count they
    /// were made for.
    notes: Vec<String>,
    noted: Option<(Option<String>, usize)>,
}

/// The table's rows as today reads them: none when unavailable, not recorded or in a gap.
fn rows(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })))
}

impl Strikes {
    pub fn new(place: Place) -> Self {
        Self {
            place,
            columns: None,
            placed: Vec::new(),
            impacts: Vec::new(),
            unplaced: 0,
            refused: None,
            events: Vec::new(),
            unplaced_events: 0,
            dirty: Dirty::default(),
            whole: true,
            status: None,
            frame: 0,
            marked: [0; 2],
            reset: false,
            notes: Vec::new(),
            noted: None,
        }
    }

    /// The impact refusals at the head as today's limitation lines: one per refusal.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// The notes made again when the refusals they were made for moved.
    fn note(&mut self) {
        if self.noted.as_ref().is_some_and(|(refused, unplaced)| (refused, *unplaced) == (&self.refused, self.unplaced)) {
            return;
        }
        let mut out = viewer_foundation::map::tables::Limitations::default();
        for refused in self.refused.iter().cloned().chain(strike_effects::unplaced_refusal(self.unplaced)) {
            out.note_field("Nuclear impact effects", &refused);
        }
        self.notes = out.into_lines();
        self.noted = Some((self.refused.clone(), self.unplaced));
    }

    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, ..Self::new(Arc::clone(&self.place)) }
    }

    fn place(&self) -> impl Fn(u32) -> Option<([f32; 3], [f32; 4])> + '_ {
        |province| (self.place)(province)
    }

    /// The impacts recorded at the head in recorded order, with today's refusals.
    pub fn product(&self) -> Impacts {
        let mut impacts: Vec<Impact> = self.impacts.iter().flatten().cloned().collect();
        impacts.sort_unstable_by_key(|impact| impact.id);
        let mut refused: Vec<String> = self.refused.iter().cloned().collect();
        refused.extend(strike_effects::unplaced_refusal(self.unplaced));
        Impacts { impacts, refused }
    }

    /// The impacts the steps brought since the last call, in birth order, and how many of them
    /// had no place: what the effect presentation starts.
    pub fn take_events(&mut self) -> (Vec<Impact>, usize) {
        (std::mem::take(&mut self.events), std::mem::take(&mut self.unplaced_events))
    }

    pub fn marked(&self) -> [usize; 2] {
        self.marked
    }

    /// One slot's impact from the mirror.
    fn read(&mut self, slot: u32, mirrors: &Mirrors) {
        let columns = self.columns.expect("declared before it settles");
        let at = slot as usize;
        if self.placed.len() <= at {
            self.placed.resize(at + 1, Placed::Empty);
        }
        let now = rows(mirrors.get(TableId::NuclearStrikes)).filter(|mirror| mirror.live(slot)).and_then(|mirror| {
            Some((mirror.key(slot)? as u64, mirror.number(columns.province, slot).value()?, mirror.number(columns.nuke_type, slot).value()?))
        });
        let place = now.map(|(id, province, nuke_type)| strike_effects::impact(id, province, nuke_type, self.place()));
        match std::mem::replace(&mut self.placed[at], Placed::Empty) {
            Placed::Unplaced => self.unplaced -= 1,
            Placed::At(held) => self.impacts[held] = None,
            Placed::Empty => {}
        }
        self.placed[at] = match place {
            None => Placed::Empty,
            Some(None) => {
                self.unplaced += 1;
                Placed::Unplaced
            }
            Some(Some(impact)) => {
                let held = self.impacts.iter().position(Option::is_none).unwrap_or_else(|| {
                    self.impacts.push(None);
                    self.impacts.len() - 1
                });
                self.impacts[held] = Some(impact);
                Placed::At(held)
            }
        };
    }

    /// The step's impacts in birth order: the rows it appended, by id, which increases through a
    /// recording.
    fn collect(&mut self, changes: &Changes, columns: Columns) {
        let mut born: Vec<(u64, u32, u32)> = Vec::new();
        if let Some(log) = changes.log()
            && let Some(keys) = log.keys()
        {
            let mut cells = vec![(None, None); log.len()];
            for written in log.columns() {
                let which = if written.column() == columns.province.column {
                    0
                } else if written.column() == columns.nuke_type.column {
                    1
                } else {
                    continue;
                };
                for (row, value) in written.values() {
                    if let Value::Number(bits) = value {
                        let cell = &mut cells[row as usize];
                        if which == 0 { cell.0 = Some(bits as u32) } else { cell.1 = Some(bits as u32) }
                    }
                }
            }
            born.extend(keys.iter().zip(cells).filter_map(|(&key, cell)| Some((key as u64, cell.0?, cell.1?))));
        }
        born.sort_unstable_by_key(|&(id, ..)| id);
        self.marked[1] += born.len();
        for (id, province, nuke_type) in born {
            match strike_effects::impact(id, province, nuke_type, self.place()) {
                Some(impact) => self.events.push(impact),
                None => self.unplaced_events += 1,
            }
        }
    }

    /// NuclearStrikes' change set of a step: the impacts it brought collected; a step that
    /// appended or forgot rows has the few it lists read again.
    fn apply_changes(&mut self, changes: &Changes, mirrors: &Mirrors) {
        let Some(columns) = self.columns else { return };
        let Some(mirror) = mirrors.get(TableId::NuclearStrikes) else { return };
        if let Some((status, _)) = changes.status()
            && self.status.replace(Some(status)) != Some(Some(status))
        {
            self.whole = true;
        }
        if rows(Some(mirror)).is_some() {
            self.collect(changes, columns);
        }
        if changes.log().is_some_and(|log| !log.is_empty() || !log.forgotten().is_empty()) {
            self.whole = true;
        }
    }

    pub fn differ(&self, other: &Self) -> (u64, Option<String>) {
        differ_impacts(&self.product(), &other.product())
    }
}

/// Impacts and refusals that differ between two products, the first described.
pub fn differ_impacts(a: &Impacts, b: &Impacts) -> (u64, Option<String>) {
    let mut differing = 0;
    let mut first = None;
    if a.impacts.len() != b.impacts.len() {
        differing += 1;
        first = Some(format!("{} impacts against {}", a.impacts.len(), b.impacts.len()));
    }
    for (x, y) in a.impacts.iter().zip(&b.impacts) {
        if x != y {
            differing += 1;
            first.get_or_insert_with(|| format!("{x:?} against {y:?}"));
        }
    }
    if a.refused != b.refused {
        differing += 1;
        first.get_or_insert_with(|| format!("refusals: {:?} against {:?}", a.refused, b.refused));
    }
    (differing, first)
}

impl Layer for Strikes {
    fn name(&self) -> &'static str {
        "strikes"
    }

    fn placed(&mut self, _view: &crate::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        self.whole = true;
        inputs.table(TableId::NuclearStrikes, true)?;
        self.columns = Some(Columns { province: inputs.number(TableId::NuclearStrikes, "province")?, nuke_type: inputs.number(TableId::NuclearStrikes, "nuke_type")? });
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let marked = self.marked;
        self.whole = true;
        self.events.clear();
        self.unplaced_events = 0;
        self.reset = true;
        self.settle(mirrors, &mut Budget::unlimited())?;
        self.marked = marked;
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        if self.columns.is_none() {
            return;
        }
        if step.gap() {
            self.whole = true;
            return;
        }
        if let Some(changes) = step.table(TableId::NuclearStrikes.index()) {
            self.apply_changes(changes, mirrors);
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        self.frame += 1;
        if self.columns.is_none() {
            return Ok(());
        }
        if std::mem::take(&mut self.whole) {
            let mirror = mirrors.get(TableId::NuclearStrikes);
            self.status = Some(mirror.and_then(Mirror::status));
            let held = (0..self.placed.len() as u32).filter(|&slot| self.placed[slot as usize] != Placed::Empty);
            let slots: Vec<u32> = held.chain(rows(mirror).into_iter().flat_map(Mirror::rows)).collect();
            for slot in slots {
                self.dirty.mark(slot, self.frame);
            }
        }
        let strikes = mirrors.get(TableId::NuclearStrikes);
        self.refused = match strikes.and_then(Mirror::status) {
            Some(Status::Complete) => None,
            Some(Status::Truncated { missing }) => Some(strike_effects::truncated(missing)),
            Some(Status::Unavailable { .. }) => Some(strike_effects::unavailable(strikes.and_then(Mirror::reason).unwrap_or(UNREAD_REASON).to_owned())),
            Some(Status::NotRecorded) | None => Some("NuclearStrikes not recorded at this hour: impacts have no effect".to_owned()),
        };
        let mut dirty = std::mem::take(&mut self.dirty);
        let settled = dirty.settle(budget, |slot| {
            self.read(slot, mirrors);
            Ok(())
        });
        self.dirty = dirty;
        self.note();
        settled
    }

    /// At a reset the head's impacts start with every slot cleared, as today's presentation of a
    /// sought hour; then each step's impacts, in birth order.
    fn upload(&mut self, gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        use viewer_foundation::map::presentation::Continuity;
        if std::mem::take(&mut self.reset) {
            gpu.scene.set_nuclear_impacts([&self.product()], Continuity::Reset);
        }
        let (impacts, _) = self.take_events();
        if !impacts.is_empty() {
            gpu.scene.set_nuclear_impacts([&Impacts { impacts, refused: Vec::new() }], Continuity::Continue);
        }
        Ok(())
    }

    fn carried(&self) -> Carried {
        Carried { entities: self.dirty.len() + usize::from(self.whole && self.columns.is_some()), frames: self.dirty.since().map_or(0, |since| self.frame.saturating_sub(since)) }
    }

    fn held(&self) -> usize {
        self.placed.capacity() * size_of::<Placed>() + (self.impacts.capacity() + self.events.capacity()) * size_of::<Option<Impact>>() + self.dirty.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
#[path = "strikes/tests.rs"]
mod tests;
