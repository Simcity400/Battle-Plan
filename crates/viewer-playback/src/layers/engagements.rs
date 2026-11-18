//! The engaged flag of every land and naval combat at the head, for the viewing country, read
//! from the engaged index the lanes fold in the background ([`crate::engaged_index::lanes`]):
//! what today's counters take from `CombatFlags::read` with the index. An hour the index has not
//! folded, or folded for another viewer, shows every flag unknown with the reason
//! ([`Provenance`]), never a guess. The recording player's undecided naval flags take the
//! recorded bytes, as today.

use super::{Budget, Carried, Gpu, Inputs, Layer, List, Upstream, View};
use crate::playhead_host::mirror::Mirrors;
use playhead::Step;
use replay_data::{
    BranchId, Hour,
    rows::{Field, NativeValue, NavalCombatSideDetails, Opaque, TableId},
};
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_foundation::{
    engaged::Engaged,
    engagement_history::{History, LAND, NAVAL, Provenance, at},
    naval_check,
};

/// The index the layer reads and the branch its hours fold into.
pub struct Index {
    pub history: Arc<History>,
    pub branch: BranchId,
}

pub struct Engagements {
    index: Option<Index>,
    /// The viewing country's canonical slot: the vision layer's when one is registered before
    /// this one, else the one given.
    viewer: Option<u32>,
    /// Whether it declares the vision layer it reads (the window); the proof tool gives a viewer.
    reads_vision: bool,
    sides: Option<List>,
    land: BTreeMap<u64, Engaged>,
    naval: BTreeMap<u64, Engaged>,
    provenance: [Provenance; 2],
    dirty: bool,
    frame: u64,
    since: Option<u64>,
    /// Combats whose flag was read, since it was made.
    marked: usize,
    /// Combats changed since the host last cleared the list: ids of kind [`KINDS`].
    changed: super::Changed,
}

/// The [`Layer::changed`] kind of a land and a naval combat's id, by [`LAND`] and [`NAVAL`].
pub const KINDS: [&str; 2] = ["land", "naval"];

impl Engagements {
    pub fn new(index: Option<Index>, viewer: Option<u32>) -> Self {
        Self { index, viewer, reads_vision: false, sides: None, land: BTreeMap::new(), naval: BTreeMap::new(), provenance: [Provenance::Unindexed; 2], dirty: true, frame: 0, since: None, marked: 0, changed: super::Changed::default() }
    }

    /// The window's: it reads the viewing country of the vision layer registered before it.
    pub fn reading_vision(self) -> Self {
        Self { reads_vision: true, ..self }
    }

    /// The vision layer's viewing country, when one is registered before this one; whether it
    /// changed.
    fn take_viewer(&mut self, upstream: &Upstream<'_>) -> bool {
        let Some(vision) = upstream.get::<crate::layers::vision::Vision>() else { return false };
        let viewer = vision.ours();
        let changed = self.viewer != viewer;
        self.viewer = viewer;
        changed
    }

    /// Land and naval flags by combat, and where each kind's came from.
    pub fn flags(&self) -> (&BTreeMap<u64, Engaged>, &BTreeMap<u64, Engaged>, [Provenance; 2]) {
        (&self.land, &self.naval, self.provenance)
    }

    pub fn marked(&self) -> usize {
        self.marked
    }

    /// The viewing country's canonical slot it reads the flags for.
    #[cfg(test)]
    pub fn viewer(&self) -> Option<u32> {
        self.viewer
    }

    pub fn fresh(&self) -> Self {
        let index = self.index.as_ref().map(|index| Index { history: Arc::clone(&index.history), branch: index.branch });
        Self { sides: self.sides, reads_vision: self.reads_vision, ..Self::new(index, self.viewer) }
    }

    /// The flags at the head, and the combats whose flag or provenance changed, began or ended
    /// since the last read added to [`Layer::changed`].
    fn read(&mut self, mirrors: &Mirrors) {
        let (land, naval, provenance) = (std::mem::take(&mut self.land), std::mem::take(&mut self.naval), self.provenance);
        self.read_flags(mirrors);
        for (kind, (old, new)) in [(&land, &self.land), (&naval, &self.naval)].into_iter().enumerate() {
            let restated = provenance[kind] != self.provenance[kind];
            let flipped = new.iter().filter(|&(combat, flag)| restated || old.get(combat) != Some(flag)).map(|(&combat, _)| combat);
            let ended = old.keys().filter(|combat| !new.contains_key(combat)).copied();
            for combat in flipped.chain(ended) {
                self.changed.mark(KINDS[kind], combat);
            }
        }
    }

    fn read_flags(&mut self, mirrors: &Mirrors) {
        self.provenance = [Provenance::Unindexed; 2];
        let (Some(index), Some(viewer)) = (&self.index, self.viewer) else { return };
        let mut hour: Option<Hour> = None;
        for (kind, table) in [(LAND, TableId::LandCombats), (NAVAL, TableId::NavalCombats)] {
            let Some(mirror) = mirrors.get(table) else { continue };
            hour = hour.or(mirror.head());
            let Some(head) = mirror.head() else { continue };
            let combats: Vec<u64> = mirror.rows().filter_map(|slot| Some(mirror.key(slot)? as u64)).collect();
            self.marked += combats.len();
            let flags = match index.history.lookup(kind, index.branch, head, viewer, &combats) {
                Ok(flags) => {
                    self.provenance[kind] = Provenance::Indexed;
                    flags
                }
                Err(provenance) => {
                    self.provenance[kind] = provenance;
                    combats.iter().map(|&combat| (combat, Engaged::Unknown)).collect()
                }
            };
            if kind == LAND { self.land = flags } else { self.naval = flags }
        }
        let undecided = self.naval.values().any(|flag| matches!(flag, Engaged::Ambiguous(_) | Engaged::Unknown));
        let Some(hour) = hour.filter(|_| undecided) else { return };
        let player = index.history.branches.lock().unwrap_or_else(|p| p.into_inner()).get(&index.branch).and_then(|covered| at(&covered.players, hour).flatten());
        if player != Some(viewer) {
            return;
        }
        let (Some(details), Some(sides)) = (mirrors.get(TableId::NavalCombatDetails), self.sides) else { return };
        let bytes: BTreeMap<u64, [u8; 2]> = details
            .rows()
            .filter_map(|slot| {
                let combat = details.key(slot)? as u64;
                let cell = details.list(sides, slot).value()?;
                let sides = <[NavalCombatSideDetails; 2]>::from_field(Field::Opaque(Opaque::Encoded(cell, replay_data::FORMAT_VERSION))).ok()?;
                Some((combat, sides.each_ref().map(|side| side.engaged)))
            })
            .collect();
        for (combat, flag) in &mut self.naval {
            *flag = naval_check::settle(*flag, bytes.get(combat));
        }
    }

    /// The engaged index it reads, which the window's marker hovers and combat window read too.
    pub fn history(&self) -> Option<&Arc<History>> {
        self.index.as_ref().map(|index| &index.history)
    }
}

impl Layer for Engagements {
    fn name(&self) -> &'static str {
        "engagements"
    }

    fn placed(&mut self, _view: &crate::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        self.dirty = true;
        if self.reads_vision {
            inputs.after("vision");
        }
        inputs.table(TableId::LandCombats, true)?;
        inputs.table(TableId::NavalCombats, true)?;
        inputs.table(TableId::NavalCombatDetails, true)?;
        self.sides = Some(inputs.list(TableId::NavalCombatDetails, "sides")?);
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let marked = self.marked;
        self.read(mirrors);
        self.marked = marked;
        self.dirty = false;
        self.since = None;
        Ok(())
    }

    /// The flags at the head change with the hour as well as with the rows: every step marks.
    fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {
        self.dirty = true;
        self.since.get_or_insert(self.frame);
    }

    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        self.take_viewer(upstream);
        self.compose(mirrors)
    }

    /// A change of the viewing country reads every flag again.
    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, budget: &mut Budget) -> io::Result<()> {
        if self.take_viewer(upstream) {
            self.dirty = true;
            self.since.get_or_insert(self.frame);
        }
        self.settle(mirrors, budget)
    }

    fn settle(&mut self, mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        self.frame += 1;
        if self.dirty && budget.left() {
            self.read(mirrors);
            self.dirty = false;
            self.since = None;
        }
        Ok(())
    }

    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }

    fn carried(&self) -> Carried {
        Carried { entities: usize::from(self.dirty), frames: self.since.map_or(0, |since| self.frame.saturating_sub(since)) }
    }

    fn held(&self) -> usize {
        (self.land.len() + self.naval.len()) * 48
    }

    fn changed(&self) -> Option<&super::Changed> {
        Some(&self.changed)
    }

    fn clear_changed(&mut self) {
        self.changed.clear();
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
#[path = "engagements/tests.rs"]
mod tests;
