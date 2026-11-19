//! The engaged flag of every land and naval combat at the head, for the viewing country, read
//! from the engaged index the lanes fold in the background ([`crate::engaged_index::lanes`]):
//! what today's counters take from `CombatFlags::read` with the index. An hour the index has not
//! folded, or folded for another viewer, shows every flag unknown with the reason
//! ([`Provenance`]), never a guess. The recording player's undecided naval flags take the
//! recorded bytes, as today.
//!
//! A step reads the flags of the combats its hours changed and no others: the index names them
//! ([`History::flags_changed`]) and the step's own rows name the combats that began or ended.
//! Only a change of the viewing country, of the recording's player, of a read table's status or
//! of where the index answers from reads every flag again.

use super::{Carried, Settling, Inputs, Keys, Layer, List, Upstream, View};
use crate::playhead_host::mirror::{Mirror, Mirrors};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::{
    BranchId, Hour,
    rows::{Field, NativeValue, NavalCombatSideDetails, Opaque, TableId},
};
use std::{collections::BTreeMap, io, sync::Arc, time::Duration};
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

/// Per kind ([`LAND`], [`NAVAL`]) the table whose rows say which combats have a flag.
const COMBATS: [TableId; 2] = [TableId::LandCombats, TableId::NavalCombats];

/// The tables the layer reads, whose status it watches.
const READ: [TableId; 3] = [TableId::LandCombats, TableId::NavalCombats, TableId::NavalCombatDetails];

/// How long a settle that waits gives the background fold to reach the head's hour.
const WHOLE: Duration = Duration::from_secs(120);

/// One kind's flags and what it owes.
struct Kind {
    flags: BTreeMap<u64, Engaged>,
    provenance: Provenance,
    /// The hour `flags` stand at; none while the kind holds none.
    at: Option<Hour>,
    /// Every flag is read again: it has never been read, or the viewing country, a read table's
    /// status or where the index answers from changed.
    all: bool,
    owed: Keys,
}

impl Kind {
    fn new() -> Self {
        Self { flags: BTreeMap::new(), provenance: Provenance::Unindexed, at: None, all: true, owed: Keys::default() }
    }
}

pub struct Engagements {
    index: Option<Index>,
    /// The viewing country's canonical slot: the vision layer's when one is registered before
    /// this one, else the one given.
    viewer: Option<u32>,
    /// Whether it declares the vision layer it reads (the window); the proof tool gives a viewer.
    reads_vision: bool,
    sides: Option<List>,
    kinds: [Kind; 2],
    /// Whether the viewing country is the recording's own player, whose undecided naval flags
    /// take their recorded bytes.
    player_is_viewer: bool,
    /// Each read table's status as last seen.
    statuses: [Option<Status>; READ.len()],
    frame: u64,
    /// Combats whose flag was read, since it was made.
    marked: usize,
    /// Combats changed since the host last cleared the list: ids of kind [`KINDS`].
    changed: super::Changed,
}

/// The [`Layer::changed`] kind of a land and a naval combat's id, by [`LAND`] and [`NAVAL`].
pub const KINDS: [&str; 2] = ["land", "naval"];

impl Engagements {
    pub fn new(index: Option<Index>, viewer: Option<u32>) -> Self {
        Self {
            index,
            viewer,
            reads_vision: false,
            sides: None,
            kinds: [Kind::new(), Kind::new()],
            player_is_viewer: false,
            statuses: [None; READ.len()],
            frame: 0,
            marked: 0,
            changed: super::Changed::default(),
        }
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
        (&self.kinds[LAND].flags, &self.kinds[NAVAL].flags, [self.kinds[LAND].provenance, self.kinds[NAVAL].provenance])
    }

    pub fn marked(&self) -> usize {
        self.marked
    }

    /// The viewing country's canonical slot it reads the flags for.
    #[cfg(any(test, feature = "test-support"))]
    pub fn viewer(&self) -> Option<u32> {
        self.viewer
    }

    pub fn fresh(&self) -> Self {
        let index = self.index.as_ref().map(|index| Index { history: Arc::clone(&index.history), branch: index.branch });
        Self { sides: self.sides, reads_vision: self.reads_vision, ..Self::new(index, self.viewer) }
    }

    /// Every flag of both kinds read again at the next settle.
    fn read_all(&mut self) {
        for kind in &mut self.kinds {
            kind.all = true;
        }
    }

    /// The hour `kind`'s combats stand at; none in a gap or before they are read.
    fn head(&self, mirrors: &Mirrors, kind: usize) -> Option<Hour> {
        mirrors.get(COMBATS[kind])?.head()
    }

    /// The hour the recording's player is read at, as a whole read takes it: the land combats'
    /// head, or the naval combats' when there are no land combats to read.
    fn player_hour(&self, mirrors: &Mirrors) -> Option<Hour> {
        [LAND, NAVAL].into_iter().find_map(|kind| self.head(mirrors, kind))
    }

    /// `kind` holds no flag: every one it held is dropped and listed as changed.
    fn forget(&mut self, kind: usize) {
        let held: Vec<u64> = self.kinds[kind].flags.keys().copied().collect();
        for combat in held {
            self.changed.mark(KINDS[kind], combat);
        }
        if self.kinds[kind].provenance != Provenance::Unindexed {
            self.changed.everything();
        }
        self.kinds[kind] = Kind { all: false, ..Kind::new() };
    }

    /// The combats `kind` owes a read, and the hour to read them at: those the index's hours and
    /// the steps' rows name, or every live and held one when a whole read is owed.
    fn mark(&mut self, mirrors: &Mirrors, kind: usize) {
        let (Some(index), Some(viewer)) = (&self.index, self.viewer) else {
            // Without an index or a viewing country no flag is known, as a whole read leaves it.
            if !self.kinds[kind].flags.is_empty() || self.kinds[kind].all {
                self.forget(kind);
            }
            return;
        };
        let (branch, history) = (index.branch, Arc::clone(&index.history));
        let Some(head) = self.head(mirrors, kind) else {
            if !self.kinds[kind].flags.is_empty() || self.kinds[kind].all {
                self.forget(kind);
            }
            return;
        };
        let provenance = history.provenance(branch, head, viewer);
        let held = &self.kinds[kind];
        let from = held.at.filter(|_| !held.all && provenance == held.provenance);
        // A hole in the index's answer over those hours, a change of provenance or a first read
        // reads every flag again; past the fold every flag is unknown whatever the hour.
        let changed = match (from, provenance) {
            (Some(from), Provenance::Indexed) => history.flags_changed(kind, branch, from, head, viewer),
            (Some(_), _) => Some(Vec::new()),
            (None, _) => None,
        };
        let marks: Vec<u64> = match changed {
            Some(changed) => changed,
            None => {
                let live = mirrors.get(COMBATS[kind]).into_iter().flat_map(|combats| combats.rows().filter_map(|slot| combats.key(slot)).map(|key| key as u64));
                live.chain(self.kinds[kind].flags.keys().copied()).collect()
            }
        };
        let frame = self.frame;
        for combat in marks {
            self.kinds[kind].owed.mark(combat, frame);
        }
        // Where the flags come from is restated for every combat, whatever their values: the
        // layers that read them name the limitation it carries.
        if provenance != self.kinds[kind].provenance {
            self.changed.everything();
        }
        (self.kinds[kind].at, self.kinds[kind].all, self.kinds[kind].provenance) = (Some(head), false, provenance);
    }

    /// The flags of the combats `kind` owes, within `settling`: each read from the index at the hour
    /// its combats stand at, an undecided naval flag of the recording's own player settled from
    /// its recorded bytes, and a combat whose row is gone dropped.
    fn settle_kind(&mut self, mirrors: &Mirrors, kind: usize) {
        if self.kinds[kind].owed.len() == 0 {
            return;
        }
        let (Some(index), Some(viewer), Some(head)) = (&self.index, self.viewer, self.kinds[kind].at) else {
            self.kinds[kind].owed.clear();
            return;
        };
        let (branch, history) = (index.branch, Arc::clone(&index.history));
        let combats = mirrors.get(COMBATS[kind]);
        let live = |combat: u64| combats.is_some_and(|combats| combats.slot(u128::from(combat)).is_some());
        let (asked, gone): (Vec<u64>, Vec<u64>) = self.kinds[kind].owed.take().into_iter().partition(|&combat| live(combat));
        self.marked += asked.len();
        for combat in gone {
            if self.kinds[kind].flags.remove(&combat).is_some() {
                self.changed.mark(KINDS[kind], combat);
            }
        }
        let found = match history.lookup(kind, branch, head, viewer, &asked) {
            Ok(found) => found,
            Err(_) => asked.iter().map(|&combat| (combat, Engaged::Unknown)).collect(),
        };
        let bytes = self.naval_bytes(mirrors, kind, &found);
        for (combat, flag) in found {
            let flag = match &bytes {
                Some(bytes) => naval_check::settle(flag, bytes.get(&combat)),
                None => flag,
            };
            if self.kinds[kind].flags.insert(combat, flag) != Some(flag) {
                self.changed.mark(KINDS[kind], combat);
            }
        }
    }

    /// The recorded engaged bytes of the naval combats of `found` whose flag is undecided, when
    /// the viewing country is the recording's own player; none otherwise.
    fn naval_bytes(&self, mirrors: &Mirrors, kind: usize, found: &BTreeMap<u64, Engaged>) -> Option<BTreeMap<u64, [u8; 2]>> {
        if kind != NAVAL || !self.player_is_viewer {
            return None;
        }
        let undecided = found.iter().filter(|(_, flag)| matches!(flag, Engaged::Ambiguous(_) | Engaged::Unknown)).map(|(&combat, _)| combat);
        let (details, sides) = (mirrors.get(TableId::NavalCombatDetails)?, self.sides?);
        let bytes: BTreeMap<u64, [u8; 2]> = undecided
            .filter_map(|combat| {
                let slot = details.slot(u128::from(combat))?;
                let cell = details.list(sides, slot).value()?;
                let sides = <[NavalCombatSideDetails; 2]>::from_field(Field::Opaque(Opaque::Encoded(cell, replay_data::FORMAT_VERSION))).ok()?;
                Some((combat, sides.each_ref().map(|side| side.engaged)))
            })
            .collect();
        Some(bytes)
    }

    /// The engaged index it reads, which the window's marker hovers and combat window read too.
    pub fn history(&self) -> Option<&Arc<History>> {
        self.index.as_ref().map(|index| &index.history)
    }

    /// Whether the recording's player is the viewing country; a change reads every naval flag
    /// again, since it decides whether their recorded bytes are read.
    fn take_player(&mut self, mirrors: &Mirrors) {
        let player = match (&self.index, self.viewer, self.player_hour(mirrors)) {
            (Some(index), Some(viewer), Some(hour)) => {
                let branches = index.history.branches.lock().unwrap_or_else(|p| p.into_inner());
                branches.get(&index.branch).and_then(|covered| at(&covered.players, hour).flatten()) == Some(viewer)
            }
            _ => false,
        };
        if player != self.player_is_viewer {
            self.player_is_viewer = player;
            self.kinds[NAVAL].all = true;
        }
    }

    /// Each read table's status; whether any changed, which reads every flag again.
    fn take_statuses(&mut self, mirrors: &Mirrors) -> bool {
        let held = READ.map(|table| mirrors.get(table).and_then(Mirror::status));
        std::mem::replace(&mut self.statuses, held) != held
    }

    /// A settle that waits shows the flags of its own hour: the index folds in the background, so
    /// each kind's head is waited for before its flags are read, and the hour's provenance then
    /// reads every flag again. A fold that does not come in [`WHOLE`] (one cancelled at shutdown,
    /// or one the head has outrun) leaves the flags unknown and every layer that reads them names
    /// that; it never refuses the settle, which would stop the window over a background pass.
    fn wait_for_the_fold(&self, mirrors: &Mirrors, settling: Settling) {
        let (Some(index), Some(viewer)) = (&self.index, self.viewer) else { return };
        let limit = settling.waiting(WHOLE);
        for kind in [LAND, NAVAL] {
            if let Some(head) = self.head(mirrors, kind) {
                index.history.reached(index.branch, head, viewer, limit);
            }
        }
    }

    fn settle_flags(&mut self, mirrors: &Mirrors, settling: Settling) {
        if self.take_statuses(mirrors) {
            self.read_all();
        }
        if settling.waits() {
            self.wait_for_the_fold(mirrors, settling);
        }
        self.take_player(mirrors);
        for kind in [LAND, NAVAL] {
            self.mark(mirrors, kind);
            self.settle_kind(mirrors, kind);
        }
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
        self.read_all();
        if self.reads_vision {
            inputs.after("vision");
        }
        for table in READ {
            inputs.table(table, true)?;
        }
        self.sides = Some(inputs.list(TableId::NavalCombatDetails, "sides")?);
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let marked = self.marked;
        self.read_all();
        self.settle_flags(mirrors, Settling::step());
        self.marked = marked;
        Ok(())
    }

    /// Only the combats whose rows the step changed are marked: the hours it moved through name
    /// the rest at the settle, from the index.
    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        let frame = self.frame;
        for changes in step.tables() {
            let table = TableId::ALL[changes.table()];
            let kind = match table {
                TableId::LandCombats => LAND,
                TableId::NavalCombats => NAVAL,
                // A detail row's bytes settle an undecided flag of the recording's own player only.
                TableId::NavalCombatDetails if self.player_is_viewer => NAVAL,
                _ => continue,
            };
            let Some((deleted, inserted)) = changes.deleted().keys().zip(changes.inserted().keys()) else {
                self.kinds[kind].all = true;
                continue;
            };
            let mut marks: Vec<u64> = deleted.iter().chain(inserted).map(|&key| key as u64).collect();
            if table == TableId::NavalCombatDetails {
                let mirror = mirrors.get(table);
                let sides = self.sides.map_or(usize::MAX, |sides| sides.column);
                marks.extend(
                    changes
                        .columns()
                        .iter()
                        .filter(|written| written.column() == sides)
                        .flat_map(|written| written.slots())
                        .filter_map(|&slot| mirror.and_then(|mirror| mirror.key(slot)))
                        .map(|key| key as u64),
                );
            }
            for combat in marks {
                self.kinds[kind].owed.mark(combat, frame);
            }
        }
    }

    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        self.take_viewer(upstream);
        self.compose(mirrors)
    }

    /// A change of the viewing country reads every flag again.
    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, settling: Settling) -> io::Result<()> {
        if self.take_viewer(upstream) {
            self.read_all();
        }
        self.settle(mirrors, settling)
    }

    fn settle(&mut self, mirrors: &Mirrors, settling: Settling) -> io::Result<()> {
        self.frame += 1;
        self.settle_flags(mirrors, settling);
        Ok(())
    }


    /// The combats whose flag it owes, a whole read counting every one it would read, and the
    /// frames since the oldest was marked.
    fn carried(&self) -> Carried {
        let owed: usize = self.kinds.iter().map(|kind| kind.owed.len() + if kind.all { kind.flags.len().max(1) } else { 0 }).sum();
        Carried { entities: owed }
    }

    fn held(&self) -> usize {
        self.kinds.iter().map(|kind| kind.flags.len() * 48 + kind.owed.bytes()).sum()
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
